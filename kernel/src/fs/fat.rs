#[derive(Debug)]
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
#[derive(Debug)]
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
type FAT32_Entry = u32;

// Standard 8.3 format
// 32 bytes
struct DirEntry {
    name: [u8; 8],
    ext: [u8; 3],
    attr: u8,
    nt_reserved: u8,
    // Creation time in hundredths of a second, although the official FAT Specification from Microsoft says it is tenths of a second. Range 0-199 inclusive. Based on simple tests, Ubuntu16.10 stores either 0 or 100 while Windows7 stores 0-199 in this field.
    creation_time_tenth: u8,
    // The time that the file was created. Multiply Seconds by 2.
    /* Hour 	5 bits
    Minutes 	6 bits
    Seconds 	5 bits  */
    creation_time: u16,
    /* The date on which the file was created.
    Year 	7 bits
    Month 	4 bits
    Day 	5 bits */
    creation_date: u16,
    // Same format as the creation date.
    last_access_date: u16,
    // The high 16 bits of this entry's first cluster number.
    first_cluster_high: u16,
    // Same format as the creation time.
    write_time: u16,
    // Same format as the creation date.
    write_date: u16,
    // The low 16 bits of this entry's first cluster number. Use this number to find the first cluster for this entry.
    first_cluster_low: u16,
    // in bytes
    file_size: u32,
}

const FAT_ATTR_READ_ONLY: u8 = 0x01;
const FAT_ATTR_HIDDEN: u8 = 0x02;
const FAT_ATTR_SYSTEM: u8 = 0x04;
const FAT_ATTR_VOLUME_ID: u8 = 0x08;
const FAT_ATTR_DIRECTORY: u8 = 0x10;
const FAT_ATTR_ARCHIVE: u8 = 0x20;
const FAT_ATTR_LFN: u8 = 0x0F;

struct Fat32 {
    bpb: BPB,
}

// fn cluster_to_sector(&self, cluster: u32) -> u32 {}

/*
* Layout:
* << BPB >>
* << FSInfo >>
*
* << FAT1 >>
* << FAT2 >>
*
*
* << DATA REGION >>
*/

// Mount: read sector 0 of disk. Verify signature and parse PBP and FSInfo
