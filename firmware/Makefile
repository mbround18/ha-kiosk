# Convenience wrapper around the cargo/espflash workflow documented in this repo's history.
#
# LIBCLANG_PATH: needed for bindgen (regenerating the BSP FFI bindings) whenever the esp-idf-sys
# build cache is missing/cleared - it's not bundled with this system's clang/LLVM install (only
# libclang-cpp is). Points at a copy pulled in by a pip/uv-installed `libclang` package; override
# with `make build LIBCLANG_PATH=/other/path` if that cache location ever changes.
LIBCLANG_PATH ?= /home/mbruno/.cache/uv/archive-v0/zssjYEjMnrRREcZR/libclang-18.1.1.data/platlib/clang/native
export LIBCLANG_PATH

PORT ?= /dev/ttyACM0
TARGET := riscv32imafc-esp-espidf

.PHONY: build release flash flash-release monitor clean chmod-port

build: ## Debug build (target/$(TARGET)/debug/firmware)
	cargo build

release: ## Release build (target/$(TARGET)/release/firmware)
	cargo build --release

# This board's native USB-JTAG-serial port doesn't reliably reset into app-run mode with a bare
# `espflash monitor` - only `espflash flash --monitor` triggers a proper reset. See
# project_hardware memory / commit history for the full story.
flash: chmod-port ## Flash + monitor the debug build
	espflash flash --bootloader target/$(TARGET)/debug/bootloader.bin --monitor \
		target/$(TARGET)/debug/firmware --port $(PORT)

flash-release: chmod-port ## Flash + monitor the release build
	espflash flash --bootloader target/$(TARGET)/release/bootloader.bin --monitor \
		target/$(TARGET)/release/firmware --port $(PORT)

monitor: chmod-port ## Attach to the running device's serial console
	espflash flash --bootloader target/$(TARGET)/debug/bootloader.bin --monitor \
		target/$(TARGET)/debug/firmware --port $(PORT)

# The serial port's group (dialout) often isn't active in the current shell session even when
# the user account is a member (needs a fresh login to pick up), and permissions reset to
# root:dialout on every USB replug/chip reset. This still needs a password unless the user has
# passwordless sudo configured - it will prompt.
chmod-port:
	@if [ ! -w $(PORT) ]; then sudo chmod 666 $(PORT); fi

clean: ## Full clean (forces the next build to regenerate all FFI bindings/CMake output)
	cargo clean
