// in src/main.rs

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(xiangqios::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::panic::PanicInfo;
use xiangqios::println;

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    println!("Hello World{}", "!");

    xiangqios::init();

    // x86_64::instructions::interrupts::int3();
    
    // trigger a page fault
    /* unsafe {
        *(0xdeadbeef as *mut u8) = 42;
    }; */
    /* fn stack_overflow() {
        stack_overflow(); // for each recursion, the return address is pushed
    }

    // trigger a stack overflow
    stack_overflow(); */

    #[cfg(test)]
    test_main();

    println!("It did not crash!");
    xiangqios::hlt_loop();
}

/// This function is called on panic.
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("{}", info);
    xiangqios::hlt_loop();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    xiangqios::test_panic_handler(info)
}
