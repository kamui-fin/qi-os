use x86_64::instructions::port::Port;

const INTERNAL_CLOCK_FREQ: u32 = 1_193_182;
const DESIRED_FREQUENCY: u32 = 1000;

const CONTROL_PORT: u16 = 0x43;
const CHANNEL_0_PORT: u16 = 0x40;

pub fn init_pit() {
    let settings_byte: u8 = 0b0011_0110;

    let mut mode_port = Port::new(CONTROL_PORT);
    unsafe {
        mode_port.write(settings_byte);
    }

    let divisor: u16 = (INTERNAL_CLOCK_FREQ / DESIRED_FREQUENCY) as u16;
    let mut channel_0_port = Port::new(CHANNEL_0_PORT);
    unsafe {
        channel_0_port.write(divisor as u8);
        channel_0_port.write((divisor >> 8) as u8);
    }
}
