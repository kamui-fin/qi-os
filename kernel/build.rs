fn main() {
    cc::Build::new().file("src/switch.s").compile("switch");

    println!("cargo:rustc-link-arg=-Tkernel.ld");
    println!("cargo:rerun-if-changed=kernel.ld");
    println!("cargo:rerun-if-changed=src/switch.s");
}
