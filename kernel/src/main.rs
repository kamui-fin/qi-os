#![no_std]
#![no_main]
#![feature(step_trait)]

use alloc::boxed::Box;
use core::panic::PanicInfo;
use kernel::graphics::Screen;
use kernel::memory::{BootInfoFrameAllocator, MemoryMapEntry, UsedRegion};
use kernel::task::executor::Executor;
use kernel::task::keyboard::print_keypresses;
use kernel::task::Task;
use kernel::{allocator, memory, println, serial, serial_println};
use x86_64::instructions::tlb::flush_all;
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::frame::{self, PhysFrameRangeInclusive};
use x86_64::structures::paging::{
    page, FrameAllocator, Mapper, OffsetPageTable, Page, PageSize, PageTable, PageTableFlags,
    PhysFrame, Size2MiB, Translate,
};
use x86_64::{PhysAddr, VirtAddr};

extern crate alloc;

/// Start address of the first frame that is not part of the lower 1MB of frames
const LOWER_MEMORY_END_PAGE: u64 = 0x10_0000;

#[no_mangle]
pub extern "C" fn _start(screen: *const Screen, kernel_size_bytes: u64) -> ! {
    serial::init();
    serial_println!("Qi OS booted up!\n");

    let phys_mem_offset = VirtAddr::new(0xFFFF_8000_0000_0000);
    let kernel_base_virt: VirtAddr = VirtAddr::new(0xFFFFFFFF80000000);
    let kernel_base_phys: PhysAddr = PhysAddr::new(0x100000);

    let mem_map: &'static mut [MemoryMapEntry] = unsafe { memory::get_mem_map() };
    mem_map.sort_unstable_by_key(|reg| reg.base_address);

    let mut frame_allocator = BootInfoFrameAllocator::starts_at(
        LOWER_MEMORY_END_PAGE,
        mem_map,
        UsedRegion {
            start_address: kernel_base_phys,
            size: kernel_size_bytes,
        },
    );

    let (mut kernel_page_table, kernel_level_4_frame) = {
        let frame: PhysFrame = frame_allocator.allocate_frame().expect("no usable memory");
        let addr = kernel_base_virt + frame.start_address().as_u64();
        let ptr: *mut PageTable = addr.as_mut_ptr();
        unsafe {
            ptr.write(PageTable::new());
        }
        let level_4_table = unsafe { &mut *ptr };
        (
            unsafe { OffsetPageTable::new(level_4_table, kernel_base_virt) },
            frame,
        )
    };

    /* map high half kernel again */
    let start_frame = PhysFrame::<Size2MiB>::containing_address(kernel_base_phys);
    let end_frame = PhysFrame::<Size2MiB>::containing_address(PhysAddr::new(
        kernel_base_phys.as_u64() + kernel_size_bytes - 1,
    ));
    let frame_range = PhysFrame::range_inclusive(start_frame, end_frame);
    for frame in frame_range {
        let phys_start_addr = frame.start_address();
        let virt_addr = VirtAddr::new(phys_start_addr.as_u64() + kernel_base_virt.as_u64());
        let page = Page::containing_address(virt_addr);
        serial_println!("Mapping {:?} page to {:?} frame", page, frame);
        let mapper_flush = unsafe {
            kernel_page_table
                .map_to(
                    page,
                    frame,
                    PageTableFlags::WRITABLE | PageTableFlags::PRESENT,
                    &mut frame_allocator,
                )
                .expect("(fixed offset mapping): unable to map page")
        };
        mapper_flush.flush();
    }

    // /* map all physical memory at a fixed offset */
    let start_frame = PhysFrame::<Size2MiB>::containing_address(PhysAddr::new(0));
    let last_address = mem_map
        .iter()
        .filter_map(|region| {
            if region.mem_type == 1 {
                Some(region.end_addr())
            } else {
                None
            }
        })
        .last()
        .expect("no usable regions");
    let end_frame = PhysFrame::<Size2MiB>::containing_address(PhysAddr::new(last_address));
    let frame_range = PhysFrame::range_inclusive(start_frame, end_frame);
    for frame in frame_range {
        let phys_start_addr = frame.start_address();
        let virt_addr = VirtAddr::new(phys_start_addr.as_u64() + phys_mem_offset.as_u64());
        let page = Page::containing_address(virt_addr);
        let mapper_flush = unsafe {
            kernel_page_table
                .map_to(
                    page,
                    frame,
                    PageTableFlags::WRITABLE | PageTableFlags::PRESENT,
                    &mut frame_allocator,
                )
                .expect("(fixed offset mapping): unable to map page")
        };
        mapper_flush.ignore();
    }

    // flush_all();

    // let s = unsafe { &*screen };
    // let fb = s.framebuffer as *mut u32; // 32-bit framebuffer
    // let width = s.width as usize;
    // let height = s.height as usize;
    // let pitch = s.bytes_per_line as usize / 4; // pitch in pixels
    // // Draw a simple gradient
    // for y in 0..height {
    //     for x in 0..width {
    //         let pixel = ((x * 255 / width) as u32) << 16  // red
    //                   | ((y * 255 / height) as u32) << 8 // green
    //                   | 0x00; // blue
    //         unsafe {
    //             *fb.add(y * pitch + x) = pixel;
    //         }
    //     }
    // }

    // // new: initialize a mapper
    // let mut mapper = unsafe { memory::init(phys_mem_offset) };

    // let addresses = [
    //     // the identity-mapped vga buffer page
    //     0xb8000,
    //     // some code page
    //     0x201008,
    //     // some stack page
    //     0x0100_0020_1a10,
    //     // virtual address mapped to physical address 0
    //     boot_info.physical_memory_offset,
    // ];

    // for &address in &addresses {
    //     let virt = VirtAddr::new(address);
    //     // new: use the `mapper.translate_addr` method
    //     let phys = mapper.translate_addr(virt);
    //     println!("{:?} -> {:?}", virt, phys);
    // }

    // let mut frame_allocator =
    //     unsafe { memory::BootInfoFrameAllocator::init(&boot_info.memory_map) };

    // allocator::init_heap(&mut mapper, &mut frame_allocator).expect("heap initialization failed");

    // let x = Box::new(41);
    // let y = Box::new(50);
    // println!("x = {:?}", x);
    // println!("y = {:?}", y);

    // let mut executor = Executor::new();
    // executor.spawn(Task::new(example_task()));
    // executor.spawn(Task::new(print_keypresses()));
    // executor.run();

    // // #[cfg(test)]
    // // test_main();

    // println!("It did not crash!");
    kernel::hlt_loop();
}

/// This function is called on panic.
// #[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("{}", info);
    kernel::hlt_loop();
}

async fn async_number() -> u32 {
    42
}

async fn example_task() {
    let number = async_number().await;
    println!("async number: {}", number);
}
