//! Data collected from Home Assistant for the kiosk's room screen (`firmware/src/ha/rooms.rs`
//! does the actual WebSocket fetching - these are just the resulting plain data + the text
//! formatting derived from it, both fully hardware-independent).

pub struct Room {
    pub area_id: String,
    pub name: String,
    /// Whether at least one light in this area is currently on - drives the toggle switch's
    /// initial position in `display::kiosk`. Kept in sync optimistically by `net::kiosk` after a
    /// tap (rather than waiting for the next refresh) so the toggle visibly flips immediately.
    pub is_on: bool,
    /// True when every light in this area is currently `unavailable` in HA - e.g. its physical
    /// power switch was cut, or it lost network. `is_on` is meaningless in that case (HA can't
    /// confirm any state), so `display::kiosk` grays the toggle out and ignores taps on it
    /// instead of showing/acting on a potentially-fictional on/off state.
    pub unavailable: bool,
}

/// Current weather + a whole-home average indoor temperature, both nice-to-haves shown above
/// the room list. `unit` is whatever `weather.attributes.temperature_unit` HA reports (typically
/// `°F` or `°C`) and is assumed to also apply to `avg_home_temp` - a real HA instance can in
/// theory mix sensor units, but in practice a single home's temperature sensors are consistently
/// in whatever unit system the instance is configured for, so per-sensor unit conversion is
/// skipped as unnecessary complexity for this MVP.
pub struct Weather {
    pub condition: String,
    pub temp: Option<f64>,
    pub unit: String,
    pub avg_home_temp: Option<f64>,
    /// Average of every `sensor.*` entity with `device_class: humidity`, as a plain percentage
    /// (humidity has no unit-system ambiguity the way temperature does).
    pub avg_home_humidity: Option<f64>,
}

/// A compact summary of every `climate.*` entity ("AC"/thermostat) in the home - shown in the
/// bottom-right corner of the kiosk screen. `avg_current_temp` averages their sensed (not
/// target) temperature the same way `Weather::avg_home_temp` averages plain sensors, since
/// that's the actual measured room condition; `mode_counts` is how many units are in each HVAC
/// mode (`off`/`heat`/`cool`/`heat_cool`/`auto`/`dry`/`fan_only`) so a mixed-mode home (one unit
/// cooling, another off) shows a compact breakdown instead of a single misleading mode.
pub struct Climate {
    pub avg_current_temp: Option<f64>,
    /// `(hvac_mode, count)`, sorted by count descending then mode name for stable output.
    pub mode_counts: Vec<(String, u32)>,
}

/// Current time/date, pre-formatted server-side by Home Assistant's `now()` (see
/// `ha::rooms::fetch_clock`) rather than computed from the device's own clock - the device only
/// has SNTP (UTC, no timezone), while HA already knows the instance's configured local timezone,
/// so asking it directly avoids needing to plumb a timezone onto the device at all. Shown by
/// `display::kiosk::show_screensaver` in place of the room grid once the screen has idled long
/// enough to dim.
pub struct Clock {
    /// e.g. "03:45 PM"
    pub time: String,
    /// e.g. "Monday, August 24"
    pub date: String,
}

/// e.g. "72F Sunny  |  Avg Home 70F  |  Avg Humidity 45%" - each piece (current temp/condition,
/// average temp, average humidity) is only included if HA actually provided it (no weather
/// entity, or no matching sensors, are both unremarkable configurations rather than errors worth
/// surfacing). Sticks to plain ASCII since the kiosk's font (embedded_graphics's `FONT_10X20`)
/// has no glyphs for `°` or `·`.
pub fn format_weather_line(w: &Weather) -> String {
    let unit_letter = w.unit.trim_start_matches('°');

    let mut current = Vec::new();
    if let Some(t) = w.temp {
        current.push(format!("{:.0}{unit_letter}", t));
    }
    if !w.condition.is_empty() && w.condition != "Unknown" {
        current.push(w.condition.clone());
    }

    let mut parts = vec![current.join(" ")];
    if let Some(avg) = w.avg_home_temp {
        parts.push(format!("Avg Home {:.0}{unit_letter}", avg));
    }
    if let Some(humidity) = w.avg_home_humidity {
        parts.push(format!("Avg Humidity {:.0}%", humidity));
    }

    parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join("  |  ")
}

/// e.g. "AC 74F  |  2xCool 1xOff" - compact by design (bottom-right corner, one line, small
/// text): a count prefix (`2x`) only appears for a mode with more than one unit in it, so a
/// single-AC home just reads e.g. "AC 74F  |  Cool" instead of "1xCool".
pub fn format_climate_line(c: &Climate, unit_letter: &str) -> String {
    let mut parts = Vec::new();
    if let Some(t) = c.avg_current_temp {
        parts.push(format!("AC {:.0}{unit_letter}", t));
    }
    if !c.mode_counts.is_empty() {
        let modes: Vec<String> = c
            .mode_counts
            .iter()
            .map(|(mode, count)| {
                let label = climate_mode_label(mode);
                if *count > 1 {
                    format!("{count}x{label}")
                } else {
                    label.to_string()
                }
            })
            .collect();
        parts.push(modes.join(" "));
    }
    parts.join("  |  ")
}

/// Short display label for an HA `climate.*` entity's `hvac_mode` state - kept to one word,
/// since this whole line has to fit in one small corner of the screen.
pub fn climate_mode_label(hvac_mode: &str) -> &str {
    match hvac_mode {
        "off" => "Off",
        "heat" => "Heat",
        "cool" => "Cool",
        "heat_cool" | "auto" => "Auto",
        "dry" => "Dry",
        "fan_only" => "Fan",
        _ => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weather(condition: &str, temp: Option<f64>, avg_home_temp: Option<f64>, avg_home_humidity: Option<f64>) -> Weather {
        Weather {
            condition: condition.to_string(),
            temp,
            unit: "°F".to_string(),
            avg_home_temp,
            avg_home_humidity,
        }
    }

    #[test]
    fn weather_line_with_everything() {
        let w = weather("Sunny", Some(72.4), Some(70.0), Some(45.2));
        assert_eq!(format_weather_line(&w), "72F Sunny  |  Avg Home 70F  |  Avg Humidity 45%");
    }

    #[test]
    fn weather_line_with_no_weather_entity_but_has_averages() {
        // condition "Unknown"/empty and no temp reading - as if there's no weather.* entity at
        // all, but there are still ambient sensors to average.
        let w = weather("Unknown", None, Some(70.0), None);
        assert_eq!(format_weather_line(&w), "Avg Home 70F");
    }

    #[test]
    fn weather_line_with_nothing_at_all_is_empty() {
        let w = weather("Unknown", None, None, None);
        assert_eq!(format_weather_line(&w), "");
    }

    #[test]
    fn weather_line_condition_without_temp_reading() {
        let w = weather("Cloudy", None, None, None);
        assert_eq!(format_weather_line(&w), "Cloudy");
    }

    fn climate(avg_current_temp: Option<f64>, mode_counts: &[(&str, u32)]) -> Climate {
        Climate {
            avg_current_temp,
            mode_counts: mode_counts.iter().map(|(m, c)| (m.to_string(), *c)).collect(),
        }
    }

    #[test]
    fn climate_line_single_unit_has_no_count_prefix() {
        let c = climate(Some(74.0), &[("cool", 1)]);
        assert_eq!(format_climate_line(&c, "F"), "AC 74F  |  Cool");
    }

    #[test]
    fn climate_line_mixed_modes_shows_counts_only_where_more_than_one() {
        let c = climate(Some(74.0), &[("cool", 2), ("off", 1)]);
        assert_eq!(format_climate_line(&c, "F"), "AC 74F  |  2xCool Off");
    }

    #[test]
    fn climate_line_no_temp_sensors_but_has_modes() {
        let c = climate(None, &[("heat", 1)]);
        assert_eq!(format_climate_line(&c, "F"), "Heat");
    }

    #[test]
    fn climate_line_nothing_at_all_is_empty() {
        let c = climate(None, &[]);
        assert_eq!(format_climate_line(&c, "F"), "");
    }

    #[test]
    fn climate_mode_label_covers_known_hvac_modes() {
        assert_eq!(climate_mode_label("off"), "Off");
        assert_eq!(climate_mode_label("heat"), "Heat");
        assert_eq!(climate_mode_label("cool"), "Cool");
        assert_eq!(climate_mode_label("heat_cool"), "Auto");
        assert_eq!(climate_mode_label("auto"), "Auto");
        assert_eq!(climate_mode_label("dry"), "Dry");
        assert_eq!(climate_mode_label("fan_only"), "Fan");
    }

    #[test]
    fn climate_mode_label_falls_back_for_unknown_modes() {
        assert_eq!(climate_mode_label("something_new"), "?");
    }
}
