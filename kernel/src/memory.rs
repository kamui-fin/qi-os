use x86_64::structures::paging::frame::PhysFrameRangeInclusive;
use x86_64::structures::paging::OffsetPageTable;
use x86_64::structures::paging::{FrameAllocator, Mapper, Page, PhysFrame, Size4KiB};
use x86_64::{structures::paging::PageTable, PhysAddr, VirtAddr};

use crate::{serial, serial_println};

/// Initialize a new OffsetPageTable.
///
/// This function is unsafe because the caller must guarantee that the
/// complete physical memory is mapped to virtual memory at the passed
/// `physical_memory_offset`. Also, this function must be only called once
/// to avoid aliasing `&mut` references (which is undefined behavior).
pub unsafe fn init(physical_memory_offset: VirtAddr) -> OffsetPageTable<'static> {
    unsafe {
        let level_4_table = active_level_4_table(physical_memory_offset);
        OffsetPageTable::new(level_4_table, physical_memory_offset)
    }
}

/// Returns a mutable reference to the active level 4 table.
///
/// This function is unsafe because the caller must guarantee that the
/// complete physical memory is mapped to virtual memory at the passed
/// `physical_memory_offset`. Also, this function must be only called once
/// to avoid aliasing `&mut` references (which is undefined behavior).
unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    use x86_64::registers::control::Cr3;

    let (level_4_table_frame, _) = Cr3::read();

    let phys = level_4_table_frame.start_address();
    let virt = physical_memory_offset + phys.as_u64();
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();

    unsafe { &mut *page_table_ptr }
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct MemoryMapEntry {
    pub base_address: u64,
    pub length: u64,
    pub mem_type: u32,
    pub acpi_flags: u32,
}

impl MemoryMapEntry {
    pub fn start_addr(&self) -> u64 {
        self.base_address
    }

    pub fn end_addr(&self) -> u64 {
        self.base_address + self.length - 1
    }
}

pub unsafe fn get_mem_map() -> &'static mut [MemoryMapEntry] {
    // 0x1004 is the length (u32)
    // 0x1008 is where the list actually begins

    let ptr = *&(0x1004 as *const u32);
    let num_entries = *ptr as u32;

    core::slice::from_raw_parts_mut(ptr.byte_add(4) as *mut MemoryMapEntry, num_entries as usize)
}

#[derive(Debug)]
pub struct BumpAllocator {
    memory_map: &'static [MemoryMapEntry],
    used_frame_range: PhysFrameRangeInclusive,

    current_region_index: usize,
    next_phys_addr: u64,
}

pub struct UsedRegion {
    pub start_address: PhysAddr,
    pub size: u64,
}

impl BumpAllocator {
    pub fn starts_at(
        start_frame_addr: u64,
        memory_map: &'static [MemoryMapEntry],
        used_region: UsedRegion,
    ) -> Self {
        let start_frame = PhysFrame::containing_address(used_region.start_address);
        let end_frame = PhysFrame::containing_address(PhysAddr::new(
            used_region.start_address.as_u64() + used_region.size - 1,
        ));

        Self {
            used_frame_range: PhysFrame::range_inclusive(start_frame, end_frame),
            memory_map,
            current_region_index: 0,
            next_phys_addr: start_frame_addr,
        }
    }

    pub fn next_frame(&mut self) -> Option<PhysFrame> {
        // get usable regions from memory map
        loop {
            let region = match self.memory_map.get(self.current_region_index) {
                Some(region) => region,
                None => return None,
            };

            if region.start_addr() > self.next_phys_addr {
                self.next_phys_addr = region.start_addr();
            }

            // region must be usable
            if region.mem_type != 1 {
                self.current_region_index += 1;
                continue;
            }

            // we must not be within kernel range
            if (self.next_phys_addr >= self.used_frame_range.start.start_address().as_u64())
                && (self.next_phys_addr < self.used_frame_range.end.start_address().as_u64() + 4096)
            {
                self.next_phys_addr = self.used_frame_range.end.start_address().as_u64() + 4096;
                continue;
            }

            // region must be within range
            if region.end_addr() < self.next_phys_addr + 4096 {
                self.current_region_index += 1;
                continue;
            }

            let align_offset = self.next_phys_addr % 4096;
            if align_offset != 0 {
                self.next_phys_addr += 4096 - align_offset;
                continue; // Address changed, re-evaluate!
            }

            // does a FULL 4KiB frame fit in this region?
            if self.next_phys_addr + 4096 > region.end_addr() {
                self.current_region_index += 1;
                continue;
            }

            // check if we're still in valid range now after bumping
            break;
        }

        // if within used range, bump to end of that range
        let curr_frame = PhysFrame::containing_address(PhysAddr::new(self.next_phys_addr));
        self.next_phys_addr += 4096;
        Some(curr_frame)
    }
}

unsafe impl FrameAllocator<Size4KiB> for BumpAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        self.next_frame()
    }
}
