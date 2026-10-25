//! Plain-text report printed by `--once`.

use std::fmt::Write;

use crate::columns::{alert_label, group_columns, memory_text, name_width, pressure_word, swap_text};
use crate::config::Config;
use crate::grouping::{Group, over_alert};
use crate::model::SystemStats;

pub fn render_report(stats: &SystemStats, groups: &[Group], config: &Config, width: usize) -> String {
    let mut out = format!(
        "{}{}{}\n",
        memory_text(stats),
        pressure_word(stats.pressure),
        swap_text(stats)
    );
    let name_width = name_width(width, true);
    for group in groups {
        // `!` marks apps above their alert; --once has no colors.
        let label = alert_label(group, over_alert(group, config));
        let _ = writeln!(out, "{}", group_columns(group, &label, name_width, stats.total, true));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attribution::Bucket;
    use crate::columns::MAX_NAME_WIDTH;
    use crate::config::Config;
    use crate::format::bar;
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
