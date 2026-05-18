use core::arch::x86_64::_mm_clflush;
use core::panicking::panic_const::panic_const_add_overflow;
use core::pin::Pin;
use core::ptr::write_volatile;
use core::task::{Context, Poll};

use alloc::vec::Vec;
use alloc::{boxed::Box, vec};
use bitfield_struct::bitfield;
use conquer_once::spin::OnceCell;
use crossbeam_queue::ArrayQueue;
use embedded_graphics::prelude::OffsetOutline;
use futures_util::task::AtomicWaker;
use futures_util::Stream;
use lazy_static::lazy_static;
use spin::Mutex;
use volatile::Volatile;
use x86_64::structures::paging::{FrameAllocator, Translate};
use x86_64::{
    align_up,
    instructions::port::Port,
    structures::paging::{
        Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame, Size2MiB, Size4KiB,
    },
    PhysAddr, VirtAddr,
};

use crate::mem::allocator::HEAP_START;
use crate::{serial_println, ALLOC, KERNEL_CONFIG, PHYS_MEM_OFFSET};

fn pci_read_32(bus: u16, slot: u16, func: u16, offset: u16) -> u32 {
    let address = (bus as u64) << 16
        | (slot as u64) << 11
        | (func as u64) << 8
        | ((offset as u64) & 0xFC)
        | 1 << 31;

    let mut addr_port = Port::new(0xCF8);
    unsafe { addr_port.write(address as u32) };

    let mut data_port = Port::<u32>::new(0xCFC);
    unsafe { data_port.read() }
}

fn pci_read_word(bus: u16, slot: u16, func: u16, offset: u16) -> u16 {
    ((pci_read_32(bus, slot, func, offset) >> (offset & 2) * 8) & 0xFFFF) as u16
}

fn pci_write(bus: u16, slot: u16, func: u16, offset: u16, value: u32) {
    let address = (bus as u64) << 16
        | (slot as u64) << 11
        | (func as u64) << 8
        | ((offset as u64) & 0xFC)
        | 1 << 31;

    let mut addr_port = Port::new(0xCF8);
    unsafe { addr_port.write(address as u32) };

    let mut data_port = Port::<u32>::new(0xCFC);
    unsafe { data_port.write(value) }
}

#[derive(Debug, Clone)]
pub enum HeaderType {
    GeneralDevice = 0x0,
    PciPciBridge = 0x1,
    PciCardBusBridge = 0x2,
}

impl TryFrom<u8> for HeaderType {
    type Error = ();

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::GeneralDevice),
            1 => Ok(Self::PciPciBridge),
            2 => Ok(Self::PciCardBusBridge),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PciDevice {
    pub bus: u16,
    pub device: u16,
    pub function_id: u16,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class: u8,
    pub subclass: u8,
    pub revision_id: u8,
    pub prog_if: u8,
    pub header_type: HeaderType,
}

pub fn pci_enumerate() -> Vec<PciDevice> {
    let mut pci_devices = vec![];
    for bus in 0..256 {
        for device in 0..32 {
            for function in 0..8 {
                let vendor_id = pci_read_word(bus, device, function, 0x00);
                if vendor_id == 0xFFFF {
                    continue;
                }
                let device_id = pci_read_word(bus, device, function, 0x00 | 0x2);

                let prog_revision = pci_read_word(bus, device, function, 0x08);
                let revision_id = (prog_revision & 0xFF) as u8;
                let prog_if = ((prog_revision >> 8) & 0xFF) as u8;

                let class_subclass = pci_read_word(bus, device, function, 0x08 | 0x2);
                let subclass = class_subclass & 0xFF;
                let class = (class_subclass >> 8) & 0xFF;

                let header_type_raw = pci_read_word(bus, device, function, 0x0C | 0x2);
                let header_type = (header_type_raw & 0xFF) as u8;
                let is_multi_functional = (header_type & 0x80) != 0; // check bit 7
                let header_type = HeaderType::try_from((header_type & 0x7F) as u8).unwrap();

                let pci_data = PciDevice {
                    bus,
                    device,
                    vendor_id,
                    device_id,
                    function_id: function,
                    class: class as u8,
                    subclass: subclass as u8,
                    header_type,
                    revision_id,
                    prog_if,
                };

                pci_devices.push(pci_data);

                if function == 0 && !is_multi_functional {
                    break;
                }
            }
        }
    }
    pci_devices
}

pub fn find_intel_hda_pci(devices: &[PciDevice]) -> Option<PciDevice> {
    devices
        .iter()
        .find(|d| d.class == 4 && d.subclass == 3)
        .cloned()
}

fn align_to(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

pub const GCTL: u8 = 0x08;
pub const STATESTS: u8 = 0x0E;

pub const INTCTL: u8 = 0x20; // 32bit
pub const INTSTS: u8 = 0x24; // 32bit

pub const CORBLBASE: u8 = 0x40;
pub const CORBUBASE: u8 = 0x44;
pub const CORBWP: u8 = 0x48;
pub const CORBRP: u8 = 0x4A;
pub const CORBCTL: u8 = 0x4C;
pub const CORBSIZE: u8 = 0x4E;

pub const RIRBLBASE: u8 = 0x50;
pub const RIRBUBASE: u8 = 0x54;
pub const RIRBWP: u8 = 0x58;
pub const RINTCNT: u8 = 0x5A;
pub const RIRBCTL: u8 = 0x5C;
pub const RIRBSIZE: u8 = 0x5E;
pub const RIRBSTS: u8 = 0x5D; // 8bit

pub const MMIO_VIRT_BASE: u64 = 0xFFFF_D000_0000_0000;

pub static HDA: OnceCell<Mutex<HdaPci>> = OnceCell::uninit();

pub static CORB: OnceCell<Mutex<CorbBuffer>> = OnceCell::uninit();
pub static RIRB: OnceCell<Mutex<RirbBuffer>> = OnceCell::uninit();

pub static RESP_WAKER: AtomicWaker = AtomicWaker::new();
pub static HDA_CMD_RESP_QUEUE: OnceCell<CmdResponsePairBuffer> = OnceCell::uninit();

// TODO: make sure allocator respects Layout::align()
#[repr(align(128))]
struct AlignedRingBuffer<T> {
    buffer: [T; 256],
}

pub struct CorbBuffer {
    buffer: Box<AlignedRingBuffer<u32>>,
    pub wp: usize,
}

pub struct CmdResponsePairBuffer {
    pub awaiting_req: ArrayQueue<u32>,
    pub ready_resp: ArrayQueue<RirbResponseEntry>,
}

pub struct CorbRespStream {
    _private: (),
}

impl CorbRespStream {
    pub fn new() -> Self {
        CorbRespStream { _private: () }
    }
}

impl Stream for CorbRespStream {
    type Item = RirbResponseEntry;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let queue = HDA_CMD_RESP_QUEUE.try_get().expect("not initialized");
        // fast path
        if let Some(resp) = queue.ready_resp.pop() {
            return Poll::Ready(Some(resp));
        }

        RESP_WAKER.register(&cx.waker());
        match queue.ready_resp.pop() {
            Some(resp) => {
                RESP_WAKER.take();
                Poll::Ready(Some(resp))
            }
            None => Poll::Pending,
        }
    }
}

impl CorbBuffer {
    fn new() -> Self {
        Self {
            buffer: Box::new(AlignedRingBuffer {
                buffer: [0u32; 256],
            }),
            wp: 0,
        }
    }

    pub fn push(&mut self, val: u32) {
        self.wp = (self.wp + 1) % 256;
        unsafe {
            core::ptr::write_volatile(&mut self.buffer.buffer[self.wp], val);
            core::arch::x86_64::_mm_clflush(&self.buffer.buffer[self.wp] as *const _ as *const u8);
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RirbResponseEntry {
    pub raw_response: u32,
    // Bits 35–32 (Codec ID / CID): The hardware index of the codec that sent the message (e.g., 0000 means Codec 0).
    // Bit 36 (Solicited vs. Unsolicited / U)
    pub metadata: u32,
}

#[repr(align(128))]
pub struct RirbBuffer {
    buffer: Box<AlignedRingBuffer<RirbResponseEntry>>,
    pub rp: u8,
}

impl RirbBuffer {
    fn new() -> Self {
        Self {
            buffer: Box::new(AlignedRingBuffer {
                buffer: [RirbResponseEntry {
                    raw_response: 0,
                    metadata: 0,
                }; 256],
            }),
            rp: 0,
        }
    }
    pub fn pop(&mut self) -> RirbResponseEntry {
        self.rp = self.rp.wrapping_add(1);
        let ptr = &self.buffer.buffer[self.rp as usize];
        let val = unsafe {
            _mm_clflush(ptr as *const _ as *const u8);
            core::ptr::read_volatile(ptr)
        };
        val
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

    pub fn read_reg<T>(&self, reg: u8) -> T {
        let ptr = (self.base + (reg as u64) + MMIO_VIRT_BASE) as *mut T;
        unsafe { ptr.read_volatile() }
    }

    pub fn write_reg<T>(&self, reg: u8, val: T) {
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
        serial_println!("RIRB Phys Addr: {:#X}", rirb_addr);
        if rirb_addr % 128 != 0 {
            panic!("MY ALLOCATOR BETRAYED ME! Memory is not 128-byte aligned!");
        }
        let low = (rirb_addr & 0xFFFFFFFF) as u32;
        let high = ((rirb_addr >> 32) & 0xFFFFFFFF) as u32;
        self.write_reg(RIRBUBASE, high);
        self.write_reg(RIRBLBASE, low);

        self.write_reg(RIRBWP, 1u16 << 15);
        self.write_reg(RIRBWP, 0u16);

        self.write_reg(RINTCNT, 1u16);
        self.write_reg(RIRBCTL, self.read_reg::<u8>(RIRBCTL) | 0b11);

        let status = self.read_reg::<u32>(INTCTL);
        self.write_reg(INTCTL, status | (1 << 31) | (1 << 30) | 1);

        CORB.init_once(|| Mutex::new(corb));
        RIRB.init_once(|| Mutex::new(rirb));
    }
}

/*
Read HDA BAR
    ↓
Map HDA MMIO regs
    ↓
Enable bus mastering
    ↓
Reset controller
    ↓
Enumerate codecs
    ↓
Send verbs
    ↓
Build CORB/RIRB
    ↓
Build stream descriptor
    ↓
DMA audio buffers
    ↓
Play sound
*/

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
    serial_println!("{:b}", status);
    let has_capabilities_list = (status & (1 << 4)) != 0;
    /* TODO:
    * Find the MSI Capability Block:
        Read the Capabilities Pointer register in the PCI Configuration Space (Offset 0x34). This gives you a byte offset into the configuration space.
        Go to that offset. Read the Capability ID (the first byte).
        If the ID is 0x05, you found the MSI Capability Block! If it's not 0x05, the second byte tells you the offset of the next capability. Keep walking the chain until you find 0x05.

    Configure the MSI Address and Data Registers:
    Once you are at the MSI block offset, the register layout looks like this:
        Message Address (32-bit): In x86_64, this must be set to a specific magic address so the system knows it's an interrupt: 0xFEE00000 | (Target_LAPIC_ID << 12). For now, targeting LAPIC 0 is standard.
        Message Data (16-bit): This is where you assign your actual IDT Vector Number. Pick a free vector slot in your kernel's Interrupt Descriptor Table (for example, 0x40).
        Message Control (16-bit): Flip Bit 0 to 1 to globally enable MSI for this PCI device.
    */

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
    serial_println!("LINE: {:b} ---- PIN: {:b}", int_line, int_pin);

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

    /*
    (1) Stop Stream Descriptor (clear bit 1 in Control register)

    (2) Reset Stream Descriptor (set bit 0 in Control register, wait until it is set, then clear it and wait for it to be cleared)

    (3) Write Buffer Descriptor List physical address

    (4) Add up length of all entries in Buffer Descriptor List and write it to register

    (5) Set number of Buffer Descriptor List entries

    (6) Set Stream Format register by same format you sended to Audio Output node

    (7) Set number of stream you want to use for streaming data (in byte 2 of Control register)

    (8) Start transfer (set bit 1 in Control register)
    */

    // Output stream 0 base: MMIO BAR0 + 0x80 + (Number of input streams * 0x20)
}

// ***********
// Buffer Descriptor List, DMA, Stream configs
// ***********

#[bitfield(u32)]
pub struct PcmSample {
    #[bits(16)]
    pub left: u64,
    #[bits(16)]
    pub right: u32,
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

pub const PCA_BUFFER_VIRT_START: u64 = MMIO_VIRT_BASE + 1024 * 16 + 1024;

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
            let page = Page::containing_address(VirtAddr::new(PCA_BUFFER_VIRT_START) + idx);
            let frame = frame_allocator.allocate_frame().unwrap();
            let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
            unsafe {
                mapper
                    .map_to(page, frame, flags, frame_allocator)
                    .unwrap()
                    .flush()
            };

            BDLEntry::new()
                .with_address(page.start_address().as_u64())
                .with_length(4096)
                .with_flags(1)
        };
        Self {
            buffer: [get_page(0), get_page(1), get_page(2), get_page(3)],
        }
    }
}

fn fill_buffer_sine_wave() {}
