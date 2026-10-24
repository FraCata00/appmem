//! Human-readable sizes and width-aware truncation.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// Two decimals in GB from 1 GiB up, whole MB below (1024 bases, like Activity Monitor).
pub fn format_bytes(bytes: u64) -> String {
    if bytes >= GIB {
        format!("{:.2} GB", bytes as f64 / GIB as f64)
    } else {
        format!("{} MB", bytes / MIB)
    }
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
}
