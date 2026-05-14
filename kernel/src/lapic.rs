use crate::acpi::{find_rsdp, get_rsdt, SdtHeader};
use crate::{serial_println, PHYS_MEM_OFFSET};
use alloc::vec;
use alloc::vec::Vec;
use core::ptr::NonNull;
use volatile::VolatilePtr;
use x86_64::registers::control::Cr3;

// Local APIC registers
const ID: u32 = 0x0020; // ID
const SVR: u32 = 0x00F0; // Spurious Interrupt Vector
const ENABLE: u32 = 0x00000100; // Unit Enable
const ESR: u32 = 0x0280; // Error Status
const ICRLO: u32 = 0x0300; // Interrupt Command
const INIT: u32 = 0x00000500; // INIT/RESET
const STARTUP: u32 = 0x00000600; // Startup IPI
const DELIVS: u32 = 0x00001000; // Delivery status
const ICRHI: u32 = 0x0310; // Interrupt Command [63:32]

const KERNEL_STACK_SIZE: usize = 32 * 1024;
const MAX_CORES: usize = 20;

// Rough outline:
// 1. detect cpus
// 2. start APs
// 3. give each cpu its own scheduler state / kernel stack
// 4. add locking
// 5. enable timer interrupts on each CPU
// each cpu needs:
// current thread
/* kernel stack
TSS
scheduler local state
run queue (optional initially) */

#[repr(C, packed)]
#[derive(Debug, Clone)]
pub struct MadtPrologue {
    pub local_apic_addr: u32,
    pub flags: u32,
}

#[repr(C, packed)]
#[derive(Debug, Clone)]
struct MadtEntry {
    entry_type: u8,
    length: u8,
}

#[derive(Debug, Clone)]
pub enum MadtEntryData {
    LocalApic(LocalApicEntry),
    IoApic(IoApicEntry),
    InterruptSourceOverride(InterruptSourceOverrideEntry),
}

/// Entry Type 0: Processor Local APIC
/// Represents a single logical processor and its local interrupt controller.
#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub struct LocalApicEntry {
    pub processor_id: u8,
    pub apic_id: u8,
    /// Flags:
    /// bit 0 = Processor Enabled (If set, CPU can be enabled)
    /// bit 1 = Online Capable (If bit 0 is clear and this is set, CPU can still be enabled)
    pub flags: u32,
}

/// Entry Type 1: I/O APIC
/// Represents an I/O APIC. The global system interrupt base is the first
/// interrupt number that this I/O APIC handles.
#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub struct IoApicEntry {
    pub io_apic_id: u8,
    pub reserved: u8,
    pub io_apic_address: u32,
    pub global_system_interrupt_base: u32,
}

/// Entry Type 2: I/O APIC Interrupt Source Override
/// Explains how IRQ sources are mapped to global system interrupts.
/// Example: IRQ source for timer (0) usually maps to global interrupt 2.
#[repr(C, packed)]
#[derive(Debug, Copy, Clone)]
pub struct InterruptSourceOverrideEntry {
    pub bus_source: u8,
    pub irq_source: u8,
    pub global_system_interrupt: u32,
    pub flags: u16,
}

pub fn parse_madt_entries(start: u64, length: u64) -> (MadtPrologue, Vec<MadtEntryData>) {
    let ptr = start as *const MadtPrologue;
    let prologue = unsafe { &*ptr };
    let start = start + core::mem::size_of::<MadtPrologue>() as u64;
    let mut curr = start;
    let mut entries = vec![];
    while curr < (start - 36) + length {
        let ptr = curr as *const MadtEntry;
        let entry_header = unsafe { &*ptr };

        if entry_header.length < 2 {}

        let entry_start = curr + 2;
        let entry = unsafe {
            match entry_header.entry_type {
                0 => MadtEntryData::LocalApic(*(entry_start as *const LocalApicEntry).clone()),
                1 => MadtEntryData::IoApic(*(entry_start as *const IoApicEntry).clone()),
                2 => MadtEntryData::InterruptSourceOverride(
                    *(entry_start as *const InterruptSourceOverrideEntry).clone(),
                ),
                _ => {
                    break;
                }
            }
        };
        entries.push(entry);
        curr += entry_header.length as u64;
    }
    (prologue.clone(), entries)
}

pub struct Lapic {
    base: u64,
}

impl Lapic {
    pub fn new(local_apic_addr: u64) -> Self {
        Self {
            base: local_apic_addr + PHYS_MEM_OFFSET,
        }
    }

    pub fn write(&self, offset: u32, value: u32) {
        let reg = unsafe {
            VolatilePtr::new(NonNull::new_unchecked(
                (self.base + offset as u64) as *mut u32,
            ))
        };
        reg.write(value);
    }

    pub fn read(&self, offset: u32) -> u32 {
        let reg = unsafe {
            VolatilePtr::new(NonNull::new_unchecked(
                (self.base + offset as u64) as *mut u32,
            ))
        };
        reg.read()
    }
}

#[repr(C, align(4096))]
struct AlignedStacks {
    data: [u8; KERNEL_STACK_SIZE * MAX_CORES],
}

static mut AP_STACKS: AlignedStacks = AlignedStacks {
    data: [0; KERNEL_STACK_SIZE * MAX_CORES],
};

pub fn allocate_stack_for_core(apic_id: usize) -> u64 {
    let base_address = unsafe { AP_STACKS.data.as_ptr() as u64 };
    let core_stack_bottom = base_address + ((apic_id as u64) * (KERNEL_STACK_SIZE as u64));
    let core_stack_top = core_stack_bottom + (KERNEL_STACK_SIZE as u64);

    core_stack_top
}

#[repr(C)]
pub struct TrampolineData {
    cr3: u32,
    _pad: u32,
    stack_top: u64,
    entry_addr: u64,
    is_ready: u64,
    apic_id: u64,
}

pub fn init_lapic() {
    let rsdp = find_rsdp();
    let sdt = get_rsdt(&rsdp);
    let madt_header = sdt.find_madt().unwrap();
    let (prologue, entries) = parse_madt_entries(
        (madt_header as *const SdtHeader as u64) + core::mem::size_of::<SdtHeader>() as u64,
        madt_header.length as u64,
    );

    // copy ap_init.asm into 0x8000
    unsafe {
        load_trampoline();
    }

    let lapic = Lapic::new(prologue.local_apic_addr as u64);

    lapic.write(SVR, ENABLE | 0xFF);

    let bsp = lapic.read(ID) as u8;

    let cr3 = Cr3::read().0.start_address().as_u64() as u32;
    let entry_addr = ap_startup as usize as u64;

    // mailbox pattern
    let boot_data = unsafe { &mut *(0x8F00 as *mut TrampolineData) };

    for entry in entries {
        if let MadtEntryData::LocalApic(LocalApicEntry {
            processor_id: _,
            apic_id,
            flags: _,
        }) = entry
        {
            if apic_id == bsp {
                continue;
            }

            let proc_stack_top = allocate_stack_for_core(apic_id as usize);
            boot_data.cr3 = cr3;
            boot_data.stack_top = proc_stack_top;
            boot_data.entry_addr = entry_addr;
            boot_data.is_ready = 0;
            boot_data.apic_id = apic_id as u64;

            // send INIT (reset signal)
            let icrhi = (apic_id as u32) << 24;
            let icrlo = INIT;
            lapic.write(ICRHI, icrhi);
            lapic.write(ICRLO, icrlo);

            while lapic.read(ICRLO) & DELIVS != 0 {}

            // clear error status
            lapic.write(ESR, 0);
            // send STARTUP IPI
            let icrhi = (apic_id as u32) << 24;
            let icrlo = STARTUP | 0x8; // start executing at 0x8000
            lapic.write(ICRHI, icrhi);
            lapic.write(ICRLO, icrlo);

            while unsafe { core::ptr::read_volatile(&boot_data.is_ready) } == 0 {
                core::hint::spin_loop();
            }
        }
    }
}

extern "C" {
    static trampoline_start: u8;
    static trampoline_end: u8;
}

unsafe fn load_trampoline() {
    let start = &trampoline_start as *const u8;
    let end = &trampoline_end as *const u8;
    let size = end as usize - start as usize;

    let dest = 0x8000 as *mut u8;
    core::ptr::copy_nonoverlapping(start, dest, size);
}

#[no_mangle]
pub extern "C" fn ap_startup(cpu_id: u64) -> ! {
    serial_println!("<<<< CPU {cpu_id} BOOTED UP! >>>>");
    loop {
        core::hint::spin_loop();
    }
}
