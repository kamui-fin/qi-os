use core::{
    hint::spin_loop,
    sync::atomic::{AtomicU32, AtomicUsize, Ordering},
};
use x86_64::instructions::port::Port;

use crate::lapic::LAPIC;

const INTERNAL_CLOCK_FREQ: u32 = 1_193_182;
const DESIRED_FREQUENCY: u32 = 100; // 10ms

const CONTROL_PORT: u16 = 0x43;
const CHANNEL_2_PORT: u16 = 0x42;
const SYSTEM_CTRL_PORT: u16 = 0x61;

pub static CALIBRATED_TICKS: AtomicU32 = AtomicU32::new(0);

pub unsafe fn pit_start_one_shot() {
    let mut mode_port = Port::new(CONTROL_PORT);
    mode_port.write(0xB0u8);

    let mut sys_port = Port::new(SYSTEM_CTRL_PORT);
    let val: u8 = sys_port.read();
    sys_port.write((val & !0b10) | 0b01);

    let divisor: u16 = (INTERNAL_CLOCK_FREQ / DESIRED_FREQUENCY) as u16;
    let mut data_port = Port::new(CHANNEL_2_PORT);

    data_port.write(divisor as u8);
    data_port.write((divisor >> 8) as u8);
}

pub unsafe fn pit_wait_timeout() {
    let mut sys_port = Port::<u8>::new(SYSTEM_CTRL_PORT);
    while sys_port.read() & (1 << 5) == 0 {
        spin_loop();
    }
}

pub fn lapic_timer_calibrate() -> u32 {
    let lapic = LAPIC.get().unwrap();
    // LVT (one-shot for now, masked)
    lapic.write(0x320, 1 << 16);
    lapic.write(0x3E0, 0x03);
    // initial count
    lapic.write(0x380, 0xFFFFFFFF);
    unsafe {
        pit_start_one_shot();
        pit_wait_timeout();
    }
    lapic.write(0x320, 1 << 16);
    let count = lapic.read(0x390);
    let ticks_per_10ms = 0xFFFFFFFF - count;
    lapic.write(0x380, 0);

    ticks_per_10ms
}

pub fn global_lapic_timer_init() {
    let lapic = LAPIC.get().unwrap();

    unsafe { per_core_init() };

    let ticks_per_10ms = lapic_timer_calibrate();
    let ticks_per_1ms = ticks_per_10ms / 10;
    CALIBRATED_TICKS.store(ticks_per_1ms, Ordering::SeqCst);

    lapic.write(0x380, ticks_per_1ms);
}

pub unsafe fn per_core_init() {
    let lapic = LAPIC.get().unwrap();

    let vector = 32;
    lapic.write(0x320, vector | (0b01 << 17));

    let calibrated_ticks = CALIBRATED_TICKS.load(core::sync::atomic::Ordering::SeqCst);
    if calibrated_ticks > 0 {
        lapic.write(0x380, calibrated_ticks);
    }
}
