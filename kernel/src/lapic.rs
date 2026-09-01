use crate::acpi::{find_rsdp, get_rsdt, SdtHeader};
use crate::driver::pit::per_core_init;
use crate::mem::gdt::{init_gdt, new_gdt, new_tss, Selectors};
use crate::spinlock::Spinlock;
use crate::task::proc::ProcessControlBlock;
use crate::task::scheduler::{self, scheduler_loop, MAX_TASKS};
use crate::task::thread::ThreadControlBlock;
use crate::{hlt_loop, serial_println, PHYS_MEM_OFFSET};
use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use conquer_once::spin::OnceCell;
use core::ptr::NonNull;
use core::slice::Iter;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize};
use lazy_static;
use volatile::VolatilePtr;
use x86_64::instructions::interrupts::{self, without_interrupts};
use x86_64::instructions::tables::load_tss;
use x86_64::registers::control::Cr3;
use x86_64::registers::model_specific::{GsBase, Msr};
use x86_64::structures::gdt::GlobalDescriptorTable;
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

pub type ThreadId = u64;
pub struct Cpu {
    pub apic_id: u8,
    pub ready: AtomicBool,

    // ready only
    pub tss: *mut TaskStateSegment,
    pub gdt: GlobalDescriptorTable,
    pub selectors: Selectors,

    pub proc: AtomicPtr<Spinlock<ProcessControlBlock>>,
    pub curr_thread: AtomicPtr<ThreadControlBlock>,
    pub main_sched_thread: AtomicPtr<ThreadControlBlock>,

    pub irq_disable_depth: AtomicUsize,
    pub int_enabled: AtomicBool,
    pub needs_resched: AtomicBool,
    pub ready_queue: Spinlock<VecDeque<ThreadId>>, //only needs to have interrupts dissabled but also needs to be atomic, spinlock is easiest
}

// guarenteed raw pointers are accessed safely
unsafe impl Sync for Cpu {}
unsafe impl Send for Cpu {}

#[derive(Debug, Copy, Clone)]
pub struct CpuRef(pub *mut Cpu);

unsafe impl Send for CpuRef {}
unsafe impl Sync for CpuRef {}

pub fn mycpu() -> &'static Cpu {
    if x86_64::instructions::interrupts::are_enabled() {
        panic!("mycpu() called with int enabled");
    }
    let cpu_ptr = GsBase::read().as_u64() as *mut Cpu;
    unsafe { &*cpu_ptr }
}

pub fn my_proc() -> &'static Spinlock<ProcessControlBlock> {
    without_interrupts(|| {
        let cpu = mycpu();
        unsafe { &*cpu.proc.load(core::sync::atomic::Ordering::Relaxed) }
    })
}

pub fn my_thread() -> &'static mut ThreadControlBlock {
    without_interrupts(|| {
        let cpu = mycpu();
        unsafe { &mut *cpu.curr_thread.load(core::sync::atomic::Ordering::Relaxed) }
    })
}

// Local APIC registers
const ID: u32 = 0x0020; // ID
const SVR: u32 = 0x00F0; // Spurious Interrupt Vector
const TPR: u32 = 0x80; // Spurious Interrupt Vector
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

static mut ISA_IRQ_TO_GSI: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

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

pub fn get_lapic() -> Lapic {
    let rsdp = find_rsdp();
    let sdt = get_rsdt(&rsdp);
    let madt_header = sdt.find_madt().unwrap();
    let start = (madt_header as *const SdtHeader as u64) + core::mem::size_of::<SdtHeader>() as u64;
    let ptr = start as *const MadtPrologue;
    let prologue = unsafe { &*ptr };
    Lapic::new(prologue.local_apic_addr as u64)
}

pub fn setup_cpu(apic_id: u8) -> *mut Cpu {
    let tss_boxed = Box::new(new_tss());
    let tss_ptr = Box::into_raw(tss_boxed);

    let (gdt, selectors) = unsafe { new_gdt(&*tss_ptr) };
    let cpu = Box::new(Cpu {
        apic_id: apic_id as u8,
        ready: AtomicBool::new(false),
        tss: tss_ptr,
        gdt,
        selectors,
        proc: AtomicPtr::new(core::ptr::null_mut()),
        curr_thread: AtomicPtr::new(core::ptr::null_mut()),
        main_sched_thread: AtomicPtr::new(core::ptr::null_mut()),
        irq_disable_depth: AtomicUsize::new(0),
        int_enabled: AtomicBool::new(false),
        needs_resched: AtomicBool::new(false),
        ready_queue: Spinlock::new(VecDeque::with_capacity(MAX_TASKS)),
    });
    let cpu_raw = Box::into_raw(cpu);
    cpu_raw
}

pub fn parse_madt_entries(start: u64, length: u64) -> Vec<MadtEntryData> {
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
    entries
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

    pub fn ack(&self) {
        self.write(0xB0, 0);
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
    cpu_ptr: u64,
}

pub static IOAPIC: OnceCell<IOApic> = OnceCell::uninit();
pub static LAPIC: OnceCell<Lapic> = OnceCell::uninit();

const T_IRQ0: usize = 0x20;

const REG_ID: usize = 0x00; // Register index: ID
const REG_VER: usize = 0x01; // Register index: version
const REG_TABLE: usize = 0x10; // Redirection table base

// The redirection table starts at REG_TABLE and uses
// two registers to configure each interrupt.
// The first (low) register in a pair contains configuration bits.
// The second (high) register contains a bitmask telling which
// CPUs can serve that interrupt.
const INT_DISABLED: usize = 0x00010000; // Interrupt disabled
const INT_LEVEL: usize = 0x00008000; // Level-triggered (vs edge-)
const INT_ACTIVELOW: usize = 0x00002000; // Active low (vs high)
const INT_LOGICAL: usize = 0x00000800; // Destination is CPU id (vs APIC ID)

pub struct IOApic {
    base: u64,
    apic_id: u8,
}

impl IOApic {
    fn new(io_apic_entry: IoApicEntry) -> Self {
        Self {
            base: (io_apic_entry.io_apic_address as u64) + PHYS_MEM_OFFSET,
            apic_id: io_apic_entry.io_apic_id,
        }
    }

    pub fn write(&self, reg: u32, value: u32) {
        let reg_addr =
            unsafe { VolatilePtr::new(NonNull::new_unchecked((self.base as u64) as *mut u32)) };
        reg_addr.write(reg);

        let data_addr = unsafe {
            VolatilePtr::new(NonNull::new_unchecked(
                (self.base + 0x10 as u64) as *mut u32,
            ))
        };
        data_addr.write(value);
    }

    pub fn read(&self, reg: u32) -> u32 {
        let reg_addr =
            unsafe { VolatilePtr::new(NonNull::new_unchecked((self.base as u64) as *mut u32)) };
        reg_addr.write(reg);

        let data_addr = unsafe {
            VolatilePtr::new(NonNull::new_unchecked(
                (self.base + 0x10 as u64) as *mut u32,
            ))
        };
        data_addr.read()
    }

    pub fn init(&self) {
        let max_intr = (self.read(REG_VER as u32) >> 16) & 0xFF;
        let id = self.read(REG_ID as u32) >> 24;

        if id != self.apic_id as u32 {
            panic!("id isn't equal to ioapicid; not a MP");
        }

        for i in 0..=max_intr {
            self.write(
                (REG_TABLE as u32) + 2 * i,
                (INT_DISABLED as u32) | ((T_IRQ0 as u32) + i),
            );
            self.write((REG_TABLE as u32) + 2 * i + 1, 0);
        }
    }

    pub fn enable(&self, irq: u32, vector: u32, cpu_id: u32) {
        let pin = unsafe { ISA_IRQ_TO_GSI[irq as usize] } as u32;
        self.write((REG_TABLE as u32) + 2 * pin, vector);
        self.write((REG_TABLE as u32) + 2 * pin + 1, cpu_id << 24);
    }
}

pub fn find_cpus() -> Vec<MadtEntryData> {
    let rsdp = find_rsdp();
    let sdt = get_rsdt(&rsdp);
    let madt_header = sdt.find_madt().unwrap();
    let entries = parse_madt_entries(
        (madt_header as *const SdtHeader as u64) + core::mem::size_of::<SdtHeader>() as u64,
        madt_header.length as u64,
    );
    entries
}

pub fn init_lapic() -> u8 {
    let lapic = LAPIC.get().unwrap();

    // globally enable lapic
    let mut apic_base_msr = Msr::new(0x1B);
    unsafe {
        apic_base_msr.write(apic_base_msr.read() | (1 << 11)); // set bit 11
    }
    lapic.write(SVR, ENABLE | 0xFF);
    lapic.write(TPR, 0); // allow all interrupts

    let apic_id = lapic.read(ID) as u8;
    apic_id
}

extern "C" {
    static trampoline_start: u8;
    static trampoline_end: u8;
}

unsafe fn load_trampoline() {
    let start = &trampoline_start as *const u8;
    let end = &trampoline_end as *const u8;
    let size = end as usize - start as usize;

    let dest = (0x8000 + PHYS_MEM_OFFSET) as *mut u8;
    core::ptr::copy_nonoverlapping(start, dest, size);
}

lazy_static::lazy_static! {
    pub static ref CPU: Spinlock<Vec<u8>> = Spinlock::new(Vec::new());
}

#[no_mangle]
pub extern "C" fn ap_startup(cpu_id: u64, cpu_addr: u64) -> ! {
    crate::interrupts::init_idt();

    GsBase::write(VirtAddr::new(cpu_addr));
    init_gdt(&mycpu().gdt, &mycpu().selectors);

    unsafe { load_tss(mycpu().selectors.tss_selector) };

    init_lapic();

    cpu_common(cpu_id);
}

pub fn cpu_common(_cpu_id: u64) -> ! {
    init_kmain();

    CPU.lock().push(_cpu_id as u8);
    unsafe {
        per_core_init();
    }
    mycpu()
        .ready
        .store(true, core::sync::atomic::Ordering::SeqCst);

    serial_println!("<<<< CPU {_cpu_id} BOOTED UP! >>>>");
    // scheduler start
    scheduler_loop();
}

fn init_kmain() {
    let cpu = mycpu();
    let ptr = Box::into_raw(Box::new(ThreadControlBlock::kmain()));
    cpu.main_sched_thread
        .store(ptr, core::sync::atomic::Ordering::Relaxed);
    cpu.curr_thread
        .store(ptr, core::sync::atomic::Ordering::Relaxed);
}

pub fn start_other_cpus(entries: Iter<MadtEntryData>, bsp: u8) {
    // copy ap_init.asm into 0x8000
    unsafe {
        load_trampoline();
    }

    let cr3 = Cr3::read().0.start_address().as_u64() as u32;
    let entry_addr = ap_startup as usize as u64;

    // mailbox pattern
    let boot_data = unsafe { &mut *((0x8F00 + PHYS_MEM_OFFSET) as *mut TrampolineData) };

    for entry in entries {
        match entry {
            MadtEntryData::IoApic(data) => {
                IOAPIC.try_get_or_init(|| IOApic::new(data.clone()));
            }
            MadtEntryData::LocalApic(data) => {
                let lapic = LAPIC.get().unwrap();
                let apic_id = data.apic_id;
                if apic_id == bsp {
                    continue;
                }

                let cpu_ptr = setup_cpu(apic_id) as u64;

                let proc_stack_top = allocate_stack_for_core(apic_id as usize);
                boot_data.cr3 = cr3;
                boot_data.stack_top = proc_stack_top;
                boot_data.entry_addr = entry_addr;
                boot_data.is_ready = 0;
                boot_data.apic_id = apic_id as u64;
                boot_data.cpu_ptr = cpu_ptr;

                // send INIT (reset signal)
                let icrhi = (apic_id as u32) << 24;
                let icrlo = INIT;
                lapic.write(ICRHI, icrhi);
                lapic.write(ICRLO, icrlo);

                // wait 10ms

                while lapic.read(ICRLO) & DELIVS != 0 {}

                // clear error status
                lapic.write(ESR, 0);
                // send STARTUP IPI
                let icrhi = (apic_id as u32) << 24;
                let icrlo = STARTUP | 0x8; // start executing at 0x8000
                lapic.write(ICRHI, icrhi);
                lapic.write(ICRLO, icrlo);

                // wait 200 us
                //
                // maybe send another STARTUP IPI?
                // for now, this works with QEMU already

                while unsafe { core::ptr::read_volatile(&boot_data.is_ready) } == 0 {
                    core::hint::spin_loop();
                }
            }
            MadtEntryData::InterruptSourceOverride(data) => {
                let InterruptSourceOverrideEntry {
                    bus_source,
                    irq_source,
                    global_system_interrupt,
                    flags,
                } = data.clone();
                if bus_source == 0 && irq_source < 16 {
                    unsafe {
                        ISA_IRQ_TO_GSI[irq_source as usize] = global_system_interrupt as u8;
                    }
                }
            }
        }
    }
}

pub fn pic_disable() {
    let mut io_pic1 = x86_64::instructions::port::Port::new(0x20 + 1); // Master (IRQs 0-7)
    let mut io_pic2 = x86_64::instructions::port::Port::new(0xA0 + 1); // Slave (IRQs 8-15)

    unsafe {
        io_pic1.write(0xFF as u8);
        io_pic2.write(0xFF as u8);
    }
}
