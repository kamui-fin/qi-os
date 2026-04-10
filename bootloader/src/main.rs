#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

use x86_64::{
    structures::paging::{
        frame::{PhysFrameRange, PhysFrameRangeInclusive},
        FrameAllocator, Mapper, OffsetPageTable, Page, PageSize, PageTable, PageTableFlags,
        PhysFrame, Size2MiB, Size4KiB,
    },
    PhysAddr, VirtAddr,
};

use crate::memory::{BootInfoFrameAllocator, MemoryMapEntry, UsedRegion};

mod memory;
mod serial;

const LOWER_MEMORY_END_PAGE: u64 = 0x10_0000;
pub const KERNEL_BASE_ADDR: usize = 0xFFFFFFFF80000000;

#[repr(C)]
pub struct Screen {
    pub width: u32,
    pub height: u32,
    pub bpp: u32,
    pub bytes_per_pixel: u32,
    pub bytes_per_line: u32,
    pub screen_size: u32,
    pub screen_size_dqwords: u32,
    pub framebuffer: u32,
    pub x: u32,
    pub y: u32,
    pub x_max: u32,
    pub y_max: u32,
}

#[repr(C)]
pub struct BootInfo<'a> {
    screen: &'a Screen,
    allocator: BootInfoFrameAllocator,
    page_table: OffsetPageTable<'a>,
    physical_memory_offset: u64,
    kernel_base_virt: VirtAddr,
    kernel_frame_range: PhysFrameRangeInclusive<Size2MiB>,
}

#[no_mangle]
#[link_section = ".text._start"]
pub extern "C" fn _start(screen: *const Screen) -> ! {
    serial::init();
    serial_println!("STAGE 3 BOOTLOADER BEGIN!");

    let (kernel_address, kernel_size_bytes) = unsafe { load_kernel_elf() };

    let screen = unsafe { &(*screen) };

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

    /* identity map the framebuffer */
    let fb_addr = PhysAddr::new(screen.framebuffer as u64);
    let start_frame: PhysFrame = PhysFrame::containing_address(fb_addr);
    let end_frame: PhysFrame =
        PhysFrame::containing_address(fb_addr + (screen.bytes_per_line * screen.height) as usize);
    let frame_range = PhysFrame::range_inclusive(start_frame, end_frame);
    for frame in frame_range {
        let mapper_flush = unsafe {
            kernel_page_table
                .identity_map(
                    frame,
                    PageTableFlags::WRITABLE | PageTableFlags::PRESENT,
                    &mut frame_allocator,
                )
                .expect("(fixed offset mapping): unable to map page")
        };
        mapper_flush.flush();
    }

    /* map high half kernel again */
    let kernel_start_frame = PhysFrame::<Size2MiB>::containing_address(kernel_base_phys);
    let kernel_end_frame = PhysFrame::<Size2MiB>::containing_address(PhysAddr::new(
        kernel_base_phys.as_u64() + kernel_size_bytes - 1,
    ));
    let kernel_frame_range = PhysFrame::range_inclusive(kernel_start_frame, kernel_end_frame);
    serial_println!("Mapping high half kernel: {:#?}", kernel_frame_range);
    for frame in kernel_frame_range {
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

    let kernel_stack_size = 512 * Size4KiB::SIZE;
    // create a stack
    let stack_start = Page::containing_address(VirtAddr::new(0xFFFFFFFF82000000));
    let stack_end_addr = stack_start.start_address() + kernel_stack_size;
    let stack_end = Page::containing_address(stack_end_addr - 1u64);
    for page in Page::range_inclusive(stack_start, stack_end) {
        let frame = frame_allocator
            .allocate_frame()
            .expect("frame allocation failed when mapping a kernel stack");
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
        match unsafe { kernel_page_table.map_to(page, frame, flags, &mut frame_allocator) } {
            Ok(tlb) => tlb.flush(),
            Err(err) => panic!("failed to map page {:?}: {:?}", page, err),
        }
    }

    /* map all physical memory at a fixed offset */
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

    /* IDENTITY MAP THE LOWER 1MB (Crucial for the CR3 switch!) */
    let lower_start = PhysFrame::<Size2MiB>::containing_address(PhysAddr::new(0));
    let lower_end =
        PhysFrame::<Size2MiB>::containing_address(PhysAddr::new(LOWER_MEMORY_END_PAGE - 1));
    let lower_range = PhysFrame::range_inclusive(lower_start, lower_end);
    for frame in lower_range {
        let mapper_flush = unsafe {
            kernel_page_table
                .identity_map(
                    frame,
                    PageTableFlags::WRITABLE | PageTableFlags::PRESENT,
                    &mut frame_allocator,
                )
                .expect("failed to identity map lower 1MB")
        };
        mapper_flush.ignore();
    }

    let kstack_top = stack_end_addr.align_down(16u8);
    let _kstack_bottom = stack_start.start_address();

    x86_64::instructions::tlb::flush_all();

    let boot_info = BootInfo {
        screen,
        allocator: frame_allocator,
        page_table: kernel_page_table,
        physical_memory_offset: phys_mem_offset.as_u64(),
        kernel_base_virt,
        kernel_frame_range,
    };

    serial_println!("Finished mapping everything.");

    unsafe {
        asm!(
            "mov cr3, {cr3_val}",
            "mov rsp, {rsp_val}",
            "xor rbp, rbp",
            "jmp {kernel_entry}",
            cr3_val = in(reg) kernel_level_4_frame.start_address().as_u64(),
            rsp_val = in(reg) kstack_top.as_u64(),
            kernel_entry = in(reg) kernel_address,
            in("rdi") &boot_info as *const _, // SysV ABI: 1st argument goes in RDI
            options(noreturn)
        );
    }
}

#[repr(C)]
struct Elf64Ehdr {
    e_ident: [u8; 16],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
}

#[repr(C)]
struct Elf64Phdr {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
}

unsafe fn load_kernel_elf() -> (u64, u64) {
    let elf_base = KERNEL_BASE_ADDR + 0x1000000;

    let ehdr = &*(elf_base as *const Elf64Ehdr);
    let phdrs = (elf_base + ehdr.e_phoff as usize) as *const Elf64Phdr;

    let mut min_vaddr = u64::MAX;
    let mut max_vaddr = 0;

    for i in 0..ehdr.e_phnum {
        let ph = &*(phdrs.add(i as usize));

        if ph.p_type != 1 {
            continue;
        }

        if ph.p_vaddr < min_vaddr {
            min_vaddr = ph.p_vaddr
        };
        if ph.p_vaddr + ph.p_memsz > max_vaddr {
            max_vaddr = ph.p_vaddr + ph.p_memsz
        };

        let src = elf_base + ph.p_offset as usize;
        let dest = ph.p_vaddr as *mut u8;

        let src_ptr = src as *const u8;
        let dst_ptr = dest as *mut u8;
        for i in 0..ph.p_filesz as usize {
            *dst_ptr.add(i) = *src_ptr.add(i);
        }

        let bss_start = (ph.p_vaddr + ph.p_filesz) as *mut u8;
        let bss_size = (ph.p_memsz - ph.p_filesz) as usize;
        for i in 0..bss_size {
            *bss_start.add(i) = 0;
        }
    }

    let kernel_size = max_vaddr - min_vaddr;

    (ehdr.e_entry, kernel_size)
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
