#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

#[repr(C, packed)]
#[allow(dead_code)]
struct MemoryMapEntry {
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

#[repr(C)]
struct Elf64Ehdr {
    e_ident: [u8; 16],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
}

#[repr(C)]
struct Elf64Phdr {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
}

const VIRTUAL_BASE_ADDR: usize = 0xFFFFFFFF80000000;

#[repr(C)]
pub struct Screen {
    pub width: u32,
    pub height: u32,
    pub bpp: u32,
    pub bytes_per_pixel: u32,
    pub bytes_per_line: u32,
    pub screen_size: u32,
    pub screen_size_dqwords: u32,
    pub framebuffer: u32,
    pub x: u32,
    pub y: u32,
    pub x_max: u32,
    pub y_max: u32,
}

#[no_mangle]
#[link_section = ".text._start"]
pub extern "C" fn _start(screen: *const Screen) -> ! {
    let elf_base = VIRTUAL_BASE_ADDR + 0x1000000;

    unsafe {
        let ehdr = &*(elf_base as *const Elf64Ehdr);
        let phdrs = (elf_base + ehdr.e_phoff as usize) as *const Elf64Phdr;

        for i in 0..ehdr.e_phnum {
            let ph = &*(phdrs.add(i as usize));

            if ph.p_type != 1 {
                continue;
            }

            let src = elf_base + ph.p_offset as usize;
            let dest = ph.p_vaddr as *mut u8;

            let src_ptr = src as *const u8;
            let dst_ptr = dest as *mut u8;
            for i in 0..ph.p_filesz as usize {
                *dst_ptr.add(i) = *src_ptr.add(i);
            }

            let bss_start = (ph.p_vaddr + ph.p_filesz) as *mut u8;
            let bss_size = (ph.p_memsz - ph.p_filesz) as usize;
            for i in 0..bss_size {
                *bss_start.add(i) = 0;
            }
        }

        asm!(
            "mov rdi, {screen}",
            "jmp {addr}",              // Far return: pops addr into RIP and sel into CS
            screen = in(reg) screen as u64,
            addr = in(reg) ehdr.e_entry,
            options(noreturn)
        );
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
