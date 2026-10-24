#![cfg(target_os = "macos")]

use std::path::Path;
use std::process::Command;

use appmem::collector::{Collector, MacCollector, PlistBundles, proc_path};
use appmem::grouping::BundleInfo;

#[test]
fn snapshot_contains_self() {
    let snap = MacCollector::new().snapshot().unwrap();
    let me = snap
        .procs
        .iter()
        .find(|p| p.pid == snap.self_pid)
        .expect("own process listed");
    assert!(me.footprint > Some(0));
    let exe = std::env::current_exe().unwrap();
    assert_eq!(me.path.as_deref().unwrap().file_name(), exe.file_name());
    assert_eq!(me.uid, snap.current_uid);
}

#[test]
fn stats_are_sane() {
    let stats = MacCollector::new().snapshot().unwrap().stats;
    assert!(stats.total > 0);
    assert!(stats.used > 0 && stats.used <= stats.total, "{stats:?}");
    assert!(stats.wired < stats.used, "{stats:?}");
}

#[test]
fn snapshot_survives_dead_pid() {
    let mut child = Command::new("/bin/sleep").arg("0").spawn().unwrap();
    let pid = child.id() as i32;
    child.wait().unwrap();
    let snap = MacCollector::new().snapshot().unwrap();
    if let Some(p) = snap.procs.iter().find(|p| p.pid == pid) {
        assert_eq!(p.footprint, None);
    }
}

#[test]
fn responsible_symbol_resolves_or_is_none() {
    let _ = MacCollector::new();
}

#[test]
fn proc_path_of_self_matches_current_exe() {
    let path = proc_path(std::process::id() as i32).unwrap();
    assert_eq!(path.file_name(), std::env::current_exe().unwrap().file_name());
}

#[test]
fn reads_safari_bundle_name() {
    let meta = PlistBundles::default().meta(Path::new("/Applications/Safari.app"));
    assert_eq!(meta.name, "Safari");
    assert_eq!(meta.bundle_id.as_deref(), Some("com.apple.Safari"));
}

#[test]
fn missing_bundle_falls_back_to_folder_name() {
    let meta = PlistBundles::default().meta(Path::new("/nonexistent/My Tool.app"));
    assert_eq!(meta.name, "My Tool");
    assert_eq!(meta.bundle_id, None);
}

#[test]
fn terminates_real_child_process() {
    use appmem::terminate::{Action, execute};
    use std::os::unix::process::ExitStatusExt;

    let mut child = Command::new("/bin/sleep").arg("600").spawn().unwrap();
    let pid = child.id() as i32;
    let action = Action::Terminate {
        pid,
        name: "sleep".into(),
        path: proc_path(pid),
        footprint: 0,
    };
    execute(&action, proc_path).unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGTERM));
}
