/*
* TODO: keep track of bad sectors
*/

use core::error::Error;

use alloc::vec::{self, Vec};
use bitflags::bitflags;
use x86_64::instructions::port::Port;

use crate::{driver::cmos::get_rtc_time, fs::fat::BlockDevice};

const PRIMARY_BASE_REGISTER: u16 = 0x1F0;
const PRIMARY_CONTROL_REGISTER: u16 = 0x3F6;

const SECONDARY_BASE_REGISTER: u16 = 0x170;
const SECONDARY_CONTROL_REGISTER: u16 = 0x376;

#[derive(Debug, Clone, Copy)]
pub enum BusType {
    Primary,
    Secondary,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct AtaError: u8 {
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
    pub struct Status: u8 {
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

#[derive(Debug)]
pub struct IdentifyDiskInfo {
    pub is_hard_disk: bool,
    pub supports_lba48: bool,
    pub num_28_bit_sectors: u32,
    pub num_48_bit_sectors: u64,
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

#[derive(Debug)]
pub struct AtaDriver {
    bus_type: BusType,
    drive: u8,
}

#[repr(u8)]
#[derive(Debug)]
pub enum DriveType {
    Master = 0xA0,
    Slave = 0xB0,
}

impl AtaDriver {
    pub fn new(bus_type: BusType, drive: DriveType) -> Self {
        Self {
            bus_type,
            drive: drive as u8,
        }
    }

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

    pub fn availability(&self) -> Result<Option<IdentifyDiskInfo>, AtaError> {
        // Disable hardware interrupts from the drive.
        self.write_u8_reg(ControlRegister::AlternateStatus, 0x02);

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

impl BlockDevice for AtaDriver {
    fn read(&self, lba: u64, sectors: u8, buffer: &mut [u8]) -> Result<usize, AtaError> {
        let actual_sectors: usize = if sectors == 0 { 256 } else { sectors as usize };
        let (sector_capacity, leftover) = (buffer.len().div_ceil(512), buffer.len() % 512);
        if leftover > 0 || sector_capacity != actual_sectors {
            return Ok(0);
        }

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

        for i in 0..(actual_sectors) {
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

            let start = i * 512;
            for j in 0..256 {
                let data = self.read_data_reg();
                let bytes = data.to_le_bytes();

                buffer[start + (j * 2)] = bytes[0];
                buffer[start + (j * 2 + 1)] = bytes[1];
            }
        }

        Ok(actual_sectors * 512)
    }

    fn write(&self, lba: u64, data: &[u8]) -> Result<(), AtaError> {
        let (actual_sectors, leftover) = (data.len().div_ceil(512), data.len() % 512);
        if leftover > 0 || actual_sectors > 256 {
            return Ok(());
        }

        let num_sectors: u8 = actual_sectors.try_into().unwrap_or(0);

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

        for i in 0..actual_sectors {
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
            for chunk in data[(i * 512)..(i + 1) * 512].chunks(2) {
                if chunk.len() == 2 {
                    let word = (chunk[1] as u16) << 8 | chunk[0] as u16;
                    self.write_data_reg(word);
                }
            }
        }

        // flush cache
        self.write_u8_reg(DataRegister::Command, 0xE7);
        self.io_wait();

        while self.get_status().contains(Status::Busy) {}

        Ok(())
    }
}
