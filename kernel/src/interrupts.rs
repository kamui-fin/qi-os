use core::arch::asm;
use core::sync::atomic::AtomicU64;
use core::sync::atomic::AtomicUsize;

use crate::hlt_loop;
use crate::lock::NEEDS_RESCHEDULE;
use crate::print;
use crate::println;
use crate::serial_print;
use crate::serial_println;
use crate::thread::switch_if_needed;
use crate::thread::BlockReason;
use crate::thread::ThreadControlBlock;
use crate::thread::ThreadState;
use crate::thread::CURR_THREAD_PTR;
use crate::thread::SCHEDULER;
use alloc::boxed::Box;
use futures_util::stream::select_with_strategy;
use x86_64::structures::idt::PageFaultErrorCode;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};

use lazy_static::lazy_static;
use pic8259::ChainedPics;
use spin;

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
        idt[0x80]
            .set_handler_fn(syscall_handler)
            .set_privilege_level(x86_64::PrivilegeLevel::Ring3);
        idt
    };
}

/* LETS IMPLEMENT SYSCALLS NOW!
*
* INT NO: 0x80
* Syscall number in RAX
*   RDI	arg1
    RSI	arg2
    RDX	arg3
    R10	arg4
    R8	arg5
    R9	arg6
* Return value RAX
*
*/

enum SysCallKind {
    Debug,
    Exit,
}

impl From<usize> for SysCallKind {
    fn from(value: usize) -> Self {
        match value {
            0x0 => Self::Debug,
            0x1 => Self::Exit,
            _ => Self::Exit,
        }
    }
}

extern "x86-interrupt" fn syscall_handler(stack_frame: InterruptStackFrame) {
    let kind: usize;
    let arg1: usize;
    let arg2: usize;
    let arg3: usize;
    let arg4: usize;
    let arg5: usize;
    let arg6: usize;
    unsafe {
        asm!("mov {0}, rax", out(reg) kind);
        asm!("mov {0}, rdi", out(reg) arg1);
        asm!("mov {0}, rsi", out(reg) arg2);
        asm!("mov {0}, rdx", out(reg) arg3);
        asm!("mov {0}, r10", out(reg) arg4);
        asm!("mov {0}, r8", out(reg) arg5);
        asm!("mov {0}, r9", out(reg) arg6);
    }

    let kind = SysCallKind::from(kind);
    match kind {
        SysCallKind::Debug => {
            // print string pointed to by arg1
        }
        SysCallKind::Exit => {}
    }
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
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }

    fn as_usize(self) -> usize {
        usize::from(self.as_u8())
    }
}

pub static ELAPSED: AtomicU64 = AtomicU64::new(0);
pub const TIME_SLICE: usize = 100 * 1_000_000;
pub const TIME_BETWEEN_TICKS: usize = 1 * 1_000_000;

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    // 1 ms passed by
    let curr_time = ELAPSED.fetch_add(1, core::sync::atomic::Ordering::Relaxed) + 1;
    let curr_time_ns = curr_time * 1_000_000;

    {
        let mut scheduler = SCHEDULER.lock();
        let mut to_wake = [0u64; 15];
        let mut count = 0;
        for thread in scheduler.threads.iter() {
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
    let mut port = Port::new(0x60);

    let scancode: u8 = unsafe { port.read() };
    crate::task::keyboard::add_scancode(scancode); // new

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
    serial_println!("Accessed Address: {:?}", Cr2::read());
    serial_println!("Error Code: {:?}", error_code);
    serial_println!("{:#?}", stack_frame);
    hlt_loop();
}

pub fn init_idt() {
    IDT.load();
}
