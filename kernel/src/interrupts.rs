use crate::driver::cmos::get_rtc_time;
use crate::driver::cmos::RTCTime;
use crate::driver::mouse::GenericPs2Packet;
use crate::hlt_loop;
use crate::lapic::mycpu;
use crate::lapic::CPU;
use crate::lapic::IOAPIC;
use crate::lapic::LAPIC;
use crate::print;
use crate::println;
use crate::random::mix_entropy;
use crate::serial_print;
use crate::serial_println;
use crate::spinlock::Spinlock;
use crate::syscall::syscall_entry;
use crate::task::proc::ProcessControlBlock;
use crate::task::proc::XIANGQI_ELF;
use crate::task::scheduler::switch_if_needed;
use crate::task::scheduler::SCHEDULER;
use crate::task::thread::BlockReason;
use crate::task::thread::ThreadControlBlock;
use crate::task::thread::ThreadState;
use alloc::vec::Vec;
use conquer_once::spin::OnceCell;
use core::arch::asm;
use core::arch::naked_asm;
use core::ffi::CStr;
use core::num;
use core::ptr;
use core::sync::atomic::AtomicU64;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;
use lazy_static::lazy_static;
use x86_64::instructions::port::Port;
use x86_64::structures::idt::PageFaultErrorCode;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};
use x86_64::VirtAddr;

pub static BOOT_RTC: OnceCell<RTCTime> = OnceCell::uninit();
pub static ELAPSED: AtomicU64 = AtomicU64::new(0);
pub const TIME_SLICE: usize = 100 * 1_000_000;
pub const TIME_BETWEEN_TICKS: usize = 1 * 1_000_000;

pub const PIC_1_OFFSET: u8 = 32;

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard = PIC_1_OFFSET + 1,
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

pub fn init_ioapic_legacy() {
    IOAPIC.get().unwrap().init();
    let ioapic = IOAPIC.get().unwrap();
    let cpus = CPU.lock();

    let irq_mapping = [
        // (0, InterruptIndex::Timer.as_u8()),
        (1, InterruptIndex::Keyboard.as_u8()),
        (12, InterruptIndex::PS2.as_u8()),
    ];

    let mut index = 0;
    for &(irq, vector) in &irq_mapping {
        ioapic.enable(irq as u32, vector as u32, cpus[index] as u32);
        index = (index + 1) % cpus.len();
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
        idt.page_fault.set_handler_fn(page_fault_handler);
        idt.general_protection_fault.set_handler_fn(gpf_handler);

        idt[InterruptIndex::Timer.as_usize()].set_handler_fn(timer_interrupt_handler);
        idt[InterruptIndex::Keyboard.as_usize()].set_handler_fn(keyboard_interrupt_handler);
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

// We want to use the ticks counter as a rough timer, but we don't know whether all the CPU timers will be synchronized, so we'll only update ticks using the first CPU to avoid those issues.

extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    mix_entropy();
    

    let cpu = mycpu();
    if cpu.apic_id == 0 {
        if !BOOT_RTC.is_initialized() {
            BOOT_RTC.init_once(|| get_rtc_time());
        }
        // 1 ms passed byf
        let curr_time = ELAPSED.fetch_add(1, Ordering::Relaxed) + 1;
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
    }

    let curr_thread = unsafe { &mut *(cpu.curr_thread.load(Ordering::SeqCst)) };
    if curr_thread.id != 1 {
        if curr_thread.time_slice_remaining <= TIME_BETWEEN_TICKS {
            curr_thread.time_slice_remaining = TIME_SLICE;
            curr_thread.state = ThreadState::Ready;
            cpu.ready_queue.lock().push_back(curr_thread.id);
                cpu.needs_resched.store(true, Ordering::SeqCst);
            } else {
                curr_thread.time_slice_remaining -= TIME_BETWEEN_TICKS;
            }
        } else {
            if !cpu.ready_queue.lock().is_empty() {
                cpu.needs_resched.store(true, Ordering::SeqCst);
            }
        }
    }

    eoc();
    switch_if_needed();
}

fn eoc() {
    LAPIC.get().unwrap().ack();
}

enum MouseDataState {
    WaitingForByte1,
    WaitingForByte2(u8),
    WaitingForByte3(u8, u8),
}

static mut ps2_mouse_state: MouseDataState = MouseDataState::WaitingForByte1;

extern "x86-interrupt" fn mouse_interrupt_handler(_stack_frame: InterruptStackFrame) {
    mix_entropy();
    unsafe {
        let mut status_port = Port::<u8>::new(0x64);
        let mut status = status_port.read();
        while status & 0x1 != 0 {
            let mut data_port = Port::<u8>::new(0x60);
            let data = data_port.read();
            handle_ps2_byte(status, data);
            status = status_port.read();
        }
    }
    eoc();
    switch_if_needed();
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    mix_entropy();
    unsafe {
        let mut status_port = Port::<u8>::new(0x64);
        let mut status = status_port.read();
        while status & 0x1 != 0 {
            let mut data_port = Port::<u8>::new(0x60);
            let data = data_port.read();
            handle_ps2_byte(status, data);
            status = status_port.read();
        }
    }
    eoc();
    switch_if_needed();
}

fn handle_ps2_byte(status: u8, data: u8) {
    if status & (1 << 5) != 0 {
        unsafe {
            match ps2_mouse_state {
                MouseDataState::WaitingForByte1 => {
                    ps2_mouse_state = MouseDataState::WaitingForByte2(data);
                }
                MouseDataState::WaitingForByte2(first_byte) => {
                    ps2_mouse_state = MouseDataState::WaitingForByte3(first_byte, data);
                }
                MouseDataState::WaitingForByte3(first_byte, second_byte) => {
                    let packet = [first_byte, second_byte, data];
                    let packet = GenericPs2Packet::new(packet);

                    if first_byte & 0x80 != 0 || first_byte & 0x40 != 0 {
                        // discard the packet
                    } else {
                        crate::task::mouse::add_packet(packet);
                    }
                    ps2_mouse_state = MouseDataState::WaitingForByte1;
                }
            }
        }
    } else {
        crate::task::keyboard::add_scancode(data);
    }
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    use x86_64::registers::control::Cr2;

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
