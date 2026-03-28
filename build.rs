use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    // 1. Get the ELF path (provided by Cargo's artifact dependency)
    let kernel_elf = PathBuf::from(env::var("CARGO_BIN_FILE_KERNEL").unwrap());

    // 2. Define exactly where you want the Makefile to find it.
    // Let's put it right in the root of the workspace/project.
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let destination = manifest_dir.join("kernel.bin");

    // 3. Run objcopy to flatten the ELF into a temp bin file
    let temp_bin = kernel_elf.with_extension("bin_temp");
    let status = Command::new("llvm-objcopy")
        .arg("-O")
        .arg("binary")
        .arg(&kernel_elf)
        .arg(&temp_bin)
        .status()
        .expect("objcopy failed");

    if !status.success() {
        panic!("objcopy failed");
    }

    // 4. Move that temp bin to your fixed destination
    fs::copy(&temp_bin, &destination).unwrap();

    // Clean up temp
    let _ = fs::remove_file(temp_bin);

    println!(
        "cargo:warning=KERNEL_BIN_EXPORTED_TO: {}",
        destination.display()
    );
    println!("cargo:rerun-if-changed=linker.ld");
}
