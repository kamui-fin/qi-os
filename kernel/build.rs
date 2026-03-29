fn main() {
    println!("cargo:rustc-link-arg=-Tkernel.ld");
    println!("cargo:rerun-if-changed=kernel.ld");
}
