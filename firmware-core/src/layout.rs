//! Reactive sizing math for the kiosk's 2-column room grid (`firmware/src/display/kiosk.rs`
//! does the actual embedded_graphics drawing) - computed continuously from how many rooms need
//! to fit, rather than a fixed handful of tiers, so a 5-room home and a 15-room home each get the
//! largest text/toggle that still lets every room fit in the available vertical space.

pub const COLUMNS: i32 = 2;

/// Below this, the kiosk's font at scale 1 (20px tall) plus a usable toggle no longer fits
/// comfortably - the floor this reactive sizing shrinks toward for a large room count.
pub const MIN_ROW_HEIGHT: i32 = 44;
/// Above this, more vertical room just adds dead space rather than more legibility - the ceiling
/// this reactive sizing grows toward for a small room count (matches what looked right for this
/// board's 5-room test home).
pub const MAX_ROW_HEIGHT: i32 = 92;

/// Width in pixels of one character of the kiosk's font (`FONT_10X20`, embedded_graphics's
/// built-in ASCII bitmap font - see `firmware::display::kiosk`) at scale 1, before the
/// `PixelDoubler`-style integer-replication scaling used for bigger text.
pub const GLYPH_WIDTH: i32 = 10;

pub fn text_width(text: &str, scale: i32) -> i32 {
    text.chars().count() as i32 * GLYPH_WIDTH * scale
}

/// The largest scale (down to 1) at which `text` fits within `max_width` pixels, starting from
/// `max_scale`. Lets header/footer text that varies with live data (weather condition names,
/// climate mode breakdowns, etc.) shrink automatically instead of running off the edge of the
/// screen, rather than always rendering at a fixed scale regardless of content length.
pub fn fit_text_scale(text: &str, max_scale: i32, max_width: i32) -> i32 {
    let mut scale = max_scale.max(1);
    while scale > 1 && text_width(text, scale) > max_width {
        scale -= 1;
    }
    scale
}

pub struct Layout {
    pub text_scale: i32,
    pub row_height: i32,
    pub toggle_width: u32,
    pub toggle_height: u32,
}

pub fn pick_layout(room_count: usize, available_height: i32) -> Layout {
    let room_count = (room_count as i32).max(1);
    let rows_needed = (room_count + COLUMNS - 1) / COLUMNS;
    let row_height = (available_height / rows_needed).clamp(MIN_ROW_HEIGHT, MAX_ROW_HEIGHT);

    // The kiosk's font is 20px tall at scale 1, 40px at scale 2 - only step up to 2x once the
    // row has comfortable room for it (glyph height + padding + a same-height toggle).
    let text_scale = if row_height >= 80 { 2 } else { 1 };

    let toggle_height = (row_height - 14).clamp(28, 64) as u32;
    let toggle_width = (toggle_height * 2).max(60);

    Layout { text_scale, row_height, toggle_width, toggle_height }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn few_rooms_get_the_maximum_row_height_and_large_text() {
        // 1 row needed either way - plenty of vertical room, so this should hit the ceiling.
        let layout = pick_layout(1, 450);
        assert_eq!(layout.row_height, MAX_ROW_HEIGHT);
        assert_eq!(layout.text_scale, 2);

        let layout = pick_layout(2, 450);
        assert_eq!(layout.row_height, MAX_ROW_HEIGHT);
    }

    #[test]
    fn five_rooms_matches_the_known_good_profile() {
        // The board's real 5-room test home: 2 columns -> 3 rows, 450px available.
        let layout = pick_layout(5, 450);
        assert_eq!(layout.row_height, 92);
        assert_eq!(layout.text_scale, 2);
    }

    #[test]
    fn fifteen_rooms_shrinks_to_smaller_text_but_stays_above_the_floor() {
        // 2 columns -> 8 rows, 450px available -> 56px/row.
        let layout = pick_layout(15, 450);
        assert_eq!(layout.row_height, 56);
        assert_eq!(layout.text_scale, 1);
        assert!(layout.row_height >= MIN_ROW_HEIGHT);
    }

    #[test]
    fn absurd_room_count_clamps_to_the_minimum_row_height() {
        let layout = pick_layout(100, 450);
        assert_eq!(layout.row_height, MIN_ROW_HEIGHT);
    }

    #[test]
    fn zero_rooms_does_not_panic_and_uses_the_max_profile() {
        let layout = pick_layout(0, 450);
        assert_eq!(layout.row_height, MAX_ROW_HEIGHT);
    }

    #[test]
    fn fit_text_scale_keeps_max_scale_when_it_fits() {
        // "72F Sunny" - 9 chars * 10px * 2 = 180px, well under a typical 964px header width.
        assert_eq!(fit_text_scale("72F Sunny", 2, 964), 2);
    }

    #[test]
    fn fit_text_scale_shrinks_when_too_wide_for_max_scale() {
        // 60 chars * 10px * 2 = 1200px > 964px, but * 1 = 600px fits.
        let text = "x".repeat(60);
        assert_eq!(fit_text_scale(&text, 2, 964), 1);
    }

    #[test]
    fn fit_text_scale_never_goes_below_one_even_if_still_too_wide() {
        let text = "x".repeat(500);
        assert_eq!(fit_text_scale(&text, 2, 964), 1);
    }

    #[test]
    fn text_width_scales_linearly_with_scale_and_length() {
        assert_eq!(text_width("hello", 1), 50);
        assert_eq!(text_width("hello", 2), 100);
        assert_eq!(text_width("", 2), 0);
    }
}
