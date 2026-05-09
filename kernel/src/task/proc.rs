use core::arch::naked_asm;
use core::ffi::{c_str, CStr};
use core::num;
use core::ptr::from_ref;
use core::sync::atomic::{AtomicU64, AtomicUsize};

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use elf::abi::PT_LOAD;
use elf::endian::LittleEndian;
use elf::ElfBytes;
use slab::Slab;
use spin::{Mutex, RwLock};
use x86_64::structures::paging::mapper::MapToError;
use x86_64::structures::paging::page::PageRangeInclusive;
use x86_64::structures::paging::page_table::PageTableEntry;
use x86_64::structures::paging::{FrameAllocator, Mapper, PageTableIndex, Size4KiB, Translate};
use x86_64::PhysAddr;
use x86_64::{
    structures::paging::{
        frame::PhysFrameRangeInclusive, OffsetPageTable, Page, PageTable, PageTableFlags,
        PhysFrame, Size2MiB,
    },
    VirtAddr,
};

use crate::driver::serial;
use crate::fs::vfs::{find_dentry, get_root_dentry, DEntry, File, OpenFlags};
use crate::task::thread::{CURR_THREAD_PTR, SCHEDULER};
use crate::{mem::memory::BumpAllocator, task::thread::ThreadControlBlock};
use crate::{serial_println, BOOT_INFO, PROC};

pub static SHELL_ELF: &[u8] = include_bytes!(env!("CARGO_BIN_FILE_USERLAND_shell"));
pub static XIANGQI_ELF: &[u8] = include_bytes!(env!("CARGO_BIN_FILE_USERLAND_xiangqi"));

const USER_STACK_SIZE: usize = 64 * 1024;
pub const MAX_FD: usize = 100;

pub static PID: AtomicU64 = AtomicU64::new(1 << 16);

pub struct AddressSpace {
    pub cr3: u64,
    pub heap_start: VirtAddr,
    pub heap_end: VirtAddr,
    program_start: u64,
    pub backbuffer_frames: Option<Vec<PhysFrame>>,
    program_end: u64,
    stack_top: VirtAddr,
}

impl AddressSpace {
    fn next_table<'a>(entry: &PageTableEntry) -> &'a PageTable {
        let boot_info = BOOT_INFO.get().expect("Boot info not initialized").lock();
        let page_table_ptr =
            VirtAddr::new(entry.addr().as_u64() + boot_info.physical_memory_offset).as_ptr();
        let page_table: &PageTable = unsafe { &*page_table_ptr };
        page_table
    }

    pub fn clone_for_fork(&self) -> Self {
        let (cr3, mut mapper) = Self::new_page_table();

        let mut new_frames = Vec::new();

        if let Some(frames) = &self.backbuffer_frames {
            for _ in 0..frames.len() {
                let mut boot_info = BOOT_INFO.get().expect("Boot info not initialized").lock();
                let new_frame = boot_info.allocator.allocate_frame().unwrap();
                new_frames.push(new_frame);
            }
        }
        let backbuffer_frames = if new_frames.is_empty() {
            None
        } else {
            Some(new_frames)
        };

        for (i4, entry) in self.get_page_table().level_4_table().iter().enumerate() {
            if i4 >= 256 {
                mapper.level_4_table()[i4] = entry.clone();
            } else {
                for (i3, entry) in Self::next_table(entry).iter().enumerate() {
                    for (i2, entry) in Self::next_table(entry).iter().enumerate() {
                        for (i1, entry) in Self::next_table(entry).iter().enumerate() {
                            if !entry.is_unused() {
                                let mut boot_info =
                                    BOOT_INFO.get().expect("Boot info not initialized").lock();
                                let og_frame = entry.frame().unwrap();
                                let new_frame = boot_info.allocator.allocate_frame().unwrap();
                                // copy original frame data into this new frame
                                copy_frame_data(og_frame, new_frame);
                                unsafe {
                                    mapper.map_to(
                                        Page::from_page_table_indices(
                                            PageTableIndex::new(i4 as u16),
                                            PageTableIndex::new(i3 as u16),
                                            PageTableIndex::new(i2 as u16),
                                            PageTableIndex::new(i1 as u16),
                                        ),
                                        new_frame,
                                        entry.flags(),
                                        &mut boot_info.allocator,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        Self {
            cr3,
            backbuffer_frames,
            ..*self
        }
    }

    pub fn from_elf(program_code: &[u8]) -> Self {
        let (cr3, mut mapper) = Self::new_page_table();
        let stack_top = Self::map_stack(&mut mapper);
        let (program_start, program_end) = Self::load_elf(program_code, &mut mapper);
        let heap_start = VirtAddr::new(program_end + 1u64).align_up(4096u64);
        // let (argv, rsp) = Self::copy_args_to_stack(c_argv, argc, stack_top, &mut mapper, program);
        Self {
            cr3,
            heap_start,
            heap_end: heap_start,
            program_start,
            program_end,
            stack_top,
            backbuffer_frames: None,
        }
    }

    pub fn load_elf(program_code: &[u8], mapper: &mut OffsetPageTable<'_>) -> (u64, u64) {
        const PAGE_SIZE: u64 = 4096;
        let mut boot_info = BOOT_INFO.get().expect("Boot info not initialized").lock();

        let file = ElfBytes::<LittleEndian>::minimal_parse(program_code).unwrap();
        let program_start = VirtAddr::new(file.ehdr.e_entry);
        let mut program_end = program_start.as_u64();
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

                program_end = core::cmp::max(program_end, (seg.p_vaddr + seg.p_memsz));

                let code = file.segment_data(&seg).unwrap();
                for (i, page) in
                    Page::range(start_page, start_page + (num_pages as u64)).enumerate()
                {
                    serial_println!("Mapping {:x}", page.start_address().as_u64());
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
        (program_start.as_u64(), program_end)
    }

    pub fn copy_args_to_stack(
        &mut self,
        c_argv: *const *const core::ffi::c_char,
        argc: usize,
        program: &'static CStr,
    ) -> (Vec<String>, VirtAddr) {
        let boot_info = BOOT_INFO.get().expect("Boot info not initialized").lock();
        let mapper = self.get_page_table();
        let mut arg_ptrs = Vec::new();

        // copy program_name
        let program_name_len = program.count_bytes() + 1;
        let mut rsp = self.stack_top - program_name_len;
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

        let mut argv: Vec<String> = Vec::with_capacity(argc);
        for i in 0..argc {
            unsafe {
                let ptr = *c_argv.add(i);
                let c_string = CStr::from_ptr(ptr);
                argv.push(c_string.to_str().unwrap().to_string());
            }
        }

        (argv, rsp)
    }

    pub fn new_page_table<'a>() -> (u64, OffsetPageTable<'a>) {
        let mut boot_info = BOOT_INFO.get().expect("Boot info not initialized").lock();

        let l4_table = boot_info.allocator.allocate_frame().unwrap();
        let cr3 = l4_table.start_address().as_u64();

        let l4_virt = VirtAddr::new(cr3 + boot_info.physical_memory_offset);
        let page_table: &mut PageTable = unsafe { &mut *l4_virt.as_mut_ptr() };

        let active_l4 = unsafe {
            &mut *((boot_info.page_table_address + boot_info.physical_memory_offset)
                as *mut PageTable)
        };
        for i in 0..512 {
            if i < 256 {
                page_table[i] = PageTableEntry::new();
            } else {
                page_table[i] = active_l4[i].clone();
            }
        }
        let mapper = unsafe {
            OffsetPageTable::new(page_table, VirtAddr::new(boot_info.physical_memory_offset))
        };

        (cr3, mapper)
    }

    pub fn map_stack(mapper: &mut OffsetPageTable<'_>) -> VirtAddr {
        let mut boot_info = BOOT_INFO.get().expect("Boot info not initialized").lock();
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

        stack_top
    }

    fn get_page_table(&self) -> OffsetPageTable<'_> {
        let boot_info = BOOT_INFO.get().expect("Boot info not initialized").lock();
        let l4_virt = VirtAddr::new(self.cr3 + boot_info.physical_memory_offset);
        let page_table: &mut PageTable = unsafe { &mut *l4_virt.as_mut_ptr() };
        unsafe { OffsetPageTable::new(page_table, VirtAddr::new(boot_info.physical_memory_offset)) }
    }
}

fn copy_frame_data(og_frame: PhysFrame, new_frame: PhysFrame) {
    todo!()
}

pub struct ProcessControlBlock {
    pub pid: u64,
    pub name: &'static str,
    pub argv: Vec<String>,
    pub tcb: Arc<Mutex<ThreadControlBlock>>,

    pub cwd: Arc<RwLock<DEntry>>,
    pub fd: Slab<Arc<Mutex<File>>>,

    pub adsp: AddressSpace,

    pub parent: Option<u64>,
    pub children: Vec<u64>,
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
        c_argv: *const *const core::ffi::c_char,
        argc: usize,
        program_bytes: &[u8],
        program_name: &'static CStr,
        directory: Arc<RwLock<DEntry>>,
        parent: Option<u64>,
    ) -> Self {
        let pid = PID.fetch_add(1u64, core::sync::atomic::Ordering::Relaxed);
        let mut addr_space = AddressSpace::from_elf(program_bytes);
        let (argv, rsp) = addr_space.copy_args_to_stack(c_argv, argc, program_name);

        let tcb = Arc::new(Mutex::new(ThreadControlBlock::new(
            pid,
            Self::user_process_hook as *const (),
            Some(addr_space.cr3 as *const usize),
            Some(addr_space.program_start),
            Some(rsp.as_u64()),
        )));

        Self {
            pid,
            tcb,
            argv,
            name: program_name.to_str().unwrap(),
            parent,
            children: Vec::new(),
            cwd: directory,
            fd: Self::setup_fd(),
            adsp: addr_space,
        }
    }

    fn setup_fd() -> Slab<Arc<Mutex<File>>> {
        let mut fds = Slab::with_capacity(MAX_FD);

        let tty = find_dentry("/dev/tty1");
        let tty = tty.unwrap().read().inode.clone();

        // TODO: actually fill this in
        let read_flags = OpenFlags::new();
        let write_flags = OpenFlags::new();

        let stdin_file = Arc::new(Mutex::new(File {
            inode: tty.clone(),
            pos: 0,
            flags: read_flags,
            ops: tty.ops.open(&tty, read_flags),
        }));
        let stdout_file = Arc::new(Mutex::new(File {
            inode: tty.clone(),
            pos: 0,
            flags: write_flags,
            ops: tty.ops.open(&tty, write_flags),
        }));
        let stderr_file = Arc::new(Mutex::new(File {
            inode: tty.clone(),
            pos: 0,
            flags: write_flags,
            ops: tty.ops.open(&tty, write_flags),
        }));

        let one = fds.insert(stdin_file);
        let two = fds.insert(stdout_file);
        let three = fds.insert(stderr_file);

        fds
    }
}

pub fn with_curr_proc_mut<F, R>(f: F) -> R
where
    F: FnOnce(&mut ProcessControlBlock) -> R,
{
    let curr_thread_id = unsafe { (*CURR_THREAD_PTR).id };
    let mut procs = PROC.get().unwrap().lock();
    let curr_proc = procs
        .iter_mut()
        .find(|p| p.tcb.lock().id == curr_thread_id)
        .unwrap();

    f(curr_proc)
}

pub fn with_curr_proc<F, R>(f: F) -> R
where
    F: FnOnce(&ProcessControlBlock) -> R,
{
    let curr_thread_id = unsafe { (*CURR_THREAD_PTR).id };
    let procs = PROC.get().unwrap().lock();
    let curr_proc = procs
        .iter()
        .find(|p| p.tcb.lock().id == curr_thread_id)
        .unwrap();

    f(curr_proc)
}

pub fn spawn_proc(program: &'static CStr, argv: *const *const core::ffi::c_char, argc: usize) {
    let binary = match program.to_str().unwrap() {
        "shell" => SHELL_ELF,
        "xiangqi" => XIANGQI_ELF,
        _ => {
            panic!("unrecognized program")
        }
    };
    serial_println!("Spawning process {}", program.to_str().unwrap());
    let proc =
        ProcessControlBlock::from_bytes(binary, argv, argc, program, get_root_dentry(), None);
    let id = proc.tcb.lock().id;
    let tcb_clone = proc.tcb.clone();

    serial_println!("{}", proc.tcb.lock().id);

    PROC.get().unwrap().lock().push(proc);

    let mut scheduler = SCHEDULER.lock();
    scheduler.threads.push(tcb_clone);
    scheduler.ready_queue.push_back(id);
}

pub fn fork() -> Option<ProcessControlBlock> {
    let child_pid = PID.fetch_add(1u64, core::sync::atomic::Ordering::Relaxed);

    with_curr_proc(|og_proc| {
        let new_address_space = og_proc.adsp.clone_for_fork();

        let tcb = Arc::new(Mutex::new(ThreadControlBlock::new(
            child_pid,
            ProcessControlBlock::user_process_hook as *const (),
            Some(new_address_space.cr3 as *const usize),
            Some(rip),
            Some(rsp),
        )));

        let new_proc = ProcessControlBlock {
            pid: child_pid,
            name: og_proc.name,
            tcb: new_tcb,
            argv: og_proc.argv.clone(),
            parent: Some(og_proc.pid),
            fd: og_proc.fd.clone(),
            cwd: og_proc.cwd.clone(),
            adsp: new_address_space,
            children: Vec::new(),
        };
        /*
        tcb,
        argv,
        name: program_name.to_str().unwrap(),
        parent,
        children: Vec::new(),
        cwd: directory,
        fd: Self::setup_fd(), */
    });

    /* let (cr3, mut mapper) = Self::setup_page_table();
    let stack_top = Self::map_stack(&mut mapper);

    let (argv, rsp) = Self::copy_args_to_stack(c_argv, argc, stack_top, &mut mapper, program);

    let (program_start, program_end) = Self::load_elf(program_code, &mut mapper);

    let heap_start = VirtAddr::new(program_end + 1u64).align_up(4096u64);

    let tcb = Arc::new(Mutex::new(ThreadControlBlock::new(
        pid,
        Self::user_process_hook as *const (),
        Some(cr3 as *const usize),
        Some(program_start),
        Some(rsp.as_u64()),
    ))); */

    None
}
