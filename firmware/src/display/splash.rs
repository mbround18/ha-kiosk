use embedded_graphics::mono_font::ascii::{FONT_10X20, FONT_6X10};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Alignment, Text};
use qrcode::QrCode;

use super::{Display, HEIGHT, WIDTH};

const BLACK: Rgb565 = Rgb565::BLACK;
const WHITE: Rgb565 = Rgb565::WHITE;

/// Shown on the device's own screen at boot whenever it has no working saved network
/// connection (main.rs's boot decision) - lets a user standing in front of the physical kiosk
/// scan their way in, rather than needing to already be on the network to see any instructions
/// (which ruled out putting these QR codes on the captive-portal web page itself).
pub fn show_setup_splash(display: &mut Display, ssid: &str, portal_url: &str) -> anyhow::Result<()> {
    display.fill(WHITE);

    let heading_style = MonoTextStyle::new(&FONT_10X20, BLACK);
    let label_style = MonoTextStyle::new(&FONT_6X10, BLACK);

    Text::with_alignment(
        "Set up this kiosk",
        Point::new((WIDTH / 2) as i32, 50),
        heading_style,
        Alignment::Center,
    )
    .draw(display)
    .ok();

    let qr_size = 280u32;
    let quarter = (WIDTH / 4) as i32;
    // Centered between the heading (ends ~y=65) and the SSID label at the bottom (HEIGHT - 40):
    // half the QR (140) plus its label (~22px above it) must both fit above this center point,
    // and the QR's bottom edge must clear the SSID label below it. y=110 previously put the QR's
    // top edge above y=0 (off-screen) since draw_qr_panel centers on the point it's given.
    let qr_center_y = (HEIGHT / 2) as i32 + 30;

    // Open network (no password) - net::ap_portal::run's AccessPointConfiguration uses
    // AuthMethod::None; see contracts/captive-portal-api.md.
    let wifi_qr = format!("WIFI:T:nopass;S:{ssid};;");
    draw_qr_panel(
        display,
        &wifi_qr,
        Point::new(quarter, qr_center_y),
        qr_size,
        "1. Scan to join WiFi",
        label_style,
    );

    draw_qr_panel(
        display,
        portal_url,
        Point::new(quarter * 3, qr_center_y),
        qr_size,
        "2. Scan to open setup",
        label_style,
    );

    Text::with_alignment(
        ssid,
        Point::new((WIDTH / 2) as i32, (HEIGHT - 40) as i32),
        label_style,
        Alignment::Center,
    )
    .draw(display)
    .ok();

    display.flush()?;

    Ok(())
}

/// Shown once WiFi is already configured (either a fresh join just completed, or the device
/// rebooted with saved WiFi credentials but HA/dashboard setup still isn't done) - a phone is
/// already on the same network at this point, so there's no need for the WiFi-join QR or the
/// step instructions from `show_setup_splash`, which would otherwise sit on screen implying
/// WiFi still needs to be joined and confuse the user.
pub fn show_finish_setup_splash(display: &mut Display, portal_url: &str) -> anyhow::Result<()> {
    display.fill(WHITE);

    let heading_style = MonoTextStyle::new(&FONT_10X20, BLACK);
    let label_style = MonoTextStyle::new(&FONT_6X10, BLACK);

    Text::with_alignment(
        "Finish setup",
        Point::new((WIDTH / 2) as i32, 50),
        heading_style,
        Alignment::Center,
    )
    .draw(display)
    .ok();

    draw_qr_panel(
        display,
        portal_url,
        Point::new((WIDTH / 2) as i32, (HEIGHT / 2) as i32 + 30),
        320,
        "Scan to configure Home Assistant",
        label_style,
    );

    display.flush()?;

    Ok(())
}

fn draw_qr_panel(
    display: &mut Display,
    data: &str,
    center: Point,
    size: u32,
    label: &str,
    label_style: MonoTextStyle<Rgb565>,
) {
    let Ok(code) = QrCode::new(data.as_bytes()) else {
        return;
    };
    let modules = code.width() as u32;
    let module_px = (size / modules).max(1);
    let rendered_size = module_px * modules;
    let origin = Point::new(
        center.x - (rendered_size as i32) / 2,
        center.y - (rendered_size as i32) / 2,
    );

    Text::with_alignment(
        label,
        Point::new(center.x, origin.y - 12),
        label_style,
        Alignment::Center,
    )
    .draw(display)
    .ok();

    let colors = code.to_colors();
    for (i, module) in colors.iter().enumerate() {
        if *module == qrcode::Color::Light {
            continue;
        }
        let mx = (i as u32 % modules) as i32;
        let my = (i as u32 / modules) as i32;
        let _ = Rectangle::new(
            Point::new(
                origin.x + mx * module_px as i32,
                origin.y + my * module_px as i32,
            ),
            Size::new(module_px, module_px),
        )
        .into_styled(PrimitiveStyle::with_fill(BLACK))
        .draw(display);
    }
}
