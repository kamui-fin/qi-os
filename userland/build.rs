fn main() {
    println!("cargo:rustc-link-arg-bins=-Tuserland.ld");
    println!("cargo:rerun-if-changed=userland.ld");
}
