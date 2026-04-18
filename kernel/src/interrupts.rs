use core::arch::asm;
use core::arch::naked_asm;
use core::ffi::c_char;
use core::ffi::c_str;
use core::ffi::CStr;
use core::num;
use core::ptr;
use core::sync::atomic::AtomicU64;
use core::sync::atomic::AtomicUsize;

use crate::hlt_loop;
use crate::lock::NEEDS_RESCHEDULE;
use crate::mouse::GenericPs2Packet;
use crate::print;
use crate::println;
use crate::proc::ProcessControlBlock;
use crate::proc::ECHO_ELF;
use crate::proc::XIANGQI_ELF;
use crate::serial_print;
use crate::serial_println;
use crate::thread::nano_sleep;
use crate::thread::switch_if_needed;
use crate::thread::terminate_task;
use crate::thread::BlockReason;
use crate::thread::ThreadControlBlock;
use crate::thread::ThreadState;
use crate::thread::CURR_THREAD_PTR;
use crate::thread::SCHEDULER;
use crate::time::get_rtc_time;
use crate::time::RTCTime;
use crate::BOOT_INFO;
use crate::PROC;
use alloc::boxed::Box;
use alloc::vec::Vec;
use common::UserWindow;
use conquer_once::spin::OnceCell;
use futures_util::stream::select_with_strategy;
use lazy_static::lazy_static;
use pic8259::ChainedPics;
use spin;
use x86_64::instructions::port::Port;
use x86_64::structures::idt::PageFaultErrorCode;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};
use x86_64::structures::paging::FrameAllocator;
use x86_64::structures::paging::Mapper;
use x86_64::structures::paging::OffsetPageTable;
use x86_64::structures::paging::Page;
use x86_64::structures::paging::PageTableFlags;
use x86_64::structures::paging::Size4KiB;
use x86_64::structures::port::PortRead;
use x86_64::VirtAddr;

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

pub static PICS: spin::Mutex<ChainedPics> =
    spin::Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(crate::gdt::DOUBLE_FAULT_IST_INDEX);
        }
        idt[InterruptIndex::Timer.as_usize()].set_handler_fn(timer_interrupt_handler);
        idt[InterruptIndex::Keyboard.as_usize()].set_handler_fn(keyboard_interrupt_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);
        idt.general_protection_fault.set_handler_fn(gpf_handler);
        idt[InterruptIndex::PS2.as_usize()].set_handler_fn(mouse_interrupt_handler);

        unsafe {
            let handler_addr = VirtAddr::new(syscall_entry as usize as u64);
            idt[0x80]
                .set_handler_addr(handler_addr)
                .set_privilege_level(x86_64::PrivilegeLevel::Ring3);
        }
        idt
    };
}

/*
* XV6 has:
    *** SYS_fork    1
    *** SYS_exit    2
    SYS_wait    3
    SYS_pipe    4
    SYS_read    5
    *** SYS_kill    6
    SYS_exec    7
    SYS_fstat   8
    SYS_chdir   9
    SYS_dup    10
    *** SYS_getpid 11
    *** SYS_sbrk   12
    *** SYS_pause  13
    *** SYS_uptime 14
    SYS_open   15
    SYS_write  16
    SYS_mknod  17
    SYS_unlink 18
    SYS_link   19
    SYS_mkdir  20
    SYS_close  21
*/

#[derive(Debug)]
enum SysCallKind {
    Write, // fd, buf, len
    Exit,  // status
    Spawn, // path, argv
    Wait,  // pid
    Alloc, // size
    GetPid,
    AllocBackBuffer,
    GraphicsFrameReady,
    Sleep,
    GetUnixTime,
}

impl From<usize> for SysCallKind {
    fn from(value: usize) -> Self {
        match value {
            0x0 => Self::Write,
            0x1 => Self::Exit,
            0x2 => Self::Spawn,
            0x3 => Self::Wait,
            0x4 => Self::Alloc,
            0x5 => Self::GetPid,
            0x6 => Self::AllocBackBuffer,
            0x7 => Self::GraphicsFrameReady,
            0x8 => Self::Sleep,
            0x9 => Self::GetUnixTime,
            _ => panic!("unknown syscall"),
        }
    }
}

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
            call {}

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

extern "C" fn syscall_handler(trap_frame: &mut TrapFrame) {
    let kind = SysCallKind::from(trap_frame.rax);
    let arg1 = trap_frame.rdi;
    let arg2 = trap_frame.rsi;
    let arg3 = trap_frame.rdx;
    let arg4 = trap_frame.r10;
    let arg5 = trap_frame.r8;
    let arg6 = trap_frame.r9;
    serial_println!("{:#?} {arg1:x} {arg2:x}", kind);
    match kind {
        SysCallKind::Write => {
            // arg1: *const str
            // arg2: length
            let slice_ptr: *const [u8] = ptr::slice_from_raw_parts(arg1 as *const u8, arg2);
            let str_ptr: *const str = slice_ptr as *const str;
            let str = unsafe { &*str_ptr };

            serial_println!("{str}");
        }
        SysCallKind::Exit => {
            // arg1: status
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
        SysCallKind::Spawn => {
            // arg1: *const c_str
            // arg2: argc
            // arg3: argv
            let prgrm_name = unsafe { CStr::from_ptr(arg1 as *const i8) };
            spawn_proc(prgrm_name, arg3 as *const *const c_char, arg2);
        }
        SysCallKind::GetPid => {
            let curr_thread_id = unsafe { (*CURR_THREAD_PTR).id };
            let procs = PROC.get().unwrap().lock();
            let curr_proc = procs
                .iter()
                .find(|p| p.tcb.lock().id == curr_thread_id)
                .unwrap();
            trap_frame.rax = curr_proc.pid as usize;
        }
        SysCallKind::Wait => {
            // block/sleep until process exits?
            // more useful when we build a shell
            unimplemented!()
        }
        SysCallKind::AllocBackBuffer => {
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
            // not seeing any prints after here???
            let mut mapper = unsafe {
                OffsetPageTable::new(
                    curr_proc.page_table,
                    VirtAddr::new(boot_info.physical_memory_offset),
                )
            };
            let buffer_num_bytes = boot_info.screen.bytes_per_line * boot_info.screen.height;

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

                let frame_ptr: *mut u8 = VirtAddr::new(
                    frame.start_address().as_u64() + boot_info.physical_memory_offset,
                )
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
            user_window_info.width = boot_info.screen.width;
            user_window_info.height = boot_info.screen.height;
            user_window_info.bytes_per_pixel = boot_info.screen.bytes_per_pixel;
            user_window_info.bytes_per_line = boot_info.screen.bytes_per_line;
        }
        SysCallKind::GraphicsFrameReady => {
            // we notify compositor that we are ready for a paint
            let mut scheduler = SCHEDULER.lock();
            scheduler.unblock_task(3);
        }
        SysCallKind::Sleep => {
            // arg1: ms
            nano_sleep((arg1 * 1_000_000) as u64);
        }
        SysCallKind::Alloc => {
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
                    let frame_ptr: *mut u8 = VirtAddr::new(
                        frame.start_address().as_u64() + boot_info.physical_memory_offset,
                    )
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
            trap_frame.rax = old_heap_end.as_u64() as usize;
        }
        SysCallKind::GetUnixTime => {
            let elapsed_sec = ELAPSED
                .load(core::sync::atomic::Ordering::Relaxed)
                .div_ceil(1000) as usize;
            let boot_unix = BOOT_RTC.try_get().unwrap().as_unix_timestamp();
            serial_println!("{}", boot_unix + elapsed_sec);
            trap_frame.rax = boot_unix + elapsed_sec;
        }
    }
}

pub fn spawn_proc(program: &'static CStr, argv: *const *const core::ffi::c_char, argc: usize) {
    let binary = match program.to_str().unwrap() {
        "echo" => ECHO_ELF,
        "xiangqi" => XIANGQI_ELF,
        _ => {
            panic!("unrecognized program")
        }
    };
    let proc = ProcessControlBlock::from_bytes(binary, argv, argc, program);
    let id = proc.tcb.lock().id;
    let tcb_clone = proc.tcb.clone();

    serial_println!("{}", proc.tcb.lock().id);

    PROC.get().unwrap().lock().push(proc);

    let mut scheduler = SCHEDULER.lock();
    scheduler.threads.push(tcb_clone);
    scheduler.ready_queue.push_back(id);
}

extern "x86-interrupt" fn gpf_handler(stack_frame: InterruptStackFrame, error_code: u64) {
    panic!(
        "EXCEPTION: GENERAL PROTECTION FAULT\nError Code: {:#x}\n{:#?}",
        error_code, stack_frame
    );
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    serial_println!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    panic!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
    PS2 = PIC_1_OFFSET + 12,
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }

    fn as_usize(self) -> usize {
        usize::from(self.as_u8())
    }
}

static BOOT_RTC: OnceCell<RTCTime> = OnceCell::uninit();
pub static ELAPSED: AtomicU64 = AtomicU64::new(0);
pub const TIME_SLICE: usize = 100 * 1_000_000;
pub const TIME_BETWEEN_TICKS: usize = 1 * 1_000_000;

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    if !BOOT_RTC.is_initialized() {
        BOOT_RTC.init_once(|| get_rtc_time());
    }
    // 1 ms passed by
    let curr_time = ELAPSED.fetch_add(1, core::sync::atomic::Ordering::Relaxed) + 1;
    let curr_time_ns = curr_time * 1_000_000;

    {
        let mut scheduler = SCHEDULER.lock();
        let mut to_wake = [0u64; 15];
        let mut count = 0;
        for thread in scheduler.threads.iter() {
            let thread = thread.lock();
            if let ThreadState::Blocked(BlockReason::Sleep(expire_time)) = thread.state {
                if expire_time <= curr_time_ns {
                    to_wake[count] = thread.id;
                    count += 1;
                }
            }
        }
        for i in 0..count {
            scheduler.unblock_task(to_wake[i]);
        }

        let curr_thread = unsafe { &mut *CURR_THREAD_PTR };
        if curr_thread.id != 1 {
            if curr_thread.time_slice_remaining <= TIME_BETWEEN_TICKS {
                curr_thread.time_slice_remaining = TIME_SLICE;
                curr_thread.state = ThreadState::Ready;
                scheduler.ready_queue.push_back(curr_thread.id);
                NEEDS_RESCHEDULE.store(true, core::sync::atomic::Ordering::SeqCst);
            } else {
                curr_thread.time_slice_remaining -= TIME_BETWEEN_TICKS;
            }
        } else {
            if !scheduler.ready_queue.is_empty() {
                NEEDS_RESCHEDULE.store(true, core::sync::atomic::Ordering::SeqCst);
            }
        }
    }

    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    }

    switch_if_needed();
}

enum MouseDataState {
    WaitingForByte1,
    WaitingForByte2(u8),
    WaitingForByte3(u8, u8),
}

static mut ps2_mouse_state: MouseDataState = MouseDataState::WaitingForByte1;

extern "x86-interrupt" fn mouse_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // i'm just lazy rn will refactor this massive unsafe block later
    unsafe {
        let mut status_port = Port::<u8>::new(0x64);
        let mut status = status_port.read();
        while status & 0x1 != 0 {
            let mut data_port = Port::<u8>::new(0x60);
            let mouse_in = data_port.read();
            if status & (1 << 5) != 0 {
                match ps2_mouse_state {
                    MouseDataState::WaitingForByte1 => {
                        ps2_mouse_state = MouseDataState::WaitingForByte2(mouse_in);
                    }
                    MouseDataState::WaitingForByte2(first_byte) => {
                        ps2_mouse_state = MouseDataState::WaitingForByte3(first_byte, mouse_in);
                    }
                    MouseDataState::WaitingForByte3(first_byte, second_byte) => {
                        // we have the full data now
                        let packet = [first_byte, second_byte, mouse_in];
                        let packet = GenericPs2Packet::new(packet);

                        // *   The top two bits of the first byte (values 0x80 and 0x40) supposedly show Y and X overflows,
                        if first_byte & 0x80 != 0 || first_byte & 0x40 != 0 {
                            // discard the packet
                        } else {
                            // send packet to ring buffer
                            crate::task::mouse::add_packet(packet);
                        }

                        /* Bit number 4 of the first byte (value 0x10) indicates that delta X (the 2nd byte) is a negative number, if it is set. This means that you should OR 0xFFFFFF00 onto the value of delta X, as a sign extension (if using 32 bits).
                        The bottom 3 bits of the first byte indicate whether the middle, right, or left mouse buttons are currently being held down, if the respective bit is set. Middle = bit 2 (value=4), right = bit 1 (value=2), left = bit 0 (value=1). */
                        ps2_mouse_state = MouseDataState::WaitingForByte1;
                    }
                }
            }
            status = status_port.read();
        }
    }

    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::PS2.as_u8());
    }
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    use pc_keyboard::{layouts, DecodedKey, HandleControl, Keyboard, ScancodeSet1};
    use spin::Mutex;
    use x86_64::instructions::port::Port;

    lazy_static! {
        static ref KEYBOARD: Mutex<Keyboard<layouts::Us104Key, ScancodeSet1>> =
            Mutex::new(Keyboard::new(
                ScancodeSet1::new(),
                layouts::Us104Key,
                HandleControl::Ignore
            ));
    }

    let mut keyboard = KEYBOARD.lock();
    let mut port = Port::<u8>::new(0x60);

    let scancode: u8 = unsafe { port.read() };
    crate::task::keyboard::add_scancode(scancode);

    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    use x86_64::registers::control::Cr2;

    serial_println!("EXCEPTION: PAGE FAULT");

    serial_println!(
        r#"EXCEPTION: PAGE FAULT
                    Accessed Address: {:?}
                    Error Code: {:?}
                    {:#?}"#,
        Cr2::read(),
        error_code,
        stack_frame
    );

    hlt_loop();
}

pub fn init_idt() {
    IDT.load();
}
