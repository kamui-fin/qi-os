#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct MemoryMapEntry {
    /*
    24-byte entry follows this structure:

    Base Address (8 bytes)

    Length (8 bytes)

    Type (4 bytes) — 1 is "Available RAM", others are reserved.

    ACPI Flags (4 bytes) */
    base_address: u64,
    length: u64,
    mem_type: u32,
    acpi_flags: u32,
}
