#![feature(abi_x86_interrupt)]
#![no_std]
#![no_main]

use crate::spinlock::Spinlock;
use alloc::{sync::Arc, vec::Vec};
use conquer_once::spin::OnceCell;
use crossbeam_queue::ArrayQueue;
use lazy_static::lazy_static;
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

pub mod acpi;
pub mod console;
pub mod driver;
pub mod fs;
pub mod graphics;
pub mod interrupts;
pub mod lapic;
pub mod mem;
pub mod random;
pub mod rwlock;
pub mod spinlock;
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

pub const BOOT_ASCII_ART: &'static str = r#"
        $$\   $$\ $$\                                $$$$$$\  $$\        $$$$$$\   $$$$$$\
        $$ |  $$ |\__|                              $$  __$$\ \__|      $$  __$$\ $$  __$$\
        \$$\ $$  |$$\  $$$$$$\  $$$$$$$\   $$$$$$\  $$ /  $$ |$$\       $$ /  $$ |$$ /  \__|
         \$$$$  / $$ | \____$$\ $$  __$$\ $$  __$$\ $$ |  $$ |$$ |      $$ |  $$ |\$$$$$$\
         $$  $$<  $$ | $$$$$$$ |$$ |  $$ |$$ /  $$ |$$ |  $$ |$$ |      $$ |  $$ | \____$$\
        $$  /\$$\ $$ |$$  __$$ |$$ |  $$ |$$ |  $$ |$$ $$\$$ |$$ |      $$ |  $$ |$$\   $$ |
        $$ /  $$ |$$ |\$$$$$$$ |$$ |  $$ |\$$$$$$$ |\$$$$$$ / $$ |       $$$$$$  |\$$$$$$  |
        \__|  \__|\__| \_______|\__|  \__| \____$$ | \___$$$\ \__|       \______/  \______/
                                          $$\   $$ |     \___|
                                          \$$$$$$  |
                                           \______/
                "#;

pub static SCREEN: OnceCell<Spinlock<Screen>> = OnceCell::uninit();

pub static PROC: OnceCell<Spinlock<Vec<Arc<Spinlock<ProcessControlBlock>>>>> = OnceCell::uninit();

// immutable
pub static KERNEL_CONFIG: OnceCell<KernelInfo> = OnceCell::uninit();

// isolating out more Spinlock resources
pub static ALLOC: OnceCell<Spinlock<BumpAllocator>> = OnceCell::uninit();
pub const PHYS_MEM_OFFSET: u64 = 0xFFFF_8000_0000_0000;
pub const KERN_BASE_VIRT: u64 = 0xFFFFFFFF80000000;

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
pub struct KernelInfo {
    pub page_table_address: u64,
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

pub fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}
