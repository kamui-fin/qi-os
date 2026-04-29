use core::arch::asm;
use core::arch::naked_asm;
use core::ffi::c_char;
use core::ffi::c_str;
use core::ffi::CStr;
use core::num;
use core::ptr;
use core::sync::atomic::AtomicU64;
use core::sync::atomic::AtomicUsize;

use crate::driver::cmos::get_rtc_time;
use crate::driver::cmos::RTCTime;
use crate::driver::mouse::GenericPs2Packet;
use crate::fs::vfs::get_root_dentry;
use crate::hlt_loop;
use crate::print;
use crate::println;
use crate::serial_print;
use crate::serial_println;
use crate::syscall::syscall_entry;
use crate::task::lock::NEEDS_RESCHEDULE;
use crate::task::proc::ProcessControlBlock;
use crate::task::proc::ECHO_ELF;
use crate::task::proc::XIANGQI_ELF;
use crate::task::thread::nano_sleep;
use crate::task::thread::switch_if_needed;
use crate::task::thread::terminate_task;
use crate::task::thread::BlockReason;
use crate::task::thread::ThreadControlBlock;
use crate::task::thread::ThreadState;
use crate::task::thread::CURR_THREAD_PTR;
use crate::task::thread::SCHEDULER;
use conquer_once::spin::OnceCell;
use lazy_static::lazy_static;
use pic8259::ChainedPics;
use spin;
use x86_64::instructions::port::Port;
use x86_64::structures::idt::PageFaultErrorCode;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};
use x86_64::VirtAddr;

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

pub static PICS: spin::Mutex<ChainedPics> =
    spin::Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

pub static BOOT_RTC: OnceCell<RTCTime> = OnceCell::uninit();
pub static ELAPSED: AtomicU64 = AtomicU64::new(0);
pub const TIME_SLICE: usize = 100 * 1_000_000;
pub const TIME_BETWEEN_TICKS: usize = 1 * 1_000_000;

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

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(crate::mem::gdt::DOUBLE_FAULT_IST_INDEX);
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
