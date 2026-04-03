#![no_std]
#![no_main]
#![feature(step_trait)]

use alloc::boxed::Box;
use core::arch::asm;
use core::panic::PanicInfo;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    prelude::*,
    text::Text,
};
use kernel::allocator::init_heap;
use kernel::graphics::Screen;
use kernel::memory::{BootInfoFrameAllocator, MemoryMapEntry, UsedRegion};
use kernel::task::executor::Executor;
use kernel::task::keyboard::print_keypresses;
use kernel::task::Task;
use kernel::thread::{
    sched, switch_to_task, Scheduler, ThreadControlBlock, CURR_THREAD_PTR, MAIN_THREAD,
};
use kernel::{allocator, init, memory, println, serial, serial_println};
use x86_64::instructions::interrupts::without_interrupts;
use x86_64::instructions::tlb::flush_all;
use x86_64::registers::control::{Cr3, Cr3Flags};
use x86_64::structures::paging::frame::{self, PhysFrameRangeInclusive};
use x86_64::structures::paging::{
    page, FrameAllocator, Mapper, OffsetPageTable, Page, PageSize, PageTable, PageTableFlags,
    PhysFrame, Size2MiB, Size4KiB, Translate,
};
use x86_64::{PhysAddr, VirtAddr};

extern crate alloc;

/// Start address of the first frame that is not part of the lower 1MB of frames

#[repr(C)]
#[derive(Debug)]
pub struct BootInfo<'a> {
    screen: &'a mut Screen,
    allocator: BootInfoFrameAllocator,
    page_table: OffsetPageTable<'a>,
    physical_memory_offset: u64,
}

#[no_mangle]
pub extern "C" fn _start(boot_info: *mut BootInfo) -> ! {
    init();

    serial_println!("Qi OS booted up!\n");
    let boot_info = unsafe { &mut *boot_info };
    serial_println!("boot_info.screen specs: {:?}", boot_info.screen);

    // Create a new character style
    let style = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);

    // Create a text at position (20, 30) and draw it using the previously defined style
    Text::new("Hello Rust!", Point::new(20, 30), style).draw(boot_info.screen);

    let mut mapper = unsafe { memory::init(VirtAddr::new(boot_info.physical_memory_offset)) };
    allocator::init_heap(&mut mapper, &mut boot_info.allocator)
        .expect("heap initialization failed");

    unsafe {
        MAIN_THREAD = Box::into_raw(Box::new(ThreadControlBlock::kmain()));
        CURR_THREAD_PTR = MAIN_THREAD as *mut ThreadControlBlock;
    }

    let mut scheduler = Scheduler::new();

    // spawn some threads
    scheduler.spawn("Second thread", thread_func as *const ());

    loop {
        scheduler.lock();
        scheduler.schedule();
        scheduler.unlock();
    }
}

fn thread_func() {
    loop {
        serial_println!("********* YOOO IM THE GUY FROM THREAD TWO!!! **********");
        sched();
    }
}

/// This function is called on panic.
// #[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("{}", info);
    kernel::hlt_loop();
}
