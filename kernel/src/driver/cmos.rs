use x86_64::instructions::port::Port;

use crate::serial_println;

// get rtc from cmos
//Basic flow

fn io_wait() {
    // 0x80 is a historically unused / POST debug port
    // cheap delay
    unsafe { Port::<u8>::new(0x80).write(0) }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
enum CmosTimeRegister {
    Seconds = 0x00,
    Minutes = 0x02,
    Hours = 0x04,
    Day = 0x07,
    Month = 0x08,
    Year = 0x09,
    B = 0x0B,
}

impl CmosTimeRegister {
    fn as_u8(self) -> u8 {
        self as u8
    }
}

fn bcd_to_decimal(bcd: u8) -> u8 {
    let high_nibble = (bcd & 0xF0) >> 4;
    let low_nibble = (bcd & 0x0F);

    high_nibble * 10 + low_nibble
}

fn read_reg(register: CmosTimeRegister) -> u8 {
    unsafe { Port::<u8>::new(0x70).write(register.as_u8() | 0x80) };
    io_wait();
    unsafe { Port::<u8>::new(0x71).read() }
}

// This is UTC time, so we need to be able to convert to UNIX timestamp + local timezone

#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub struct RTCTime {
    pub year: u32,
    pub month: u8,
    pub day: u8,
    pub hours: u8,
    pub minutes: u8,
    pub second: u8,
}

fn is_leap_year(year: usize) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

impl RTCTime {
    pub fn as_unix_timestamp(&self) -> usize {
        const DAYS_IN_MONTH: [usize; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

        let mut days = 0 as usize;
        for i in 1970..self.year {
            let _days = if is_leap_year(i as usize) { 366 } else { 365 };
            days += _days;
        }
        for i in 1..self.month {
            days += DAYS_IN_MONTH[(i as usize) - 1];
            if is_leap_year(self.year as usize) && i == 2 {
                days += 1;
            }
        }
        days += (self.day - 1) as usize;

        (days * 24 * 3600)
            + ((self.hours as usize) * 3600 + (self.minutes as usize) * 60 + (self.second as usize))
    }
}

fn _get_time() -> RTCTime {
    let reg_b = read_reg(CmosTimeRegister::B);
    let is_bcd = reg_b & 0x04 == 0;
    let is_24_hr = reg_b & 0x02 != 0;

    let mut second = read_reg(CmosTimeRegister::Seconds);
    let mut minutes = read_reg(CmosTimeRegister::Minutes);
    let mut hours = read_reg(CmosTimeRegister::Hours);
    let mut day = read_reg(CmosTimeRegister::Day);
    let mut month = read_reg(CmosTimeRegister::Month);
    let mut year = read_reg(CmosTimeRegister::Year);

    let pm = hours & 0x80 != 0;
    hours = hours & 0x7F;

    // convert to bcd if needed
    if is_bcd {
        second = bcd_to_decimal(second);
        minutes = bcd_to_decimal(minutes);
        hours = bcd_to_decimal(hours);
        day = bcd_to_decimal(day);
        month = bcd_to_decimal(month);
        year = bcd_to_decimal(year);
    }

    if !is_24_hr {
        if pm && hours != 12 {
            hours += 12;
        }
        if !pm && hours == 12 {
            hours = 0;
        }
    }

    let full_year = if year >= 90 {
        1900 + year as u32
    } else {
        2000 + year as u32
    };

    RTCTime {
        second,
        minutes,
        hours,
        day,
        month,
        year: full_year,
    }
}

pub fn get_rtc_time() -> RTCTime {
    loop {
        let time_a = _get_time();
        let time_b = _get_time();

        if time_a == time_b {
            return time_a;
        }
    }
}

// We'll use UNIX

// maintain UTC time
// convert to user local timezone
