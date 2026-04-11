#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // TEST SYSTEM CALL
    unsafe {
        core::arch::asm!(
            "mov rax, 0x7",
            "mov rdi, 0xDEADBEEF",
            "int 0x80",
            options(noreturn)
        );
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
