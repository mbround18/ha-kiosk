use embedded_graphics::pixelcolor::raw::RawU16;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use esp_idf_svc::sys::*;

mod kiosk;
mod splash;
mod trust_confirm;
pub use kiosk::{show_rooms, update_toggle, KioskButtons};
pub use splash::{show_finish_setup_splash, show_setup_splash};
pub use trust_confirm::{show_trust_confirm, TrustButtons};

/// This board's confirmed panel (research.md / user-confirmed hardware): EK79007, 1024x600,
/// via the espressif/esp32_p4_function_ev_board_noglib BSP (Cargo.toml `bindings_header`),
/// matching `bsp/display.h`'s `BSP_LCD_TYPE_1024_600` default.
pub const WIDTH: usize = 1024;
pub const HEIGHT: usize = 600;

/// Onboard MIPI-DSI display, brought up via the board's official BSP (`bsp_display_new`)
/// rather than hand-derived GPIO pins - see research.md's display bring-up decision for why
/// (the alternative risks another silent boot crash like the earlier chip-revision/Ethernet
/// issues). Drawing goes into a plain heap-backed buffer and is pushed to the panel via
/// `esp_lcd_panel_draw_bitmap` rather than writing directly into the DMA framebuffer returned
/// by `esp_lcd_dpi_panel_get_frame_buffer` - direct writes never appeared on screen in testing
/// (backlight came on, but content stayed blank/gray), almost certainly a cache-coherency gap
/// between CPU-cached PSRAM writes and the DMA engine reading the same memory;
/// `esp_lcd_panel_draw_bitmap` is the documented, cache-safe way to push pixel data to an
/// esp_lcd panel and is what every ESP-IDF example does.
pub struct Display {
    panel: esp_lcd_panel_handle_t,
    buffer: Vec<u16>,
    /// Null if `bsp_touch_new` failed - touch is a nice-to-have for the trust-confirmation
    /// prompt (`display::trust_confirm`), not required for the rest of the display to work.
    touch: esp_lcd_touch_handle_t,
    /// Whether `esp_lcd_touch_read_data` is currently failing - tracked purely so `poll_tap`
    /// logs on *transitions* (start/stop failing) rather than every single poll (this is called
    /// every `TICK`, so unconditional logging would flood the debug ring buffer in seconds). See
    /// `poll_tap`'s doc comment for why this exists at all.
    touch_failing: bool,
}

unsafe impl Send for Display {}

impl Display {
    pub fn init() -> anyhow::Result<Self> {
        let mut cfg: bsp_display_config_t = unsafe { std::mem::zeroed() };
        // Zero-initializing leaves `dsi_bus.lane_bit_rate_mbps` at 0, which
        // `esp_lcd_new_dsi_bus` rejects outright ("invalid lane bit rate 0.00") rather than
        // substituting the BSP's own documented default - found via on-hardware testing.
        cfg.dsi_bus.lane_bit_rate_mbps = BSP_LCD_MIPI_DSI_LANE_BITRATE_MBPS;
        let mut panel: esp_lcd_panel_handle_t = std::ptr::null_mut();
        let mut io: esp_lcd_panel_io_handle_t = std::ptr::null_mut();

        unsafe {
            esp!(bsp_display_new(&cfg, &mut panel, &mut io))?;
            // The EK79007 DPI panel driver doesn't implement on/off - it displays continuously
            // once initialized - and returns ESP_ERR_NOT_SUPPORTED rather than ESP_OK for this
            // call (found via on-hardware testing). Not fatal: the panel is already effectively
            // "on" at this point.
            match esp_lcd_panel_disp_on_off(panel, true) {
                ESP_OK => {}
                ESP_ERR_NOT_SUPPORTED => {}
                other => esp!(other)?,
            }
            esp!(bsp_display_brightness_init())?;
            esp!(bsp_display_backlight_on())?;
        }

        let mut touch: esp_lcd_touch_handle_t = std::ptr::null_mut();
        let touch_cfg = bsp_touch_config_t {
            dummy: std::ptr::null_mut(),
        };
        match unsafe { esp!(bsp_touch_new(&touch_cfg, &mut touch)) } {
            Ok(()) => {}
            Err(e) => {
                log::warn!("Touch init failed, continuing without touch input: {e}");
                touch = std::ptr::null_mut();
            }
        }

        Ok(Self {
            panel,
            buffer: vec![0u16; WIDTH * HEIGHT],
            touch,
            touch_failing: false,
        })
    }

    /// Backlight brightness via the BSP's PWM-driven backlight control (`bsp_display_brightness_
    /// init`'d in `init()` above) - used by `net::kiosk`'s idle auto-dim.
    pub fn set_brightness(&mut self, percent: u8) -> anyhow::Result<()> {
        unsafe { esp!(bsp_display_brightness_set(percent as i32))? };
        Ok(())
    }

    /// Polls the touch controller for a currently-pressed point, in panel pixel coordinates.
    /// Returns `None` if nothing is touched right now, or if touch init failed at startup.
    ///
    /// A screen that stops responding to taps after a long uptime, while everything else (the
    /// HA WebSocket refresh, WiFi) keeps working, has only one place to actually look: this
    /// function used to return `None` on a read failure with zero logging, indistinguishable
    /// from "nothing is touched right now" - the GT911 (new `i2c_master` driver) shares its I2C
    /// bus with the ES8311 audio codec (legacy I2C driver, `CONFIG_I2C_SKIP_LEGACY_CONFLICT_
    /// CHECK=y` - see `audio.rs`'s doc comment, which flagged this exact risk), so a bus fault
    /// surfacing only after extended runtime is a real possibility, not just theoretical.
    pub fn poll_tap(&mut self) -> Option<Point> {
        if self.touch.is_null() {
            return None;
        }
        unsafe {
            if esp_lcd_touch_read_data(self.touch) != ESP_OK {
                if !self.touch_failing {
                    self.touch_failing = true;
                    crate::debug_log::event("touch: esp_lcd_touch_read_data started failing - taps will stop registering until this recovers");
                }
                return None;
            }
            if self.touch_failing {
                self.touch_failing = false;
                crate::debug_log::event("touch: esp_lcd_touch_read_data recovered");
            }
            let mut x = [0u16; 1];
            let mut y = [0u16; 1];
            let mut strength = [0u16; 1];
            let mut point_num = 0u8;
            let touched = esp_lcd_touch_get_coordinates(
                self.touch,
                x.as_mut_ptr(),
                y.as_mut_ptr(),
                strength.as_mut_ptr(),
                &mut point_num,
                1,
            );
            (touched && point_num > 0).then(|| Point::new(x[0] as i32, y[0] as i32))
        }
    }

    fn set_pixel(&mut self, x: usize, y: usize, color: Rgb565) {
        if x >= WIDTH || y >= HEIGHT {
            return;
        }
        self.buffer[y * WIDTH + x] = RawU16::from(color).into_inner();
    }

    pub fn fill(&mut self, color: Rgb565) {
        let raw = RawU16::from(color).into_inner();
        self.buffer.fill(raw);
    }

    /// Push the drawn buffer to the panel. Must be called after drawing - nothing appears on
    /// screen until this runs (see the struct-level doc comment for why this isn't just a
    /// direct-framebuffer write).
    pub fn flush(&mut self) -> anyhow::Result<()> {
        unsafe {
            esp!(esp_lcd_panel_draw_bitmap(
                self.panel,
                0,
                0,
                WIDTH as i32,
                HEIGHT as i32,
                self.buffer.as_ptr() as *const std::ffi::c_void,
            ))?;
        }
        Ok(())
    }
}

impl DrawTarget for Display {
    type Color = Rgb565;
    type Error = std::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels {
            if coord.x >= 0 && coord.y >= 0 {
                self.set_pixel(coord.x as usize, coord.y as usize, color);
            }
        }
        Ok(())
    }
}

impl OriginDimensions for Display {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}
