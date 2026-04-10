fn main() {
    println!("cargo:rustc-link-arg=-Tuserland.ld");
    println!("cargo:rerun-if-changed=userland.ld");
}
