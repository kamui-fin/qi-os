use x86_64::{
    structures::paging::{frame::PhysFrameRangeInclusive, FrameAllocator, PhysFrame, Size4KiB},
    PhysAddr,
};

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

pub struct BootInfoFrameAllocator {
    memory_map: &'static [MemoryMapEntry],
    used_frame_range: PhysFrameRangeInclusive,
    start_frame_addr: u64,
    next: usize,
}

pub struct UsedRegion {
    pub start_address: PhysAddr,
    pub size: u64,
}

impl BootInfoFrameAllocator {
    pub fn next_free_phys_addr(&self) -> u64 {
        self.usable_frames()
            .nth(self.next)
            .unwrap()
            .start_address()
            .as_u64()
    }

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
            next: 0,
            start_frame_addr,
            used_frame_range: PhysFrame::range_inclusive(start_frame, end_frame),
            memory_map,
        }
    }

    /// Returns an iterator over the usable frames specified in the memory map.
    pub fn usable_frames(&self) -> impl Iterator<Item = PhysFrame> + '_ {
        // get usable regions from memory map
        let regions = self.memory_map.iter();
        let usable_regions = regions.filter(|r| r.mem_type == 1);
        // map each region to its address range
        let addr_ranges = usable_regions.map(|r| r.start_addr()..r.end_addr());
        // transform to an iterator of frame start addresses
        let frame_addresses = addr_ranges
            .flat_map(|r| r.step_by(4096))
            .filter(|r| *r >= self.start_frame_addr);
        // create `PhysFrame` types from the start addresses
        frame_addresses
            .map(|addr| PhysFrame::containing_address(PhysAddr::new(addr)))
            .filter(|pf| (*pf > self.used_frame_range.end) || (*pf < self.used_frame_range.start))
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        let frame = self.usable_frames().nth(self.next);
        self.next += 1;
        frame
    }
}
