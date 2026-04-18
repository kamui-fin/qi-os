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
use kernel::allocator::init_heap;
use kernel::graphics::Screen;
use kernel::interrupts::spawn_proc;
use kernel::lock::NEEDS_RESCHEDULE;
use kernel::memory::{BumpAllocator, MemoryMapEntry, UsedRegion};
use kernel::proc::{ProcessControlBlock, ECHO_ELF};
use kernel::task::executor::Executor;
use kernel::task::keyboard::print_keypresses;
use kernel::task::mouse::print_mouse_movement;
use kernel::task::tty::{init_console_char_queue, Color, ColorCode, ConsoleStream, ScreenChar};
use kernel::task::Task;
use kernel::thread::{
    block_task, get_time_since_boot, nano_sleep, switch_if_needed, switch_to_task, terminate_task,
    yield_sched, BlockReason, Scheduler, ThreadControlBlock, ThreadState, CURR_THREAD_PTR,
    MAIN_THREAD, SCHEDULER,
};
use kernel::time::get_rtc_time;
use kernel::{
    allocator, hlt_loop, init, memory, mouse, println, serial, serial_print, serial_println,
    BootInfo, RawBootInfo, BOOT_INFO, PROC,
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
        serial_println!("boot_info.screen specs: {:?}", boot_info.screen);

        kernel::pit::init_pit();
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

    {
        let mut scheduler = SCHEDULER.lock();
        scheduler.spawn(2, cleaner_task as *const ());
        // For now, compositor is just a kernel task
        // scheduler.spawn(3, compositor_task as *const ());
        // scheduler.spawn(4, async_executor_task as *const ());
    }

    println!("[ OK ] Started threads + async executor");
    println!("Ready!");

    let args = [c"test".as_ptr()];
    spawn_proc(c"xiangqi", args.as_ptr(), 1);

    hlt_loop();
}

fn async_executor_task() {
    let mut executor = Executor::new();
    // executor.spawn(Task::new(print_keypresses()));
    // executor.spawn(Task::new(print_mouse_movement()));
    // executor.spawn(Task::new(render_tty_buffer()));
    executor.run();
}

const BUFFER_HEIGHT: usize = 40;
const BUFFER_WIDTH: usize = 150;

struct ConsoleRenderer {
    buffer: [[ScreenChar; BUFFER_WIDTH]; BUFFER_HEIGHT],
    column_position: usize,
}

impl ConsoleRenderer {
    fn new() -> Self {
        Self {
            buffer: [[ScreenChar::default(); BUFFER_WIDTH]; BUFFER_HEIGHT],
            column_position: 0,
        }
    }
    fn new_line(&mut self) {
        for row in 1..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                let character = self.buffer[row][col];
                self.buffer[row - 1][col] = character;
            }
        }
        self.clear_row(BUFFER_HEIGHT - 1);
        self.column_position = 0;
    }

    fn clear_row(&mut self, row: usize) {
        for col in 0..BUFFER_WIDTH {
            self.buffer[row][col] = ScreenChar::default();
        }
    }

    pub fn write_byte(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            byte => {
                if self.column_position >= BUFFER_WIDTH {
                    self.new_line();
                }

                let row = BUFFER_HEIGHT - 1;
                let col = self.column_position;

                let default_color = ColorCode::new(Color::Green, Color::Black);
                self.buffer[row][col] = ScreenChar {
                    ascii_character: byte,
                    color_code: default_color,
                };
                self.column_position += 1;
            }
        }
    }

    pub fn paint(&mut self) {
        // holding tihs lock throughout render pass might be bad... isolate out screen lock
        let boot_info = BOOT_INFO.get().unwrap().lock();
        let mut screen = boot_info.screen;

        let line_height = 3;
        let font = &FONT_10X20;

        let style = MonoTextStyleBuilder::new()
            .font(font)
            .text_color(Rgb565::CSS_FOREST_GREEN)
            .background_color(Rgb565::BLACK)
            .build();

        for row in 0..BUFFER_HEIGHT {
            for col in 0..BUFFER_WIDTH {
                let y = row * (font.character_size.height + line_height) as usize;
                let x = col * (font.character_size.width + font.character_spacing) as usize;

                let character = self.buffer[row][col];

                let mut buf = [0u8; 1];
                buf[0] = character.ascii_character;
                let string = core::str::from_utf8(&buf).unwrap_or(" ");

                // serial_println!("{string} at ({y}, {x})");
                Text::new(&string, Point::new(x as i32, y as i32), style)
                    .draw(&mut screen)
                    .unwrap();
            }
        }
    }
}

impl Default for ConsoleRenderer {
    fn default() -> Self {
        Self::new()
    }
}

async fn render_tty_buffer() {
    let mut renderer = ConsoleRenderer::new();
    let mut console_chars = ConsoleStream::new();
    while let Some(char) = console_chars.next().await {
        renderer.write_byte(char.ascii_character);
        // flush rest of queue
        while let Some(Some(char)) = console_chars.next().now_or_never() {
            renderer.write_byte(char.ascii_character);
        }

        renderer.paint();
    }
}

fn compositor_task() {
    // paint wallpaper (z-index 0)
    {
        let boot_info = BOOT_INFO.get().unwrap().lock();
        let mut screen = boot_info.screen;
        // screen.clear(Rgb565::new(40, 40, 40)).unwrap();
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
