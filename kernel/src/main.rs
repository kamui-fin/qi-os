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
    sched, switch_to_task, Scheduler, ThreadControlBlock, CURR_THREAD_PTR, MAIN_THREAD, SCHEDULER,
};
use kernel::{allocator, hlt_loop, init, memory, println, serial, serial_println};
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

    kernel::pit::init_pit();

    let style = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);
    Text::new("Hello Rust!", Point::new(20, 30), style).draw(boot_info.screen);

    let mut mapper = unsafe { memory::init(VirtAddr::new(boot_info.physical_memory_offset)) };
    allocator::init_heap(&mut mapper, &mut boot_info.allocator)
        .expect("heap initialization failed");

    unsafe {
        MAIN_THREAD = Box::into_raw(Box::new(ThreadControlBlock::kmain()));
        CURR_THREAD_PTR = MAIN_THREAD as *mut ThreadControlBlock;
    }

    // idle tasks always have ID 1
    let idle_task = Box::new(ThreadControlBlock::new(1, hlt_loop as *const ()));
    let cleaner_task = Box::new(ThreadControlBlock::new(2, cleaner_task as *const ()));

    /* let mut scheduler = Scheduler::new();
    scheduler.spawn("Second thread", thread_func as *const ());
    loop {
        scheduler.lock();
        scheduler.schedule();
        scheduler.unlock();
    } */

    hlt_loop();
}

fn cleaner_task() {
    loop {
        let scheduler = SCHEDULER.lock();
        // TODO: 
    }
}
/*
* void cleaner_task(void) {
    thread_control_block *task;

    lock_stuff();

    while(terminated_task_list != NULL) {
        task = terminated_task_list;
        terminated_task_list = task->next;
        cleanup_terminated_task(task);
    }

    block_task(PAUSED);
    unlock_stuff();
}

void cleanup_terminated_task(thread_control_block * task) {
        kfree(task->kernel_stack_top - KERNEL_STACK_SIZE);
        kfree(task);
}
*/

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
