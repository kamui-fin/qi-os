#![feature(abi_x86_interrupt)]
#![no_std]
#![no_main]

use x86_64::{structures::paging::OffsetPageTable, VirtAddr};

use crate::{graphics::Screen, memory::BootInfoFrameAllocator};

extern crate alloc;

pub mod allocator;
pub mod gdt;
pub mod graphics;
pub mod interrupts;
pub mod lock;
pub mod memory;
pub mod pit;
pub mod serial;
pub mod task;
pub mod thread;
pub mod vga_buffer;

#[repr(C)]
#[derive(Debug)]
pub struct BootInfo<'a> {
    pub screen: &'a mut Screen,
    pub allocator: BootInfoFrameAllocator,
    pub page_table: OffsetPageTable<'a>,
    pub physical_memory_offset: u64,
}

pub fn init() {
    serial::init();
    gdt::init();
    interrupts::init_idt();
    unsafe { interrupts::PICS.lock().initialize() };
}

pub fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}
