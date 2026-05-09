#![feature(abi_x86_interrupt)]
#![no_std]
#![no_main]

use alloc::{sync::Arc, vec::Vec};
use conquer_once::spin::OnceCell;
use crossbeam_queue::ArrayQueue;
use lazy_static::lazy_static;
use spin::Mutex;
use x86_64::{
    structures::paging::{frame::PhysFrameRangeInclusive, OffsetPageTable, PageTable, Size2MiB},
    VirtAddr,
};

use crate::{
    graphics::{BootScreenInfo, Screen},
    mem::memory::BumpAllocator,
    task::proc::ProcessControlBlock,
};

extern crate alloc;

pub mod console;
pub mod driver;
pub mod fs;
pub mod graphics;
pub mod interrupts;
pub mod mem;
pub mod random;
pub mod syscall;
pub mod task;
pub mod tty;

lazy_static! {
    pub static ref UNAME: UtsName = UtsName {
        sysname: "XiangQi OS",
        nodename: "qi-box",
        release: "unreleased",
        version: "0.1 (in dev)",
        machine: "x86_64",
    };
}

pub static BOOT_INFO: OnceCell<Mutex<BootInfo>> = OnceCell::uninit();
pub static SCREEN: OnceCell<Mutex<Screen>> = OnceCell::uninit();
pub static PROC: OnceCell<Mutex<Vec<ProcessControlBlock>>> = OnceCell::uninit();

// nodename - hardcoded due to lack of network support
// machine - hardcoded due to only support for x86_64
#[derive(Debug)]
pub struct UtsName {
    pub sysname: &'static str,
    pub nodename: &'static str,
    pub release: &'static str,
    pub version: &'static str,
    pub machine: &'static str,
}

#[derive(Debug)]
pub struct BootInfo {
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
    crate::driver::serial::init();
    crate::mem::gdt::init();
    crate::interrupts::init_idt();
    unsafe { crate::interrupts::PICS.lock().initialize() };
}

pub fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}
