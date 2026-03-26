use std::{env, path::PathBuf};

fn main() {
    // set by cargo for the kernel artifact dependency
    let kernel_path = PathBuf::from(env::var("CARGO_BIN_FILE_KERNEL").unwrap());
    println!("cargo:warning=Debug: {:#?}", kernel_path);
}
