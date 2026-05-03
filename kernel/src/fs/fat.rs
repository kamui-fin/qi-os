// CAUTION: this fat driver is very much still a rough sketch.
//
// TODO:
// - last access date update
// - fix O(n) find free cluster
// - respect READ_ONLY
// - write to multiple FATs
// - heavy testing with all sorts of edge cases
// - proper error handling

use core::mem;

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use alloc::{format, slice, vec};
use bitflags::{bitflags, Flags};
use spin::Mutex;
use x86_64::align_down;

use crate::driver::cmos::get_unix_time;
use crate::fs::vfs::{
    DEntry, DEntryMinimal, FileOps, INode, INodeData, INodeOps, NodeType, Stat, SuperBlock,
};
use crate::{driver::ata::AtaError, serial_println};

use super::vfs;

const EOC: u32 = 0x0FFFFFFF;

#[derive(Debug, Clone)]
#[repr(packed, C)]
pub struct BPB {
    jmp_bytes: [u8; 3],
    oem_identifier: [u8; 8],

    // BPB (common)
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    // The boot record sectors are included in this value.
    reserved_sector_count: u16,
    // Often this value is 2.
    num_fats: u8,
    root_entry_count: u16, // 0 for FAT32
    // If this value is 0, it means there are more than 65535 sectors in the volume, and the actual count is stored in the Large Sector Count entry at 0x20.
    total_sectors_16: u16,
    // Pretty sure we ignore this
    media: u8,
    fat_size_16: u16, // 0 for FAT32
    sectors_per_track: u16,
    num_heads: u16,
    hidden_sectors: u32,
    total_sectors_32: u32,

    // FAT32 Extended BPB
    // Sectors per FAT. The size of the FAT in sectors.
    pub fat_size_32: u32,
    ext_flags: u16,
    // FAT version number. The high byte is the major version and the low byte is the minor version. FAT drivers should respect this field.
    fs_ver: u16,
    // The cluster number of the root directory. Often this field is set to 2.
    pub root_cluster: u32,
    // The sector number of the FSInfo structure.
    pub fs_info: u16,
    // The sector number of the backup boot sector.
    backup_boot_sector: u16,
    reserved: [u8; 12],

    // Drive info
    // The values here are identical to the values returned by the BIOS interrupt 0x13. 0x00 for a floppy disk and 0x80 for hard disks.
    drive_num: u8,
    reserved_1: u8,
    // Signature (must be 0x28 or 0x29).
    boot_signature: u8,
    // Used for tracking volumes between computers
    volume_id: u32,
    // padded with spaces
    volume_label: [u8; 11],
    // Always "FAT32   ".
    filesys_type: [u8; 8],

    // Boot code.
    boot_code: [u8; 420],
    boot_sector_signature: u16, // 0xAA55
}

#[repr(C, packed)]
#[derive(Debug, Clone)]
pub struct FSInfo {
    lead_signature: u32, // 0x41615252
    reserved_1: [u8; 480],
    struct_signature: u32, // 0x61417272
    // If the value is 0xFFFFFFFF, then the free count is unknown and must be computed. However, this value might be incorrect and should at least be range checked (<= volume cluster count)
    free_count: u32, // free clusters (hint)
    // Indicates the cluster number at which the filesystem driver should start looking for available clusters. If the value is 0xFFFFFFFF, then there is no hint and the driver should start searching at 2. Typically this value is set to the last allocated cluster number. As the previous field, this value should be range checked.
    next_free: u32, // next free cluster hint
    reserved2: [u8; 12],
    trail_signature: u32, // 0xAA550000
}

// 0x00000000: Cluster is free.
/* 0x00000002 through 0x0FFFFFEF: The ID of the next cluster in the file.
0x0FFFFFF8 through 0x0FFFFFFF: EOF (End of File). */
pub type Fat32Entry = u32;

// Standard 8.3 format
// 32 bytes
#[repr(C, packed)]
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: [u8; 8],
    pub ext: [u8; 3],
    pub attr: DirEntryFlags,
    nt_reserved: u8,
    // Creation time in hundredths of a second, although the official FAT Specification from Microsoft says it is tenths of a second. Range 0-199 inclusive. Based on simple tests, Ubuntu16.10 stores either 0 or 100 while Windows7 stores 0-199 in this field.
    pub creation_time_tenth: u8,
    // The time that the file was created. Multiply Seconds by 2.
    /* Hour 	5 bits
    Minutes 	6 bits
    Seconds 	5 bits  */
    pub creation_time: u16,
    /* The date on which the file was created.
    Year 	7 bits
    Month 	4 bits
    Day 	5 bits */
    pub creation_date: u16,
    // Same format as the creation date.
    pub last_access_date: u16,
    // The high 16 bits of this entry's first cluster number.
    first_cluster_high: u16,
    // Same format as the creation time.
    pub write_time: u16,
    // Same format as the creation date.
    pub write_date: u16,
    // The low 16 bits of this entry's first cluster number. Use this number to find the first cluster for this entry.
    first_cluster_low: u16,
    // in bytes
    pub file_size: u32,
}

#[derive(Debug)]
pub struct DirEntryWithLoc {
    pub entry: DirEntry,
    pub loc: ClusterOffset,
}

const DOT: [u8; 8] = [0x2E, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20];
const DOT_DOT: [u8; 8] = [0x2E, 0x2E, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20];

impl DirEntry {
    fn first_cluster(&self) -> u32 {
        (self.first_cluster_high as u32) << 16 | (self.first_cluster_low as u32)
    }

    fn set_first_cluster(&mut self, first_new: u32) {
        self.first_cluster_low = first_new as u16;
        self.first_cluster_high = (first_new >> 16) as u16;
    }

    fn is_dot(&self) -> bool {
        self.name == DOT || self.name == DOT_DOT
    }

    pub fn name_as_str(&self) -> String {
        let name = str::from_utf8(&self.name).unwrap().trim();
        let ext = str::from_utf8(&self.ext).unwrap().trim();

        format!("{name}.{ext}")
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct DirEntryFlags: u8 {
        const FAT_ATTR_READ_ONLY = 0x01;
        const FAT_ATTR_HIDDEN = 0x02;
        const FAT_ATTR_SYSTEM = 0x04;
        const FAT_ATTR_VOLUME_ID = 0x08;
        const FAT_ATTR_DIRECTORY = 0x10;
        const FAT_ATTR_ARCHIVE = 0x20;
        const FAT_ATTR_LFN = 0x0F;
    }
}

#[derive(Clone)]
pub struct Fat32<D: BlockDevice> {
    bpb: BPB,
    fs_info: FSInfo,
    disk: D,

    // derived
    fat_start: u32,
    data_start: u32,
}

#[derive(Debug)]
pub struct SectorOffset {
    pub sector: u32,
    pub offset: u32,
}

#[derive(Debug)]
pub struct ClusterOffset {
    pub cluster: u32,
    pub offset: u32,
}

fn is_eoc(val: u32) -> bool {
    val >= 0x0FFFFFF8
}

fn is_free(val: u32) -> bool {
    val == 0
}

fn is_bad(val: u32) -> bool {
    val == 0x0FFFFFF7
}

fn is_valid_next(val: u32) -> bool {
    val >= 2 && val <= 0x0FFFFFEF
}

fn get_fat_timedate() -> (u16, u16) {
    let unix_time = get_unix_time();

    // format unix_time into <hour 5 bit><minute 6 bit><sec 5 bit>
    let seconds_in_day = unix_time % 86400;
    let hour = (seconds_in_day / 3600) as u16;
    let minute = ((seconds_in_day % 3600) / 60) as u16;
    let second = (seconds_in_day % 60) as u16;
    let fat_time = (hour << 11) | (minute << 5) | (second / 2);

    // format unix_time into <year 7 bit><month 4 bit><day 5 bit>

    // https://howardhinnant.github.io/date_algorithms.html
    let days_since_epoch = (unix_time / 86400) as i32;
    // shift epoch from 1970-01-01 to 0000-03-01 for easier leap year math
    let z = days_since_epoch + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i32) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    // fat32 year 0 = 1980
    let fat_year = if year >= 1980 {
        (year - 1980) as u16
    } else {
        0
    };
    let fat_date = (fat_year << 9) | ((m as u16) << 5) | (d as u16);

    (fat_time, fat_date)
}

// TODO: make Error type generic!
pub trait BlockDevice {
    fn id(&self) -> u64;
    fn read(&self, lba: u64, sectors: u8, buffer: &mut [u8]) -> Result<usize, AtaError>;
    fn write(&self, lba: u64, data: &[u8]) -> Result<(), AtaError>;
}

impl<D: BlockDevice> Fat32<D> {
    fn cluster_to_lba(&self, cluster: u32) -> u32 {
        self.data_start + (cluster - 2) * self.bpb.sectors_per_cluster as u32
    }

    fn cluster_to_fat_entry(&self, cluster: u32) -> SectorOffset {
        let fat_offset = cluster * 4;
        let fat_sector = self.fat_start + (fat_offset / self.bpb.bytes_per_sector as u32);
        let entry_offset = fat_offset % self.bpb.bytes_per_sector as u32;

        SectorOffset {
            sector: fat_sector,
            offset: entry_offset,
        }
    }

    pub fn new(bpb: BPB, fs_info: FSInfo, disk: D) -> Self {
        let fat_start = bpb.reserved_sector_count as u32;
        let data_start = fat_start + (bpb.num_fats as u32 * bpb.fat_size_32) as u32;

        Self {
            bpb,
            fs_info,
            fat_start,
            data_start,
            disk,
        }
    }

    pub fn read_buffer(&self, dir: &DirEntryWithLoc, buffer: &mut [u8]) -> usize {
        let target_read_bytes = core::cmp::min(buffer.len(), dir.entry.file_size as usize);
        if target_read_bytes == 0 {
            return 0;
        }

        let mut cluster = dir.entry.first_cluster();
        let mut bytes_read = 0;

        let cluster_bytes = self.bpb.sectors_per_cluster as usize * 512;

        while bytes_read < buffer.len() && !is_eoc(cluster) {
            let lba = self.cluster_to_lba(cluster) as u64;
            let remaining_to_read = target_read_bytes - bytes_read;
            let bytes_to_copy = core::cmp::min(cluster_bytes, remaining_to_read);

            let mut temp_cluster_buf = vec![0u8; cluster_bytes];

            self.disk
                .read(lba, self.bpb.sectors_per_cluster, &mut temp_cluster_buf);

            let dest_range = bytes_read..(bytes_read + bytes_to_copy);
            buffer[dest_range].copy_from_slice(&temp_cluster_buf[..bytes_to_copy]);

            bytes_read += bytes_to_copy;
            cluster = self.get_next_cluster(cluster);
        }

        bytes_read
    }

    fn compare_dirent(&self, segment: &str, dir: &DirEntry) -> bool {
        if let Some((filename, ext)) = segment.split_once(".") {
            self.compare_string_bytes(filename, &dir.name)
                && self.compare_string_bytes(ext, &dir.ext)
        } else {
            self.compare_string_bytes(segment, &dir.name) && dir.ext.iter().all(|x| *x == 0x20)
        }
    }

    fn compare_string_bytes(&self, string: &str, bytes: &[u8]) -> bool {
        let upper = string.to_uppercase();
        let target_bytes = upper.as_bytes();
        let actual_fat_len = bytes.iter().rposition(|&b| b != 0x20).map_or(0, |i| i + 1);
        target_bytes.len() == actual_fat_len && target_bytes == &bytes[0..actual_fat_len]
    }

    pub fn find(&self, parent_cluster: u32, name: &str) -> Option<DirEntryWithLoc> {
        let dir_list = self.read_dir_from_cluster(parent_cluster).into_iter();
        for dir in dir_list {
            if self.compare_dirent(name, &dir.entry) {
                return Some(dir);
            }
        }
        None
    }

    fn read_dir_from_cluster(&self, start_cluster: u32) -> Vec<DirEntryWithLoc> {
        let mut entries = vec![];
        let mut cluster = start_cluster;
        while !is_eoc(cluster) {
            // read data in cluster
            let lba = self.cluster_to_lba(cluster);
            let cluster_size = 512 * self.bpb.sectors_per_cluster as usize;
            let mut buffer = vec![0u8; cluster_size];
            self.disk
                .read(lba as u64, self.bpb.sectors_per_cluster, &mut buffer);

            // Parse DirEntries
            for i in 0..(cluster_size / 32) {
                let bytes = &buffer[(i * 32)..((i + 1) * 32)];
                let first_byte = bytes[0];
                // if invalid, stop immediately. no more entries to parse
                if first_byte == 0x00u8 {
                    break;
                }
                // if deleted
                if first_byte == 0xE5u8 {
                    continue;
                }
                let dir = unsafe { core::ptr::read_unaligned(bytes.as_ptr() as *const DirEntry) };
                let dir_with_loc = DirEntryWithLoc {
                    entry: dir,
                    loc: ClusterOffset {
                        cluster,
                        offset: (i * 32) as u32,
                    },
                };
                entries.push(dir_with_loc);
            }

            cluster = self.get_next_cluster(cluster);
        }

        entries
    }

    fn get_next_cluster(&self, cluster: u32) -> Fat32Entry {
        let start = self.cluster_to_fat_entry(cluster);
        let mut buffer = [0u8; 512];
        self.disk.read(start.sector as u64, 1, &mut buffer);

        let byte_offset = start.offset as usize;

        let bytes: [u8; 4] = buffer[byte_offset..byte_offset + 4].try_into().unwrap();

        u32::from_le_bytes(bytes) & 0x0FFFFFFF
    }

    /* Writing */

    fn link_clusters(&self, from_cluster: u32, to_cluster: u32) {
        let start = self.cluster_to_fat_entry(from_cluster);
        let mut buffer = [0u8; 512];
        self.disk.read(start.sector as u64, 1, &mut buffer);

        let old_value = u32::from_le_bytes(
            buffer[(start.offset as usize)..(start.offset + 4) as usize]
                .try_into()
                .unwrap(),
        );
        let new_value = (to_cluster & 0x0FFFFFFF) | (old_value & 0xF0000000);

        buffer[start.offset as usize..(start.offset + 4) as usize]
            .copy_from_slice(&new_value.to_le_bytes());
        self.disk.write(start.sector as u64, &buffer);
    }

    fn get_free_cluster(&self) -> Option<u32> {
        let start = self.bpb.root_cluster + 1;
        let total_num_clusters =
            (self.bpb.total_sectors_32 - self.data_start) / self.bpb.sectors_per_cluster as u32;

        for i in start..total_num_clusters {
            if self.get_next_cluster(i) == 0 {
                return Some(i);
            }
        }

        None
    }

    fn alloc_clusters(&self, n: usize) -> Option<u32> {
        if n == 0 {
            return None;
        }

        let start = self.get_free_cluster().unwrap();
        let mut curr = start;
        self.link_clusters(curr, EOC);

        for _ in 1..n {
            let new_cluster = self.get_free_cluster().unwrap();
            self.link_clusters(curr, new_cluster);
            self.link_clusters(new_cluster, EOC);
            curr = new_cluster;
        }

        self.link_clusters(curr, EOC);

        // TODO:
        // decrement FSInfo free_count
        // update next_free

        return Some(start);
    }

    fn free_clusters(&self, start: u32) {
        let mut curr = start;
        while !is_eoc(curr) {
            let next = self.get_next_cluster(curr);
            self.link_clusters(curr, 0);
            curr = next;
        }
    }

    fn cluster_offset_to_lba(&self, cluster: u32, pos: usize) -> u64 {
        let start_lba = self.cluster_to_lba(cluster) as u64;
        // figure out which sector index within cluster pos resides in
        let sector_index = (pos / 512) as u64;
        start_lba + sector_index
    }

    // only writes WITHIN a single cluster
    fn write_cluster(&self, cluster: u32, cluster_offset: usize, buffer: &[u8]) {
        serial_println!(
            "Writing to {cluster:x}:{cluster_offset:x} -- buflen: {}",
            buffer.len()
        );
        /*
          012 345 678 9..
        [ ooo|ooo|ooo|ooo ]
            ^ ^^^ ^
        b: [1,1,1,1,1]
              ^ ^ ^
        */

        /*
         * Assume buffer is never empty.
         *
         * Case 1: write_cluster translates into a single write_sector
         *          -> handles intra secor partial write
         * Case 2: everything is aligned, multiple sectors **perfect** n >= 2
         * Case 3: head is unaligned, NO middle, tail is unaligned
         * Case 4: head is unaligned, middle sector(s), tail is unaligned
         * Case 5: case 4 but head is aligned
         */

        let total_write_bytes = buffer.len();
        let end_pos = cluster_offset + total_write_bytes; // exclusive

        // If our head is unaligned
        let lba = self.cluster_offset_to_lba(cluster, cluster_offset) as u64;
        let mut buffer_cursor = 0;

        if cluster_offset % 512 != 0 || total_write_bytes <= 512 {
            // if NOT aligned to sector, then we must read and write to first sector
            let mut temp_sector = vec![0u8; 512];
            self.disk.read(lba, 1, &mut temp_sector);

            let head_bytes_written = core::cmp::min(buffer.len(), (512 - cluster_offset % 512));
            let src = &buffer[0..head_bytes_written];
            &temp_sector[(cluster_offset % 512)..(cluster_offset % 512 + src.len())]
                .copy_from_slice(src);
            self.disk.write(lba, &mut temp_sector);

            buffer_cursor = head_bytes_written;
        }

        if buffer_cursor == total_write_bytes {
            return;
        }

        // start should be aligned now

        // write the middle sectors normally
        let has_head = if buffer_cursor > 0 { 1 } else { 0 };
        let buffer_middle_start = buffer_cursor;
        let cluster_middle_end = align_down(end_pos as u64, 512) as usize;

        if cluster_middle_end > buffer_middle_start + cluster_offset {
            let middle_lba = lba + has_head;
            self.disk.write(
                middle_lba,
                &buffer[buffer_middle_start..(cluster_middle_end - cluster_offset)],
            );
        }

        if end_pos % 512 != 0 {
            // tail is unaligned!
            // apply same read + write pattern to tail sector
            let end_lba = self.cluster_offset_to_lba(cluster, end_pos);
            let mut temp_sector = vec![0u8; 512];
            self.disk.read(end_lba, 1, &mut temp_sector);
            &temp_sector[0..(end_pos % 512)]
                .copy_from_slice(&buffer[cluster_middle_end - cluster_offset..]);

            self.disk.write(end_lba, &mut temp_sector);
        }
    }

    pub fn write_buffer(&self, dir: &mut DirEntryWithLoc, offset: usize, buffer: &[u8]) -> usize {
        let cluster_bytes = self.bpb.sectors_per_cluster as usize * 512;

        let required_size = offset + buffer.len();
        let required_clusters = required_size.div_ceil(cluster_bytes);

        let curr_num_clusters = (dir.entry.file_size as usize).div_ceil(cluster_bytes);

        let cluster_index = offset / cluster_bytes;
        let cluster_offset = offset % cluster_bytes;

        // walk the chain up until start of cluster_index
        let mut curr = dir.entry.first_cluster();
        let mut prev = curr;
        let mut i = 0;
        while i < cluster_index && !is_eoc(curr) && curr != 0 {
            prev = curr;
            curr = self.get_next_cluster(curr);
            i += 1;
        }

        if curr_num_clusters < required_clusters {
            //  -> extend if needed
            let new_clusters = required_clusters - curr_num_clusters;
            let first_new = self.alloc_clusters(new_clusters).unwrap();

            // only if NOT new file (contains some cluster atleast)
            if prev != 0 {
                // we MUST find end of existing chain first
                let mut end = prev;
                loop {
                    let next = self.get_next_cluster(end);
                    if is_eoc(next) {
                        break;
                    }
                    end = next;
                }
                self.link_clusters(end, first_new);
            } else {
                dir.entry.set_first_cluster(first_new);
                self.update_dir(dir);
                serial_println!("Set {} = first_new", dir.entry.first_cluster());
            }

            if curr == 0 || is_eoc(curr) {
                curr = first_new;
            }
        }

        while i < cluster_index {
            prev = curr;
            curr = self.get_next_cluster(curr);
            i += 1;
        }

        let mut bytes_written = 0;

        // write into cluster at offset, and iterate through
        // first cluster, we can only write first cluster_bytes - cluster_offset bytes of buffer
        let mut i = core::cmp::min(cluster_bytes - cluster_offset, buffer.len());
        self.write_cluster(curr, cluster_offset, &buffer[0..i]);
        bytes_written += i + 1;
        while i < buffer.len() {
            curr = self.get_next_cluster(curr);
            let data = &buffer[i..core::cmp::min(i + cluster_bytes, buffer.len())];
            self.write_cluster(curr, 0, data);
            i += cluster_bytes;
            bytes_written += data.len();
        }

        if required_size > dir.entry.file_size as usize {
            dir.entry.file_size = required_size as u32;
            self.update_dir(dir);
        }

        bytes_written
    }

    fn update_dir(&self, dirloc: &DirEntryWithLoc) {
        let bytes: &[u8] = unsafe {
            slice::from_raw_parts(
                &dirloc.entry as *const DirEntry as *const u8,
                mem::size_of::<DirEntry>(),
            )
        };
        self.write_cluster(dirloc.loc.cluster, dirloc.loc.offset as usize, bytes);
    }

    fn get_free_dirent_pos(&self, first_cluster: u32) -> Option<ClusterOffset> {
        //  -> find empty 32 byte spot in parent dir
        let mut free_position = None;
        let mut cluster = first_cluster;
        'outer: while !is_eoc(cluster) {
            // read data in cluster
            let lba = self.cluster_to_lba(cluster);
            let cluster_size = 512 * self.bpb.sectors_per_cluster as usize;
            let mut buffer = vec![0u8; cluster_size];
            self.disk
                .read(lba as u64, self.bpb.sectors_per_cluster, &mut buffer);
            for i in 0..(cluster_size / 32) {
                let bytes = &buffer[(i * 32)..((i + 1) * 32)];
                let first_byte = bytes[0];
                if first_byte == 0x00u8 || first_byte == 0xE5 {
                    // first free byte
                    free_position = Some(ClusterOffset {
                        cluster,
                        offset: (i * 32) as u32,
                    });
                    break 'outer;
                }
            }
            cluster = self.get_next_cluster(cluster);
        }

        free_position
    }

    pub fn create_dir(&self, parent_direntry: &DirEntryWithLoc, dir_name: &str) {
        // TODO: handle directory full
        let free_position = self
            .get_free_dirent_pos(parent_direntry.entry.first_cluster())
            .unwrap();

        let mut name: [u8; 8] = [0x20; 8];
        let ext: [u8; 3] = [0x20; 3];
        for (i, c) in dir_name.chars().take(8).enumerate() {
            name[i] = c.to_ascii_uppercase() as u8;
        }

        let (fat_time, fat_date) = get_fat_timedate();

        let new_cluster = self.alloc_clusters(1).unwrap();

        // zero out cluster
        let cluster_size = 512 * self.bpb.sectors_per_cluster as usize;
        let zeros = vec![0u8; cluster_size];
        self.disk
            .write(self.cluster_to_lba(new_cluster) as u64, &zeros);

        let mut dot = DirEntry {
            name: DOT,
            ext: [0; 3],
            nt_reserved: 0u8,
            attr: DirEntryFlags::FAT_ATTR_DIRECTORY,
            creation_time_tenth: 0,
            creation_time: fat_time,
            creation_date: fat_date,
            last_access_date: fat_date,
            first_cluster_low: 0,
            first_cluster_high: 0,
            write_time: fat_time,
            write_date: fat_date,
            file_size: 0,
        };
        // point to current dir
        dot.set_first_cluster(free_position.cluster);

        let mut dotdot = DirEntry {
            name: DOT_DOT,
            ext: [0; 3],
            nt_reserved: 0u8,
            attr: DirEntryFlags::FAT_ATTR_DIRECTORY,
            creation_time_tenth: 0,
            creation_time: fat_time,
            creation_date: fat_date,
            last_access_date: fat_date,
            first_cluster_low: 0,
            first_cluster_high: 0,
            write_time: fat_time,
            write_date: fat_date,
            file_size: 0,
        };
        // point to parent dir
        let parent_cluster = if parent_direntry.loc.cluster == self.bpb.root_cluster {
            0
        } else {
            parent_direntry.loc.cluster
        };
        dotdot.set_first_cluster(parent_cluster);

        // persist
        self.update_dir(&DirEntryWithLoc {
            entry: dot,
            loc: ClusterOffset {
                cluster: new_cluster,
                offset: 0,
            },
        });
        self.update_dir(&DirEntryWithLoc {
            entry: dotdot,
            loc: ClusterOffset {
                cluster: new_cluster,
                offset: 32,
            },
        });

        let mut new_dir = DirEntry {
            name,
            ext,
            nt_reserved: 0u8,
            attr: DirEntryFlags::FAT_ATTR_DIRECTORY,
            creation_time_tenth: 0,
            creation_time: fat_time,
            creation_date: fat_date,
            last_access_date: fat_date,
            first_cluster_low: 0,
            first_cluster_high: 0,
            write_time: fat_time,
            write_date: fat_date,
            file_size: 0,
        };
        new_dir.set_first_cluster(new_cluster);

        let new_dir_with_loc = DirEntryWithLoc {
            entry: new_dir,
            loc: free_position,
        };
        self.update_dir(&new_dir_with_loc);
    }

    pub fn create_file(&self, parent_cluster: u32, name: &str) {
        //  -> find empty 32 byte spot in parent dir
        let free_position = self.get_free_dirent_pos(parent_cluster).unwrap();

        let (name_str, ext_str) = match name.rsplit_once(".") {
            Some((n, e)) => (n, e),
            None => (name, ""),
        };
        let mut name: [u8; 8] = [0x20; 8];
        let mut ext: [u8; 3] = [0x20; 3];
        for (i, c) in name_str.chars().take(8).enumerate() {
            name[i] = c.to_ascii_uppercase() as u8;
        }
        for (i, c) in ext_str.chars().take(8).enumerate() {
            ext[i] = c.to_ascii_uppercase() as u8;
        }

        let (fat_time, fat_date) = get_fat_timedate();

        let new_dir = DirEntry {
            name,
            ext,
            nt_reserved: 0u8,
            attr: DirEntryFlags::FAT_ATTR_ARCHIVE,
            creation_time_tenth: 0,
            creation_time: fat_time,
            creation_date: fat_date,
            last_access_date: fat_date,
            first_cluster_low: 0,
            first_cluster_high: 0,
            write_time: fat_time,
            write_date: fat_date,
            file_size: 0,
        };
        let new_dir_with_loc = DirEntryWithLoc {
            entry: new_dir,
            loc: free_position,
        };
        self.update_dir(&new_dir_with_loc);
    }

    pub fn mv(&self, dirent: &mut DirEntryWithLoc, to: &str) {
        let (name_str, ext_str) = match to.rsplit_once(".") {
            Some((n, e)) => (n, e),
            None => (to, ""),
        };
        let mut name: [u8; 8] = [0x20; 8];
        let mut ext: [u8; 3] = [0x20; 3];
        for (i, c) in name_str.chars().take(8).enumerate() {
            name[i] = c.to_ascii_uppercase() as u8;
        }
        for (i, c) in ext_str.chars().take(8).enumerate() {
            ext[i] = c.to_ascii_uppercase() as u8;
        }

        dirent.entry.name = name;
        dirent.entry.ext = ext;

        self.update_dir(&dirent);
    }

    pub fn delete_file(&self, dirent: &mut DirEntryWithLoc) {
        dirent.entry.name[0] = 0xE5;
        self.free_clusters(dirent.entry.first_cluster());
        self.update_dir(&dirent);
    }

    pub fn delete_dir(&self, dirent: &mut DirEntryWithLoc) {
        let children = self.read_dir_from_cluster(dirent.entry.first_cluster());

        let mut is_safe_to_del = true;
        for child in children {
            if !child.entry.is_dot() {
                is_safe_to_del = false;
            }
        }

        if is_safe_to_del {
            dirent.entry.name[0] = 0xE5;
            self.free_clusters(dirent.entry.first_cluster());
            self.update_dir(&dirent);
        }
    }

    pub fn get_root_inode(
        &self,
        ops: Arc<dyn INodeOps>,
        superblock: Arc<SuperBlock>,
    ) -> alloc::sync::Arc<INode> {
        let dirent = self.get_dir_entry(self.bpb.root_cluster as u64);
        Arc::new(INode {
            inum: dirent.entry.first_cluster() as u64,
            fs: superblock,
            meta: Mutex::new(vfs::FsMetadata {
                size: dirent.entry.file_size as usize,
                mtime: 0,
                dirty: false,
            }),
            mode: vfs::NodeType::Directory,
            data: INodeData::FatNode(Mutex::new(dirent)),
            ops,
        })
    }

    fn get_dir_entry(&self, inum: u64) -> DirEntryWithLoc {
        // inum is the byte offset in disk of DirEntry
        // convert to sector offset and read 32 bytes
        let mut buffer = [0; 512];
        let offset = (inum & 0x1FF) as usize;
        self.disk.read(inum >> 9, 1, &mut buffer);

        let entry_bytes = &buffer[offset..(offset + 32)];
        let dir = unsafe { core::ptr::read_unaligned(entry_bytes.as_ptr() as *const DirEntry) };

        // just reorganizing formula
        let bytes_per_cluster = (self.bpb.sectors_per_cluster as u64) * 512;
        let data_start_bytes = self.data_start as u64 * 512;
        let relative_bytes = inum - data_start_bytes;
        let cluster = (relative_bytes / bytes_per_cluster) as u32 + 2;
        let offset = (relative_bytes % bytes_per_cluster) as u32;

        DirEntryWithLoc {
            entry: dir,
            loc: ClusterOffset { cluster, offset },
        }
    }
}

impl<D: BlockDevice + Sync + Send> FileOps for Fat32<D> {
    fn read(&self, file: &vfs::File, buffer: &mut [u8]) -> usize {
        let INodeData::FatNode(data) = &file.inode.data else {
            panic!("fat32 driver received non-FAT inode {}!", file.inode.inum);
        };

        self.read_buffer(&data.lock(), buffer)
    }

    fn write(&self, file: &vfs::File, buffer: &[u8]) -> usize {
        let INodeData::FatNode(data) = &file.inode.data else {
            panic!("fat32 driver received non-FAT inode {}!", file.inode.inum);
        };
        self.write_buffer(&mut data.lock(), file.pos, buffer)
    }
}

impl<D: BlockDevice + Sync + Send + Clone + 'static> INodeOps for Fat32<D> {
    fn open(&self, inode: &INode, flags: vfs::OpenFlags) -> Arc<dyn vfs::FileOps> {
        Arc::new(self.clone())
    }

    fn lookup(&self, parent: &INode, name: &str) -> Option<INode> {
        let to_inode = |d: DirEntryWithLoc| {
            let bytes_per_cluster = (self.bpb.sectors_per_cluster as usize) * 512;
            let data_start_bytes = self.data_start as u64 * 512;

            let inum = data_start_bytes
                + (d.loc.cluster - 2) as u64 * bytes_per_cluster as u64
                + d.loc.offset as u64;

            let mode = if d.entry.attr.contains(DirEntryFlags::FAT_ATTR_DIRECTORY) {
                NodeType::Directory
            } else {
                NodeType::File
            };

            INode {
                inum,
                fs: parent.fs.clone(),
                meta: Mutex::new(vfs::FsMetadata {
                    size: d.entry.file_size as usize,
                    mtime: 0,
                    dirty: false,
                }),
                mode,
                ops: parent.ops.clone(),
                data: vfs::INodeData::FatNode(Mutex::new(d)),
            }
        };

        let dir = self.find(parent.inum as u32, name);
        dir.map(|d| to_inode(d))
    }
    fn create_file(&self, dir: &INode, name: &str) {
        self.create_file(dir.inum as u32, name);
    }

    fn rename(&self, node: &INode, to: &str) {
        let INodeData::FatNode(data) = &node.data else {
            panic!("fat32 driver received non-FAT inode {}!", node.inum);
        };
        self.mv(&mut data.lock(), to);
    }

    fn delete_file(&self, file: &INode) {
        let INodeData::FatNode(data) = &file.data else {
            panic!("fat32 driver received non-FAT inode {}!", file.inum);
        };
        self.delete_file(&mut data.lock());
    }

    fn mkdir(&self, dir: &INode, name: &str) {
        let INodeData::FatNode(data) = &dir.data else {
            panic!("fat32 driver received non-FAT inode {}!", dir.inum);
        };
        self.create_dir(&data.lock(), name);
    }

    fn rmdir(&self, dir: &INode) {
        let INodeData::FatNode(data) = &dir.data else {
            panic!("fat32 driver received non-FAT inode {}!", dir.inum);
        };
        self.delete_dir(&mut data.lock());
    }

    fn readdir(&self, dir: &INode) -> Vec<DEntryMinimal> {
        let attr_to_node_type =
            |attr: DirEntryFlags| match attr.contains(DirEntryFlags::FAT_ATTR_DIRECTORY) {
                true => NodeType::Directory,
                false => NodeType::File,
            };
        let entries = self.read_dir_from_cluster(dir.inum as u32);
        entries
            .iter()
            .map(|e| DEntryMinimal {
                name: e.entry.name_as_str(),
                inum: e.entry.first_cluster(),
                size: e.entry.file_size as usize,
                filetype: attr_to_node_type(e.entry.attr),
            })
            .collect()
    }

    fn stat(&self, node: &INode) -> Stat {
        let meta = node.meta.lock();
        Stat {
            dev: self.disk.id(),
            ino: node.inum,
            mode: node.mode,
            rdev: 0,
            nlink: 1,
            size: meta.size,
            blksize: 512,
            blocks: meta.size.div_ceil(512) as u64,
            mtime: meta.mtime,
        }
    }
}
