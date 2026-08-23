use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::{Alignment, Text};

use super::{Display, HEIGHT, WIDTH};

const BLACK: Rgb565 = Rgb565::BLACK;
const WHITE: Rgb565 = Rgb565::WHITE;

const BUTTON_WIDTH: u32 = 260;
const BUTTON_HEIGHT: u32 = 90;
const BUTTON_Y: i32 = HEIGHT as i32 - 170;

/// On-screen hit regions for the two buttons drawn by `show_trust_confirm`, so
/// `net::ap_portal`'s main loop can test a `Display::poll_tap()` point against them without
/// redoing the layout math itself.
pub struct TrustButtons {
    trust: Rectangle,
    cancel: Rectangle,
}

/// `Some(true)` for a tap on Trust, `Some(false)` for Cancel, `None` if `point` hit neither.
impl TrustButtons {
    pub fn hit(&self, point: Point) -> Option<bool> {
        if rect_contains(&self.trust, point) {
            Some(true)
        } else if rect_contains(&self.cancel, point) {
            Some(false)
        } else {
            None
        }
    }
}

fn rect_contains(r: &Rectangle, p: Point) -> bool {
    p.x >= r.top_left.x
        && p.y >= r.top_left.y
        && p.x < r.top_left.x + r.size.width as i32
        && p.y < r.top_left.y + r.size.height as i32
}

/// Shown whenever every automatic HTTPS trust path (`ha::rest::verify_connection`) has failed
/// and a human needs to decide whether to trust the certificate `ha::tofu::fetch_peer_cert`
/// captured - since nothing in firmware can tell a legitimate private CA from a MITM's own, this
/// is the only thing standing between "unreachable" and blindly trusting an unverified cert.
pub fn show_trust_confirm(
    display: &mut Display,
    host: &str,
    issuer: &str,
    fingerprint: &str,
) -> anyhow::Result<TrustButtons> {
    display.fill(WHITE);

    let heading_style = MonoTextStyle::new(&FONT_10X20, BLACK);
    // Bumped from FONT_6X10 - this notice text was reported hard to read on the physical
    // screen, unlike the buttons (already FONT_10X20).
    let label_style = MonoTextStyle::new(&FONT_10X20, BLACK);
    let center_x = (WIDTH / 2) as i32;

    Text::with_alignment(
        "Untrusted certificate found",
        Point::new(center_x, 60),
        heading_style,
        Alignment::Center,
    )
    .draw(display)
    .ok();

    // The full fingerprint (32 bytes, hex+colons) is ~95 characters - too wide to read
    // comfortably at this larger font size. Truncated here for display only; the full value is
    // still what's sent over the API and what actually gets pinned.
    let short_fingerprint = fingerprint
        .splitn(9, ':')
        .take(8)
        .collect::<Vec<_>>()
        .join(":");

    for (i, line) in [
        format!("Host: {host}"),
        format!("Issuer: {issuer}"),
        format!("Fingerprint: {short_fingerprint}..."),
        "Only trust if you recognize this (e.g. your router).".to_string(),
    ]
    .iter()
    .enumerate()
    {
        Text::with_alignment(
            line,
            Point::new(center_x, 110 + i as i32 * 30),
            label_style,
            Alignment::Center,
        )
        .draw(display)
        .ok();
    }

    let trust = Rectangle::new(
        Point::new(center_x - BUTTON_WIDTH as i32 - 20, BUTTON_Y),
        Size::new(BUTTON_WIDTH, BUTTON_HEIGHT),
    );
    let cancel = Rectangle::new(
        Point::new(center_x + 20, BUTTON_Y),
        Size::new(BUTTON_WIDTH, BUTTON_HEIGHT),
    );

    draw_button(display, &trust, Rgb565::GREEN, "Trust");
    draw_button(display, &cancel, Rgb565::RED, "Cancel");

    display.flush()?;

    Ok(TrustButtons { trust, cancel })
}

fn draw_button(display: &mut Display, rect: &Rectangle, fill: Rgb565, label: &str) {
    rect.into_styled(PrimitiveStyle::with_fill(fill))
        .draw(display)
        .ok();
    let text_style = MonoTextStyle::new(&FONT_10X20, WHITE);
    Text::with_alignment(
        label,
        Point::new(
            rect.top_left.x + rect.size.width as i32 / 2,
            rect.top_left.y + rect.size.height as i32 / 2 + 7,
        ),
        text_style,
        Alignment::Center,
    )
    .draw(display)
    .ok();
}
