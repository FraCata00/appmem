//! The group row and header text shared by `--once` and the interactive view.

use std::borrow::Cow;

use crate::format::{bar, format_bytes, gb, pad};
use crate::grouping::Group;
use crate::model::{Pressure, SystemStats};

pub const BAR_WIDTH: usize = 13;
/// Widest the name column gets, so totals stay next to names on wide terminals.
pub const MAX_NAME_WIDTH: usize = 40;
const MIN_NAME_WIDTH: usize = 8;
/// Columns after the name: gap, process count or pid, gap, total.
pub const TAIL_COLUMNS: usize = 2 + 8 + 2 + 9;
/// Columns the bar adds after the tail.
const BAR_COLUMNS: usize = 2 + BAR_WIDTH;

/// Width of the name column when `available` columns are left for the row.
pub fn name_width(available: usize, with_bar: bool) -> usize {
    let tail = TAIL_COLUMNS + if with_bar { BAR_COLUMNS } else { 0 };
    available.saturating_sub(tail).clamp(MIN_NAME_WIDTH, MAX_NAME_WIDTH)
}

/// Name, process count, total and optionally the bar.
pub fn group_columns(group: &Group, label: &str, name_width: usize, total_memory: u64, with_bar: bool) -> String {
    let mut text = format!(
        "{}  {:>3} proc  {:>9}",
        pad(label, name_width),
        group.procs.len(),
        total_label(group)
    );
    if with_bar {
        text.push_str("  ");
        text.push_str(&bar(group.total, total_memory, BAR_WIDTH));
    }
    text
}

/// The group name, with a leading `!` when it is over its alert and colors can't show it.
pub fn alert_label(group: &Group, mark: bool) -> Cow<'_, str> {
    if mark {
        Cow::Owned(format!("!{}", group.name))
    } else {
        Cow::Borrowed(&group.name)
    }
}

/// Total with a `~` prefix when it includes an estimate.
pub fn total_label(group: &Group) -> String {
    let prefix = if group.estimated { "~" } else { "" };
    format!("{prefix}{}", format_bytes(group.total))
}

pub fn pressure_word(pressure: Pressure) -> &'static str {
    match pressure {
        Pressure::Normal => "normal",
        Pressure::Warn => "warn",
        Pressure::Critical => "critical",
    }
}

/// Header text before the pressure indicator.
pub fn memory_text(stats: &SystemStats) -> String {
    format!(
        "Memory {}/{} GB (wired {})  Pressure ",
        gb(stats.used),
        gb(stats.total),
        gb(stats.wired)
    )
}

/// Header text after the pressure indicator.
pub fn swap_text(stats: &SystemStats) -> String {
    format!("  Swap {} GB", gb(stats.swap_used))
}
