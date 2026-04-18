// PS/2 Mouse driver
// TODO: scrolling

use alloc::task::Wake;
use bitflags::bitflags;
use x86_64::instructions::port::Port;

bitflags! {
    /*
        Bit number 5 of the first byte (value 0x20) indicates that delta Y (the 3rd byte) is a negative number, if it is set. This means that you should OR 0xFFFFFF00 onto the value of delta Y, as a sign extension (if using 32 bits).
        Bit number 4 of the first byte (value 0x10) indicates that delta X (the 2nd byte) is a negative number, if it is set. This means that you should OR 0xFFFFFF00 onto the value of delta X, as a sign extension (if using 32 bits).
        Bit number 3 of the first byte (value 0x8) is supposed to be always set. This helps to maintain and verify packet alignment. Unfortunately, some older mice (such as 10 year old Microspeed 2 button trackballs) do not set this bit. RBIL claims that this bit should be 0, but it is wrong.
        The bottom 3 bits of the first byte indicate whether the middle, right, or left mouse buttons are currently being held down, if the respective bit is set. Middle = bit 2 (value=4), right = bit 1 (value=2), left = bit 0 (value=1).
    */
     /// Represents a set of flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    struct Ps2Flags: u8 {
        // Button Left (Normally Off = 0)
        const bl = 1 << 0;
        // Button Right (Normally Off = 0)
        const br = 1 << 1;
        // Button Middle (Normally Off = 0)
        const bm = 1 << 2;
        const always_one = 1 << 3;
        // X-Axis Sign Bit (9-Bit X-Axis Relative Offset)
        const xs = 1 << 4;
        // Y-Axis Sign Bit (9-Bit Y-Axis Relative Offset)
        const ys = 1 << 5;
        // X-Axis Overflow
        const xo = 1 << 6;
        // Y-Axis Overflow
        const yo = 1 << 7;
    }
}

#[repr(C)]
#[repr(packed)]
#[derive(Clone, Copy, Debug)]
pub struct GenericPs2Packet {
    pub status: Ps2Flags,
    pub x_mov: u8,
    pub y_mov: u8,
}

impl GenericPs2Packet {
    pub(crate) fn new(packet: [u8; 3]) -> Self {
        Self {
            status: Ps2Flags::from_bits(packet[0]).unwrap(),
            x_mov: packet[1],
            y_mov: packet[2],
        }
    }

    pub(crate) fn get_x(&self) -> i16 {
        let mut x = self.x_mov as u16;
        if self.status.contains(Ps2Flags::xs) {
            x |= 0xFF00;
        }
        x as i16
    }

    pub(crate) fn get_y(&self) -> i16 {
        let mut y = self.y_mov as u16;
        if self.status.contains(Ps2Flags::ys) {
            y |= 0xFF00;
        }
        y as i16
    }
}

// In general, it is a good idea to also have timeouts everywhere that the system is waiting for a response from the mouse, because it may never come.

// 3. Wait logic (critical!)

/* PS/2 controller status = port 0x64

Bit	Meaning
0	Output buffer full (data ready to read from 0x60)
1	Input buffer full (cannot write yet) */
// Before writing → wait until bit1 == 0
// Before reading → wait until bit0 == 1
//
// Send 0x20 → port 0x64   (get status byte)

enum WaitSignal {
    TimedOut,
    Success,
}

pub unsafe fn wait_read_signal() -> WaitSignal {
    let mut status_port = Port::<u8>::new(0x64);
    let timeout = 100_000;
    for _ in 0..timeout {
        if status_port.read() & 0x1 != 0 {
            return WaitSignal::Success;
        }
        core::hint::spin_loop();
    }
    return WaitSignal::TimedOut;
}

pub unsafe fn flush_output_buffer() {
    let mut status_port = Port::<u8>::new(0x64);
    let mut data_port = Port::<u8>::new(0x60);
    while status_port.read() & 0x1 != 0 {
        data_port.read();
    }
}

pub unsafe fn wait_write_signal() -> WaitSignal {
    let timeout = 100_000;
    let mut status_port = Port::<u8>::new(0x64);
    for _ in 0..timeout {
        if status_port.read() & 0x2 == 0 {
            return WaitSignal::Success;
        }
        core::hint::spin_loop();
    }
    return WaitSignal::TimedOut;
}

pub unsafe fn write(port: u16, value: u8) -> Result<(), &'static str> {
    if let WaitSignal::Success = wait_write_signal() {
        let mut port = Port::<u8>::new(port);
        return Ok(port.write(value));
    }
    return Err("ps/2 controller not responding to write");
}

pub unsafe fn read(port: u16) -> Result<u8, &'static str> {
    if let WaitSignal::Success = wait_read_signal() {
        let mut port = Port::<u8>::new(port);
        return Ok(port.read());
    }
    return Err("ps/2 controller not responding to read");
}

pub unsafe fn send_cmd(cmd: u8) -> bool {
    write(0x64, 0xD4);
    write(0x60, cmd);
    // Wait for ACK from Mouse
    let ack = read(0x60).unwrap_or_default();
    return ack == 0xFA;
}

pub unsafe fn reset() {
    send_cmd(0xFF);
}
pub unsafe fn disable_streaming() {
    send_cmd(0xF5);
}
pub unsafe fn enable_streaming() {
    send_cmd(0xF4);
}
pub unsafe fn set_defaults() {
    send_cmd(0xF6);
}
pub unsafe fn get_device_id() -> Result<u8, &'static str> {
    send_cmd(0xF2);
    read(0x60)
}

pub unsafe fn init_ps2() {
    /* 1. Disable both ports
    2. Flush output buffer
    3. Read controller config (0x20)
    4. Modify:
       - enable IRQ12 (bit1)
       - disable mouse clock bit5 = 0
    5. Write config back (0x60)
    6. Enable second port (0xA8) */
    write(0x64, 0xAD); // disable keyboard port
    write(0x64, 0xA7); // disable mouse port
    flush_output_buffer();

    write(0x64, 0x20); // read controller conf
    let data = read(0x60).unwrap() & !(1 << 0) & !(1 << 5) & !(1 << 1);
    write(0x64, 0x60);
    write(0x60, data);

    write(0x64, 0xAE); // enable keyboard
    write(0x64, 0xA8); // enable mouse port

    write(0x64, 0x20);
    let data = (read(0x60).unwrap() | (1 << 1)) & !(1 << 5);
    write(0x64, 0x60);
    write(0x60, data);
}

pub unsafe fn init_ps2_mouse() -> u8 {
    reset();
    read(0x60); // 0xAA
    if let WaitSignal::Success = wait_read_signal() {
        read(0x60); // optionally drain one more byte
    }

    disable_streaming();
    let mouse_id = get_device_id().expect("Unable to fetch mouse ID");
    set_defaults();
    enable_streaming();

    mouse_id
}
