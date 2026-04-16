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
use kernel::memory::{BumpAllocator, MemoryMapEntry, UsedRegion};
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
    RawBootInfo, BOOT_INFO, PROC,
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
pub extern "C" fn _start(boot_info: *mut RawBootInfo) -> ! {
    init();

    // 1. Get the BootInfo struct
    let boot_info = unsafe { &*(boot_info as *const RawBootInfo) };
    serial_println!("{:#?}", boot_info);
    let phys_offset = boot_info.physical_memory_offset;
    let screen_virt = boot_info.screen_phys_addr + phys_offset;
    let mut screen = unsafe { (*(screen_virt as *const Screen)).clone() };

    let mem_map_virt = boot_info.mem_map_phys_addr + phys_offset;
    let mem_map: &'static mut [MemoryMapEntry] = unsafe {
        core::slice::from_raw_parts_mut(
            mem_map_virt as *mut MemoryMapEntry,
            boot_info.mem_map_entry_count,
        )
    };
    let allocator = BumpAllocator::starts_at(
        boot_info.free_memory_start_phys,
        mem_map,
        UsedRegion {
            start_address: PhysAddr::new(boot_info.kernel_loaded_address),
            size: boot_info.kernel_size,
        },
    );

    let boot_info = BootInfo {
        screen,
        allocator,
        page_table_address: boot_info.l4_table_phys_addr,
        physical_memory_offset: phys_offset,
        kernel_base_virt: boot_info.kernel_base_virt,
    };

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
    }

    PROC.init_once(|| Mutex::new(Vec::<ProcessControlBlock>::with_capacity(15)));

    {
        let mut scheduler = SCHEDULER.lock();
        scheduler.spawn(2, cleaner_task as *const ());
        // For now, compositor is just a kernel task
        scheduler.spawn(3, compositor_task as *const ());
    }

    let args = [c"test".as_ptr()];
    spawn_proc(c"xiangqi", args.as_ptr(), 1);

    hlt_loop();
}

fn compositor_task() {
    loop {
        // Wait for a commit_frame() syscall
        serial_println!("[compositor] Going to zzzz..");
        block_task(BlockReason::CompositorWait);

        let procs = PROC.get().unwrap().lock();
        serial_println!("[compositor] Unblocked, going thru {} procs", procs.iter().len());

        // paint each proccess backbuffer, for now there's no z-index
        for curr_proc in procs.iter() {
            serial_println!("{}", curr_proc.backbuffer_frames.is_none());
            if let Some(bb_frames) = &curr_proc.backbuffer_frames {
                serial_println!("Painting frame!");
                let mut boot_info = BOOT_INFO.get().unwrap().lock();
                let lfb_start_ptr = boot_info.screen.buffer_mut().as_mut_ptr();
                let mut bytes_remaining = boot_info.screen.buffer_mut().len();
                for (i, frame) in bb_frames.iter().enumerate() {
                    // copy this physical frame to our LFB
                    let offset = i * 4096;
                    let frame_ptr: *mut u8 = VirtAddr::new(
                        frame.start_address().as_u64() + boot_info.physical_memory_offset,
                    )
                    .as_mut_ptr();
                    let bytes_to_copy = core::cmp::min(4096, bytes_remaining);
                    unsafe {
                        let dst_ptr = lfb_start_ptr.add(offset);
                        core::ptr::copy_nonoverlapping(frame_ptr, dst_ptr, 4096);
                    }
                    bytes_remaining -= bytes_to_copy;
                }
            }
        }
    }
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
