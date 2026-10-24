//! Terminal setup and the event loop.

use std::io;
use std::time::{Duration, Instant};

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use crate::collector::{Collector, PlistBundles, proc_path};
use crate::grouping::{Group, build_groups};
use crate::model::Snapshot;
use crate::terminate::{Action, confirm_prompt, execute, plan_quit, plan_terminate};
use crate::ui::render::{View, draw};
use crate::ui::state::{RowId, UiState};

const STATUS_DURATION: Duration = Duration::from_secs(3);

struct App<'c> {
    collector: &'c mut dyn Collector,
    bundles: PlistBundles,
    snap: Snapshot,
    groups: Vec<Group>,
    state: UiState,
    pending: Option<Action>,
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
        let mut app = App {
            collector,
            bundles: PlistBundles::default(),
            groups: Vec::new(),
            snap,
            state: UiState::default(),
            pending: None,
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

    fn set_status(&mut self, message: String) {
        self.status = Some((message, Instant::now()));
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal, interval: Duration) -> io::Result<()> {
        let mut next_refresh = Instant::now() + interval;
        loop {
            if self
                .status
                .as_ref()
                .is_some_and(|(_, at)| at.elapsed() >= STATUS_DURATION)
            {
                self.status = None;
            }
            let prompt = self.pending.as_ref().map(confirm_prompt);
            terminal.draw(|frame| {
                let view = View {
                    stats: &self.snap.stats,
                    groups: &self.groups,
                    state: &self.state,
                    status: self.status.as_ref().map(|(m, _)| m.as_str()),
                    prompt: prompt.as_deref(),
                    color: self.color,
                };
                draw(frame, &view);
            })?;

            let timeout = next_refresh
                .saturating_duration_since(Instant::now())
                .min(STATUS_DURATION);
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
        if let Some(action) = self.pending.take() {
            if matches!(code, KeyCode::Char('y' | 'Y')) {
                let message = match execute(&action, proc_path) {
                    Ok(()) => match &action {
                        Action::Terminate { name, .. } => format!("Sent terminate to {name}"),
                        Action::Quit { name, .. } => format!("Asked {name} to quit"),
                    },
                    Err(err) => err,
                };
                self.set_status(message);
                self.refresh();
            } else {
                self.set_status("Cancelled".to_string());
            }
            return true;
        }
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return false,
            KeyCode::Up => self.state.move_selection(&self.groups, -1),
            KeyCode::Down => self.state.move_selection(&self.groups, 1),
            KeyCode::Enter => self.state.toggle_expand(),
            KeyCode::Char('k') => match self.plan() {
                Some(Ok(action)) => self.pending = Some(action),
                Some(Err(message)) => self.set_status(message),
                None => {}
            },
            _ => {}
        }
        true
    }

    fn plan(&self) -> Option<Result<Action, String>> {
        match self.state.selected.as_ref()? {
            RowId::Group(key) => {
                let group = self.groups.iter().find(|g| &g.key == key)?;
                Some(plan_quit(group, &self.snap))
            }
            RowId::Proc(key, pid) => {
                let group = self.groups.iter().find(|g| &g.key == key)?;
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
                pressure: Pressure::Normal,
            },
            current_uid: 501,
            self_pid: 9999,
        })
    }

    fn status(app: &App) -> Option<String> {
        app.status.as_ref().map(|(m, _)| m.clone())
    }

    #[test]
    fn k_on_system_shows_refusal() {
        let mut c = collector();
        let mut app = App::new(&mut c).unwrap();
        app.handle_key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(app.state.selected, Some(RowId::Group(Bucket::System)));
        app.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        assert_eq!(status(&app).as_deref(), Some("System processes can't be terminated"));
        assert!(app.pending.is_none());
    }

    #[test]
    fn k_on_process_asks_then_cancels() {
        let mut c = collector();
        let mut app = App::new(&mut c).unwrap();
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        app.handle_key(KeyCode::Down, KeyModifiers::NONE);
        app.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        let prompt = app.pending.as_ref().map(confirm_prompt).unwrap();
        assert!(prompt.starts_with("Terminate p1"), "{prompt}");
        assert!(app.handle_key(KeyCode::Char('n'), KeyModifiers::NONE));
        assert!(app.pending.is_none());
        assert_eq!(status(&app).as_deref(), Some("Cancelled"));
    }

    #[test]
    fn k_on_app_without_bundle_id_is_refused() {
        let mut c = collector();
        let mut app = App::new(&mut c).unwrap();
        app.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        // The fake bundle has no Info.plist, so there is no bundle id to quit by.
        assert!(app.pending.is_none());
        assert_eq!(status(&app).as_deref(), Some("Can't identify this app to quit it"));
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
}
