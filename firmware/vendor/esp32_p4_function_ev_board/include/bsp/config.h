/*
 * SPDX-FileCopyrightText: 2024 Espressif Systems (Shanghai) CO LTD
 *
 * SPDX-License-Identifier: Apache-2.0
 */

#pragma once

/**************************************************************************************************
 * BSP configuration
 **************************************************************************************************/
// By default, this BSP is shipped with LVGL graphical library. Enabling this option will exclude it.
// If you want to use BSP without LVGL, select BSP version with 'noglib' suffix.
#if !defined(BSP_CONFIG_NO_GRAPHIC_LIB) // Check if the symbol is not coming from compiler definitions (-D...)
// Flipped from the upstream default (0): this app draws directly via embedded-graphics, not
// LVGL, and the `espressif/esp_lvgl_port` version this BSP's idf_component.yml pulled in didn't
// build against either of our pinned ESP-IDF versions (v5.5.3 or v5.5.4) - see
// specs/001-wifi-ha-provisioning/research.md's display bring-up decision. LVGL/esp_lvgl_port
// were also removed from idf_component.yml in this vendored copy to match.
#define BSP_CONFIG_NO_GRAPHIC_LIB (1)
#endif
