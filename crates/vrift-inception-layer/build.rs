//! Build script for vrift-shim
//!
//! Compiles C variadic wrappers that correctly handle va_list on macOS ARM64.
//! C compiler generates proper ABI code for variadic functions.

fn main() {
    // Compile C shim on macOS and Linux
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "macos" || target_os == "linux" {
        println!("cargo:rerun-if-changed=src/c/variadic_inception.c");

        cc::Build::new()
            .file("src/c/variadic_inception.c")
            .opt_level(3)
            .compile("variadic_inception");
    }
}
