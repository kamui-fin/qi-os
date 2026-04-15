#![feature(abi_x86_interrupt)]
#![no_std]
#![no_main]

use alloc::vec::Vec;
use conquer_once::spin::OnceCell;
use crossbeam_queue::ArrayQueue;
use spin::Mutex;
use x86_64::{
    structures::paging::{frame::PhysFrameRangeInclusive, OffsetPageTable, Size2MiB},
    VirtAddr,
};

use crate::{graphics::Screen, memory::BootInfoFrameAllocator, proc::ProcessControlBlock};

extern crate alloc;

pub mod allocator;
pub mod gdt;
pub mod graphics;
pub mod interrupts;
pub mod lock;
pub mod memory;
pub mod pit;
pub mod proc;
pub mod serial;
pub mod task;
pub mod thread;
pub mod vga_buffer;

pub static BOOT_INFO: OnceCell<Mutex<&'static mut BootInfo>> = OnceCell::uninit();
pub static PROC: OnceCell<Mutex<Vec<ProcessControlBlock>>> = OnceCell::uninit();

#[repr(C)]
#[derive(Debug)]
pub struct BootInfo<'a> {
    pub screen: &'a mut Screen,
    pub allocator: BootInfoFrameAllocator,
    pub page_table: OffsetPageTable<'a>,
    pub physical_memory_offset: u64,
    pub kernel_base_virt: VirtAddr,
    pub kernel_frame_range: PhysFrameRangeInclusive<Size2MiB>,
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
