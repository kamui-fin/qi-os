// Don't forget TLB flush upon Proc <-> Proc context switch
// Spawn instead of fork

use core::arch::naked_asm;
use core::ffi::{c_str, CStr};
use core::num;
use core::ptr::from_ref;
use core::sync::atomic::{AtomicU64, AtomicUsize};

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use elf::abi::PT_LOAD;
use elf::endian::LittleEndian;
use elf::ElfBytes;
use spin::Mutex;
use x86_64::structures::paging::mapper::MapToError;
use x86_64::structures::paging::page::PageRangeInclusive;
use x86_64::structures::paging::page_table::PageTableEntry;
use x86_64::structures::paging::{FrameAllocator, Mapper, Size4KiB, Translate};
use x86_64::PhysAddr;
use x86_64::{
    structures::paging::{
        frame::PhysFrameRangeInclusive, OffsetPageTable, Page, PageTable, PageTableFlags,
        PhysFrame, Size2MiB,
    },
    VirtAddr,
};

use crate::{memory::BootInfoFrameAllocator, thread::ThreadControlBlock};
use crate::{serial_println, BOOT_INFO};

pub static ECHO_ELF: &[u8] = include_bytes!(env!("CARGO_BIN_FILE_USERLAND_echo"));

pub static PID: AtomicU64 = AtomicU64::new(1 << 16);

pub struct ProcessControlBlock<'a> {
    pub pid: u64,
    pub tcb: Arc<Mutex<ThreadControlBlock>>,
    pub page_table: &'a mut PageTable,
    pub argv: Vec<&'a str>,
    pub heap_start: VirtAddr,
    pub heap_end: VirtAddr,
    pub name: &'static str,
}

const USER_STACK_SIZE: usize = 64 * 1024;

impl<'a> ProcessControlBlock<'a> {
    /*
    Function given curr ESP, reload new SS:ESP
    EIP store on old stack, new EIP popped off new stack when function returns

    An interrupt generated while the processor is in ring 3 will switch the stack to the resulting permission level stack entry in the TSS. During a software context switch the values for SS0:ESP0 (and possibly SS1:ESP1 or SS2:ESP2) will need to be set in the TSS.
    If the processor is operating in Long Mode, the stack selectors are no longer present and the RSP0-2 fields are used to provide the destination stack address.

    Whenever a system call occurs, the CPU gets the SS0 and ESP0-value in its TSS and assigns the stack-pointer to it. So one or more kernel-stacks need to be set up for processes doing system calls. Be aware that a thread's/process' time-slice may end during a system call, passing control to another thread/process which may as well perform a system call, ending up in the same stack. Solutions are to create a private kernel-stack for each thread/process and re-assign esp0 at any task-switch or to disable scheduling during a system-call

    Set up a barebones TSS with an ESP0 stack.
    When an interrupt (be it fault, IRQ, or software interrupt) happens while the CPU is in user mode, the CPU needs to know where the kernel stack is located. This location is stored in the ESP0 (0 for ring 0) entry of the TSS.
    Set up an IDT entry for ring 3 system call interrupts
    */

    // swtch.s changes kernel stacks
    // we need a new user-process hook, that swtch returns to (ring 0)
    // pushes 5 magic values into KERNEL stack, then executes iretq
    //
    #[unsafe(naked)]
    pub unsafe extern "C" fn user_process_hook() {
        naked_asm!(
            "push 0x6<<3|0b011", // ss
            "push r14",          // rsp
            "push 1<<9|1<<1",    // rflags
            "push 0x5<<3|0b011", // cs
            "push r13",          // rip
            "iretq",
        );
    }

    // TODO: handle memory leaks with these manual allocations upon proc die
    pub fn from_bytes(
        program_code: &[u8],
        c_argv: *const *const core::ffi::c_char,
        argc: usize,
        program: &'static CStr,
    ) -> Self {
        // Paging, start with kernel mapped
        let mut boot_info = BOOT_INFO.get().expect("Boot info not initialized").lock();

        let l4_table = boot_info.allocator.allocate_frame().unwrap();
        let cr3 = l4_table.start_address().as_u64();

        let l4_virt = VirtAddr::new(cr3 + boot_info.physical_memory_offset);
        let page_table: &mut PageTable = unsafe { &mut *l4_virt.as_mut_ptr() };

        let active_l4 = boot_info.page_table.level_4_table();
        for i in 0..512 {
            if i < 256 {
                page_table[i] = PageTableEntry::new();
            } else {
                page_table[i] = active_l4[i].clone();
            }
        }
        let mut mapper = unsafe {
            OffsetPageTable::new(page_table, VirtAddr::new(boot_info.physical_memory_offset))
        };
        // Then map the User Stack: High up at e.g. 0x0000_7FFF_FFFF_0000 (USER BIT SET)
        let stack_top = VirtAddr::new(0x0000_7FFF_FFFF_0000);
        let stack_pages = USER_STACK_SIZE / 4096;

        for i in 0..stack_pages {
            let page = Page::<Size4KiB>::containing_address(stack_top - (i + 1) * 4096);
            let frame = boot_info
                .allocator
                .allocate_frame()
                .expect("proc_init: out of mem");
            unsafe {
                let _ = mapper
                    .map_to(
                        page,
                        frame,
                        PageTableFlags::WRITABLE
                            | PageTableFlags::PRESENT
                            | PageTableFlags::USER_ACCESSIBLE,
                        &mut boot_info.allocator,
                    )
                    .expect("(fixed offset mapping): unable to map frame");
            }
        }

        // ---------------------------------

        let mut arg_ptrs = Vec::new();

        // copy program_name
        let program_name_len = program.count_bytes() + 1;
        let mut rsp = stack_top - program_name_len;
        let program_name_ptr = (mapper.translate_addr(rsp).unwrap().as_u64()
            + boot_info.physical_memory_offset) as *mut u8;
        arg_ptrs.push(rsp);
        unsafe {
            core::ptr::copy_nonoverlapping(
                program.as_ptr() as *const u8,
                program_name_ptr,
                program_name_len,
            );
        }

        // copy arg raw char data
        for i in 0..argc {
            let c_string = unsafe { CStr::from_ptr(*c_argv.add(i)) };
            let arglen = c_string.count_bytes() + 1;
            rsp -= arglen;
            let arg_ptr = (mapper.translate_addr(rsp).unwrap().as_u64()
                + boot_info.physical_memory_offset) as *mut u8;
            arg_ptrs.push(rsp);
            unsafe {
                core::ptr::copy_nonoverlapping(c_string.as_ptr() as *const u8, arg_ptr, arglen);
            }
        }

        // align rsp to 16 bytes
        rsp = rsp.align_down(16u64);

        // copy NULL
        rsp -= core::mem::size_of::<usize>();
        let null_ptr = (mapper.translate_addr(rsp).unwrap().as_u64()
            + boot_info.physical_memory_offset) as *mut usize;
        unsafe {
            core::ptr::write(null_ptr, 0usize);
        }

        // create pointers
        for i in (0..(argc + 1)).rev() {
            rsp -= core::mem::size_of::<usize>();
            let arg_ptr = arg_ptrs[i].as_u64() as usize;
            let dst = (mapper.translate_addr(rsp).unwrap().as_u64()
                + boot_info.physical_memory_offset) as *mut usize;
            unsafe {
                core::ptr::write(dst, arg_ptr);
            }
        }

        // copy argc
        rsp -= core::mem::size_of::<usize>();
        let argc_ptr = (mapper.translate_addr(rsp).unwrap().as_u64()
            + boot_info.physical_memory_offset) as *mut usize;
        unsafe {
            core::ptr::write(argc_ptr, argc + 1);
        }

        // ----------------------------------------

        let mut argv: Vec<&str> = Vec::with_capacity(argc);
        for i in 0..argc {
            unsafe {
                let ptr = *c_argv.add(i);
                let c_string = CStr::from_ptr(ptr);
                argv.push(c_string.to_str().unwrap());
            }
        }

        // Map wherever the instructions are in memory
        // ELF LOADER

        const PAGE_SIZE: u64 = 4096;

        let file = ElfBytes::<LittleEndian>::minimal_parse(program_code).unwrap();
        let program_start = VirtAddr::new(file.ehdr.e_entry);
        let mut program_end = program_start;
        let segs = file.segments().unwrap();
        for seg in segs {
            if seg.p_type == PT_LOAD {
                // TODO: set flags
                // let flags = seg.p_flags;

                // What is the offset WITHIN the page
                let start_offset = seg.p_vaddr % PAGE_SIZE;

                // Which page do we start with?
                let start_page = Page::<Size4KiB>::containing_address(VirtAddr::new(seg.p_vaddr));
                let num_pages = (seg.p_memsz + start_offset).div_ceil(PAGE_SIZE) as usize;

                program_end += seg.p_memsz;

                let code = file.segment_data(&seg).unwrap();
                for (i, page) in
                    Page::range(start_page, start_page + (num_pages as u64)).enumerate()
                {
                    if let Ok(frame) = mapper.translate_page(page) {
                        let frame_ptr: *mut u8 = VirtAddr::new(
                            frame.start_address().as_u64() + boot_info.physical_memory_offset,
                        )
                        .as_mut_ptr();
                        unsafe {
                            let offset_within_frame =
                                if i == 0 { start_offset } else { 0 } as usize;
                            let offset_within_code =
                                (i * 4096).saturating_sub(start_offset as usize) as u64;
                            let remaining_file_bytes =
                                seg.p_filesz.saturating_sub(offset_within_code);

                            let bytes_to_copy = core::cmp::min(
                                4096 - (offset_within_frame as u64),
                                seg.p_memsz - offset_within_code,
                            ) as usize;

                            let bytes_from_file =
                                core::cmp::min(bytes_to_copy, remaining_file_bytes as usize);

                            if bytes_from_file > 0 {
                                core::ptr::copy_nonoverlapping(
                                    code.as_ptr().add(offset_within_code as usize),
                                    frame_ptr.add(offset_within_frame),
                                    bytes_from_file,
                                );
                            }
                        }
                        continue;
                    }

                    let frame = boot_info
                        .allocator
                        .allocate_frame()
                        .expect("proc_init: out of mem");

                    let frame_ptr: *mut u8 = VirtAddr::new(
                        frame.start_address().as_u64() + boot_info.physical_memory_offset,
                    )
                    .as_mut_ptr();

                    // copy over
                    unsafe {
                        // memset 0 first
                        core::ptr::write_bytes(frame_ptr, 0, 4096);

                        // what part of code do we load into this frame??

                        let offset_within_frame = if i == 0 { start_offset } else { 0 } as usize;
                        let offset_within_code =
                            (i * 4096).saturating_sub(start_offset as usize) as u64;
                        let remaining_file_bytes = seg.p_filesz.saturating_sub(offset_within_code);

                        let bytes_to_copy = core::cmp::min(
                            4096 - (offset_within_frame as u64),
                            seg.p_memsz - offset_within_code,
                        ) as usize;

                        let bytes_from_file =
                            core::cmp::min(bytes_to_copy, remaining_file_bytes as usize);

                        if bytes_from_file > 0 {
                            core::ptr::copy_nonoverlapping(
                                code.as_ptr().add(offset_within_code as usize),
                                frame_ptr.add(offset_within_frame),
                                bytes_from_file,
                            );
                        }
                    }

                    unsafe {
                        let mapper_flush = mapper.map_to(
                            page,
                            frame,
                            PageTableFlags::WRITABLE
                                | PageTableFlags::PRESENT
                                | PageTableFlags::USER_ACCESSIBLE,
                            &mut boot_info.allocator,
                        );
                        if let Ok(mapper_flush) = mapper_flush {
                            mapper_flush.ignore();
                        } else if let Err(MapToError::PageAlreadyMapped(_)) = mapper_flush {
                            continue;
                        } else {
                            panic!("unable to map page");
                        }
                    };
                }
            }
        }

        let heap_start = (program_end + 1usize).align_up(4096u64);

        serial_println!("Jumping to {:x}", program_start.as_u64());

        let pid = PID.fetch_add(1u64, core::sync::atomic::Ordering::Relaxed);
        let tcb = ThreadControlBlock::new(
            pid,
            Self::user_process_hook as *const (),
            Some(cr3 as *const usize),
            Some(program_start.as_u64()),
            Some(rsp.as_u64()),
        );

        let tcb = Arc::new(Mutex::new(tcb));

        Self {
            pid,
            tcb,
            page_table,
            argv,
            heap_start,
            heap_end: heap_start,
            name: program.to_str().unwrap(),
        }
    }
}
