#![feature(abi_x86_interrupt)]
#![no_std]
#![no_main]

use alloc::vec::Vec;
use conquer_once::spin::OnceCell;
use crossbeam_queue::ArrayQueue;
use spin::Mutex;
use x86_64::{
    structures::paging::{frame::PhysFrameRangeInclusive, OffsetPageTable, PageTable, Size2MiB},
    VirtAddr,
};

use crate::{graphics::Screen, memory::BumpAllocator, proc::ProcessControlBlock};

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

pub static BOOT_INFO: OnceCell<Mutex<BootInfo>> = OnceCell::uninit();
pub static PROC: OnceCell<Mutex<Vec<ProcessControlBlock>>> = OnceCell::uninit();

#[derive(Debug)]
pub struct BootInfo {
    pub screen: Screen,
    pub allocator: BumpAllocator,
    pub page_table_address: u64,
    pub physical_memory_offset: u64,
    pub kernel_base_virt: u64,
}

#[repr(C)]
#[derive(Debug)]
pub struct RawBootInfo {
    pub screen_phys_addr: u64,
    pub physical_memory_offset: u64,
    pub kernel_base_virt: u64,
    pub kernel_loaded_address: u64,
    pub kernel_size: u64,
    pub kstack_top: u64,
    pub kstack_bottom: u64,
    pub mem_map_phys_addr: u64,
    pub mem_map_entry_count: usize,
    pub l4_table_phys_addr: u64,
    pub free_memory_start_phys: u64,
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
