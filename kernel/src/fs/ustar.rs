/* Offset 	Size 	Field
0 	100 	File name
100 	8 	File mode (octal)
108 	8 	Owner's numeric user ID (octal)
116 	8 	Group's numeric user ID (octal)
124 	12 	File size in bytes (octal)
136 	12 	Last modification time in numeric Unix time format (octal)
148 	8 	Checksum for header record
156 	1 	Type flag
157 	100 	Name of linked file
257 	6 	UStar indicator, "ustar", then NULL
263 	2 	UStar version, "00" (it is a string)
265 	32 	Owner user name
297 	32 	Owner group name
329 	8 	Device major number
337 	8 	Device minor number
345 	155 	Filename prefix */

use core::{ffi::CStr, str};

use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;

use crate::serial_println;

#[repr(C)]
#[derive(Debug)]
pub struct Header {
    pub file_name: [u8; 100],
    pub file_mode: [u8; 8],
    pub owner_user_id: [u8; 8],
    pub group_user_id: [u8; 8],
    pub file_size: [u8; 12],
    pub last_mod_time: [u8; 12],
    pub checkum: [u8; 8],
    /* Value 	Meaning
    '0' 	(ASCII Null) Normal file
    '1' 	Hard link
    '2' 	Symbolic link
    '3' 	Character Special Device
    '4' 	Block device
    '5' 	Directory
    '6' 	Named Pipe */
    pub type_flag: u8,
    pub linked_file_name: [u8; 100],
    pub ustar_indicator: [u8; 6],
    pub ustar_version: [u8; 2],
    pub owner_user_name: [u8; 32],
    pub owner_group_name: [u8; 32],
    pub device_major: [u8; 8],
    pub device_minor: [u8; 8],
    /* Treat the checksum field (offset 148–155) as spaces (' ') 0x20
    Sum all 512 bytes as unsigned bytes
    Compare with stored octal checksum */
    pub filename_prefix: [u8; 155],
    _padding: [u8; 12],
}

pub struct USTAR<'a> {
    fs_data: &'a [u8],
}

#[derive(Debug)]
pub struct TarEntry<'a> {
    pub header: &'a Header,
    pub data_start: usize,
    pub data_end: usize,
}

impl<'a> USTAR<'a> {
    pub fn new(fs_data: &'a [u8]) -> Self {
        Self { fs_data }
    }

    // pub fn read_dir(&self, path: &str) -> Vec<Header>;

    // TODO: support offset
    pub fn read(&self, entry: TarEntry, buffer: &mut [u8], offset: usize) -> usize {
        let file_len = octascii_to_dec(&entry.header.file_size);
        let target_read_bytes = core::cmp::min(file_len, buffer.len());
        &buffer[0..target_read_bytes].copy_from_slice(
            &self.fs_data[entry.data_start..(entry.data_start + target_read_bytes)],
        );

        target_read_bytes
    }

    pub fn file_lookup(&self, query: &str) -> Option<TarEntry> {
        let mut zero_count = 0;
        let mut start = 0;
        while zero_count < 2 {
            if start + 512 > self.fs_data.len() {
                break;
            }

            let header_bytes = &self.fs_data[start..(start + 512)];
            let my_checksum = header_bytes.iter().map(|&b| b as usize).sum::<usize>()
                - &header_bytes[148..156]
                    .iter()
                    .map(|&b| b as usize)
                    .sum::<usize>()
                + (0x20 * 8);

            let header = unsafe { &*(header_bytes.as_ptr() as *const Header) };
            let checksum = octascii_to_dec(&header.checkum);
            if checksum != my_checksum {
                serial_println!("[WARN] checksum mismatch");
                break;
            }
            if is_zeroed(header_bytes) {
                zero_count += 1;
                start += 512;
                continue;
            }
            zero_count = 0;

            let parsed_prefix = CStr::from_bytes_until_nul(&header.filename_prefix)
                .unwrap_or_default()
                .to_str()
                .unwrap_or("");

            let parsed_name = CStr::from_bytes_until_nul(&header.file_name)
                .unwrap_or_default()
                .to_str()
                .unwrap_or("");

            let filename = if header.filename_prefix[0] != 0 {
                format!("{}/{}", parsed_prefix, parsed_name)
            } else {
                parsed_name.to_string()
            };

            let file_size = octascii_to_dec(&header.file_size);
            let file_size = ((file_size + 511) / 512) * 512;

            if filename == query {
                return Some(TarEntry {
                    header,
                    data_start: start + 512,
                    data_end: start + 512 + file_size, // not inclusive
                });
            }

            start += 512 + file_size;
        }

        None
    }
}

fn is_zeroed(data: &[u8]) -> bool {
    for byte in data {
        if *byte != 0 {
            return false;
        }
    }
    true
}

pub fn octascii_to_dec(number: &[u8]) -> usize {
    let mut result = 0;

    for &byte in number {
        if byte == 0 || byte == b' ' {
            break;
        }
        let digit = (byte - b'0') as usize;
        result = result * 8 + digit;
    }

    result
}
