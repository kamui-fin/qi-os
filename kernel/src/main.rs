#![no_std]
#![no_main]
#![feature(step_trait)]

use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::vec::Vec;
use alloc::{task, vec};
use conquer_once::spin::OnceCell;
use core::arch::asm;
use core::ffi::c_char;
use core::intrinsics::copy_nonoverlapping;
use core::panic::PanicInfo;
use crossbeam_queue::ArrayQueue;
use elf::abi::PT_LOAD;
use elf::endian::{AnyEndian, LittleEndian};
use elf::ElfBytes;
use embedded_graphics::mono_font::ascii::{FONT_10X20, FONT_8X13};
use embedded_graphics::mono_font::MonoTextStyleBuilder;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    prelude::*,
    text::Text,
};
use futures_util::{FutureExt, StreamExt};
use kernel::console::render_tty_buffer;
use kernel::driver::cmos::get_rtc_time;
use kernel::fs::fat::{BlockDevice, FSInfo, Fat32, BPB};
use kernel::fs::ustar::{octascii_to_dec, USTAR};
use kernel::fs::vfs::{get_root_dentry, init_vfs};
use kernel::graphics::{BootScreenInfo, Screen};
use kernel::mem::allocator::init_heap;
use kernel::mem::memory::{BumpAllocator, MemoryMapEntry, UsedRegion};
use kernel::random::{get_rand_range, get_random_number, init_rand};
use kernel::task::executor::Executor;
use kernel::task::keyboard::print_keypresses;
use kernel::task::lock::NEEDS_RESCHEDULE;
use kernel::task::mouse::print_mouse_movement;
use kernel::task::proc::{ProcessControlBlock, ECHO_ELF};
use kernel::task::thread::{
    block_task, get_time_since_boot, nano_sleep, switch_if_needed, switch_to_task, terminate_task,
    yield_sched, BlockReason, Scheduler, ThreadControlBlock, ThreadState, CURR_THREAD_PTR,
    MAIN_THREAD, SCHEDULER,
};
use kernel::task::tty::{init_console_char_queue, Color, ColorCode, ConsoleStream, ScreenChar};
use kernel::task::Task;
use kernel::{
    driver::mouse, driver::serial, hlt_loop, init, mem::allocator, mem::memory, println,
    serial_print, serial_println, BootInfo, RawBootInfo, BOOT_INFO, PROC, SCREEN,
};
use spin::Mutex;
use volatile::Volatile;
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
    let mut screen = unsafe { (*(screen_virt as *const BootScreenInfo)).clone() };

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

        let screen = Screen::new(screen);
        SCREEN.init_once(|| Mutex::new(screen));

        init_console_char_queue();

        // Xiangqi OS boot message
        println!(
            r#"
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
        "#
        );
        println!("[ OK ] Heap initialized");

        serial_println!("Qi OS booted up!\n");

        kernel::driver::pit::init_pit();
        println!("[ OK ] Timer setup");

        unsafe {
            mouse::init_ps2();
            mouse::init_ps2_mouse();
        }
        println!("[ OK ] PS/2 Mouse initialized");

        unsafe {
            MAIN_THREAD = Box::into_raw(Box::new(ThreadControlBlock::kmain()));
            CURR_THREAD_PTR = MAIN_THREAD;
        }

        x86_64::instructions::interrupts::enable();
    }

    PROC.init_once(|| Mutex::new(Vec::<ProcessControlBlock>::with_capacity(15)));

    init_rand();

    {
        let mut scheduler = SCHEDULER.lock();
        scheduler.spawn(2, cleaner_task as *const ());
        scheduler.spawn(3, compositor_task as *const ());
        scheduler.spawn(4, async_executor_task as *const ());
        scheduler.spawn(5, random_test as *const ());
        /* let args = [c"test".as_ptr()];
        spawn_proc(c"xiangqi", args.as_ptr(), 1); */
    }

    // init_vfs();

    println!("[ OK ] Started threads + async executor");
    println!("Ready!");

    hlt_loop();
}

#[repr(C, packed)]
struct IdtPtr {
    limit: u16,
    base: u64,
}

fn reboot() {
    let idt = IdtPtr { limit: 0, base: 0 };
    unsafe {
        asm!(
            "cli",
            "lidt [{0}]",
            "int3",
            in(reg) &idt,
            options(noreturn)
        );
    }
}

fn random_test() {
    nano_sleep(1_000_000 * 1000);
    for _ in 0..100 {
        serial_print!(" {} ", get_rand_range(1, 30));
    }
    terminate_task(0)
}

fn async_executor_task() {
    let mut executor = Executor::new();
    executor.spawn(Task::new(print_keypresses()));
    executor.spawn(Task::new(print_mouse_movement()));
    // executor.spawn(Task::new(render_tty_buffer()));
    executor.run();
}

fn compositor_task() {
    // paint wallpaper (z-index 0)
    /* {
        let mut screen = SCREEN.get().unwrap().lock();
        screen.clear(Rgb565::new(5, 10, 5)).unwrap();
        screen.flush();
    } */

    loop {
        // Wait for a commit_frame() syscall
        serial_println!("[compositor] Going to zzzz..");
        block_task(BlockReason::CompositorWait);

        let procs = PROC.get().unwrap().lock();
        serial_println!(
            "[compositor] Unblocked, going thru {} procs",
            procs.iter().len()
        );

        // paint each proccess backbuffer, for now there's no z-index
        for curr_proc in procs.iter() {
            serial_println!("{}", curr_proc.backbuffer_frames.is_none());
            if let Some(bb_frames) = &curr_proc.backbuffer_frames {
                serial_println!("Painting frame!");
                let boot_info = BOOT_INFO.get().unwrap().lock();
                let mut screen = SCREEN.get().unwrap().lock();
                let mut bytes_remaining = screen.buffer_mut().len();
                for (i, frame) in bb_frames.iter().enumerate() {
                    // copy this physical frame to our LFB
                    let offset = i * 4096;
                    let frame_ptr: *mut u8 = VirtAddr::new(
                        frame.start_address().as_u64() + boot_info.physical_memory_offset,
                    )
                    .as_mut_ptr();
                    let main_back_buffer = screen.back_lfb.as_mut_ptr();
                    let bytes_to_copy = core::cmp::min(4096, bytes_remaining);
                    unsafe {
                        let dst_ptr = main_back_buffer.add(offset);
                        core::ptr::copy_nonoverlapping(frame_ptr, dst_ptr, 4096);
                    }
                    bytes_remaining -= bytes_to_copy;
                }
            }
        }

        SCREEN.get().unwrap().lock().flush();
    }
}

fn cleaner_task() {
    loop {
        let removed_task = {
            let mut scheduler = SCHEDULER.lock();
            let task_index = scheduler.threads.iter().position(|t| {
                if let ThreadState::Blocked(kernel::task::thread::BlockReason::Terminated(_)) =
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
