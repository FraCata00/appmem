//! Terminal setup and the event loop.

use std::io;
use std::path::PathBuf;
use std::process::Child;
use std::time::{Duration, Instant};

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use crate::collector::{Collector, PlistBundles, proc_path};
use crate::config::{Config, config_path};
use crate::format::{format_bytes, gb_to_bytes};
use crate::grouping::{Group, alert_key, build_groups};
use crate::model::Snapshot;
use crate::terminate::{Action, confirm_prompt, plan_quit, plan_terminate, quit_result, start_quit, terminate};
use crate::ui::render::{View, draw};
use crate::ui::state::{RowId, UiState};

const STATUS_DURATION: Duration = Duration::from_secs(3);
/// How often to check on a pending quit request.
const QUIT_POLL: Duration = Duration::from_millis(200);
const CANCELLED: &str = "Cancelled";

/// What the keyboard is driving right now.
enum Mode {
    Browse,
    /// Waiting for y/N on a terminate or quit.
    Confirm(Action),
    /// Typing an alert threshold for an app.
    EditAlert(AlertEdit),
}

struct AlertEdit {
    key: String,
    name: String,
    input: String,
}

struct App<'c> {
    collector: &'c mut dyn Collector,
    bundles: PlistBundles,
    snap: Snapshot,
    groups: Vec<Group>,
    state: UiState,
    mode: Mode,
    /// An app we asked to quit, with osascript still waiting for its answer.
    /// Runs in the background whatever the mode.
    quitting: Option<(String, Child)>,
    config: Config,
    config_path: Option<PathBuf>,
    status: Option<(String, Instant)>,
    color: bool,
}

/// Runs the interactive view until the user quits. Restores the terminal on exit and on panic.
pub fn run(collector: &mut dyn Collector, interval: Duration) -> io::Result<()> {
    let mut app = App::new(collector)?;
    // ratatui::init also installs a panic hook that restores the terminal.
    let mut terminal = ratatui::init();
    let result = app.event_loop(&mut terminal, interval);
    ratatui::restore();
    result
}

impl<'c> App<'c> {
    fn new(collector: &'c mut dyn Collector) -> io::Result<App<'c>> {
        let snap = collector.snapshot()?;
        let config_path = config_path();
        let mut app = App {
            collector,
            bundles: PlistBundles::default(),
            groups: Vec::new(),
            snap,
            state: UiState::default(),
            mode: Mode::Browse,
            quitting: None,
            config: config_path.as_deref().map(Config::load).unwrap_or_default(),
            config_path,
            status: None,
            color: std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()),
        };
        app.regroup();
        Ok(app)
    }

    fn regroup(&mut self) {
        self.groups = build_groups(&self.snap, &mut self.bundles);
        self.state.reconcile(&self.groups);
    }

    fn refresh(&mut self) {
        match self.collector.snapshot() {
            Ok(snap) => {
                self.snap = snap;
                self.regroup();
            }
            Err(err) => self.set_status(format!("Refresh failed: {err}")),
        }
    }

    fn set_status(&mut self, message: impl Into<String>) {
        self.status = Some((message.into(), Instant::now()));
    }

    /// The status line: a pending quit, else the latest message.
    fn status_text(&self) -> Option<String> {
        match &self.quitting {
            Some((name, _)) => Some(format!("Asking {name} to quit…")),
            None => self.status.as_ref().map(|(m, _)| m.clone()),
        }
    }

    /// What the bottom line asks for, if anything.
    fn prompt(&self) -> Option<String> {
        match &self.mode {
            Mode::Browse => None,
            Mode::Confirm(action) => Some(confirm_prompt(action)),
            Mode::EditAlert(edit) => Some(format!("Alert when {} uses more than (GB): {}", edit.name, edit.input)),
        }
    }

    fn start_alert_edit(&mut self) {
        let Some(group) = self.state.selected_group(&self.groups) else {
            return;
        };
        match alert_key(group) {
            Some(key) => {
                let name = group.name.clone();
                self.mode = Mode::EditAlert(AlertEdit {
                    key,
                    name,
                    input: String::new(),
                });
            }
            None => self.set_status("System can't have an alert"),
        }
    }

    /// Applies a typed alert (empty or 0 removes it) and saves the config.
    fn save_alert(&mut self, edit: &AlertEdit) -> String {
        let text = edit.input.replace(',', ".");
        let gb = if text.is_empty() {
            0.0
        } else {
            text.parse::<f64>().unwrap_or(f64::NAN)
        };
        if !gb.is_finite() {
            return format!("Not a number: {}", edit.input);
        }
        let bytes = gb_to_bytes(gb);
        self.config.set_alert(&edit.key, bytes);
        if let Some(path) = &self.config_path
            && let Err(err) = self.config.save(path)
        {
            return format!("Couldn't save {}: {err}", path.display());
        }
        match bytes {
            Some(bytes) => format!("Alert for {} above {}", edit.name, format_bytes(bytes)),
            None => format!("Alert for {} removed", edit.name),
        }
    }

    /// Reports the outcome of a quit request once osascript has finished.
    fn poll_quit(&mut self) {
        let Some((_, child)) = self.quitting.as_mut() else {
            return;
        };
        match child.try_wait() {
            Ok(None) => {}
            Ok(Some(_)) => {
                let (name, child) = self.quitting.take().unwrap();
                let outcome = child.wait_with_output().map_err(|e| e.to_string());
                let message = match outcome.and_then(|output| quit_result(&output)) {
                    Ok(()) => format!("{name} quit"),
                    Err(err) => err,
                };
                self.set_status(message);
                self.refresh();
            }
            Err(err) => {
                self.quitting = None;
                self.set_status(err.to_string());
            }
        }
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal, interval: Duration) -> io::Result<()> {
        let mut next_refresh = Instant::now() + interval;
        loop {
            self.poll_quit();
            if self
                .status
                .as_ref()
                .is_some_and(|(_, at)| at.elapsed() >= STATUS_DURATION)
            {
                self.status = None;
            }
            let prompt = self.prompt();
            let status = self.status_text();
            terminal.draw(|frame| {
                let view = View {
                    stats: &self.snap.stats,
                    groups: &self.groups,
                    state: &self.state,
                    status: status.as_deref(),
                    prompt: prompt.as_deref(),
                    config: &self.config,
                    color: self.color,
                };
                draw(frame, &view);
            })?;

            let mut timeout = next_refresh
                .saturating_duration_since(Instant::now())
                .min(STATUS_DURATION);
            if self.quitting.is_some() {
                timeout = timeout.min(QUIT_POLL);
            }
            if event::poll(timeout)?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && !self.handle_key(key.code, key.modifiers)
            {
                return Ok(());
            }
            if Instant::now() >= next_refresh {
                self.refresh();
                next_refresh = Instant::now() + interval;
            }
        }
    }

    /// Returns false when the user asked to quit.
    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            return false;
        }
        match std::mem::replace(&mut self.mode, Mode::Browse) {
            Mode::Browse => return self.browse_key(code),
            Mode::Confirm(action) if matches!(code, KeyCode::Char('y' | 'Y')) => self.confirm(action),
            Mode::Confirm(_) => self.set_status(CANCELLED),
            Mode::EditAlert(mut edit) => match code {
                KeyCode::Char(c) if c.is_ascii_digit() || c == '.' || c == ',' => {
                    edit.input.push(c);
                    self.mode = Mode::EditAlert(edit);
                }
                KeyCode::Backspace => {
                    edit.input.pop();
                    self.mode = Mode::EditAlert(edit);
                }
                KeyCode::Esc => self.set_status(CANCELLED),
                KeyCode::Enter => {
                    let message = self.save_alert(&edit);
                    self.set_status(message);
                }
                _ => self.mode = Mode::EditAlert(edit),
            },
        }
        true
    }

    fn browse_key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return false,
            KeyCode::Up => self.state.move_selection(&self.groups, -1),
            KeyCode::Down => self.state.move_selection(&self.groups, 1),
            KeyCode::Enter => self.state.toggle_expand(),
            KeyCode::Char('t') => self.start_alert_edit(),
            KeyCode::Char('k') => match self.plan() {
                // One quit at a time: the first one is still waiting for its app.
                Some(Ok(Action::Quit { .. })) if self.quitting.is_some() => {}
                Some(Ok(action)) => self.mode = Mode::Confirm(action),
                Some(Err(message)) => self.set_status(message),
                None => {}
            },
            _ => {}
        }
        true
    }

    fn confirm(&mut self, action: Action) {
        match action {
            Action::Quit { name, bundle_id } => match start_quit(&bundle_id) {
                Ok(child) => self.quitting = Some((name, child)),
                Err(err) => self.set_status(err),
            },
            Action::Terminate { name, pid, path, .. } => {
                let message = match terminate(pid, &path, proc_path) {
                    Ok(()) => format!("Sent terminate to {name}"),
                    Err(err) => err,
                };
                self.set_status(message);
                self.refresh();
            }
        }
    }

    fn plan(&self) -> Option<Result<Action, String>> {
        let group = self.state.selected_group(&self.groups)?;
        match self.state.selected.as_ref()? {
            RowId::Group(_) => Some(plan_quit(group, &self.snap)),
            RowId::Proc(_, pid) => {
                let proc = group.procs.iter().find(|p| p.pid == *pid)?;
                Some(plan_terminate(proc, group, &self.snap))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attribution::Bucket;
    use crate::config::Config;
    use crate::model::{Pressure, ProcInfo, SystemStats};

    struct FakeCollector(Snapshot);

    impl Collector for FakeCollector {
        fn snapshot(&mut self) -> io::Result<Snapshot> {
            Ok(self.0.clone())
        }
    }

    fn proc(pid: i32, path: &str) -> ProcInfo {
        ProcInfo {
            pid,
            ppid: 1,
            uid: 501,
            name: format!("p{pid}"),
            path: Some(path.into()),
            footprint: Some(1 << 30),
            responsible_pid: None,
        }
    }

    fn collector() -> FakeCollector {
        FakeCollector(Snapshot {
            procs: vec![
                proc(10, "/nonexistent/Fake.app/Contents/MacOS/Fake"),
                proc(11, "/nonexistent/Fake.app/Contents/Helpers/helper"),
                proc(20, "/usr/libexec/sysd"),
            ],
            stats: SystemStats {
                total: 8 << 30,
                used: 3 << 30,
                wired: 0,
                swap_used: 0,
                app_footprint: 0,
                pressure: Pressure::Normal,
            },
            current_uid: 501,
            self_pid: 9999,
        })
    }

    fn status(app: &App) -> Option<String> {
        app.status_text()
    }

    #[test]
    fn k_on_system_shows_refusal() {
        let mut c = collector();
        let mut app = App::new(&mut c).unwrap();
        app.handle_key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(app.state.selected, Some(RowId::Group(Bucket::System)));
        app.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(status(&app).as_deref(), Some("System processes can't be terminated"));
        assert!(matches!(app.mode, Mode::Browse));
    }

    #[test]
    fn k_on_process_asks_then_cancels() {
        let mut c = collector();
        let mut app = App::new(&mut c).unwrap();
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        app.handle_key(KeyCode::Down, KeyModifiers::NONE);
        app.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        let prompt = app.prompt().unwrap();
        assert!(prompt.starts_with("Terminate p1"), "{prompt}");
        assert!(app.handle_key(KeyCode::Char('n'), KeyModifiers::NONE));
        assert!(matches!(app.mode, Mode::Browse));
        assert_eq!(status(&app).as_deref(), Some("Cancelled"));
    }

    #[test]
    fn k_on_app_without_bundle_id_is_refused() {
        let mut c = collector();
        let mut app = App::new(&mut c).unwrap();
        app.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        // The fake bundle has no Info.plist, so there is no bundle id to quit by.
        assert!(matches!(app.mode, Mode::Browse));
        assert_eq!(status(&app).as_deref(), Some("Can't identify this app to quit it"));
    }

    #[test]
    fn confirmed_quit_does_not_block_the_ui() {
        let mut c = collector();
        let mut app = App::new(&mut c).unwrap();
        app.mode = Mode::Confirm(Action::Quit {
            name: "Fake".into(),
            bundle_id: "invalid.appmem.test".into(),
        });
        let started = Instant::now();
        app.handle_key(KeyCode::Char('y'), KeyModifiers::NONE);
        assert_eq!(status(&app).as_deref(), Some("Asking Fake to quit…"));
        assert!(app.quitting.is_some());

        // A second quit while the first is in flight is refused.
        app.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        assert!(matches!(app.mode, Mode::Browse));

        while app.quitting.is_some() && started.elapsed() < Duration::from_secs(20) {
            app.poll_quit();
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(app.quitting.is_none());
        let done = status(&app).unwrap();
        assert_ne!(done, "Asking Fake to quit…");
        assert!(!done.is_empty());
    }

    #[test]
    fn q_esc_and_ctrl_c_quit() {
        let mut c = collector();
        let mut app = App::new(&mut c).unwrap();
        assert!(!app.handle_key(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(!app.handle_key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.handle_key(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.handle_key(KeyCode::Char('x'), KeyModifiers::NONE));
    }

    fn with_temp_config(app: &mut App, name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("appmem-run-{}-{name}/config.toml", std::process::id()));
        app.config = Config::default();
        app.config_path = Some(path.clone());
        path
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            app.handle_key(KeyCode::Char(c), KeyModifiers::NONE);
        }
    }

    const FAKE_KEY: &str = "/nonexistent/Fake.app";

    #[test]
    fn t_then_number_saves_alert() {
        let mut c = collector();
        let mut app = App::new(&mut c).unwrap();
        let path = with_temp_config(&mut app, "save");
        app.handle_key(KeyCode::Char('t'), KeyModifiers::NONE);
        assert_eq!(app.prompt().as_deref(), Some("Alert when Fake uses more than (GB): "));
        type_text(&mut app, "1,5");
        assert_eq!(
            app.prompt().as_deref(),
            Some("Alert when Fake uses more than (GB): 1,5")
        );
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.config.alert(FAKE_KEY), Some(3 << 29));
        assert_eq!(status(&app).as_deref(), Some("Alert for Fake above 1.50 GB"));
        assert_eq!(Config::load(&path), app.config);
        assert!(app.prompt().is_none());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn empty_value_removes_alert() {
        let mut c = collector();
        let mut app = App::new(&mut c).unwrap();
        let path = with_temp_config(&mut app, "remove");
        app.config.set_alert(FAKE_KEY, Some(1 << 30));
        app.handle_key(KeyCode::Char('t'), KeyModifiers::NONE);
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.config.alert(FAKE_KEY), None);
        assert_eq!(status(&app).as_deref(), Some("Alert for Fake removed"));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn editing_ignores_letters_and_esc_cancels() {
        let mut c = collector();
        let mut app = App::new(&mut c).unwrap();
        with_temp_config(&mut app, "cancel");
        app.handle_key(KeyCode::Char('t'), KeyModifiers::NONE);
        type_text(&mut app, "2xq");
        app.handle_key(KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(app.prompt().as_deref(), Some("Alert when Fake uses more than (GB): "));
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.prompt().is_none());
        assert_eq!(app.config.alert(FAKE_KEY), None);
        assert_eq!(status(&app).as_deref(), Some("Cancelled"));
    }

    #[test]
    fn invalid_number_is_rejected() {
        let mut c = collector();
        let mut app = App::new(&mut c).unwrap();
        with_temp_config(&mut app, "invalid");
        app.handle_key(KeyCode::Char('t'), KeyModifiers::NONE);
        type_text(&mut app, "..");
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(status(&app).as_deref(), Some("Not a number: .."));
        assert_eq!(app.config.alert(FAKE_KEY), None);
    }

    #[test]
    fn t_on_system_is_refused() {
        let mut c = collector();
        let mut app = App::new(&mut c).unwrap();
        app.handle_key(KeyCode::Down, KeyModifiers::NONE);
        app.handle_key(KeyCode::Char('t'), KeyModifiers::NONE);
        assert!(app.prompt().is_none());
        assert_eq!(status(&app).as_deref(), Some("System can't have an alert"));
    }
}
