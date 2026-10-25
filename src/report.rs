//! Plain-text report printed by `--once`.

use std::fmt::Write;

use crate::config::Config;
use crate::format::{format_bytes, truncate_to_width};
use crate::grouping::{Group, over_alert};
use crate::model::{Pressure, SystemStats};

pub const BAR_WIDTH: usize = 13;
/// Widest the name column gets, so totals stay next to names on wide terminals.
pub const MAX_NAME_WIDTH: usize = 40;
/// Columns used by everything except the name: gaps, process count, total and bar.
const FIXED_COLUMNS: usize = 2 + 3 + 5 + 2 + 9 + 2 + BAR_WIDTH;

pub fn pressure_word(pressure: Pressure) -> &'static str {
    match pressure {
        Pressure::Normal => "normal",
        Pressure::Warn => "warn",
        Pressure::Critical => "critical",
    }
}

/// "6.90/8.00" style pair, both in GB with two decimals.
pub fn gb(bytes: u64) -> String {
    format!("{:.2}", bytes as f64 / (1u64 << 30) as f64)
}

/// `━`/`─` bar of `width` cells (thin glyphs, so bars on adjacent lines stay apart) for `value` out of `max`.
pub fn bar(value: u64, max: u64, width: usize) -> String {
    let filled = if max == 0 {
        0
    } else {
        ((value as f64 / max as f64) * width as f64).round() as usize
    };
    let filled = filled.min(width);
    format!("{}{}", "━".repeat(filled), "─".repeat(width - filled))
}

/// Total with a `~` prefix when it includes an estimate.
pub fn total_label(group: &Group) -> String {
    let prefix = if group.estimated { "~" } else { "" };
    format!("{prefix}{}", format_bytes(group.total))
}

pub fn render_report(stats: &SystemStats, groups: &[Group], config: &Config, width: usize) -> String {
    let mut out = format!(
        "Memory {}/{} GB (wired {})  Pressure {}  Swap {} GB\n",
        gb(stats.used),
        gb(stats.total),
        gb(stats.wired),
        pressure_word(stats.pressure),
        gb(stats.swap_used)
    );
    let name_width = width.saturating_sub(FIXED_COLUMNS).clamp(8, MAX_NAME_WIDTH);
    for group in groups {
        // `!` marks apps above their alert; --once has no colors.
        let label = if over_alert(group, config) {
            format!("!{}", group.name)
        } else {
            group.name.clone()
        };
        let name = truncate_to_width(&label, name_width);
        let pad = name_width - unicode_width::UnicodeWidthStr::width(name.as_str());
        let _ = writeln!(
            out,
            "{name}{}  {:>3} proc  {:>9}  {}",
            " ".repeat(pad),
            group.procs.len(),
            total_label(group),
            bar(group.total, stats.total, BAR_WIDTH)
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attribution::Bucket;
    use crate::config::Config;
    use crate::model::{Pressure, SystemStats};

    const GB: u64 = 1024 * 1024 * 1024;

    fn group(key: Bucket, name: &str, total: u64, estimated: bool) -> Group {
        Group {
            key,
            name: name.into(),
            bundle_id: None,
            procs: vec![],
            total,
            estimated,
        }
    }

    #[test]
    fn report_lists_system_last_with_tilde() {
        let stats = SystemStats {
            total: 8 * GB,
            used: 6 * GB,
            wired: GB,
            swap_used: 2 * GB,
            app_footprint: 0,
            pressure: Pressure::Warn,
        };
        let groups = vec![
            group(
                Bucket::App("/Applications/Google Chrome.app".into()),
                "Google Chrome",
                3 * GB,
                false,
            ),
            group(Bucket::System, "System", GB, true),
        ];
        let out = render_report(&stats, &groups, &Config::default(), 80);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines[0],
            "Memory 6.00/8.00 GB (wired 1.00)  Pressure warn  Swap 2.00 GB"
        );
        assert!(lines[1].starts_with("Google Chrome"));
        assert!(lines.last().unwrap().starts_with("System"));
        assert!(lines.last().unwrap().contains("~1.00 GB"));
        assert!(out.lines().all(|l| unicode_width::UnicodeWidthStr::width(l) <= 80));
    }

    #[test]
    fn caps_name_column_on_wide_terminals() {
        let stats = SystemStats {
            total: 8 * GB,
            used: 6 * GB,
            wired: GB,
            swap_used: 0,
            app_footprint: 0,
            pressure: Pressure::Normal,
        };
        let groups = vec![group(
            Bucket::App("/Applications/Safari.app".into()),
            "Safari",
            3 * GB,
            false,
        )];
        let out = render_report(&stats, &groups, &Config::default(), 250);
        let row = out.lines().nth(1).unwrap();
        assert_eq!(row.find(" proc").unwrap(), MAX_NAME_WIDTH + 5, "{row}");
    }

    #[test]
    fn bars_use_thin_line_glyphs() {
        assert_eq!(bar(1, 2, 4), "━━──");
    }

    #[test]
    fn marks_apps_over_their_alert() {
        let stats = SystemStats {
            total: 8 * GB,
            used: 6 * GB,
            wired: GB,
            swap_used: 0,
            app_footprint: 0,
            pressure: Pressure::Normal,
        };
        let mut safari = group(Bucket::App("/Applications/Safari.app".into()), "Safari", 3 * GB, false);
        safari.bundle_id = Some("com.apple.Safari".into());
        let mut config = Config::default();
        config.set_alert("com.apple.Safari", Some(2 * GB));
        let out = render_report(&stats, &[safari], &config, 80);
        assert!(out.lines().nth(1).unwrap().starts_with("!Safari"));
    }
}
