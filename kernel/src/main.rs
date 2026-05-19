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
use kernel::console::{handle_keyboard, init_ttys, listen_console_buffer, CONS};
use kernel::driver::cmos::get_rtc_time;
use kernel::driver::sound::corb::{CorbRespStream, RirbResponseEntry, CORB, HDA_CMD_RESP_QUEUE};
use kernel::driver::sound::hda::{init_pci, CORBWP, HDA};
use kernel::fs::fat::{BlockDevice, FSInfo, Fat32, BPB};
use kernel::fs::ustar::{octascii_to_dec, USTAR};
use kernel::fs::vfs::{get_root_dentry, init_vfs};
use kernel::graphics::{BootScreenInfo, Screen};
use kernel::mem::allocator::init_heap;
use kernel::mem::memory::{BumpAllocator, MemoryMapEntry, UsedRegion};
use kernel::random::{get_rand_range, get_random_number, init_rand};
use kernel::task::executor::Executor;
use kernel::task::lock::NEEDS_RESCHEDULE;
//use kernel::task::mouse::print_mouse_movement;
use kernel::task::proc::{spawn_proc, ProcessControlBlock};
use kernel::task::thread::{
    block_task, get_time_since_boot, nano_sleep, switch_if_needed, switch_to_task, terminate_task,
    yield_sched, BlockReason, Scheduler, ThreadControlBlock, ThreadState, CURR_THREAD_PTR,
    MAIN_THREAD, SCHEDULER,
};
use kernel::task::tty::init_console_char_queue;
use kernel::task::Task;
use kernel::{
    driver::mouse, driver::serial, hlt_loop, init, mem::allocator, mem::memory, println,
    serial_print, serial_println, KernelInfo, RawBootInfo, KERNEL_CONFIG, PROC, SCREEN,
};
use kernel::{ALLOC, PHYS_MEM_OFFSET};
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
    let screen = unsafe { (*(screen_virt as *const BootScreenInfo)).clone() };

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

    let boot_info = KernelInfo {
        page_table_address: boot_info.l4_table_phys_addr,
    };

    KERNEL_CONFIG.init_once(|| boot_info);
    ALLOC.init_once(|| Mutex::new(allocator));

    {
        let mut alloc = ALLOC.get().unwrap().lock();
        let mut mapper = unsafe { memory::init(VirtAddr::new(PHYS_MEM_OFFSET)) };
        allocator::init_heap(&mut mapper, &mut *alloc).expect("heap initialization failed");

        let screen = Screen::new(screen);
        SCREEN.init_once(|| Mutex::new(screen));

        init_console_char_queue();
        init_ttys();

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
        kernel::task::mouse::init_packet_queue();
        println!("[ OK ] PS/2 Mouse initialized");

        unsafe {
            MAIN_THREAD = Box::into_raw(Box::new(ThreadControlBlock::kmain()));
            CURR_THREAD_PTR = MAIN_THREAD;
        }
    }

    PROC.init_once(|| Mutex::new(Vec::<Arc<Mutex<ProcessControlBlock>>>::with_capacity(15)));

    init_rand();
    init_vfs();

    serial_println!("Setup VFS!");

    init_pci();
    serial_println!("Enumerated PCI and initialized Intel HDA!");

    {
        let mut scheduler = SCHEDULER.lock();
        scheduler.spawn(2, cleaner_task as *const ());
        scheduler.spawn(3, compositor_task as *const ());
        scheduler.spawn(4, async_executor_task as *const ());
        scheduler.spawn(5, render_tty_task as *const ());
    }

    println!("[ OK ] Started threads + async executor");
    println!("Ready!");
    println!("========================================================\n");

    x86_64::instructions::interrupts::enable();

    {
        let args = [c"test".as_ptr()];
        spawn_proc(c"shell", args.as_ptr(), 1);
        //spawn_proc(c"printmousemovement",args.as_ptr(),1);
    }

    hlt_loop();
}

async fn hda_qemu_setup() {
    // hda_discovery().await;
    // speakers config
    hda_request_verb_no_res(0u32, 3u32, 0x707u32, 0x40).await; // turn on electricity
    hda_request_verb_no_res(0u32, 3u32, 0x300u32, 0xB000 | 0x40).await; // unmute amp

    // dac config
    hda_request_verb_no_res(0u32, 2u32, 0x300u32, 0xB000 | 0x40).await; // unmute amp
    hda_request_verb_no_res(0u32, 2u32, 0x706u32, 0x10).await; // stream 1 channel 0
    hda_request_verb_no_res(0u32, 2u32, 0x2u32, 0x0011).await; // 48kHz, 16-bit, 2-channels

    without_interrupts(|| {
        let hda = HDA.get().unwrap().lock();
        hda.start_dma();
    });
}

// BIG TODO: This is a WIP. Real HDA drivers should parse a node graph and dynamically select a stream path, checking for capabilities
// main focus atm is js QEMU, so we'll be configuring hardcoded path and stream config instead lmao
async fn hda_discovery() {
    let codec_addr = 0u32;
    let node_id = 0u32;

    // GET_PARAMETER(VENDOR_ID)
    let resp = hda_request_verb(codec_addr, node_id, 0xF00u32, 0u32).await;
    let vendor_id = (resp.raw_response >> 16) as u16;
    let codec_device_id = resp.raw_response as u16;

    serial_println!("vendor id: {:#X}", vendor_id);
    serial_println!("device id: {:#X}", codec_device_id);

    // Get Subordinate Node Count
    let resp = hda_request_verb(codec_addr, node_id, 0xF00u32, 0x04u32).await;
    let start_node = (resp.raw_response >> 16) as u8;
    let total_nodes = resp.raw_response as u8;

    serial_println!(
        "func groups -> start node: {:#?}, total nodes: {:#?}",
        start_node,
        total_nodes
    );

    for fg_node in start_node..(start_node + total_nodes) {
        // node function groups
        let resp = hda_request_verb(codec_addr, fg_node as u32, 0xF00u32, 0x05u32).await;
        let function_group = resp.raw_response as u8;

        serial_println!("func group node {:#?}: type {:#X}", fg_node, function_group);

        // get widgets
        let resp = hda_request_verb(codec_addr, fg_node as u32, 0xF00u32, 0x04u32).await;
        let start_node = (resp.raw_response >> 16) as u8;
        let total_nodes = resp.raw_response as u8;

        serial_println!(
            "  widgets -> start node: {:#?}, total widgets: {:#?}",
            start_node,
            total_nodes
        );

        // for each widget
        for wg_node in start_node..(start_node + total_nodes) {
            // identify widget type
            let resp = hda_request_verb(codec_addr, wg_node as u32, 0xF00u32, 0x09u32).await;
            let widget_type = ((resp.raw_response >> 20) & 0xF) as u8;
            serial_println!("    widget Node {:#?}: type {:#X}", wg_node, widget_type);
            /*
                0x0 = output converter (DAC)
                0x1 = input converter (ADC)
                0x2 = mixer
                0x3 = selector
                0x4 = pin complex
            */
            if widget_type == 0x4 {
                // Pin config default
            }

            // Connection list length
            let resp = hda_request_verb(codec_addr, wg_node as u32, 0xF00u32, 0x0Eu32).await;
            let list_length = (resp.raw_response & 0b0011_1111) as u8;
            let _long_form = ((resp.raw_response >> 7) & 1) as u8;

            if list_length > 0 {
                // connection list entry at index 0
                let resp = hda_request_verb(codec_addr, wg_node as u32, 0xF02u32, 0x00u32).await;
                // nid
                let list_entry = resp.raw_response as u8;
                serial_println!("        connection entry -> {list_entry}");
                // hardcode a path for now
            }
        }
    }
}

async fn hda_request_verb_no_res(codec_addr: u32, node_id: u32, verb: u32, payload: u32) {
    let hda_verb: u32 = (codec_addr << 28) | (node_id << 20) | (verb << 8) | payload;
    let wp = {
        let mut corb = CORB.get().unwrap().lock();
        corb.push(hda_verb);
        corb.wp
    };
    without_interrupts(|| {
        let hda = HDA.get().unwrap().lock();
        hda.write_reg(CORBWP, wp as u16)
    });
}

async fn hda_request_verb(
    codec_addr: u32,
    node_id: u32,
    verb: u32,
    payload: u32,
) -> RirbResponseEntry {
    let hda_verb: u32 = (codec_addr << 28) | (node_id << 20) | (verb << 8) | payload;

    let wp = {
        let mut corb = CORB.get().unwrap().lock();
        corb.push(hda_verb);

        let kern_hda_resp_buffer = HDA_CMD_RESP_QUEUE.get().unwrap();
        kern_hda_resp_buffer.awaiting_req.force_push(hda_verb);

        corb.wp
    };

    without_interrupts(|| {
        let hda = HDA.get().unwrap().lock();
        hda.write_reg(CORBWP, wp as u16)
    });

    let mut stream = CorbRespStream::new();
    stream.next().await.unwrap()
}

fn async_executor_task() {
    let mut executor = Executor::new();
    executor.spawn(Task::new(hda_qemu_setup()));
    //executor.spawn(Task::new(print_mouse_movement()));
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
