//! Builds the firmware crate (release) and flashes it via `espflash`, automating the exact
//! manual sequence used throughout this project's development: build, locate the bootloader +
//! app ELF, then `espflash flash --bootloader <bootloader> <elf> --port <port>`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;

use crate::{port, repo};

pub fn run(port: Option<String>, monitor: bool) -> anyhow::Result<()> {
    let firmware_dir = repo::firmware_dir();
    anyhow::ensure!(
        firmware_dir.join("Cargo.toml").exists(),
        "expected {} to exist - is this repo laid out as usual?",
        firmware_dir.join("Cargo.toml").display()
    );

    println!("==> Building firmware (release)...");
    let mut build_cmd = Command::new("cargo");
    build_cmd.arg("build").arg("--release").current_dir(&firmware_dir);
    // `flasher` itself typically runs via `cargo run -p flasher` at the workspace root (stable
    // toolchain, no override there) - rustup sets `RUSTUP_TOOLCHAIN` for that invocation, and a
    // child process inherits it by default, which would otherwise *pin* this nested `cargo` to
    // the same (wrong) toolchain instead of letting it re-resolve firmware's own pinned nightly
    // via firmware/rust-toolchain.toml based on this child's own working directory.
    build_cmd.env_remove("RUSTUP_TOOLCHAIN");

    if std::env::var_os("LIBCLANG_PATH").is_none() {
        if let Some(libclang_dir) = find_libclang(&firmware_dir) {
            println!("    (LIBCLANG_PATH not set - using bundled {})", libclang_dir.display());
            build_cmd.env("LIBCLANG_PATH", libclang_dir);
        }
    }

    let status = build_cmd.status().context("failed to run `cargo build` - is cargo on your PATH?")?;
    anyhow::ensure!(status.success(), "firmware build failed");

    let target_dir = firmware_dir.join("target/riscv32imafc-esp-espidf/release");
    let bootloader = target_dir.join("bootloader.bin");
    let elf = target_dir.join("firmware");
    anyhow::ensure!(
        bootloader.exists() && elf.exists(),
        "build succeeded but expected artifacts are missing ({}, {}) - unexpected build layout?",
        bootloader.display(),
        elf.display()
    );

    let port = port::find_board_port(port.as_deref())
        .context("pass --port explicitly, or run `flasher ports` to see what's connected")?;

    println!("==> Flashing via {port}...");
    let mut flash_cmd = Command::new("espflash");
    flash_cmd.arg("flash").arg("--bootloader").arg(&bootloader).arg(&elf).arg("--port").arg(&port);
    if monitor {
        flash_cmd.arg("--monitor");
    }
    let status = flash_cmd
        .status()
        .context("failed to run `espflash` - install it with `cargo install espflash`")?;
    anyhow::ensure!(status.success(), "espflash failed");

    println!("==> Flashed.");
    Ok(())
}

/// This project's `LIBCLANG_PATH` need comes from bindgen (used by esp-idf-sys) needing a real
/// `libclang` shared library, which isn't always on a fresh machine's default search path. Rather
/// than requiring every contributor to discover and export this by hand, check the same bundled
/// copy (from the Python `clang` package, installed into `firmware/.tools-env`'s venv as part of
/// the ESP-IDF export tooling) that's been used throughout this project's development.
fn find_libclang(firmware_dir: &Path) -> Option<PathBuf> {
    let lib_dir = firmware_dir.join(".tools-env/.venv/lib");
    for entry in std::fs::read_dir(lib_dir).ok()?.flatten() {
        let candidate = entry.path().join("site-packages/clang/native/libclang.so");
        if candidate.exists() {
            return candidate.parent().map(Path::to_path_buf);
        }
    }
    None
}
