#![no_std]
#![no_main]

extern crate alloc;
use alloc::{
    ffi::CString,
    string::{String, ToString},
    vec::Vec,
};
use core::{
    arch::global_asm,
    ffi::{c_char, CStr},
    panic::PanicInfo,
};
use userland::{
    chdir, close, execvp, exit, fork, get_dents, init_heap, open, print, println, pwd, read,
    waitpid, 

};

global_asm!(
    ".section .text._start",
    ".global _start",
    "_start:      ",
    "   xor rbp, rbp ",
    "   pop rdi      ",
    "   mov rsi, rsp ",
    "   and rsp, -16 ",
    "   call main    ",
    "   mov rdi, rax ",
    "   mov rax, 20   ",
    "   int 0x80     ",
);

fn read_line() -> String {
    let mut buffer = [0u8; 100];
    let bytes_read = read(0, &mut buffer);
    let s = str::from_utf8(&buffer[..bytes_read]).expect("Invalid UTF-8");
    s.to_string()
}

const PROMPT_PREFIX: &'static str = "root@qi:";

#[no_mangle]
pub extern "C" fn main(argc: usize, argv: *const *const c_char) -> ! {
    init_heap();

    let mut cwd = pwd();
    print!("{}{cwd} $ ", PROMPT_PREFIX);
    loop {
        let line = read_line();

        if !line.trim().is_empty() {
            let parsed = parse_shell_input(line);
            if let Some(parsed) = parsed {
                let program_name = parsed[0].clone();
                let args = &parsed[1..];

                // TODO: fg, bg, jobs
                match program_name.as_str() {
                    "cd" => chdir(args[0].clone()),
                    "ls" => {
                        let fd = open(cwd.as_str());
                        let entries = get_dents(fd);
                        for entry in entries {
                            println!("{}", entry);
                        }
                        close(fd);
                    }
                    "pwd" => {
                        println!("{}", cwd);
                    }
                    "export" => {}
                    "unset" => {}
                    _ => {
                        // treat as standard program
                        /* let childpid = fork();
                        if childpid == 0 {
                            execvp(&program_name, args);
                        } else {
                            waitpid(childpid);
                        } */
                    }
                }
            }
        }
        cwd = pwd();
        print!("{}{cwd} $ ", PROMPT_PREFIX);
    }
}

fn parse_shell_input(line: String) -> Option<Vec<String>> {
    shell_words::split(&line).ok()
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    println!("panic: {:#?}", _info);
    exit(1);
    unreachable!();
}
