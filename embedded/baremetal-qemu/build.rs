//! Put the target's memory linker script on the linker search path. Both
//! cortex-m-rt's (ARM) and riscv-rt's (RISC-V) generated `link.x` pull it in
//! via the same literal `INCLUDE memory.x` directive, so this picks the
//! arch-appropriate source content and writes it under that name into
//! `OUT_DIR` (never a collision: each target triple builds into its own).
//!
//! The two source files are named `memory-arm.x` / `memory-riscv.x` — NOT
//! `memory.x` — deliberately: GNU ld's (and rust-lld's `-flavor gnu`)
//! `INCLUDE` directive searches the linker's current *working* directory
//! (the crate root, where cargo invokes the linker from) before it searches
//! `-L` directories. A `memory.x` sitting directly in the crate root would
//! therefore always win over this `OUT_DIR`+`-L` mechanism regardless of
//! target — which is exactly what happened here: with a checked-in
//! `memory.x`, ARM builds worked by accident (its FLASH/RAM layout happens
//! to satisfy cortex-m-rt), while RISC-V builds failed with "memory region
//! not defined: REGION_TEXT" because ld found the ARM file via the crate
//! root before ever consulting `-L`. Renaming both sources off the literal
//! `memory.x` name removes the ambiguity so `-L` resolution is the only path.
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let target = env::var("TARGET").unwrap();
    let memory_x: &[u8] = if target.starts_with("riscv32") {
        include_bytes!("memory-riscv.x")
    } else {
        include_bytes!("memory-arm.x")
    };
    fs::write(out.join("memory.x"), memory_x).unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory-arm.x");
    println!("cargo:rerun-if-changed=memory-riscv.x");
    println!("cargo:rerun-if-changed=build.rs");
}
