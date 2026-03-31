#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

pub const KERNEL_BASE_ADDR: usize = 0xFFFFFFFF80000000;

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
    unsafe {
        let (kernel_address, size) = load_kernel_elf();

        asm!(
            "mov rdi, {screen}",
            "mov rsi, {size}",
            "jmp {addr}",
            screen = in(reg) screen as u64,
            size = in(reg) size,
            addr = in(reg) kernel_address,
            options(noreturn)
        );
    }
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

unsafe fn load_kernel_elf() -> (u64, u64) {
    let elf_base = KERNEL_BASE_ADDR + 0x1000000;

    let ehdr = &*(elf_base as *const Elf64Ehdr);
    let phdrs = (elf_base + ehdr.e_phoff as usize) as *const Elf64Phdr;

    let mut min_vaddr = u64::MAX;
    let mut max_vaddr = 0;

    for i in 0..ehdr.e_phnum {
        let ph = &*(phdrs.add(i as usize));

        if ph.p_type != 1 {
            continue;
        }

        if ph.p_vaddr < min_vaddr {
            min_vaddr = ph.p_vaddr
        };
        if ph.p_vaddr + ph.p_memsz > max_vaddr {
            max_vaddr = ph.p_vaddr + ph.p_memsz
        };

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

    let kernel_size = max_vaddr - min_vaddr;

    (ehdr.e_entry, kernel_size)
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
