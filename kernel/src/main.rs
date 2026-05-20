#![no_std]
#![no_main]
#![feature(step_trait)]

use alloc::boxed::Box;
use alloc::string::ToString;
use alloc::sync::Arc;
use alloc::vec::Vec;
use alloc::{task, vec};
use conquer_once::spin::OnceCell;
use core::arch::asm;
use core::ffi::c_char;
use core::panic::PanicInfo;
use core::ptr::NonNull;
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
use kernel::acpi::{find_rsdp, get_rsdt, SdtHeader};
use kernel::console::{handle_keyboard, init_ttys, listen_console_buffer, CONS};
use kernel::driver::cmos::get_rtc_time;
use kernel::fs::fat::{BlockDevice, FSInfo, Fat32, BPB};
use kernel::fs::ustar::{octascii_to_dec, USTAR};
use kernel::fs::vfs::{get_root_dentry, init_vfs};
use kernel::graphics::{BootScreenInfo, Screen};
use kernel::lapic::{
    cpu_common, find_cpus, init_lapic, ioapic_init, mycpu, pic_disable, start_other_cpus,
};
use kernel::mem::allocator::init_heap;
use kernel::mem::gdt::init_gdt;
use kernel::mem::memory::{BumpAllocator, MemoryMapEntry, UsedRegion};
use kernel::random::{get_rand_range, get_random_number, init_rand};
use kernel::task::executor::Executor;
use kernel::task::lock::NEEDS_RESCHEDULE;
use kernel::task::mouse::print_mouse_movement;
use kernel::task::proc::{spawn_proc, ProcessControlBlock};
use kernel::task::thread::{
    block_task, get_time_since_boot, nano_sleep, switch_if_needed, switch_to_task, terminate_task,
    yield_sched, BlockReason, Scheduler, ThreadControlBlock, ThreadState, CURR_THREAD_PTR,
    MAIN_THREAD, SCHEDULER,
};
use kernel::task::tty::init_console_char_queue;
use kernel::task::Task;
use kernel::{
    driver::mouse, driver::serial, hlt_loop, mem::allocator, mem::memory, println, serial_print,
    serial_println, KernelInfo, RawBootInfo, KERNEL_CONFIG, PROC, SCREEN,
};
use kernel::{interrupts, ALLOC, BOOT_ASCII_ART, PHYS_MEM_OFFSET};
use crate::spinlock::Spinlock;
use volatile::VolatilePtr;
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
    init_gdt(&mycpu().gdt, &mycpu().selectors);

    kernel::driver::serial::init();
    init_boot_info(boot_info);
    init_kheap();

    let (lapic, entries) = find_cpus();
    let bsp = init_lapic(&lapic);
    pic_disable();
    ioapic_init();

    init_screen(boot_info);
    init_console_char_queue();
    init_ttys();

    kernel::driver::pit::init_pit(); // needs fixing?

    mouse::init_ps2();
    mouse::init_ps2_mouse();

    init_kmain();
    init_rand();
    init_vfs();

    start_other_cpus(lapic, bsp);
    start_threads();

    println!("{}", BOOT_ASCII_ART);

    start_init_proc();

    x86_64::instructions::interrupts::enable();
    cpu_common(bsp as u64);
}

fn init_kheap() {
    let mut alloc = ALLOC.get().unwrap().lock();
    let mut mapper = unsafe { memory::init(VirtAddr::new(PHYS_MEM_OFFSET)) };
    allocator::init_heap(&mut mapper, &mut *alloc).expect("heap initialization failed");
}

fn init_kmain() {
    unsafe {
        MAIN_THREAD = Box::into_raw(Box::new(ThreadControlBlock::kmain()));
        CURR_THREAD_PTR = MAIN_THREAD;
    }
}

fn start_threads() {
    let mut scheduler = SCHEDULER.lock();
    scheduler.spawn(2, cleaner_task as *const ());
    scheduler.spawn(3, compositor_task as *const ());
    scheduler.spawn(4, async_executor_task as *const ());
    scheduler.spawn(5, render_tty_task as *const ());
}

fn init_boot_info(boot_info: *mut RawBootInfo) {
    let boot_info = unsafe { &*(boot_info as *const RawBootInfo) };
    let phys_offset = boot_info.physical_memory_offset;

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

    ALLOC.init_once(|| Spinlock::new(allocator));

    let boot_info = KernelInfo {
        page_table_address: boot_info.l4_table_phys_addr,
    };

    KERNEL_CONFIG.init_once(|| boot_info);
}

fn init_screen(boot_info: *mut RawBootInfo) {
    let boot_info = unsafe { &*(boot_info as *const RawBootInfo) };
    let screen_virt = boot_info.screen_phys_addr + boot_info.physical_memory_offset;
    let screen = unsafe { (*(screen_virt as *const BootScreenInfo)).clone() };
    let screen = Screen::new(screen);
    SCREEN.init_once(|| Spinlock::new(screen));
}

fn start_init_proc() {
    PROC.init_once(|| Spinlock::new(Vec::<Arc<Spinlock<ProcessControlBlock>>>::with_capacity(15)));

    let args = [c"test".as_ptr()];
    spawn_proc(c"shell", args.as_ptr(), 1);
}

fn async_executor_task() {
    let mut executor = Executor::new();
    executor.spawn(Task::new(print_mouse_movement()));
    executor.spawn(Task::new(handle_keyboard()));
    executor.spawn(Task::new(listen_console_buffer()));
    executor.run();
}

fn render_tty_task() {
    loop {
        serial_println!("[tty renderer] Going to sleep");
        block_task(BlockReason::TtyRenderWait);
        let mut cons = CONS.get().unwrap().lock();
        cons.paint();
        serial_println!("[tty renderer] Done painting");
    }
}

fn compositor_task() {
    // paint wallpaper (z-index 0)
    {
        let mut screen = SCREEN.get().unwrap().lock();
        screen.clear(Rgb565::new(3, 6, 3)).unwrap();
        screen.flush();
    }

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
            let curr_proc = curr_proc.lock();
            serial_println!("{}", curr_proc.adsp.backbuffer_frames.is_none());
            if let Some(bb_frames) = &curr_proc.adsp.backbuffer_frames {
                serial_println!("Painting frame!");
                let mut screen = SCREEN.get().unwrap().lock();
                let mut bytes_remaining = screen.buffer_mut().len();
                for (i, frame) in bb_frames.iter().enumerate() {
                    // copy this physical frame to our LFB
                    let offset = i * 4096;
                    let frame_ptr: *mut u8 =
                        VirtAddr::new(frame.start_address().as_u64() + PHYS_MEM_OFFSET)
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

/// This function is called on panic.
// #[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("kpanic: {}", info);
    kernel::hlt_loop();
}
