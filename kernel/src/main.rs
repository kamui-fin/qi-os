#![no_std]
#![no_main]
#![feature(step_trait)]

use alloc::boxed::Box;
use core::arch::asm;
use core::panic::PanicInfo;
use kernel::graphics::Screen;
use kernel::memory::{BootInfoFrameAllocator, MemoryMapEntry, UsedRegion};
use kernel::task::executor::Executor;
use kernel::task::keyboard::print_keypresses;
use kernel::task::Task;
use kernel::{allocator, init, memory, println, serial, serial_println};
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
    screen: &'a Screen,
    allocator: BootInfoFrameAllocator,
    page_table: OffsetPageTable<'a>,
    physical_memory_offset: u64,
}

#[no_mangle]
pub extern "C" fn _start(boot_info: *mut BootInfo) -> ! {
    init();

    serial_println!("Qi OS booted up!\n");
    let boot_info = unsafe { &mut *boot_info };

    let s = boot_info.screen;
    let fb = s.framebuffer as *mut u32; // 32-bit framebuffer
    let width = s.width as usize;
    let height = s.height as usize;
    let pitch = s.bytes_per_line as usize / 4; // pitch in pixels

    for byte in fb.buffer_mut() {
        *byte = 0x90;
    }

    // new: initialize a mapper
    let mut mapper = unsafe { memory::init(VirtAddr::new(boot_info.physical_memory_offset)) };

    let addresses = [
        // the identity-mapped vga buffer page
        0xb8000,
        0xFFFFFFFF80000001,
        0xFFFFFFFF8000A000,
        // virtual address mapped to physical address 0
        boot_info.physical_memory_offset,
    ];

    for &address in &addresses {
        let virt = VirtAddr::new(address);
        // new: use the `mapper.translate_addr` method
        let phys = mapper.translate_addr(virt);
        serial_println!("{:?} -> {:?}", virt, phys);
    }

    allocator::init_heap(&mut mapper, &mut boot_info.allocator)
        .expect("heap initialization failed");

    let x = Box::new(41);
    let y = Box::new(50);
    serial_println!("x = {:?}", x);
    serial_println!("y = {:?}", y);

    let mut executor = Executor::new();
    executor.spawn(Task::new(example_task()));
    executor.spawn(Task::new(print_keypresses()));
    // executor.run();

    // // #[cfg(test)]
    // // test_main();

    serial_println!("It did not crash!");
    kernel::hlt_loop();
}

/// This function is called on panic.
// #[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("{}", info);
    kernel::hlt_loop();
}

async fn async_number() -> u32 {
    42
}

async fn example_task() {
    let number = async_number().await;
    serial_println!("async number: {}", number);
}
