/* Exposes the espressif/esp32_p4_function_ev_board_noglib BSP's display API to Rust via
 * esp-idf-sys bindgen (firmware/Cargo.toml's `bindings_header` for that extra_component). See
 * specs/001-wifi-ha-provisioning/research.md's display bring-up decision. */
#include "bsp/display.h"
#include "bsp/touch.h"
#include "bsp/esp32_p4_function_ev_board.h"
