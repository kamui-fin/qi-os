// Don't forget TLB flush upon Proc <-> Proc context switch
// Spawn instead of fork

use core::arch::naked_asm;
use core::num;
use core::ptr::from_ref;
use core::sync::atomic::{AtomicU64, AtomicUsize};

use alloc::boxed::Box;
use alloc::vec;
use elf::abi::PT_LOAD;
use elf::endian::LittleEndian;
use elf::ElfBytes;
use x86_64::structures::paging::page::PageRangeInclusive;
use x86_64::structures::paging::page_table::PageTableEntry;
use x86_64::structures::paging::{FrameAllocator, Mapper, Size4KiB};
use x86_64::PhysAddr;
use x86_64::{
    structures::paging::{
        frame::PhysFrameRangeInclusive, OffsetPageTable, Page, PageTable, PageTableFlags,
        PhysFrame, Size2MiB,
    },
    VirtAddr,
};

use crate::BOOT_INFO;
use crate::{memory::BootInfoFrameAllocator, thread::ThreadControlBlock};

pub static ECHO_ELF: &[u8] = include_bytes!(env!("CARGO_BIN_FILE_USERLAND_echo"));

pub static PID: AtomicU64 = AtomicU64::new(1 << 16);

pub struct ProcessControlBlock {
    pub tcb: ThreadControlBlock,
    pub page_table: Box<PageTable>,
}

const USER_STACK_SIZE: usize = 64 * 1024;

struct ProgramMapRegion<'a> {
    start: Page<Size4KiB>,
    num_pages: usize,
    flags: u32,
    code: &'a [u8],
}

impl ProcessControlBlock {
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
            "mov rax, 0x00007FFFFFFF0000",
            "push 0x6<<3|0b011", // ss
            "push rax",          // rsp
            "push 1<<9|1<<1",    // rflags
            "push 0x5<<3|0b011", // cs
            "push r13",          // rip
            "iretq",
        );
    }

    pub fn new() -> Self {
        // Paging, start with kernel mapped
        let mut boot_info = BOOT_INFO.get().expect("Boot info not initialized").lock();
        let mut page_table = Box::new(boot_info.page_table.level_4_table().clone());
        for i in 0..256 {
            page_table[i] = PageTableEntry::new();
        }
        let mut mapper = unsafe {
            OffsetPageTable::new(
                &mut page_table,
                VirtAddr::new(boot_info.physical_memory_offset),
            )
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

        // Map wherever the instructions are in memory
        // ELF LOADER

        const PAGE_SIZE: u64 = 4096;

        let file = ElfBytes::<LittleEndian>::minimal_parse(ECHO_ELF).unwrap();
        let program_start = VirtAddr::new(file.ehdr.e_entry);
        let segs = file.segments().unwrap();
        for seg in segs {
            if seg.p_type == PT_LOAD {
                // TODO: set flags
                let flags = seg.p_flags;
                let start_offset = seg.p_vaddr % PAGE_SIZE;
                let start_page = Page::<Size4KiB>::containing_address(VirtAddr::new(seg.p_vaddr));
                let num_pages = (seg.p_memsz + start_offset).div_ceil(PAGE_SIZE) as usize;

                let code = file.segment_data(&seg).unwrap();
                for (i, page) in
                    Page::range(start_page, start_page + (num_pages as u64)).enumerate()
                {
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
                        // DOUBLE CHECK IF THIS LOGIC IS RIGHT

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
                        let mapper_flush = mapper
                            .map_to(
                                page,
                                frame,
                                PageTableFlags::WRITABLE
                                    | PageTableFlags::PRESENT
                                    | PageTableFlags::USER_ACCESSIBLE,
                                &mut boot_info.allocator,
                            )
                            .expect("(fixed offset mapping): unable to map frame");
                        mapper_flush.ignore();
                    };
                }
            }
        }

        let cr3 = &*page_table as *const _ as u64;
        let cr3 = cr3 - boot_info.physical_memory_offset;

        let tcb = ThreadControlBlock::new(
            PID.fetch_add(1u64, core::sync::atomic::Ordering::Relaxed),
            Self::user_process_hook as *const (),
            Some(cr3 as *const usize),
            Some(program_start.as_u64()),
        );

        Self { tcb, page_table }
    }
}
