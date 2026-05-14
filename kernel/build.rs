use std::{env, path::PathBuf, process::Command};

fn main() {
    cc::Build::new().file("src/switch.s").compile("switch");

    nasm_rs::Build::new()
        .file("src/ap_init.asm")
        .compile("ap_init")
        .unwrap();

    println!("cargo:rustc-link-lib=static=ap_init");

    println!("cargo:rustc-link-arg=-Tkernel.ld");
    println!("cargo:rerun-if-changed=kernel.ld");

    println!("cargo:rerun-if-changed=src/switch.s");
    println!("cargo:rerun-if-changed=src/ap_init.asm");
}
