#![no_std]
#![no_main]
#![feature(step_trait)]

use alloc::boxed::Box;
use alloc::vec::Vec;
use alloc::{task, vec};
use conquer_once::spin::OnceCell;
use core::arch::asm;
use core::ffi::c_char;
use core::panic::PanicInfo;
use crossbeam_queue::ArrayQueue;
use elf::abi::PT_LOAD;
use elf::endian::{AnyEndian, LittleEndian};
use elf::ElfBytes;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    prelude::*,
    text::Text,
};
use kernel::allocator::init_heap;
use kernel::graphics::Screen;
use kernel::interrupts::spawn_proc;
use kernel::lock::NEEDS_RESCHEDULE;
use kernel::memory::{BootInfoFrameAllocator, MemoryMapEntry, UsedRegion};
use kernel::proc::{ProcessControlBlock, ECHO_ELF};
use kernel::task::executor::Executor;
use kernel::task::keyboard::print_keypresses;
use kernel::task::Task;
use kernel::thread::{
    block_task, get_time_since_boot, nano_sleep, switch_if_needed, switch_to_task, terminate_task,
    yield_sched, BlockReason, Scheduler, ThreadControlBlock, ThreadState, CURR_THREAD_PTR,
    MAIN_THREAD, SCHEDULER,
};
use kernel::{
    allocator, hlt_loop, init, memory, println, serial, serial_print, serial_println, BootInfo,
    BOOT_INFO, PROC,
};
use spin::Mutex;
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

#[no_mangle]
pub extern "C" fn _start(boot_info: *mut BootInfo<'static>) -> ! {
    init();

    let boot_info: &'static mut BootInfo = unsafe { &mut *boot_info };
    BOOT_INFO.init_once(|| Mutex::new(boot_info));

    {
        let mut boot_info = BOOT_INFO.get().expect("Boot info not initialized").lock();

        let mut mapper = unsafe { memory::init(VirtAddr::new(boot_info.physical_memory_offset)) };
        allocator::init_heap(&mut mapper, &mut boot_info.allocator)
            .expect("heap initialization failed");

        serial_println!("Qi OS booted up!\n");
        serial_println!("boot_info.screen specs: {:?}", boot_info.screen);

        kernel::pit::init_pit();

        unsafe {
            MAIN_THREAD = Box::into_raw(Box::new(ThreadControlBlock::kmain()));
            CURR_THREAD_PTR = MAIN_THREAD;
        }

        x86_64::instructions::interrupts::enable();

        let style = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);
        Text::new("Hello Rust!", Point::new(20, 30), style)
            .draw(boot_info.screen)
            .unwrap();
    }

    PROC.init_once(|| Mutex::new(Vec::<ProcessControlBlock>::with_capacity(15)));

    {
        let mut scheduler = SCHEDULER.lock();
        scheduler.spawn(2, cleaner_task as *const ());
    }

    let args = [
        c"hello".as_ptr(),
        c"world".as_ptr(),
        c"good".as_ptr(),
        c"bye".as_ptr(),
    ];

        spawn_proc(c"echo", args.as_ptr(), 4);

    hlt_loop();
}

fn cleaner_task() {
    loop {
        let removed_task = {
            let mut scheduler = SCHEDULER.lock();
            let task_index = scheduler.threads.iter().position(|t| {
                if let ThreadState::Blocked(kernel::thread::BlockReason::Terminated(_)) =
                    t.lock().state
                {
                    true
                } else {
                    false
                }
            });
            if let Some(task_index) = task_index {
                Some(scheduler.threads.remove(task_index))
            } else {
                None
            }
        };

        // block itself if queue is empty
        if removed_task.is_none() {
            block_task(BlockReason::Paused);
        }

        // Why is rust so goated?
        // After this scope, the task's stack and the TCB itself will automatically be dropped
        // due to Box<T>!
        // No manual kfree required!
    }
}

/// This function is called on panic.
// #[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("kpanic: {}", info);
    kernel::hlt_loop();
}
