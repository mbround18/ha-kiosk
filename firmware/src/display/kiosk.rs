use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::{Rgb565, Rgb888};
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Circle, CornerRadii, PrimitiveStyle, Rectangle, RoundedRectangle};
use embedded_graphics::text::{Alignment, Text};

use firmware_core::home::{format_climate_line, format_weather_line, Climate, Room, Weather};
use firmware_core::layout::{fit_text_scale, pick_layout, COLUMNS};

use super::{Display, HEIGHT, WIDTH};

/// Converts an 8-bit-per-channel color (as given in a hex palette) to this display's native
/// 5-6-5 format - `Rgb565::new` itself expects values already in that native bit depth, which is
/// easy to get wrong by hand, so this goes through `Rgb888` and lets embedded_graphics's own
/// conversion do the bit rescaling.
fn rgb(r: u8, g: u8, b: u8) -> Rgb565 {
    Rgb888::new(r, g, b).into()
}

/// The kiosk's color theme, dark-mode variant: `--text` and `--background` swapped from the
/// given light palette (dark slate background, light gray text) per explicit request, rest
/// unchanged. The toggle's "off" track and its muted/cooldown tints aren't part of that palette
/// (a toggle switch needs an inactive-state color the theme doesn't define) and are derived as
/// neutral grays in the same Tailwind family as the original `--background` (`#F3F4F6` is
/// gray-100) rather than invented from scratch.
fn theme_text() -> Rgb565 {
    rgb(0xF3, 0xF4, 0xF6)
}
fn theme_background() -> Rgb565 {
    rgb(0x0F, 0x17, 0x2A)
}
fn theme_primary() -> Rgb565 {
    rgb(0x4F, 0x46, 0xE5)
}
fn theme_primary_muted() -> Rgb565 {
    rgb(0xA5, 0xB4, 0xFC) // indigo-300 - lighter tint of --primary for the post-tap cooldown
}
fn theme_secondary() -> Rgb565 {
    rgb(0xFF, 0xFF, 0xFF)
}
fn theme_accent() -> Rgb565 {
    rgb(0xF5, 0x9E, 0x0B)
}
fn theme_off_track() -> Rgb565 {
    rgb(0xD1, 0xD5, 0xDB) // gray-300 - not in the palette; derived neutral for an inactive toggle
}
fn theme_off_track_muted() -> Rgb565 {
    rgb(0xE5, 0xE7, 0xEB) // gray-200
}
fn theme_unavailable() -> Rgb565 {
    rgb(0x37, 0x41, 0x51) // gray-700 - dim against the dark background, reads as disabled
}

const HEADER_HEIGHT: i32 = 150;
/// Reserved strip at the bottom of the screen for the compact climate/AC summary (see
/// `format_climate_line`) - kept to one small-text line since screen space is tight and this is
/// a secondary, nice-to-have readout compared to the room toggles above it.
const FOOTER_HEIGHT: i32 = 36;
const OUTER_MARGIN: i32 = 30;
const COLUMN_GAP: i32 = 40;

struct RoomButton {
    area_id: String,
    toggle: Rectangle,
}

/// On-screen hit regions from the most recent `show_rooms` draw, for hit-testing a
/// `Display::poll_tap()` point against. Each room now has a single toggle control (not separate
/// On/Off buttons) - `net::kiosk` decides on vs. off by looking at the room's currently-known
/// state.
pub struct KioskButtons(Vec<RoomButton>);

impl KioskButtons {
    /// The area id of whichever room's toggle `point` landed in, if any.
    pub fn hit(&self, point: Point) -> Option<&str> {
        self.0
            .iter()
            .find(|rb| rect_contains(&rb.toggle, point))
            .map(|rb| rb.area_id.as_str())
    }

    /// The on-screen rectangle for one room's toggle, for drawing an immediate state update
    /// right where the child/user just tapped (see `net::kiosk`'s post-tap handling).
    pub fn rect_for(&self, area_id: &str) -> Option<Rectangle> {
        self.0.iter().find(|rb| rb.area_id == area_id).map(|rb| rb.toggle)
    }
}

fn rect_contains(r: &Rectangle, p: Point) -> bool {
    p.x >= r.top_left.x
        && p.y >= r.top_left.y
        && p.x < r.top_left.x + r.size.width as i32
        && p.y < r.top_left.y + r.size.height as i32
}

/// Renders a 2-column grid of room name + On/Off toggle, with current weather + whole-home
/// average temperature/humidity above it. Row size (and therefore text size) is chosen
/// reactively (see `pick_layout`) so that, within reason, every room is shown rather than a
/// fixed number getting cut off - only an unreasonably large area count would still overflow,
/// in which case the excess is simply not shown (acceptable for how many areas a typical home
/// has).
pub fn show_rooms(
    display: &mut Display,
    rooms: &[Room],
    weather: Option<&Weather>,
    climate: Option<&Climate>,
) -> anyhow::Result<KioskButtons> {
    display.fill(theme_background());

    // Content here comes straight from live HA data (weather condition names, climate mode
    // breakdowns) whose length isn't under our control - a fixed text scale would eventually run
    // some combination off the edge of the screen, so both lines pick the largest scale (down to
    // 1) that actually fits the available width instead.
    let header_max_width = (WIDTH as i32) - 2 * OUTER_MARGIN;
    if let Some(line) = weather.map(format_weather_line).filter(|l| !l.is_empty()) {
        let scale = fit_text_scale(&line, 2, header_max_width);
        draw_text(display, &line, Point::new((WIDTH / 2) as i32, 40), theme_text(), Alignment::Center, scale);
    }
    draw_text(display, "Rooms", Point::new((WIDTH / 2) as i32, 115), theme_accent(), Alignment::Center, 2);

    if let Some(line) = climate
        .map(|c| format_climate_line(c, weather.map(|w| w.unit.as_str()).unwrap_or("°F")))
        .filter(|l| !l.is_empty())
    {
        let footer_max_width = (WIDTH as i32) - OUTER_MARGIN;
        let scale = fit_text_scale(&line, 1, footer_max_width);
        draw_text(
            display,
            &line,
            Point::new((WIDTH as i32) - OUTER_MARGIN, (HEIGHT as i32) - FOOTER_HEIGHT / 2 + 7),
            theme_text(),
            Alignment::Right,
            scale,
        );
    }

    let available_height = (HEIGHT as i32) - HEADER_HEIGHT - FOOTER_HEIGHT;
    let layout = pick_layout(rooms.len(), available_height);

    let column_width = (WIDTH as i32 - 2 * OUTER_MARGIN - (COLUMNS - 1) * COLUMN_GAP) / COLUMNS;
    let max_rows = (available_height / layout.row_height).max(1);
    let shown = &rooms[..rooms.len().min((max_rows * COLUMNS) as usize)];

    let mut buttons = Vec::with_capacity(shown.len());

    for (i, room) in shown.iter().enumerate() {
        let i = i as i32;
        let column = i % COLUMNS;
        let row = i / COLUMNS;
        let x = OUTER_MARGIN + column * (column_width + COLUMN_GAP);
        let y = HEADER_HEIGHT + row * layout.row_height;
        let row_center = y + layout.row_height / 2;

        draw_text(
            display,
            &room.name,
            Point::new(x, row_center + 7 * layout.text_scale),
            theme_text(),
            Alignment::Left,
            layout.text_scale,
        );

        let toggle_rect = Rectangle::new(
            Point::new(x + column_width - layout.toggle_width as i32, row_center - layout.toggle_height as i32 / 2),
            Size::new(layout.toggle_width, layout.toggle_height),
        );

        if room.unavailable {
            // Every light in this area is unavailable (physical power cut, lost network, etc.) -
            // HA can't confirm any real state here, so this toggle isn't drawn as on/off (that'd
            // be showing a potentially-fictional state) and isn't registered as tappable at all:
            // it's simply left out of `buttons`, so a tap here just falls through as if nothing
            // was there.
            draw_unavailable_toggle(display, toggle_rect);
        } else {
            draw_toggle(display, toggle_rect, room.is_on, false);
            buttons.push(RoomButton {
                area_id: room.area_id.clone(),
                toggle: toggle_rect,
            });
        }
    }

    display.flush()?;

    Ok(KioskButtons(buttons))
}

/// Redraws one room's toggle in its new (optimistic) position right after a tap, `muted` (a
/// desaturated track color) for the duration of that room's post-tap cooldown (see
/// `net::kiosk`'s `ACTION_COOLDOWN`) so it's visually obvious to a button-mashing kid that this
/// room just changed and further taps on it aren't doing anything yet.
pub fn update_toggle(display: &mut Display, rect: Rectangle, is_on: bool, muted: bool) {
    draw_toggle(display, rect, is_on, muted);
    display.flush().ok();
}

/// An iOS-style toggle switch: a pill-shaped track (`--primary`/on, neutral gray/off -
/// desaturated further while `muted`) with a `--secondary` (white) knob at the appropriate end.
/// Chosen over the old large On/Off block buttons specifically so two columns of rooms fit
/// comfortably on screen at once.
fn draw_toggle(display: &mut Display, rect: Rectangle, is_on: bool, muted: bool) {
    let track_color = match (is_on, muted) {
        (true, false) => theme_primary(),
        (true, true) => theme_primary_muted(),
        (false, false) => theme_off_track(),
        (false, true) => theme_off_track_muted(),
    };
    let radius = rect.size.height;
    RoundedRectangle::new(rect, CornerRadii::new(Size::new(radius, radius)))
        .into_styled(PrimitiveStyle::with_fill(track_color))
        .draw(display)
        .ok();

    let knob_diameter = rect.size.height.saturating_sub(8);
    let knob_y = rect.top_left.y + 4;
    let knob_x = if is_on {
        rect.top_left.x + rect.size.width as i32 - knob_diameter as i32 - 4
    } else {
        rect.top_left.x + 4
    };
    Circle::new(Point::new(knob_x, knob_y), knob_diameter)
        .into_styled(PrimitiveStyle::with_fill(theme_secondary()))
        .draw(display)
        .ok();
}

/// A flat, dim, disabled-looking track with a centered (not left/right, unlike an on/off toggle)
/// knob - deliberately doesn't look like a valid on or off position, since neither is true: HA
/// has told us it can't confirm this room's light state at all right now.
fn draw_unavailable_toggle(display: &mut Display, rect: Rectangle) {
    let radius = rect.size.height;
    RoundedRectangle::new(rect, CornerRadii::new(Size::new(radius, radius)))
        .into_styled(PrimitiveStyle::with_fill(theme_unavailable()))
        .draw(display)
        .ok();

    let knob_diameter = rect.size.height.saturating_sub(8);
    let knob_x = rect.top_left.x + (rect.size.width as i32 - knob_diameter as i32) / 2;
    let knob_y = rect.top_left.y + 4;
    Circle::new(Point::new(knob_x, knob_y), knob_diameter)
        .into_styled(PrimitiveStyle::with_fill(theme_off_track()))
        .draw(display)
        .ok();
}

/// Draws `text` at `scale`x size by rendering `FONT_10X20` (the largest font embedded_graphics
/// bundles) through `PixelDoubler`, which fans each source pixel out into a `scale`x`scale`
/// block on the real display - crisp, unlike interpolated scaling, since it's just replication.
/// `position` is in real screen pixel coordinates (same as every other coordinate in this
/// module); this function handles the scaled-space conversion internally.
fn draw_text(display: &mut Display, text: &str, position: Point, color: Rgb565, alignment: Alignment, scale: i32) {
    let style = MonoTextStyle::new(&FONT_10X20, color);
    if scale <= 1 {
        Text::with_alignment(text, position, style, alignment).draw(display).ok();
        return;
    }
    let scaled_position = Point::new(position.x / scale, position.y / scale);
    let mut doubler = PixelDoubler { display, scale };
    Text::with_alignment(text, scaled_position, style, alignment)
        .draw(&mut doubler)
        .ok();
}

/// A `DrawTarget` adapter that fans every pixel it's asked to draw out into a `scale`x`scale`
/// block on the wrapped `Display`.
struct PixelDoubler<'a> {
    display: &'a mut Display,
    scale: i32,
}

impl DrawTarget for PixelDoubler<'_> {
    type Color = Rgb565;
    type Error = std::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        let scale = self.scale;
        let expanded = pixels.into_iter().flat_map(move |Pixel(p, c)| {
            let base_x = p.x * scale;
            let base_y = p.y * scale;
            (0..scale).flat_map(move |dy| (0..scale).map(move |dx| Pixel(Point::new(base_x + dx, base_y + dy), c)))
        });
        self.display.draw_iter(expanded)
    }
}

impl OriginDimensions for PixelDoubler<'_> {
    fn size(&self) -> Size {
        let real = self.display.size();
        Size::new(real.width / self.scale as u32, real.height / self.scale as u32)
    }
}
