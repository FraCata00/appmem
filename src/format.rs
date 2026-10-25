//! Human-readable sizes, bars and width-aware truncation.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub const MIB: u64 = 1024 * 1024;
pub const GIB: u64 = 1024 * MIB;

/// Two decimals in GB from 1 GiB up, whole MB below (1024 bases, like Activity Monitor).
pub fn format_bytes(bytes: u64) -> String {
    if bytes >= GIB {
        format!("{} GB", gb(bytes))
    } else {
        format!("{} MB", bytes / MIB)
    }
}

/// Bytes as GB with two decimals, without the unit.
pub fn gb(bytes: u64) -> String {
    format!("{:.2}", bytes as f64 / GIB as f64)
}

/// A positive, finite amount of GB in bytes; `None` for anything else.
pub fn gb_to_bytes(gb: f64) -> Option<u64> {
    (gb.is_finite() && gb > 0.0).then(|| (gb * GIB as f64).round() as u64)
}

/// `━`/`─` bar of `width` cells for `value` out of `max`. Thin glyphs keep bars
/// on adjacent lines apart.
pub fn bar(value: u64, max: u64, width: usize) -> String {
    let filled = if max == 0 {
        0
    } else {
        ((value as f64 / max as f64) * width as f64).round() as usize
    };
    let filled = filled.min(width);
    format!("{}{}", "━".repeat(filled), "─".repeat(width - filled))
}

/// Truncates or pads `s` to exactly `width` columns.
pub fn pad(s: &str, width: usize) -> String {
    let cut = truncate_to_width(s, width);
    let fill = width.saturating_sub(cut.width());
    format!("{cut}{}", " ".repeat(fill))
}

/// Cuts `s` to at most `width` terminal columns, ending with `…` when cut.
pub fn truncate_to_width(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// A duration as seconds, without trailing zeros: "2s", "0.5s".
pub fn seconds(d: std::time::Duration) -> String {
    let text = format!("{:.2}", d.as_secs_f64());
    format!("{}s", text.trim_end_matches('0').trim_end_matches('.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Pressure;

    #[test]
    fn formats_megabytes() {
        assert_eq!(format_bytes(820 * 1024 * 1024), "820 MB");
    }

    #[test]
    fn formats_gigabytes() {
        assert_eq!(format_bytes(3_350_000_000), "3.12 GB");
    }

    #[test]
    fn formats_zero() {
        assert_eq!(format_bytes(0), "0 MB");
    }

    #[test]
    fn truncates_by_display_width() {
        assert_eq!(truncate_to_width("Google Chrome", 8), "Google …");
        assert_eq!(truncate_to_width("Xcode", 8), "Xcode");
    }

    #[test]
    fn truncates_wide_chars_by_width() {
        // Each rocket is two columns wide.
        assert_eq!(truncate_to_width("🚀🚀🚀", 4), "🚀…");
    }

    #[test]
    fn pressure_levels() {
        assert_eq!(Pressure::from_level(1), Pressure::Normal);
        assert_eq!(Pressure::from_level(2), Pressure::Warn);
        assert_eq!(Pressure::from_level(4), Pressure::Critical);
    }

    #[test]
    fn converts_gigabytes() {
        assert_eq!(gb_to_bytes(1.5), Some(GIB + GIB / 2));
        assert_eq!(gb_to_bytes(0.0), None);
        assert_eq!(gb_to_bytes(-1.0), None);
        assert_eq!(gb_to_bytes(f64::NAN), None);
    }

    #[test]
    fn formats_seconds() {
        use std::time::Duration;
        assert_eq!(seconds(Duration::from_secs(2)), "2s");
        assert_eq!(seconds(Duration::from_millis(500)), "0.5s");
        assert_eq!(seconds(Duration::from_secs_f64(1.333333)), "1.33s");
    }
}
