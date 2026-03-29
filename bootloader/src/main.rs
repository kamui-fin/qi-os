#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

use crate::printer::print_debug;

mod printer;

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

#[no_mangle]
#[link_section = ".text._start"]
pub extern "C" fn _start() -> ! {
    print_debug(&"Rust stage3 bootloader activated", 0, 0, 0x0F);

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
            "push {sel:r}",       // Push segment selector (cast to register width)
            "push {addr}",        // Push the offset (entry point)
            "retfq",              // Far return: pops addr into RIP and sel into CS
            sel = in(reg) 0x08u64,
            addr = in(reg) ehdr.e_entry,
            options(noreturn)
        );
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // 0x4F is White text on a Red background
    print_debug(_info, 24, 0, 0x4F);
    loop {}
}
