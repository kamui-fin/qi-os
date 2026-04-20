/*
* KEEP TRACK OF BAD SECTORS
* These ports behave like registers:
* | Port  | Name             | Purpose              |
* | ----- | ---------------- | -------------------- |
* | 0x1F0 | Data             | 16-bit data transfer |
* | 0x1F1 | Error / Features |                      |
* | 0x1F2 | Sector Count     |                      |
* | 0x1F3 | LBA low          |                      |
* | 0x1F4 | LBA mid          |                      |
* | 0x1F5 | LBA high         |                      |
* | 0x1F6 | Drive/Head       |                      |
* | 0x1F7 | Status / Command |                      |
LBA = sector index

Split 28 bit LBA across:
LBA bits:
[27:24] → drive/head register (0x1F6)
[23:16] → 0x1F5
[15:8]  → 0x1F4
[7:0]   → 0x1F3

# READ Sector
--------
1. Wait until not busy
while (BSY == 1) → wait

2. Select drive + high LBA bits

Write to 0x1F6:

0xE0 | (drive << 4) | (LBA >> 24)
0xE0 = LBA mode + master
0xF0 = LBA mode + slave

3. Set sector Count

0x1F2 = number of sectors (usually 1)

4. Set LBA low/mid/high

5. Send & wait for data & read

0x1F7 = 0x20   (READ SECTORS)
Poll until: BSY = 0 DRQ = 1
Read 256 words (16-bit) from port 0x1F0. That equals 512 bytes

7. Writing a sector
Same idea, but:
- Command = 0x30
- After DRQ = 1 → you WRITE 256 words to 0x1F0
- Then wait for completion


GOTCHA:
❗ DRQ must be set before data transfer
❗ ERR bit checks
❗ 400ns delay After selecting drive: read status port 4 times

You must read/write 16-bit values, not bytes.
Data comes as little-endian words.
3. Strings are weird
Model string is: byte-swapped per word & you must fix it

# DISK ENMERATIONS
----------------
1. SELECT DRIVE Write to 0x1F6 (master/slave)
2. Write 0 to: 0x1F2, 0x1F3, 0x1F4, 0x1F5
3. IDENTIFY CMD 0x1F7 = 0xEC

If LBA mid/high are non-zero after IDENTIFY → it's not ATA (might be ATAPI)

Step 4: Check status
If status = 0 → no device
Otherwise wait for:
BSY = 0
DRQ = 1

Step 5: Read IDENTIFY data
256 words from 0x1F0
This gives: model name, capabilities, total sectors
*/

use core::error::Error;

use alloc::vec::{self, Vec};
use bitflags::bitflags;
use x86_64::instructions::port::Port;

use crate::driver::cmos::get_rtc_time;

const PRIMARY_BASE_REGISTER: u16 = 0x1F0;
const PRIMARY_CONTROL_REGISTER: u16 = 0x3F6;

const SECONDARY_BASE_REGISTER: u16 = 0x170;
const SECONDARY_CONTROL_REGISTER: u16 = 0x376;

#[derive(Clone, Copy)]
enum BusType {
    Primary,
    Secondary,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    struct AtaError: u8 {
        const BadBlock = 1 << 7;
        const UncorrectableDataError = 1 << 6;
        const MediaChanged = 1 << 5;
        const SectorNotFound = 1 << 4;
        const MediaChangeRequest = 1 << 3;
        const CommandAborted = 1 << 2;
        const TrackZeroNotFound = 1 << 1;
        const AddressMarkNotFound = 1 << 0;
    }
}

trait AtaRegister {
    fn absolute(&self, bus_type: BusType) -> u16;
}

#[repr(u16)]
#[derive(Clone, Copy)]
enum DataRegister {
    Data = 0,
    ErrorStatus = 1,
    SectorCount = 2,
    LbaLow = 3,
    LbaMid = 4,
    LbaHigh = 5,
    Drive = 6,
    Command = 7,
}

impl AtaRegister for DataRegister {
    fn absolute(&self, bus_type: BusType) -> u16 {
        let offset = match bus_type {
            BusType::Primary => PRIMARY_BASE_REGISTER,
            BusType::Secondary => SECONDARY_BASE_REGISTER,
        };
        offset + (*self as u16)
    }
}

#[repr(u16)]
#[derive(Clone, Copy)]
enum ControlRegister {
    AlternateStatus = 0,
}

impl AtaRegister for ControlRegister {
    fn absolute(&self, bus_type: BusType) -> u16 {
        let offset = match bus_type {
            BusType::Primary => PRIMARY_CONTROL_REGISTER,
            BusType::Secondary => SECONDARY_CONTROL_REGISTER,
        };
        offset + (*self as u16)
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    struct Status: u8 {
        const Busy = 1 << 7;
        const Ready = 1 << 6;
        const DriveFaultError = 1 << 5;
        const OverlappedServiceRequest = 1 << 4;
        const DataRequestReady = 1 << 3;
        const CorrectData = 1 << 2;
        const Index = 1 << 1;
        const Error = 1 << 0;
    }
}

struct IdentifyDiskInfo {
    is_hard_disk: bool,
    supports_lba48: bool,
    num_28_bit_sectors: u32,
    num_48_bit_sectors: u64,
}

fn parse_disk_info(buffer: [u16; 256]) -> IdentifyDiskInfo {
    IdentifyDiskInfo {
        is_hard_disk: buffer[0] >> 15 & 1 == 0, // else, it's ATAPI,
        supports_lba48: buffer[83] >> 10 & 1 == 1,
        num_28_bit_sectors: (buffer[61] as u32) << 16 | (buffer[60] as u32),
        num_48_bit_sectors: (buffer[103] as u64) << 48
            | ((buffer[102] as u64) << 32)
            | ((buffer[101] as u64) << 16)
            | (buffer[100] as u64),
    }
}

struct AtaDriver {
    bus_type: BusType,
    drive: u8,
}

impl AtaDriver {
    fn read_data_reg(&self) -> u16 {
        let mut port = Port::new(DataRegister::Data.absolute(self.bus_type));
        unsafe { port.read() }
    }

    fn write_data_reg(&self, data: u16) {
        let mut port = Port::new(DataRegister::Data.absolute(self.bus_type));
        unsafe { port.write(data) }
    }

    fn read_u8_reg<T: AtaRegister>(&self, reg: T) -> u8 {
        let mut port = Port::new(reg.absolute(self.bus_type));
        unsafe { port.read() }
    }

    fn write_u8_reg<T: AtaRegister>(&self, reg: T, val: u8) {
        let mut port = Port::new(reg.absolute(self.bus_type));
        unsafe { port.write(val) }
    }

    fn get_status(&self) -> Status {
        Status::from_bits_truncate(self.read_u8_reg(DataRegister::Command) as u8)
    }

    fn get_ata_error(&self) -> AtaError {
        AtaError::from_bits_truncate(self.read_u8_reg(DataRegister::ErrorStatus) as u8)
    }

    // call after drive selection or new command
    // no need if its >= 2nd cmd to same drive
    fn io_wait(&self) {
        for _ in 0..15 {
            self.read_u8_reg(ControlRegister::AlternateStatus);
        }
    }

    pub fn read(&self, lba: u64, sectors: u8, buffer: &[u16]) -> Result<(), AtaError> {
        while self.get_status().contains(Status::Busy) {}

        let lba_bytes = lba.to_le_bytes();

        self.write_u8_reg(DataRegister::SectorCount, sectors);
        self.write_u8_reg(DataRegister::LbaLow, lba_bytes[0]);
        self.write_u8_reg(DataRegister::LbaMid, lba_bytes[1]);
        self.write_u8_reg(DataRegister::LbaHigh, lba_bytes[2]);
        self.write_u8_reg(
            DataRegister::Drive,
            (lba_bytes[3] & 0b1111) | self.drive | 0x40,
        );
        self.write_u8_reg(DataRegister::Command, 0x20);

        self.io_wait();

        let mut data = Vec::new();
        let actual_sectors: usize = if sectors == 0 { 256 } else { sectors as usize };
        for _ in 0..(actual_sectors) {
            // poll loop
            loop {
                let status = self.get_status();
                if !status.contains(Status::Busy) && status.contains(Status::DataRequestReady) {
                    break;
                }
                if status.contains(Status::Error) {
                    return Err(self.get_ata_error());
                }
            }

            let mut sector_buffer = [0u16; 256];
            for i in 0..256 {
                sector_buffer[i] = self.read_data_reg();
            }
            data.push(sector_buffer)
        }

        Ok(())
    }

    pub fn write(&self, lba: u64, data: &[u16]) -> Result<(), AtaError> {
        let num_sectors: u8 = data.len().try_into().unwrap_or(0);

        while self.get_status().contains(Status::Busy) {}

        let lba_bytes = lba.to_le_bytes();

        self.write_u8_reg(DataRegister::SectorCount, num_sectors);
        self.write_u8_reg(DataRegister::LbaLow, lba_bytes[0]);
        self.write_u8_reg(DataRegister::LbaMid, lba_bytes[1]);
        self.write_u8_reg(DataRegister::LbaHigh, lba_bytes[2]);
        self.write_u8_reg(
            DataRegister::Drive,
            (lba_bytes[3] & 0b1111) | self.drive | 0x40,
        );
        self.write_u8_reg(DataRegister::Command, 0x30);

        self.io_wait();

        for sector in data {
            // poll loop
            loop {
                let status = self.get_status();
                if !status.contains(Status::Busy) && status.contains(Status::DataRequestReady) {
                    break;
                }
                if status.contains(Status::Error) {
                    return Err(self.get_ata_error());
                }
            }
            for word in sector {
                self.write_data_reg(word);
            }
        }

        // flush cache
        self.write_u8_reg(DataRegister::Command, 0xE7);
        self.io_wait();

        while self.get_status().contains(Status::Busy) {}

        Ok(())
    }

    fn availability(&self) -> Result<Option<IdentifyDiskInfo>, AtaError> {
        if self.read_u8_reg(DataRegister::Command) == 0xFF {
            return Ok(None);
        }

        self.write_u8_reg(DataRegister::Drive, self.drive);
        // zero out registers
        self.write_u8_reg(DataRegister::SectorCount, 0);
        self.write_u8_reg(DataRegister::LbaLow, 0);
        self.write_u8_reg(DataRegister::LbaMid, 0);
        self.write_u8_reg(DataRegister::LbaHigh, 0);
        // IDENTIFY
        self.write_u8_reg(DataRegister::Command, 0xEC);
        // 400 ns delay
        self.io_wait();
        // check status
        let status = self.read_u8_reg(DataRegister::Command);
        if status == 0 {
            return Ok(None);
        }

        // wait till not BSY
        while self.get_status().contains(Status::Busy) {}

        let lba_mid = self.read_u8_reg(DataRegister::LbaMid);
        let lba_high = self.read_u8_reg(DataRegister::LbaHigh);

        if lba_mid > 0 || lba_high > 0 {
            return Ok(None);
        }

        loop {
            let status = self.get_status();
            if status.contains(Status::Error) {
                return Err(self.get_ata_error());
            }
            if status.contains(Status::DataRequestReady) {
                break;
            }
        }

        let mut buffer = [0u16; 256];
        for i in 0..256 {
            buffer[i] = self.read_data_reg();
        }

        return Ok(Some(parse_disk_info(buffer)));
    }
}
