use crate::driver::pci::{pci_enumerate, pci_read_32, pci_read_word, pci_write, PciDevice};
use crate::driver::sound::corb::{
    AlignedRingBuffer, CmdResponsePairBuffer, CorbBuffer, RirbBuffer, RirbResponseEntry, CORB,
    HDA_CMD_RESP_QUEUE, RIRB,
};
use crate::driver::sound::wav::setup_bg_stream;
use crate::{serial_println, ALLOC, KERNEL_CONFIG, PHYS_MEM_OFFSET};
use alloc::vec::Vec;
use alloc::{boxed::Box, vec};
use bitfield_struct::bitfield;
use conquer_once::spin::OnceCell;
use core::arch::x86_64::_mm_clflush;
use core::pin::Pin;
use core::sync::atomic::AtomicUsize;
use core::task::{Context, Poll};
use crossbeam_queue::ArrayQueue;
use futures_util::task::AtomicWaker;
use futures_util::Stream;
use lazy_static::lazy_static;
use spin::Mutex;
use x86_64::structures::paging::{FrameAllocator, Translate};
use x86_64::{
    instructions::port::Port,
    structures::paging::{
        Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame, Size4KiB,
    },
    PhysAddr, VirtAddr,
};

pub const GCAP: u16 = 0x00;
pub const GCTL: u16 = 0x08;
pub const STATESTS: u16 = 0x0E;

pub const INTCTL: u16 = 0x20;
pub const INTSTS: u16 = 0x24;

pub const CORBLBASE: u16 = 0x40;
pub const CORBUBASE: u16 = 0x44;
pub const CORBWP: u16 = 0x48;
pub const CORBRP: u16 = 0x4A;
pub const CORBCTL: u16 = 0x4C;
pub const CORBSIZE: u16 = 0x4E;

pub const RIRBLBASE: u16 = 0x50;
pub const RIRBUBASE: u16 = 0x54;
pub const RIRBWP: u16 = 0x58;
pub const RINTCNT: u16 = 0x5A;
pub const RIRBCTL: u16 = 0x5C;
pub const RIRBSIZE: u16 = 0x5E;
pub const RIRBSTS: u16 = 0x5D;

pub const MMIO_VIRT_BASE: u64 = 0xFFFF_D000_0000_0000;

pub static HDA: OnceCell<Mutex<HdaPci>> = OnceCell::uninit();

pub fn find_intel_hda_pci(devices: &[PciDevice]) -> Option<PciDevice> {
    devices
        .iter()
        .find(|d| d.class == 4 && d.subclass == 3)
        .cloned()
}

pub fn init_pci() {
    let devices = pci_enumerate();
    let hda_pci = find_intel_hda_pci(&devices).unwrap();

    let cmd = pci_read_word(hda_pci.bus, hda_pci.device, hda_pci.function_id, 0x04) as u32;
    pci_write(
        hda_pci.bus,
        hda_pci.device,
        hda_pci.function_id,
        0x04,
        cmd | (1 << 1) | (1 << 2),
    );
    let status = pci_read_word(hda_pci.bus, hda_pci.device, hda_pci.function_id, 0x04 | 0x2);
    let has_capabilities_list = (status & (1 << 4)) != 0;
    let bar0 = pci_read_32(hda_pci.bus, hda_pci.device, hda_pci.function_id, 0x10);
    pci_write(
        hda_pci.bus,
        hda_pci.device,
        hda_pci.function_id,
        0x10,
        0xFFFFFFFF,
    );
    let size = pci_read_32(hda_pci.bus, hda_pci.device, hda_pci.function_id, 0x10);
    pci_write(hda_pci.bus, hda_pci.device, hda_pci.function_id, 0x10, bar0);
    let size = (!(size & !0xF)).wrapping_add(1) as usize;
    let size = x86_64::align_up(size as u64, 1024 * 4);

    let bar1 = pci_read_32(hda_pci.bus, hda_pci.device, hda_pci.function_id, 0x14);
    let hda_mmio_base = ((bar1 as u64) << 32) | ((bar0 & !0xF) as u64);

    let interrupts = pci_read_word(hda_pci.bus, hda_pci.device, hda_pci.function_id, 0x3C);
    let bytes = interrupts.to_le_bytes();

    // Specifies which input of the system interrupt controllers the device's interrupt pin is connected to
    // and is implemented by any device that makes use of an interrupt pin. For the x86 architecture this register
    // corresponds to the PIC IRQ numbers 0-15 (and not I/O APIC IRQ numbers) and a value of 0xFF defines no connection.
    let int_line = bytes[0];
    // Specifies which interrupt pin the device uses. Where a value of 0x1 is INTA#, 0x2 is INTB#, 0x3 is INTC#,
    // 0x4 is INTD#, and 0x0 means the device does not use an interrupt pin.
    let int_pin = bytes[1];

    // On QEMU its usually always 11 so i'm gonna hardcode in IDT for now
    let pic_irq = int_line + 32;

    let hda = HdaPci {
        base: hda_mmio_base,
        mmio_region_size: size,
    };
    hda.setup();

    HDA.init_once(|| Mutex::new(hda));
    HDA_CMD_RESP_QUEUE.init_once(|| CmdResponsePairBuffer {
        awaiting_req: ArrayQueue::new(256),
        ready_resp: ArrayQueue::new(256),
    });
}

// ***********
// Buffer Descriptor List, DMA, Stream configs
// ***********

#[bitfield(u32)]
pub struct PcmSample {
    #[bits(16)]
    pub left: i16,
    #[bits(16)]
    pub right: i16,
}

#[bitfield(u128)]
pub struct BDLEntry {
    #[bits(64)]
    pub address: u64,
    #[bits(32)]
    pub length: u32,
    #[bits(32)]
    flags: u32,
}

pub const PCA_BUFFER_VIRT_START: u64 = MMIO_VIRT_BASE + 1024 * 16;

// The BDL should not be modified unless the RUN bit is 0
#[repr(align(128))]
struct BufferDescriptorList {
    buffer: [BDLEntry; 4], // each entry is a page totaling 16kb cyclic buffer
}

impl BufferDescriptorList {
    // This allocates 4 pages
    pub fn new(
        mapper: &mut impl Mapper<Size4KiB>,
        frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    ) -> Self {
        let mut get_page = |idx: u64| -> BDLEntry {
            let page = Page::containing_address(VirtAddr::new(PCA_BUFFER_VIRT_START)) + idx;
            let frame = frame_allocator.allocate_frame().unwrap();
            let flags =
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_CACHE;
            unsafe {
                mapper
                    .map_to(page, frame, flags, frame_allocator)
                    .unwrap()
                    .flush()
            };

            BDLEntry::new()
                .with_address(frame.start_address().as_u64())
                .with_length(4096)
                .with_flags(1)
        };
        let buffer = [get_page(0), get_page(1), get_page(2), get_page(3)];
        Self { buffer }
    }
}

pub struct HdaPci {
    base: u64,
    mmio_region_size: u64,
}

impl HdaPci {
    fn map_hdi_mmio(&self) {
        let mut frame_allocator = ALLOC.get().unwrap().lock();
        let kern_config = KERNEL_CONFIG.get().unwrap();
        let page_table =
            unsafe { &mut *((kern_config.page_table_address + PHYS_MEM_OFFSET) as *mut PageTable) };
        let mut mapper =
            unsafe { OffsetPageTable::new(page_table, VirtAddr::new(PHYS_MEM_OFFSET)) };
        let start_frame = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(self.base));
        let end_frame = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(
            self.base + self.mmio_region_size,
        ));
        let frame_range = PhysFrame::range_inclusive(start_frame, end_frame);
        for frame in frame_range {
            let phys_start_addr = frame.start_address();
            let virt_addr = VirtAddr::new(phys_start_addr.as_u64() + MMIO_VIRT_BASE);
            let page = Page::containing_address(virt_addr);
            let mapper_flush = unsafe {
                mapper
                    .map_to(
                        page,
                        frame,
                        PageTableFlags::WRITABLE
                            | PageTableFlags::PRESENT
                            | PageTableFlags::NO_CACHE,
                        &mut *frame_allocator,
                    )
                    .unwrap()
            };
            mapper_flush.flush();
        }
    }

    pub fn read_reg<T>(&self, reg: u16) -> T {
        let ptr = (self.base + (reg as u64) + MMIO_VIRT_BASE) as *mut T;
        unsafe { ptr.read_volatile() }
    }

    pub fn write_reg<T>(&self, reg: u16, val: T) {
        let ptr = (self.base + (reg as u64) + MMIO_VIRT_BASE) as *mut T;
        unsafe { ptr.write_volatile(val) }
    }

    fn translate<T>(&self, ptr: *const T) -> u64 {
        let kern_config = KERNEL_CONFIG.get().unwrap();
        let page_table =
            unsafe { &mut *((kern_config.page_table_address + PHYS_MEM_OFFSET) as *mut PageTable) };
        let mapper = unsafe { OffsetPageTable::new(page_table, VirtAddr::new(PHYS_MEM_OFFSET)) };
        let virt_addr = VirtAddr::from_ptr(ptr);
        let phys_addr = mapper
            .translate_addr(virt_addr)
            .expect("CORB must be mapped");
        phys_addr.as_u64()
    }

    fn setup(&self) {
        // map to virtual memory
        self.map_hdi_mmio();
        self.write_reg(GCTL, self.read_reg::<u32>(GCTL) & !1);
        while (self.read_reg::<u32>(GCTL) & 1) != 0 {
            core::hint::spin_loop();
        }
        self.write_reg(GCTL, self.read_reg::<u32>(GCTL) | 1);
        while (self.read_reg::<u32>(GCTL) & 1) == 0 {
            core::hint::spin_loop();
        }
        for _ in 0..50000 {
            core::hint::spin_loop();
        }

        let gcap = self.read_reg::<u32>(GCAP);
        let iss = ((gcap >> 8) & 0xF) as u8;
        let oss = ((gcap >> 12) & 0xF) as u8;

        let codecs = self.read_reg::<u16>(STATESTS);
        // corb setup
        // set CORBSIZE = 256 entries = 1kb
        self.write_reg(CORBSIZE, 0b0000_0010u8);
        let corb = CorbBuffer::new();
        // To initialize the CORB, first the software must make sure that the CORB is stopped by making sure that the CORBRUN bit in the CORBCTL register is 0.
        self.write_reg(CORBCTL, self.read_reg::<u8>(CORBCTL) & !0b10);
        // set CORBUBASE, CORBLBASE
        let corb_addr = self.translate(&*corb.buffer as *const AlignedRingBuffer<u32>);
        let low = (corb_addr & 0xFFFFFFFF) as u32;
        let high = ((corb_addr >> 32) & 0xFFFFFFFF) as u32;
        self.write_reg(CORBUBASE, high);
        self.write_reg(CORBLBASE, low);
        // CORBRPRST bit is used to reset the Read Pointer to 0
        // Software must write 0 to the Write Pointer to clear the Write Pointer
        self.write_reg(CORBWP, 0u16);

        self.write_reg(CORBRP, 1u16 << 15);
        // qemu quirk... apparently intel-hda.c detects writing and wipes the enire register to 0
        /* while (self.read_reg::<u16>(CORBRP) & (1 << 15)) == 0 {
            core::hint::spin_loop();
        } */
        self.write_reg(CORBRP, 0u16);
        self.write_reg(CORBCTL, self.read_reg::<u8>(CORBCTL) | 0b10);

        // set RIRBSIZE = 256
        let rirb = RirbBuffer::new();
        self.write_reg(RIRBCTL, self.read_reg::<u8>(RIRBCTL) & !0b10);
        self.write_reg(RIRBSIZE, 0b0000_0010 as u8);

        let rirb_addr =
            self.translate(&*rirb.buffer as *const AlignedRingBuffer<RirbResponseEntry>);
        let low = (rirb_addr & 0xFFFFFFFF) as u32;
        let high = ((rirb_addr >> 32) & 0xFFFFFFFF) as u32;
        self.write_reg(RIRBUBASE, high);
        self.write_reg(RIRBLBASE, low);

        self.write_reg(RIRBWP, 1u16 << 15);
        self.write_reg(RIRBWP, 0u16);

        self.write_reg(RINTCNT, 1u16);
        self.write_reg(RIRBCTL, self.read_reg::<u8>(RIRBCTL) | 0b11);

        let status = self.read_reg::<u32>(INTCTL);
        self.write_reg(INTCTL, status | (1 << 31) | (1 << 30) | (1 << iss));

        CORB.init_once(|| Mutex::new(corb));
        RIRB.init_once(|| Mutex::new(rirb));

        // Output stream 0 base: MMIO BAR0 + 0x80 + (Number of input streams * 0x20)
        let stream_base = 0x80 + (iss * 0x20u8) as u16;

        // stop stream descriptor
        let mut ctl_val = self.read_reg::<u32>(stream_base);
        ctl_val &= !(1 << 1); // disable DMA
        self.write_reg(stream_base, ctl_val);
        /*
         * TODO:
        (2) Reset Stream Descriptor (set bit 0 in Control register, wait until it is set, then clear it and wait for it to be cleared)
        */
        let ctl_val = self.read_reg::<u32>(stream_base);
        self.write_reg(stream_base, ctl_val | 1);
        while (self.read_reg::<u32>(stream_base) & 1) == 0 {
            core::hint::spin_loop();
        }
        self.write_reg(stream_base, ctl_val & !1);
        while (self.read_reg::<u32>(stream_base) & 1) != 0 {
            core::hint::spin_loop();
        }

        // BDPL & BDPU - 0x14 & 0x18 (4 bytes each)
        let bdpl = self.create_bdpl();
        let bdpl = Box::leak(bdpl);
        setup_bg_stream();

        let bdpl_addr = self.translate(bdpl);
        let low = (bdpl_addr & 0xFFFFFFFF) as u32;
        let high = ((bdpl_addr >> 32) & 0xFFFFFFFF) as u32;
        self.write_reg(stream_base + 0x1C, high);
        self.write_reg(stream_base + 0x18, low);
        // CBL - 0x08 (4 bytes)
        self.write_reg(stream_base + 0x08, 4096 * 4 as u32);
        // LVI - 0x0C (2 bytes)
        self.write_reg(stream_base + 0x0C, 3 as u16);
        // FMT - 0x12 (2 byte)
        self.write_reg(stream_base + 0x12, 0x0011 as u16);
        // STS - 0x03 (1 byte)
        self.write_reg(stream_base + 0x03, 0xFF as u8);
        // LPIB - 0x04 (4 bytes)
    }

    pub fn start_dma(&self) {
        let gcap = self.read_reg::<u32>(GCAP);
        let iss = ((gcap >> 8) & 0xF) as u8;
        // Output stream 0 base: MMIO BAR0 + 0x80 + (Number of input streams * 0x20)
        let stream_base = 0x80 + (iss * 0x20u8) as u16;
        // CTL - 0x00 (3 bytes)
        let stream_no = 1;
        let stream_mask = ((1u32 << 4) - 1) << 20;
        let mut ctl_val = self.read_reg::<u32>(stream_base);
        ctl_val &= !1; // clear reset bit
        ctl_val |= 1 << 1; // RUN
        ctl_val |= 1 << 2; // Interrupt on completion
        ctl_val |= 1 << 3; // Interrupt if underrun
        ctl_val |= 1 << 4; // Interrupt if descriptor error
        let ctl_val = (ctl_val & !stream_mask) | ((stream_no << 20) & stream_mask);
        self.write_reg(stream_base, ctl_val);
    }

    fn create_bdpl(&self) -> Box<BufferDescriptorList> {
        let mut frame_allocator = ALLOC.get().unwrap().lock();
        let kern_config = KERNEL_CONFIG.get().unwrap();
        let page_table =
            unsafe { &mut *((kern_config.page_table_address + PHYS_MEM_OFFSET) as *mut PageTable) };
        let mut mapper =
            unsafe { OffsetPageTable::new(page_table, VirtAddr::new(PHYS_MEM_OFFSET)) };
        Box::new(BufferDescriptorList::new(
            &mut mapper,
            &mut *frame_allocator,
        ))
    }
}
