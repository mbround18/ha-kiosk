//! Locates this repo's important paths relative to `flasher`'s own crate directory (rather than
//! the current working directory), so `flasher` works the same whether it's run from the repo
//! root, from inside `flasher/`, or installed and invoked from elsewhere entirely.

use std::path::{Path, PathBuf};

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("flasher/ always has a parent directory")
        .to_path_buf()
}

pub fn firmware_dir() -> PathBuf {
    root().join("firmware")
}

pub fn env_file() -> PathBuf {
    root().join(".env")
}
