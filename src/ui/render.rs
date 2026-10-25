//! Draws the header, the tree list and the status line.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::config::Config;
use crate::format::{format_bytes, truncate_to_width};
use crate::grouping::Group;
use crate::grouping::{alert_key, over_alert};
use crate::model::{Pressure, SystemStats};
use crate::report::{BAR_WIDTH, MAX_NAME_WIDTH, bar, gb, pressure_word, total_label};
use crate::ui::state::{Row, RowId, UiState};

pub const KEY_HELP: &str = "↑↓ move  ⏎ expand/collapse  k terminate  t alert  q quit";
/// Columns after the name: gap, count or pid, gap, total.
const TAIL_COLUMNS: usize = 2 + 8 + 2 + 9;
/// Below this width the bar is dropped so names stay readable.
const BAR_MIN_WIDTH: usize = 60;

pub struct View<'a> {
    pub stats: &'a SystemStats,
    pub groups: &'a [Group],
    pub state: &'a UiState,
    pub status: Option<&'a str>,
    pub prompt: Option<&'a str>,
    pub config: &'a Config,
    pub color: bool,
}

pub fn draw(frame: &mut Frame, view: &View) {
    let [header, list, footer] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());

    frame.render_widget(Paragraph::new(header_line(view)), header);

    let rows = view.state.rows(view.groups);
    let selected = view.state.selected_index(view.groups);
    let height = list.height as usize;
    let offset = match selected {
        Some(i) if i >= height => i + 1 - height,
        _ => 0,
    };
    let width = list.width as usize;
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(i, row)| {
            let alert = matches!(row, Row::Group(group, _) if over_alert(group, view.config));
            // Without colors, a leading `!` marks the app instead of red.
            let mut style = Style::new();
            if alert && view.color {
                style = style.fg(Color::Red);
            }
            if Some(i) == selected {
                style = style.add_modifier(Modifier::REVERSED);
            }
            Line::from(row_text(row, view.stats.total, width, alert && !view.color)).style(style)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), list);

    let footer_line = match (view.prompt, view.status) {
        (Some(prompt), _) => Line::from(prompt).style(Style::new().add_modifier(Modifier::BOLD)),
        (None, Some(status)) => Line::from(status),
        (None, None) => match selected_alert(view) {
            Some(limit) => Line::from(format!("Alert above {}  {KEY_HELP}", format_bytes(limit))),
            None => Line::from(KEY_HELP),
        },
    };
    frame.render_widget(Paragraph::new(footer_line), footer);
}

/// The alert threshold of the app the selection is in, if it has one.
fn selected_alert(view: &View) -> Option<u64> {
    let key = match view.state.selected.as_ref()? {
        RowId::Group(key) | RowId::Proc(key, _) => key,
    };
    let group = view.groups.iter().find(|g| &g.key == key)?;
    view.config.alert(&alert_key(group)?)
}

fn header_line(view: &View) -> Line<'static> {
    let stats = view.stats;
    let (filled, color) = match stats.pressure {
        Pressure::Normal => (2, Color::Green),
        Pressure::Warn => (5, Color::Yellow),
        Pressure::Critical => (8, Color::Red),
    };
    let style = if view.color {
        Style::new().fg(color)
    } else {
        Style::new()
    };
    Line::from(vec![
        Span::raw(format!(
            "Memory {}/{} GB (wired {})  Pressure ",
            gb(stats.used),
            gb(stats.total),
            gb(stats.wired)
        )),
        Span::styled(
            format!(
                "{}{} {}",
                "■".repeat(filled),
                "□".repeat(8 - filled),
                pressure_word(stats.pressure)
            ),
            style,
        ),
        Span::raw(format!("  Swap {} GB", gb(stats.swap_used))),
    ])
}

fn row_text(row: &Row, total_memory: u64, width: usize, mark_alert: bool) -> String {
    let with_bar = width >= BAR_MIN_WIDTH;
    let tail = if with_bar {
        TAIL_COLUMNS + 2 + BAR_WIDTH
    } else {
        TAIL_COLUMNS
    };
    let name_width = width.saturating_sub(2 + tail).clamp(4, MAX_NAME_WIDTH);
    match row {
        Row::Group(group, open) => {
            let mut text = format!(
                "{} {}  {:>3} proc  {:>9}",
                if *open { '▾' } else { '▸' },
                pad(
                    &if mark_alert {
                        format!("!{}", group.name)
                    } else {
                        group.name.clone()
                    },
                    name_width
                ),
                group.procs.len(),
                total_label(group),
            );
            if with_bar {
                text.push_str("  ");
                text.push_str(&bar(group.total, total_memory, BAR_WIDTH));
            }
            text
        }
        Row::Proc(_, proc) => format!(
            "    {}  {:>8}  {:>9}",
            pad(&proc.name, name_width - 2),
            proc.pid,
            format_bytes(proc.footprint.unwrap_or(0))
        ),
        Row::More { count, total, .. } => {
            format!(
                "    {}  {:>8}  {:>9}",
                pad(&format!("… {count} more"), name_width - 2),
                "",
                format_bytes(*total)
            )
        }
    }
}

/// Truncates or pads `s` to exactly `width` columns.
fn pad(s: &str, width: usize) -> String {
    let cut = truncate_to_width(s, width);
    let fill = width.saturating_sub(cut.width());
    format!("{cut}{}", " ".repeat(fill))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attribution::Bucket;
    use crate::config::Config;
    use crate::model::{Pressure, ProcInfo};
    use crate::ui::state::RowId;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use unicode_width::UnicodeWidthStr;

    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * MB;

    fn stats() -> SystemStats {
        SystemStats {
            total: 8 * GB,
            used: 7 * GB,
            wired: GB,
            swap_used: 2 * GB,
            app_footprint: 0,
            pressure: Pressure::Warn,
        }
    }

    fn group(name: &str, n: i32, total: u64) -> Group {
        let procs = (0..n)
            .map(|i| ProcInfo {
                pid: 1000 + i,
                ppid: 1,
                uid: 501,
                name: format!("{name} Helper {i}"),
                path: None,
                footprint: Some(total / n.max(1) as u64),
                responsible_pid: None,
            })
            .collect();
        Group {
            key: Bucket::App(format!("/Applications/{name}.app").into()),
            name: name.into(),
            bundle_id: None,
            procs,
            total,
            estimated: false,
        }
    }

    fn system() -> Group {
        Group {
            key: Bucket::System,
            name: "System".into(),
            bundle_id: None,
            procs: vec![],
            total: GB,
            estimated: true,
        }
    }

    fn screen(w: u16, h: u16, groups: &[Group], state: &UiState, prompt: Option<&str>) -> Vec<String> {
        let config = Config::default();
        render(w, h, groups, state, prompt, &config, true).0
    }

    fn render(
        w: u16,
        h: u16,
        groups: &[Group],
        state: &UiState,
        prompt: Option<&str>,
        config: &Config,
        color: bool,
    ) -> (Vec<String>, Buffer) {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let st = stats();
        let view = View {
            stats: &st,
            groups,
            state,
            status: None,
            prompt,
            config,
            color,
        };
        terminal.draw(|f| draw(f, &view)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let lines = (0..h)
            .map(|y| {
                let mut line = String::new();
                let mut x = 0;
                while x < w {
                    let symbol = buf[(x, y)].symbol();
                    line.push_str(symbol);
                    // A wide character fills the next cell too.
                    x += symbol.width().max(1) as u16;
                }
                line.trim_end().to_string()
            })
            .collect();
        (lines, buf)
    }

    fn safari_alert(limit: u64) -> (Vec<Group>, Config) {
        let mut safari = group("Safari", 3, 3 * GB);
        safari.bundle_id = Some("com.apple.Safari".into());
        let mut config = Config::default();
        config.set_alert("com.apple.Safari", Some(limit));
        (vec![safari, group("Xcode", 1, GB)], config)
    }

    fn selected(groups: &[Group]) -> UiState {
        let mut s = UiState::default();
        s.reconcile(groups);
        s
    }

    #[test]
    fn renders_header_and_groups_80x24() {
        let groups = vec![group("Google Chrome", 3, 3 * GB), system()];
        let lines = screen(80, 24, &groups, &selected(&groups), None);
        assert!(lines[0].starts_with("Memory 7.00/8.00 GB (wired 1.00)  Pressure"));
        assert!(lines[0].contains("warn"));
        let list: Vec<&String> = lines[1..23].iter().filter(|l| !l.is_empty()).collect();
        assert!(list[0].starts_with("▸ Google Chrome"));
        assert!(list.last().unwrap().starts_with("▸ System"));
        assert!(list.last().unwrap().contains("~1.00 GB"));
        assert!(lines[23].contains("k terminate"));
    }

    #[test]
    fn renders_expanded_more_row() {
        let groups = vec![group("Google Chrome", 9, 9 * GB), system()];
        let mut state = selected(&groups);
        state.toggle_expand();
        let lines = screen(80, 24, &groups, &state, None);
        assert!(lines[1].starts_with("▾ Google Chrome"));
        let procs = lines
            .iter()
            .filter(|l| l.starts_with("    Google Chrome Helper"))
            .count();
        assert_eq!(procs, 6);
        assert!(lines.iter().any(|l| l.starts_with("    … 3 more")));
    }

    #[test]
    fn renders_in_tiny_terminal() {
        let groups: Vec<Group> = (0..20)
            .map(|i| group(&format!("App{i:02}"), 1, (20 - i) as u64 * MB))
            .collect();
        let mut state = selected(&groups);
        state.move_selection(&groups, 14);
        assert_eq!(state.selected, Some(RowId::Group(groups[14].key.clone())));
        let lines = screen(40, 10, &groups, &state, None);
        assert!(lines.iter().any(|l| l.starts_with("▸ App14")), "{lines:#?}");
    }

    #[test]
    fn truncates_wide_names() {
        let groups = vec![
            group("🚀🚀🚀 Very Long Application Name That Goes On And On", 1, GB),
            group("Xcode", 1, GB / 2),
        ];
        let lines = screen(80, 24, &groups, &selected(&groups), None);
        let long = &lines[1];
        let short = &lines[2];
        assert!(long.contains('…'));
        let col = |l: &str| unicode_width::UnicodeWidthStr::width(&l[..l.find(" proc").unwrap()]);
        assert_eq!(col(long), col(short));
    }

    #[test]
    fn shows_prompt_on_last_line() {
        let groups = vec![group("Google Chrome", 1, GB)];
        let lines = screen(80, 24, &groups, &selected(&groups), Some("Quit Google Chrome? y/N"));
        assert_eq!(lines[23], "Quit Google Chrome? y/N");
    }

    #[test]
    fn caps_name_column_on_wide_terminals() {
        let groups = vec![group("Safari", 1, 3 * GB)];
        let lines = screen(250, 10, &groups, &selected(&groups), None);
        let row = &lines[1];
        let col = row[..row.find(" proc").unwrap()].width();
        assert_eq!(col, 2 + crate::report::MAX_NAME_WIDTH + 5, "{row}");
    }

    #[test]
    fn row_over_alert_is_red() {
        let (groups, config) = safari_alert(2 * GB);
        let mut state = selected(&groups);
        state.move_selection(&groups, 1);
        let (_, buf) = render(80, 10, &groups, &state, None, &config, true);
        assert_eq!(buf[(2, 1)].fg, Color::Red);
        assert_ne!(buf[(2, 2)].fg, Color::Red);
    }

    #[test]
    fn row_under_alert_is_plain() {
        let (groups, config) = safari_alert(4 * GB);
        let (_, buf) = render(80, 10, &groups, &UiState::default(), None, &config, true);
        assert_ne!(buf[(2, 1)].fg, Color::Red);
    }

    #[test]
    fn no_color_marks_alert_with_bang() {
        let (groups, config) = safari_alert(2 * GB);
        let (lines, _) = render(80, 10, &groups, &selected(&groups), None, &config, false);
        assert!(lines[1].starts_with("▸ !Safari"), "{}", lines[1]);
        assert!(lines[2].starts_with("▸ Xcode"));
    }

    #[test]
    fn footer_shows_alert_of_selected_app() {
        let (groups, config) = safari_alert(2 * GB);
        let (lines, _) = render(80, 10, &groups, &selected(&groups), None, &config, true);
        assert!(lines[9].starts_with("Alert above 2.00 GB  "), "{}", lines[9]);
        assert!(lines[9].contains("t alert"));
    }
}
