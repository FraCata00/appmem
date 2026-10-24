//! Terminating a process or quitting an app, with the safety checks from the spec.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::attribution::Bucket;
use crate::format::format_bytes;
use crate::grouping::Group;
use crate::model::{ProcInfo, Snapshot};

const SYSTEM_REFUSAL: &str = "System processes can't be terminated";
const NOT_YOURS: &str = "Not your process";
const PROTECTED: &str = "That's appmem or the terminal running it";
const NOT_QUITTABLE: &str = "This group can't be quit as a whole";
const NO_BUNDLE_ID: &str = "Can't identify this app to quit it";
const EXITED: &str = "Process already exited";
const AUTOMATION_DENIED: &str = "Allow appmem's terminal in System Settings → Privacy & Security → Automation";

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Terminate { pid: i32, name: String, path: Option<PathBuf>, footprint: u64 },
    Quit { name: String, bundle_id: String },
}

/// appmem itself and every ancestor up to (not including) launchd.
pub fn protected_pids(snap: &Snapshot) -> HashSet<i32> {
    let parents: HashMap<i32, i32> = snap.procs.iter().map(|p| (p.pid, p.ppid)).collect();
    let mut protected = HashSet::new();
    let mut pid = snap.self_pid;
    while pid > 1 && protected.insert(pid) {
        match parents.get(&pid) {
            Some(&ppid) => pid = ppid,
            None => break,
        }
    }
    protected
}

pub fn plan_terminate(proc: &ProcInfo, group: &Group, snap: &Snapshot) -> Result<Action, String> {
    if group.key == Bucket::System {
        return Err(SYSTEM_REFUSAL.into());
    }
    if proc.uid != snap.current_uid {
        return Err(NOT_YOURS.into());
    }
    if protected_pids(snap).contains(&proc.pid) {
        return Err(PROTECTED.into());
    }
    Ok(Action::Terminate {
        pid: proc.pid,
        name: proc.name.clone(),
        path: proc.path.clone(),
        footprint: proc.footprint.unwrap_or(0),
    })
}

pub fn plan_quit(group: &Group, snap: &Snapshot) -> Result<Action, String> {
    if matches!(group.key, Bucket::CommandLine | Bucket::System) {
        return Err(NOT_QUITTABLE.into());
    }
    let protected = protected_pids(snap);
    if group.procs.iter().any(|p| protected.contains(&p.pid)) {
        return Err(PROTECTED.into());
    }
    let Some(bundle_id) = group.bundle_id.clone() else {
        return Err(NO_BUNDLE_ID.into());
    };
    // Asking an app that isn't running to quit would launch it first.
    if !group.procs.iter().any(|p| p.path.as_deref().is_some_and(|path| is_main_executable(&group.key, path))) {
        return Err(format!("{} isn't running, only its helpers", group.name));
    }
    Ok(Action::Quit { name: group.name.clone(), bundle_id })
}

fn is_main_executable(key: &Bucket, path: &Path) -> bool {
    match key {
        Bucket::App(bundle) => path.starts_with(bundle.join("Contents/MacOS")),
        Bucket::Simulator => path.to_string_lossy().contains("/Simulator.app/Contents/MacOS/"),
        Bucket::CommandLine | Bucket::System => false,
    }
}

pub fn confirm_prompt(action: &Action) -> String {
    match action {
        Action::Terminate { pid, name, footprint, .. } => {
            format!("Terminate {name} (pid {pid}, {})? y/N", format_bytes(*footprint))
        }
        Action::Quit { name, .. } => format!("Quit {name}? y/N"),
    }
}

/// Runs a confirmed action. `current_path` re-reads a pid's executable so a
/// reused pid is never signalled.
pub fn execute(action: &Action, current_path: impl Fn(i32) -> Option<PathBuf>) -> Result<(), String> {
    match action {
        Action::Terminate { pid, path, .. } => {
            if current_path(*pid) != *path {
                return Err(EXITED.into());
            }
            // SAFETY: kill only sends a signal; the pid was re-checked just above.
            if unsafe { libc::kill(*pid, libc::SIGTERM) } == 0 {
                return Ok(());
            }
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ESRCH) { Err(EXITED.into()) } else { Err(err.to_string()) }
        }
        Action::Quit { bundle_id, .. } => {
            if bundle_id.contains(['"', '\\']) {
                return Err(NO_BUNDLE_ID.into());
            }
            let script = format!("tell application id \"{bundle_id}\" to quit");
            let output = Command::new("/usr/bin/osascript").args(["-e", &script]).output().map_err(|e| e.to_string())?;
            if output.status.success() {
                return Ok(());
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("-1743") {
                return Err(AUTOMATION_DENIED.into());
            }
            Err(stderr.lines().next().unwrap_or("osascript failed").to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Pressure, SystemStats};

    const MB: u64 = 1024 * 1024;
    const CHROME: &str = "/Applications/Google Chrome.app";
    const TERMINAL: &str = "/System/Applications/Utilities/Terminal.app";

    fn proc(pid: i32, ppid: i32, uid: u32, path: &str, name: &str) -> ProcInfo {
        ProcInfo {
            pid,
            ppid,
            uid,
            name: name.into(),
            path: Some(path.into()),
            footprint: Some(820 * MB),
            responsible_pid: None,
        }
    }

    fn snapshot(procs: Vec<ProcInfo>) -> Snapshot {
        Snapshot {
            procs,
            stats: SystemStats { total: 0, used: 0, wired: 0, swap_used: 0, pressure: Pressure::Normal },
            current_uid: 501,
            self_pid: 900,
        }
    }

    fn chain() -> Vec<ProcInfo> {
        vec![
            proc(900, 800, 501, "/Users/u/.cargo/bin/appmem", "appmem"),
            proc(800, 700, 501, "/bin/zsh", "zsh"),
            proc(700, 1, 501, &format!("{TERMINAL}/Contents/MacOS/Terminal"), "Terminal"),
            proc(1, 0, 0, "/sbin/launchd", "launchd"),
        ]
    }

    fn group(key: Bucket, name: &str, bundle_id: Option<&str>, procs: Vec<ProcInfo>) -> Group {
        Group { key, name: name.into(), bundle_id: bundle_id.map(Into::into), procs, total: 0, estimated: false }
    }

    fn chrome_group() -> (Group, ProcInfo) {
        let renderer = proc(
            812,
            1,
            501,
            &format!("{CHROME}/Contents/Frameworks/Helpers/Google Chrome Helper (Renderer).app/Contents/MacOS/r"),
            "Google Chrome Helper (Renderer)",
        );
        let main = proc(500, 1, 501, &format!("{CHROME}/Contents/MacOS/Google Chrome"), "Google Chrome");
        let g = group(Bucket::App(CHROME.into()), "Google Chrome", Some("com.google.Chrome"), vec![renderer.clone(), main]);
        (g, renderer)
    }

    #[test]
    fn plans_terminate_for_own_app_process() {
        let (g, renderer) = chrome_group();
        let action = plan_terminate(&renderer, &g, &snapshot(chain())).unwrap();
        assert_eq!(confirm_prompt(&action), "Terminate Google Chrome Helper (Renderer) (pid 812, 820 MB)? y/N");
    }

    #[test]
    fn refuses_system_group() {
        let d = proc(300, 1, 501, "/usr/libexec/sharingd", "sharingd");
        let g = group(Bucket::System, "System", None, vec![d.clone()]);
        assert_eq!(plan_terminate(&d, &g, &snapshot(chain())).unwrap_err(), "System processes can't be terminated");
    }

    #[test]
    fn refuses_other_users_process() {
        let (g, mut renderer) = chrome_group();
        renderer.uid = 0;
        assert_eq!(plan_terminate(&renderer, &g, &snapshot(chain())).unwrap_err(), "Not your process");
    }

    #[test]
    fn refuses_self_and_parent_shell() {
        let snap = snapshot(chain());
        assert_eq!(protected_pids(&snap), HashSet::from([900, 800, 700]));
        for p in &snap.procs[..3] {
            let g = group(Bucket::CommandLine, "Command-line tools", None, vec![p.clone()]);
            assert_eq!(plan_terminate(p, &g, &snap).unwrap_err(), "That's appmem or the terminal running it");
        }
    }

    #[test]
    fn plans_quit_for_running_app() {
        let (g, _) = chrome_group();
        let action = plan_quit(&g, &snapshot(chain())).unwrap();
        assert_eq!(confirm_prompt(&action), "Quit Google Chrome? y/N");
    }

    #[test]
    fn quit_refused_for_group_containing_terminal() {
        let snap = snapshot(chain());
        let g = group(Bucket::App(TERMINAL.into()), "Terminal", Some("com.apple.Terminal"), vec![snap.procs[2].clone()]);
        assert_eq!(plan_quit(&g, &snap).unwrap_err(), "That's appmem or the terminal running it");
    }

    #[test]
    fn quit_refused_for_command_line_and_system_groups() {
        let snap = snapshot(chain());
        for key in [Bucket::CommandLine, Bucket::System] {
            let g = group(key, "x", None, vec![]);
            assert_eq!(plan_quit(&g, &snap).unwrap_err(), "This group can't be quit as a whole");
        }
    }

    #[test]
    fn quit_refused_without_bundle_id() {
        let (mut g, _) = chrome_group();
        g.bundle_id = None;
        assert_eq!(plan_quit(&g, &snapshot(chain())).unwrap_err(), "Can't identify this app to quit it");
    }

    #[test]
    fn quit_refused_when_only_helpers_run() {
        let widget = proc(
            400,
            1,
            501,
            "/System/Applications/Weather.app/Contents/PlugIns/WeatherWidget.appex/Contents/MacOS/WeatherWidget",
            "WeatherWidget",
        );
        let g = group(Bucket::App("/System/Applications/Weather.app".into()), "Weather", Some("com.apple.weather"), vec![widget]);
        assert_eq!(plan_quit(&g, &snapshot(chain())).unwrap_err(), "Weather isn't running, only its helpers");
    }

    #[test]
    fn quit_allowed_for_simulator() {
        let sim = proc(
            450,
            1,
            501,
            "/Applications/Xcode.app/Contents/Developer/Applications/Simulator.app/Contents/MacOS/Simulator",
            "Simulator",
        );
        let g = group(Bucket::Simulator, "Simulator", Some("com.apple.iphonesimulator"), vec![sim]);
        assert!(plan_quit(&g, &snapshot(chain())).is_ok());
    }

    #[test]
    fn refuses_when_pid_now_points_elsewhere() {
        let action = Action::Terminate { pid: i32::MAX, name: "x".into(), path: Some("/a".into()), footprint: 0 };
        assert_eq!(execute(&action, |_| Some("/b".into())), Err("Process already exited".to_string()));
    }

    #[test]
    fn exited_process_reports_already_exited() {
        let action = Action::Terminate { pid: i32::MAX, name: "x".into(), path: Some("/a".into()), footprint: 0 };
        assert_eq!(execute(&action, |_| Some("/a".into())), Err("Process already exited".to_string()));
    }

    #[test]
    fn rejects_quote_in_bundle_id() {
        let action = Action::Quit { name: "x".into(), bundle_id: "a\" to do shell script \"b".into() };
        assert_eq!(execute(&action, |_| None), Err("Can't identify this app to quit it".to_string()));
    }
}
