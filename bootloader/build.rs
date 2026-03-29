fn main() {
    println!("cargo:rustc-link-arg=-Tbootloader.ld");
    println!("cargo:rerun-if-changed=bootloader.ld");
}
