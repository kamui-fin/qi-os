use crate::PHYS_MEM_OFFSET;

// 36 bytes
#[repr(C, packed)]
#[derive(Clone, Debug)]
pub struct RSDP {
    /// An 8 byte magic number used for locating the RSDP, containing RSD PTR.
    signature: [u8; 8],
    /// A byte used to verify the first 20 bytes of the RSDP.
    checksum: u8,
    /// An OEM-supplied string that identified the OEM.
    oemid: [u8; 6],
    /// The RSDP revision, used for determining which fields are available.
    revision: u8,
    /// A 32-bit physical address pointing to the RSDT.
    rsdt_address: u32,

    /// The size of the RSDP.
    length: u32,
    /// A 64-bit physical address pointing to the XSDT. If the revision is at least 2, the XSDT should be used regardless of architecture, as the RSDT was deprecated.
    xsdt_address: u64,
    /// A checksum used for the entire table.
    extended_checksum: u8,
    reserved: [u8; 3],
}

/* On IA-PC systems, the RSDP is either located within the first 1 KiB of the EBDA
(Extended BIOS Data Area; a 2 byte address to the start of it is located at 0x40E),
or in the memory region from 0x000E0000 to 0x000FFFFF.
To find the table, the operating system has to find the RSD PTR signature (notice the last space character)
in one of the two areas. The signature always starts on a 16 byte boundary. */

pub fn find_rsdp() -> RSDP {
    let mut start_addr: Option<u64> = None;

    let ebda_addr = (0x40E + PHYS_MEM_OFFSET) as *const u16;
    let ebda_base = (unsafe { *ebda_addr } as u64) << 4;
    // some firmware returns 0
    if ebda_base != 0 {
        // First 1 KB of the EBDA
        for addr in (ebda_base..(ebda_base + 1024)).step_by(16) {
            let magic =
                unsafe { core::slice::from_raw_parts((addr + PHYS_MEM_OFFSET) as *const u8, 8) };
            if magic == b"RSD PTR " {
                start_addr = Some(addr);
                break;
            }
        }
    }
    if start_addr.is_none() {
        for addr in (0xE0000..=0xFFFFF).step_by(16) {
            let magic =
                unsafe { core::slice::from_raw_parts((addr + PHYS_MEM_OFFSET) as *const u8, 8) };
            if magic == b"RSD PTR " {
                start_addr = Some(addr);
                break;
            }
        }

        if start_addr.is_none() {
            panic!("BIOS doesn't support ACPI");
        }
    }

    let start_addr = start_addr.unwrap();
    let rsdp = (start_addr + PHYS_MEM_OFFSET) as *const RSDP;
    let rsdp = unsafe { &*rsdp };

    let first_twenty: &[u8] = unsafe { core::slice::from_raw_parts(start_addr as *const u8, 20) };
    let mut calculated_checksum: u8 = 0;
    for byte in first_twenty {
        calculated_checksum = calculated_checksum.wrapping_add(*byte);
    }

    if calculated_checksum != 0 {
        panic!("broken rsdp");
    }

    if rsdp.revision >= 2 {
        let full_bytes: &[u8] =
            unsafe { core::slice::from_raw_parts(start_addr as *const u8, rsdp.length as usize) };
        let mut calculated_checksum: u8 = 0;
        for byte in full_bytes {
            calculated_checksum = calculated_checksum.wrapping_add(*byte);
        }
        if calculated_checksum != 0 {
            panic!("broken rsdp");
        }
    }

    return rsdp.clone();
}

#[repr(C, packed)]
#[derive(Debug, Clone)]
pub struct SdtHeader {
    pub signature: [u8; 4],
    pub length: u32,
    pub revision: u8,
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub oem_table_id: [u8; 8],
    pub oem_revision: u32,
    pub creator_id: u32,
    pub creator_revision: u32,
}

pub struct Rsdt {
    header: &'static SdtHeader,
    count: usize,
    base_entry_ptr: *const u32,
}

impl Rsdt {
    pub fn get_entry(&self, index: usize) -> u32 {
        let entry_addr = unsafe { self.base_entry_ptr.add(index).read_unaligned() };
        entry_addr
    }

    pub fn find_madt(&self) -> Option<&'static SdtHeader> {
        for i in 0..self.count {
            let entry =
                unsafe { &*((self.get_entry(i) as u64 + PHYS_MEM_OFFSET) as *const SdtHeader) };
            if &entry.signature == b"APIC" {
                return Some(entry);
            }
        }
        None
    }
}

pub fn get_rsdt(rsdp: &RSDP) -> Rsdt {
    let addr = (rsdp.rsdt_address as u64) + PHYS_MEM_OFFSET;
    let ptr = addr as *const SdtHeader;
    let header = unsafe { &*ptr };
    let header_size = core::mem::size_of::<SdtHeader>();

    let full_bytes: &[u8] =
        unsafe { core::slice::from_raw_parts(addr as *const u8, header.length as usize) };
    let mut calculated_checksum: u8 = 0;
    for byte in full_bytes {
        calculated_checksum = calculated_checksum.wrapping_add(*byte);
    }
    if calculated_checksum != 0 {
        panic!("broken rsdp");
    }

    let count = (header.length as usize - header_size) / 4;
    let base_entry_ptr = (addr + header_size as u64) as *const u32;

    Rsdt {
        header,
        count,
        base_entry_ptr,
    }
}
