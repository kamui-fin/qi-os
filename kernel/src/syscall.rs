use crate::driver::serial;
use crate::fs::vfs::find_dentry;
use crate::fs::vfs::full_path;
use crate::fs::vfs::pipe::Pipe;
use crate::fs::vfs::pipe::PipeInodeOps;
use crate::fs::vfs::pipe::PipeOps;
use crate::fs::vfs::pipe::PIPE_FS;
use crate::fs::vfs::pipe::PIPE_ID_COUNT;
use crate::fs::vfs::sys::sys_close;
use crate::fs::vfs::sys::sys_open;
use crate::fs::vfs::sys::sys_read;
use crate::fs::vfs::sys::sys_write;
use crate::fs::vfs::DEntryMinimal;
use crate::fs::vfs::File;
use crate::fs::vfs::FsMetadata;
use crate::fs::vfs::INode;
use crate::fs::vfs::OpenFlags;
use crate::fs::vfs::Stat;
use crate::fs::vfs::StatusFlags;
use crate::interrupts;
use crate::interrupts::BOOT_RTC;
use crate::interrupts::ELAPSED;
use crate::lapic::my_proc;
use crate::lapic::mycpu;
use crate::spinlock::Spinlock;
use crate::task::proc::exec;
use crate::task::proc::fork;
use crate::task::proc::wait_pid;
use crate::task::proc::MAX_FD;
use crate::task::scheduler::nano_sleep;
use crate::task::scheduler::terminate_task;
use crate::task::scheduler::yield_sched;
use crate::task::scheduler::SCHEDULER;
use crate::task::thread::BlockReason;
use crate::task::thread::ThreadState;
use crate::UtsName;
use crate::ALLOC;
use crate::KERNEL_CONFIG;
use crate::PHYS_MEM_OFFSET;
use crate::PROC;
use crate::SCREEN;
use crate::UNAME;
use alloc::ffi::CString;
use alloc::sync::Arc;
use alloc::vec::Vec;
use common::UserWindow;
use core::arch::naked_asm;
use core::str::FromStr;
use core::sync::atomic::Ordering;
use core::{
    ffi::{c_char, CStr},
    ptr,
};
use crossbeam_queue::ArrayQueue;
use x86_64::structures::paging::FrameAllocator;
use x86_64::structures::paging::Mapper;
use x86_64::structures::paging::OffsetPageTable;
use x86_64::structures::paging::Page;
use x86_64::structures::paging::PageTableFlags;
use x86_64::structures::paging::Size4KiB;

use x86_64::VirtAddr;

// This is the starting virt addr within user proc that we'll map any shm into
const MMAP_BASE: usize = 0x0000_4000_0000_0000;

use crate::{serial_println, task::proc::spawn_proc};

#[repr(C)]
#[derive(Debug, Clone)]
pub struct TrapFrame {
    pub r15: usize,
    pub r14: usize,
    pub r13: usize,
    pub r12: usize,
    pub r11: usize,
    pub r10: usize,
    pub r9: usize,
    pub r8: usize,
    pub rbp: usize,
    pub rdi: usize,
    pub rsi: usize,
    pub rdx: usize,
    pub rcx: usize,
    pub rbx: usize,
    pub rax: usize,
    pub rip: usize,
    pub cs: usize,
    pub rflags: usize,
    pub rsp: usize,
    pub ss: usize,
}

impl TrapFrame {
    pub fn zero_out(&mut self) {
        self.r15 = 0;
        self.r14 = 0;
        self.r13 = 0;
        self.r12 = 0;
        self.r11 = 0;
        self.r10 = 0;
        self.r9 = 0;
        self.r8 = 0;
        self.rbp = 0;
        self.rdi = 0;
        self.rsi = 0;
        self.rdx = 0;
        self.rcx = 0;
        self.rbx = 0;
        self.rax = 0;
        self.rip = 0;
        self.cs = 0;
        self.rflags = 0;
        self.rsp = 0;
        self.ss = 0;
    }
}

#[unsafe(naked)]
pub unsafe extern "C" fn syscall_entry() {
    naked_asm!(
        r#"
            push RAX
            push RBX
            push RCX
            push RDX
            push RSI
            push RDI
            push RBP
            push R8
            push R9
            push R10
            push R11
            push R12
            push R13
            push R14
            push R15

            mov rdi, rsp

            mov rbp, rsp
            and rsp, -16
            call {}
            mov rsp, rbp

            pop R15
            pop R14
            pop R13
            pop R12
            pop R11
            pop R10
            pop R9
            pop R8
            pop RBP
            pop RDI
            pop RSI
            pop RDX
            pop RCX
            pop RBX
            pop RAX

            iretq
        "#,
        sym syscall_handler
    );
}

// TODO to be more POSIX-like:
//  - fork, exec, process tree
//  - kill, sigaction, sigreturn: send signal, register signal handler
//  - poll / select / epoll: sleep until any of a set of FDs is ready
//  - mmap, munmap: maps files or devices into memory (COMPLEX)
//  - shmctl (shared memory)
//  - mknod
//  - utime, ulimit, times
//  - mount, umount, sync
//  - nice (scheduling)
//  - pause (block until signal)

#[derive(Debug)]
enum SysCallKind {
    Open,
    Close,

    Read,
    Write,
    Lseek,

    // dir ops
    Chdir,
    Getcwd,
    Mkdir,
    Rmdir,
    Getdents,

    // file ops
    Fstat,
    Delete,
    Rename,

    Dup,
    Dup2,
    Pipe,

    //  (fd, request, datapointer) manipulates the underlying device parameters of special files
    Ioctl,
    //  ioctl but manage the fd itself. e.g. set to non-blocking!
    Fcntl,

    Fork,
    Exec,
    WaitPid,

    Yield,
    Exit,
    Alloc,
    GetPid,
    AllocBackBuffer,
    GraphicsFrameReady,
    Sleep,
    GetUnixTime,
    Uname,
}

impl From<usize> for SysCallKind {
    fn from(value: usize) -> Self {
        match value {
            0 => Self::Open,
            1 => Self::Close,
            2 => Self::Read,
            3 => Self::Write,
            4 => Self::Lseek,
            5 => Self::Ioctl,
            6 => Self::Fcntl,

            7 => Self::Chdir,
            8 => Self::Getcwd,
            9 => Self::Mkdir,
            10 => Self::Rmdir,
            11 => Self::Getdents,

            12 => Self::Fstat,
            13 => Self::Delete,
            14 => Self::Rename,

            15 => Self::Dup,
            16 => Self::Dup2,
            17 => Self::Pipe,

            18 => Self::Fork,
            28 => Self::Exec,
            29 => Self::WaitPid,

            19 => Self::Yield,
            20 => Self::Exit,
            21 => Self::GetPid,
            22 => Self::Sleep,

            23 => Self::Alloc,
            24 => Self::AllocBackBuffer,
            25 => Self::GraphicsFrameReady,

            26 => Self::GetUnixTime,
            27 => Self::Uname,

            _ => panic!("unknown syscall number {}", value),
        }
    }
}

enum FcntlCommand {
    SetFlags(u32),
    GetFlags,
}

extern "C" fn syscall_handler(trap_frame: &mut TrapFrame) {
    x86_64::instructions::interrupts::enable();

    let kind = SysCallKind::from(trap_frame.rax);
    let arg1 = trap_frame.rdi;
    let arg2 = trap_frame.rsi;
    let arg3 = trap_frame.rdx;
    let arg4 = trap_frame.r10;
    let arg5 = trap_frame.r8;
    let arg6 = trap_frame.r9;
    serial_println!("{:#?}()", kind);
    let return_value: usize = match kind {
        SysCallKind::Fork => {
            // no args
            // return pid of forked process
            fork(trap_frame) as usize
        }
        SysCallKind::Exec => {
            // arg1: *const c_str
            // arg2: argc
            // arg3: argv
            let prgrm_name = unsafe { CStr::from_ptr(arg1 as *const i8) };
            let argv = arg3 as *const *const c_char;
            let argc = arg2;
            exec(prgrm_name, argv, argc, trap_frame);
            0
        }
        SysCallKind::WaitPid => {
            let pid = arg1 as u64;
            serial_println!("Waiting for child...");
            wait_pid(pid);
            serial_println!("Done! I'm back!");
            0
        }
        SysCallKind::Open => {
            // arg1: pathname *const c_cstr
            // arg2: flags
            sys_open(arg1, OpenFlags::from(arg2 as u64)).unwrap() as usize
        }
        SysCallKind::Close => {
            // arg1: fd (u64)
            sys_close(arg1 as u64);
            0
        }
        SysCallKind::Read => {
            // arg1: fd (u64)
            // arg2: buffer *mut u8
            // arg3: len (usize)
            serial_println!("Reading from {} {}", arg1, arg3);
            let buffer = unsafe { core::slice::from_raw_parts_mut(arg2 as *mut u8, arg3) };
            sys_read(arg1 as u64, buffer)
        }
        SysCallKind::Write => {
            // arg1: fd (u64)
            // arg2: buffer *u8
            // arg3: len (usize)
            let buffer = unsafe { core::slice::from_raw_parts(arg2 as *const u8, arg3) };
            sys_write(arg1 as u64, buffer)
        }
        SysCallKind::Chdir => {
            // arg1: path *const c_char
            let path = (unsafe { CStr::from_ptr(arg1 as *const i8) })
                .to_str()
                .unwrap();
            let dentry = find_dentry(path).unwrap();

            let p = my_proc();
            let mut p = p.lock();
            p.cwd = dentry;

            0
        }
        SysCallKind::Mkdir => {
            // arg1: path *const c_char
            let path = (unsafe { CStr::from_ptr(arg1 as *const i8) })
                .to_str()
                .unwrap();
            let (parent, child) = path.rsplit_once('/').unwrap();
            let dir = find_dentry(parent).unwrap();
            let dir_guard = dir.read();
            dir_guard.inode.ops.mkdir(&dir_guard.inode, child);

            0
        }
        SysCallKind::Rmdir => {
            // arg1: path *const c_char
            let path = (unsafe { CStr::from_ptr(arg1 as *const i8) })
                .to_str()
                .unwrap();
            let dir = find_dentry(path).unwrap();
            let dir_guard = dir.read();
            dir_guard.inode.ops.rmdir(&dir_guard.inode);

            0
        }
        SysCallKind::Fstat => {
            // arg1: fd (u64)
            // arg2: *mut Stat
            let p = my_proc();
            let p = p.lock();
            let file = p.fd.get(arg1 as usize).unwrap().lock();
            let stat = file.inode.ops.stat(&file.inode);

            let user_stat = arg1 as *mut Stat;
            unsafe { ptr::copy(&stat as *const Stat, user_stat, 1) };

            0
        }
        // TODO: refactor to use unlink and only delete fr if link = 0
        SysCallKind::Delete => {
            // arg1: filepath  *const c_char
            // TODO: check if any process is using file
            let filepath = (unsafe { CStr::from_ptr(arg1 as *const i8) })
                .to_str()
                .unwrap();
            let path_dent = find_dentry(filepath).unwrap();
            let path_dent = path_dent.read();
            path_dent.inode.ops.delete_file(&path_dent.inode);

            0
        }
        SysCallKind::Dup => {
            // arg1: oldfd u64
            let p = my_proc();
            let mut p = p.lock();
            let oldfd = p.fd.get(arg1).unwrap();
            let lowest_fd = (0..MAX_FD).find(|i| p.fd.contains(*i)).unwrap();
            p.fd[lowest_fd] = oldfd.clone();
            lowest_fd
        }
        SysCallKind::Dup2 => {
            // arg1: oldfd u64
            // arg2: newfd u64
            if arg1 != arg2 {
                let p = my_proc();
                let mut p = p.lock();
                let oldfd = p.fd.get(arg1).unwrap();
                p.fd[arg2] = oldfd.clone();
            }

            arg2
        }
        SysCallKind::Yield => {
            yield_sched();
            unreachable!();
        }
        SysCallKind::Uname => {
            // arg1: *mut UtsName
            let utsname = arg1 as *mut UtsName;
            unsafe { ptr::copy(&*UNAME as *const UtsName, utsname, 1) };
            0
        }
        SysCallKind::Getcwd => {
            // arg1: buffer *u8
            // arg2: len (usize)
            let p = my_proc();
            let p = p.lock();
            let abs_cwd = full_path(p.cwd.clone());
            let ptr = arg1 as *mut u8;
            let copy_len = core::cmp::min(arg2, abs_cwd.len());
            unsafe { ptr::copy_nonoverlapping(abs_cwd.as_ptr(), ptr, copy_len) };

            copy_len
        }
        SysCallKind::Lseek => {
            // arg1: fd u64
            // arg2: offset usize
            // arg3 (for now we won't implement): whence (SEEK_START, SEEK_CUR, SEEK_END) like starting from where
            let p = my_proc();
            let p = p.lock();
            let mut file = p.fd[arg1].lock();
            let pos = file.pos;
            let new_pos = core::cmp::min(file.inode.meta.lock().size, pos + arg2);
            file.pos = new_pos;

            0
        }
        SysCallKind::Getdents => {
            // arg1: fd u64
            // arg2: addr of *mut DEntryMinimal
            // arg3: count usize
            let p = my_proc();
            let p = p.lock();
            let file = p.fd.get(arg1 as usize).unwrap().lock();
            let entries = file.inode.ops.readdir(&file.inode);
            let offset = file.pos;

            let user_dentry = arg2 as *mut DEntryMinimal;

            let mut entries_written = 0;

            for i in (offset)..(entries.len()) {
                if entries_written >= arg3 {
                    break;
                }
                let entry = &entries[i];
                unsafe { ptr::write(user_dentry.add(entries_written), entry.clone()) }
                entries_written += 1;
            }

            entries_written
        }
        SysCallKind::Ioctl => {
            // arg1: fd u64
            // arg2: request u64
            // arg3: args void*
            let p = my_proc();
            let p = p.lock();
            let fd = p.fd.get(arg1).unwrap().lock();
            fd.ops.ioctl(arg2 as u64, arg3 as u64);

            0
        }
        SysCallKind::Fcntl => {
            // arg1: fd u64
            // arg2: cmd u64
            // arg3: args void*

            // this looks redundant rn but it's the groundwork for a refactor coming soon
            let cmd = match arg2 {
                1 => FcntlCommand::GetFlags,
                2 => FcntlCommand::SetFlags(arg3 as u32),
                _ => unimplemented!(),
            };

            let p = my_proc();
            let p = p.lock();
            match cmd {
                FcntlCommand::SetFlags(flags) => {
                    let mut fd = p.fd.get(arg1).unwrap().lock();
                    fd.set_status(StatusFlags::from_bits_truncate(flags));
                    0
                }
                FcntlCommand::GetFlags => u64::from(p.fd.get(arg1).unwrap().lock().flags) as usize,
            }
        }
        SysCallKind::Pipe => {
            // arg1: pipefd int[2]
            let pipe = Pipe {
                buffer: Spinlock::new(ArrayQueue::new(4096)),
                readers: 1,
                writers: 1,
            };
            let inode = Arc::new(INode {
                inum: PIPE_ID_COUNT.fetch_add(1, Ordering::Relaxed),
                fs: PIPE_FS.clone(),
                mode: crate::fs::vfs::NodeType::Pipe,
                data: crate::fs::vfs::INodeData::Pipe(Arc::new(pipe)),
                meta: Spinlock::new(FsMetadata {
                    size: 4096,
                    mtime: 0,
                    dirty: false,
                }),
                ops: Arc::new(PipeInodeOps),
            });
            let write_file = Arc::new(Spinlock::new(File {
                inode: inode.clone(),
                pos: 0,
                flags: OpenFlags::new(),
                ops: Arc::new(PipeOps),
            }));
            let read_file = Arc::new(Spinlock::new(File {
                inode: inode.clone(),
                pos: 0,
                flags: OpenFlags::new(),
                ops: Arc::new(PipeOps),
            }));

            let p = my_proc();
            let mut p = p.lock();
            let rfd = p.fd.insert(read_file) as u64;
            let wfd = p.fd.insert(write_file) as u64;
            let rfd_user = arg1 as *mut u64;

            unsafe {
                let wfd_user = rfd_user.add(1);
                *rfd_user = rfd;
                *wfd_user = wfd;
            }

            0
        }
        // NOTE: this can only rename files within the same dir currently
        // TODO: make this operate more like the `mv` cmd!
        SysCallKind::Rename => {
            // arg1: old filepath  *const c_char
            // arg2: new filename  *const c_char
            let old_path = (unsafe { CStr::from_ptr(arg1 as *const i8) })
                .to_str()
                .unwrap();
            let new_filename = (unsafe { CStr::from_ptr(arg1 as *const i8) })
                .to_str()
                .unwrap();
            let old_path = find_dentry(old_path).unwrap();
            let old_dir_guard = old_path.read();
            old_dir_guard
                .inode
                .ops
                .rename(&old_dir_guard.inode, new_filename);
            0
        }
        SysCallKind::Exit => {
            // arg1: status
            sys_exit(arg1);
            0
        }
        SysCallKind::GetPid => sys_get_pid(),
        SysCallKind::AllocBackBuffer => {
            // arg1: *mut UserWindow
            sys_alloc_back_buffer(arg1);
            0
        }
        SysCallKind::GraphicsFrameReady => {
            sys_frame_ready();
            0
        }
        SysCallKind::Sleep => {
            // arg1: ms
            sys_sleep(arg1);
            0
        }
        SysCallKind::Alloc => sys_alloc(arg1),
        SysCallKind::GetUnixTime => sys_get_unix_time(),
    };
    trap_frame.rax = return_value;
}

fn sys_get_unix_time() -> usize {
    let elapsed_sec = ELAPSED.load(Ordering::Relaxed).div_ceil(1000) as usize;
    let boot_unix = BOOT_RTC.try_get().unwrap().as_unix_timestamp();
    boot_unix + elapsed_sec
}

fn sys_alloc(arg1: usize) -> usize {
    // arg1: size
    let my_proc = my_proc();
    let mut my_proc = my_proc.lock();

    let old_heap_end = my_proc.adsp.heap_end;
    let new_heap_end = old_heap_end + arg1;

    let old_mapped_end = old_heap_end.align_up(4096u64);
    let new_mapped_end = new_heap_end.align_up(4096u64);

    if new_mapped_end > old_mapped_end {
        let start_page = Page::<Size4KiB>::containing_address(old_mapped_end);
        let end_page = Page::<Size4KiB>::containing_address(new_mapped_end - 1u64);

        let mut mapper = my_proc.adsp.get_page_table(PHYS_MEM_OFFSET);
        // map pages in-between
        for page in Page::range_inclusive(start_page, end_page) {
            let mut alloc = ALLOC.get().unwrap().lock();
            let frame = alloc.allocate_frame().expect("proc_init: out of mem");
            let frame_ptr: *mut u8 =
                VirtAddr::new(frame.start_address().as_u64() + PHYS_MEM_OFFSET).as_mut_ptr();
            // clear frame
            unsafe {
                core::ptr::write_bytes(frame_ptr, 0, 4096);
            }

            let mapper_flush = unsafe {
                mapper
                    .map_to(
                        page,
                        frame,
                        PageTableFlags::WRITABLE
                            | PageTableFlags::PRESENT
                            | PageTableFlags::USER_ACCESSIBLE,
                        &mut *alloc,
                    )
                    .expect("(fixed offset mapping): unable to map frame")
            };
            mapper_flush.flush();
        }
    }

    my_proc.adsp.heap_end = new_heap_end;
    old_heap_end.as_u64() as usize
}

fn sys_sleep(arg1: usize) {
    nano_sleep((arg1 * 1_000_000) as u64);
}

fn sys_frame_ready() {
    // we notify compositor that we are ready for a paint
    let mut scheduler = SCHEDULER.lock();
    scheduler.unblock_task(3);
}

fn sys_alloc_back_buffer(arg1: usize) {
    let my_proc = my_proc();
    let mut my_proc = my_proc.lock();

    // get the length of LFB and allocate needed frames, zero out, map into userspace
    let screen = SCREEN.get().unwrap().lock();
    // not seeing any prints after here???
    let mut mapper = my_proc.adsp.get_page_table(PHYS_MEM_OFFSET);
    let buffer_num_bytes = screen.bytes_per_line * screen.height;

    let num_frames = buffer_num_bytes.div_ceil(4096);
    // that's how many pages we'll have
    let start_page = Page::containing_address(VirtAddr::new(MMAP_BASE as u64));
    let end_page = start_page + num_frames as u64;

    let mut backbuffer_frames = Vec::new();
    let mut alloc = ALLOC.get().unwrap().lock();
    for page in Page::range(start_page, end_page) {
        let frame = alloc.allocate_frame().expect("proc_init: out of mem");

        backbuffer_frames.push(frame);

        let frame_ptr: *mut u8 =
            VirtAddr::new(frame.start_address().as_u64() + PHYS_MEM_OFFSET).as_mut_ptr();
        // clear frame
        unsafe {
            core::ptr::write_bytes(frame_ptr, 0, 4096);
        }
        let mapper_flush = unsafe {
            mapper
                .map_to(
                    page,
                    frame,
                    PageTableFlags::WRITABLE
                        | PageTableFlags::PRESENT
                        | PageTableFlags::USER_ACCESSIBLE,
                    &mut *alloc,
                )
                .expect("(fixed offset mapping): unable to map frame")
        };
        mapper_flush.flush();
    }

    my_proc.adsp.backbuffer_frames = Some(backbuffer_frames);

    // Fill in user passed in FrameBufferInfo struct
    let user_window_info = unsafe { &mut *(arg1 as *mut UserWindow) };
    user_window_info.base_addr = MMAP_BASE as u64;
    user_window_info.width = screen.width;
    user_window_info.height = screen.height;
    user_window_info.bytes_per_pixel = screen.bytes_per_pixel;
    user_window_info.bytes_per_line = screen.bytes_per_line;
}

fn sys_get_pid() -> usize {
    let p = my_proc();
    let p = p.lock();
    p.pid as usize
}

fn sys_spawn(arg1: usize, arg2: usize, arg3: usize) {
    let prgrm_name = unsafe { CStr::from_ptr(arg1 as *const i8) };
    spawn_proc(prgrm_name, arg3 as *const *const c_char, arg2);
}

fn sys_exit(arg1: usize) {
    let curr_thread_id = unsafe { (*mycpu().curr_thread.load(Ordering::Relaxed)).id };
    let parent_id = {
        let mut procs = PROC.get().unwrap().lock();
        let my_proc_index = procs
            .iter()
            .position(|p| p.lock().pid == curr_thread_id)
            .unwrap();
        procs.remove(my_proc_index).lock().parent
    };

    // notify parent
    if let Some(parent_id) = parent_id {
        serial_println!("Checking if parent is waiting...");
        let unblock_parent = {
            let scheduler = SCHEDULER.lock();
            let parent_proc = &scheduler
                .threads
                .iter()
                .find(|t| t.id == parent_id as u64)
                .unwrap()
                .state;
            serial_println!("{:#?}", parent_proc);
            if let ThreadState::Blocked(BlockReason::WaitThread(child_pid)) = parent_proc {
                if *child_pid == curr_thread_id as u64 {
                    serial_println!("MUST NOTIFY PARENT THAT I DIED");
                    Some(parent_id)
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(parent_id) = unblock_parent {
            let mut scheduler = SCHEDULER.lock();
            scheduler.unblock_task(parent_id);
        }
    }

    serial_println!("Exiting {curr_thread_id} with status {arg1}");
    terminate_task(arg1 as u8);
}
