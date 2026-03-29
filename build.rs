use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    let root_dir = env::current_dir().unwrap();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());

    println!("cargo:rerun-if-changed=bootloader/x86_64-stage-3.json");
    println!("cargo:rerun-if-changed=bootloader/src");

    // --- 1. Build Stage 3 via Recursive Cargo ---
    let status = Command::new(cargo)
        .args(["install", "--path"])
        .arg(root_dir.join("bootloader"))
        .args(["--root", out_dir.to_str().unwrap()])
        .args([
            "--target",
            root_dir
                .join("bootloader/x86_64-stage-3.json")
                .to_str()
                .unwrap(),
            "-Zjson-target-spec",
            "-Zbuild-std=core",
            "-Zbuild-std-features=compiler-builtins-mem",
            "--profile",
            "stage-3",
        ])
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .status()
        .expect("Failed to execute cargo for Stage 3");

    assert!(status.success(), "Bootloader Stage 3 build failed");

    // --- 2. Locate Artifacts ---
    let stage3_elf = out_dir.join("bin").join("bootloader");
    let kernel_elf =
        PathBuf::from(env::var("CARGO_BIN_FILE_KERNEL").expect("Kernel bin not found"));
    let stage3_bin = out_dir.join("stage3.bin");
    let boot_asm_bin = out_dir.join("boot.bin");
    let final_disk_img = root_dir.join("target").join("disk.img");

    // --- 3. Objcopy: Convert Stage 3 to Flat Binary (Keep Kernel as ELF!) ---
    let status = Command::new("llvm-objcopy")
        .args([
            "-O",
            "binary",
            stage3_elf.to_str().unwrap(),
            stage3_bin.to_str().unwrap(),
        ])
        .status()
        .unwrap();

    assert!(status.success(), "llvm-objcopy failed on Stage 3");

    // --- 4. Assemble Bootloader (ASM) ---
    let stage3_data = fs::read(&stage3_bin).unwrap();
    let kernel_data = fs::read(&kernel_elf).unwrap(); // Direct read of the ELF file

    let status = Command::new("nasm")
        .args([
            "-f",
            "bin",
            "os.asm",
            "-o",
            boot_asm_bin.to_str().unwrap(),
            &format!("-DKERNEL_BYTES={}", kernel_data.len()),
            &format!("-DSTAGE3_BYTES={}", stage3_data.len()),
        ])
        .current_dir(root_dir.join("bootloader/asm"))
        .status()
        .unwrap();

    assert!(status.success(), "NASM failed to assemble bootloader");

    // --- 5. Stitch Final Disk Image ---
    let mut disk_img = fs::read(&boot_asm_bin).unwrap();
    disk_img.extend_from_slice(&stage3_data);

    // Padding for kernel sector alignment
    let padding_needed = (512 - (disk_img.len() % 512)) % 512;
    disk_img.extend(std::iter::repeat(0).take(padding_needed));

    // Append the pristine ELF kernel
    disk_img.extend_from_slice(&kernel_data);

    fs::write(&final_disk_img, disk_img).expect("Failed to write final disk.img");
}
