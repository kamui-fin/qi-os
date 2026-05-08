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
use crate::task::proc::with_curr_proc;
use crate::task::proc::with_curr_proc_mut;
use crate::task::proc::MAX_FD;
use crate::task::thread::nano_sleep;
use crate::task::thread::terminate_task;
use crate::task::thread::yield_sched;
use crate::task::thread::SCHEDULER;
use crate::UtsName;
use crate::BOOT_INFO;
use crate::PROC;
use crate::SCREEN;
use crate::UNAME;
use alloc::ffi::CString;
use alloc::sync::Arc;
use alloc::vec::Vec;
use common::UserWindow;
use core::arch::naked_asm;
use core::str::FromStr;
use core::{
    ffi::{c_char, CStr},
    ptr,
};
use crossbeam_queue::ArrayQueue;
use spin::Mutex;
use x86_64::structures::paging::FrameAllocator;
use x86_64::structures::paging::Mapper;
use x86_64::structures::paging::OffsetPageTable;
use x86_64::structures::paging::Page;
use x86_64::structures::paging::PageTableFlags;
use x86_64::structures::paging::Size4KiB;

use x86_64::VirtAddr;

use crate::{
    serial_println,
    task::{proc::spawn_proc, thread::CURR_THREAD_PTR},
};

#[repr(C)]
#[derive(Debug)]
struct TrapFrame {
    r15: usize,
    r14: usize,
    r13: usize,
    r12: usize,
    r11: usize,
    r10: usize,
    r9: usize,
    r8: usize,
    rbp: usize,
    rdi: usize,
    rsi: usize,
    rdx: usize,
    rcx: usize,
    rbx: usize,
    rax: usize,
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
            sys_read(arg1 as u64, buffer);
            0
        }
        SysCallKind::Write => {
            // arg1: fd (u64)
            // arg2: buffer *u8
            // arg3: len (usize)
            let buffer = unsafe { core::slice::from_raw_parts(arg2 as *const u8, arg3) };
            sys_write(arg1 as u64, buffer);
            0
        }
        SysCallKind::Chdir => {
            // arg1: path *const c_char
            let path = (unsafe { CStr::from_ptr(arg1 as *const i8) })
                .to_str()
                .unwrap();
            let dentry = find_dentry(path).unwrap();
            with_curr_proc_mut(|proc| {
                proc.cwd = dentry;
            });

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
            let stat = with_curr_proc(|p| {
                let file = p.fd.get(arg1 as usize).unwrap().lock();
                file.inode.ops.stat(&file.inode)
            });

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
            with_curr_proc_mut(|p| {
                let oldfd = p.fd.get(arg1).unwrap();
                let lowest_fd = (0..MAX_FD).find(|i| p.fd.contains(*i)).unwrap();
                p.fd[lowest_fd] = oldfd.clone();
                lowest_fd
            })
        }
        SysCallKind::Dup2 => {
            // arg1: oldfd u64
            // arg2: newfd u64
            if arg1 != arg2 {
                with_curr_proc_mut(|p| {
                    let oldfd = p.fd.get(arg1).unwrap();
                    p.fd[arg2] = oldfd.clone();
                });
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
            let abs_cwd = with_curr_proc(|p| full_path(p.cwd.clone()));
            let abs_cwd = CString::from_str(&abs_cwd).unwrap();
            let ptr = arg1 as *mut c_char;
            unsafe { ptr::copy_nonoverlapping(abs_cwd.as_ptr(), ptr, arg2) };

            0
        }
        SysCallKind::Lseek => {
            // arg1: fd u64
            // arg2: offset usize
            // arg3 (for now we won't implement): whence (SEEK_START, SEEK_CUR, SEEK_END) like starting from where
            with_curr_proc_mut(|p| {
                let mut file = p.fd[arg1].lock();
                let pos = file.pos;
                let new_pos = core::cmp::min(file.inode.meta.lock().size, pos + arg2);
                file.pos = new_pos;
            });

            0
        }
        SysCallKind::Getdents => {
            // arg1: fd u64
            // arg2: addr of *mut DEntryMinimal
            // arg3: count usize
            let (entries, offset) = with_curr_proc(|p| {
                let file = p.fd.get(arg1 as usize).unwrap().lock();
                (file.inode.ops.readdir(&file.inode), file.pos)
            });

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
            with_curr_proc_mut(|p| {
                let fd = p.fd.get(arg1).unwrap().lock();
                fd.ops.ioctl(arg2 as u64, arg3 as u64);
            });

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

            match cmd {
                FcntlCommand::SetFlags(flags) => with_curr_proc_mut(|p| {
                    let mut fd = p.fd.get(arg1).unwrap().lock();
                    fd.set_status(StatusFlags::from_bits_truncate(flags));
                    0
                }),
                FcntlCommand::GetFlags => {
                    with_curr_proc(|p| u64::from(p.fd.get(arg1).unwrap().lock().flags)) as usize
                }
            }
        }
        SysCallKind::Pipe => {
            // arg1: pipefd int[2]
            let pipe = Pipe {
                buffer: Mutex::new(ArrayQueue::new(4096)),
                readers: 1,
                writers: 1,
            };
            let inode = Arc::new(INode {
                inum: PIPE_ID_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed),
                fs: PIPE_FS.clone(),
                mode: crate::fs::vfs::NodeType::Pipe,
                data: crate::fs::vfs::INodeData::Pipe(Arc::new(pipe)),
                meta: Mutex::new(FsMetadata {
                    size: 4096,
                    mtime: 0,
                    dirty: false,
                }),
                ops: Arc::new(PipeInodeOps),
            });
            let write_file = Arc::new(Mutex::new(File {
                inode: inode.clone(),
                pos: 0,
                flags: OpenFlags::new(),
                ops: Arc::new(PipeOps),
            }));
            let read_file = Arc::new(Mutex::new(File {
                inode: inode.clone(),
                pos: 0,
                flags: OpenFlags::new(),
                ops: Arc::new(PipeOps),
            }));

            with_curr_proc_mut(|p| {
                let rfd = p.fd.insert(read_file) as u64;
                let wfd = p.fd.insert(write_file) as u64;
                let rfd_user = arg1 as *mut u64;

                unsafe {
                    let wfd_user = rfd_user.add(1);
                    *rfd_user = rfd;
                    *wfd_user = wfd;
                }
            });

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
        SysCallKind::Fork => {
            // no args
            // return pid of forked process

            0
        }
        SysCallKind::Exec => {
            // arg1: *const c_str
            // arg2: argc
            // arg3: argv
            sys_spawn(arg1, arg2, arg3);
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
    let elapsed_sec = ELAPSED
        .load(core::sync::atomic::Ordering::Relaxed)
        .div_ceil(1000) as usize;
    let boot_unix = BOOT_RTC.try_get().unwrap().as_unix_timestamp();
    boot_unix + elapsed_sec
}

fn sys_alloc(arg1: usize) -> usize {
    let mut boot_info = BOOT_INFO.get().expect("Boot info not initialized").lock();
    // arg1: size
    let curr_thread_id = unsafe { (*CURR_THREAD_PTR).id };
    let mut procs = PROC.get().unwrap().lock();
    let curr_proc = procs
        .iter_mut()
        .find(|p| p.tcb.lock().id == curr_thread_id)
        .unwrap();

    let old_heap_end = curr_proc.heap_end;
    let new_heap_end = old_heap_end + arg1;

    let old_mapped_end = old_heap_end.align_up(4096u64);
    let new_mapped_end = new_heap_end.align_up(4096u64);

    if new_mapped_end > old_mapped_end {
        let start_page = Page::<Size4KiB>::containing_address(old_mapped_end);
        let end_page = Page::<Size4KiB>::containing_address(new_mapped_end - 1u64);

        let mut mapper = unsafe {
            OffsetPageTable::new(
                curr_proc.page_table,
                VirtAddr::new(boot_info.physical_memory_offset),
            )
        };
        // map pages in-between
        for page in Page::range_inclusive(start_page, end_page) {
            let frame = boot_info
                .allocator
                .allocate_frame()
                .expect("proc_init: out of mem");
            let frame_ptr: *mut u8 =
                VirtAddr::new(frame.start_address().as_u64() + boot_info.physical_memory_offset)
                    .as_mut_ptr();
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
                        &mut boot_info.allocator,
                    )
                    .expect("(fixed offset mapping): unable to map frame")
            };
            mapper_flush.flush();
        }
    }

    curr_proc.heap_end = new_heap_end;
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
    // get the length of LFB and allocate needed frames, zero out, map into userspace

    // This is the starting virt addr within user proc that we'll map any shm into
    const MMAP_BASE: usize = 0x0000_4000_0000_0000;

    let curr_thread_id = unsafe { (*CURR_THREAD_PTR).id };
    let mut procs = PROC.get().unwrap().lock();
    let curr_proc = procs
        .iter_mut()
        .find(|p| p.tcb.lock().id == curr_thread_id)
        .unwrap();

    let mut boot_info = BOOT_INFO.get().unwrap().lock();
    let screen = SCREEN.get().unwrap().lock();
    // not seeing any prints after here???
    let mut mapper = unsafe {
        OffsetPageTable::new(
            curr_proc.page_table,
            VirtAddr::new(boot_info.physical_memory_offset),
        )
    };
    let buffer_num_bytes = screen.bytes_per_line * screen.height;

    let num_frames = buffer_num_bytes.div_ceil(4096);
    // that's how many pages we'll have
    let start_page = Page::containing_address(VirtAddr::new(MMAP_BASE as u64));
    let end_page = start_page + num_frames as u64;

    let mut backbuffer_frames = Vec::new();
    for page in Page::range(start_page, end_page) {
        let frame = boot_info
            .allocator
            .allocate_frame()
            .expect("proc_init: out of mem");

        backbuffer_frames.push(frame);

        let frame_ptr: *mut u8 =
            VirtAddr::new(frame.start_address().as_u64() + boot_info.physical_memory_offset)
                .as_mut_ptr();
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
                    &mut boot_info.allocator,
                )
                .expect("(fixed offset mapping): unable to map frame")
        };
        mapper_flush.flush();
    }

    curr_proc.backbuffer_frames = Some(backbuffer_frames);

    // Fill in user passed in FrameBufferInfo struct
    let user_window_info = unsafe { &mut *(arg1 as *mut UserWindow) };
    user_window_info.base_addr = MMAP_BASE as u64;
    user_window_info.width = screen.width;
    user_window_info.height = screen.height;
    user_window_info.bytes_per_pixel = screen.bytes_per_pixel;
    user_window_info.bytes_per_line = screen.bytes_per_line;
}

fn sys_get_pid() -> usize {
    let curr_thread_id = unsafe { (*CURR_THREAD_PTR).id };
    let procs = PROC.get().unwrap().lock();
    let curr_proc = procs
        .iter()
        .find(|p| p.tcb.lock().id == curr_thread_id)
        .unwrap();
    curr_proc.pid as usize
}

fn sys_spawn(arg1: usize, arg2: usize, arg3: usize) {
    let prgrm_name = unsafe { CStr::from_ptr(arg1 as *const i8) };
    spawn_proc(prgrm_name, arg3 as *const *const c_char, arg2);
}

fn sys_exit(arg1: usize) {
    let curr_thread_id = unsafe { (*CURR_THREAD_PTR).id };
    {
        let mut procs = PROC.get().unwrap().lock();
        let curr_proc_index = procs
            .iter()
            .position(|p| p.tcb.lock().id == curr_thread_id)
            .unwrap();
        procs.remove(curr_proc_index);
    }
    serial_println!("Exiting {curr_thread_id} with status {arg1}");
    terminate_task(arg1 as u8);
}
