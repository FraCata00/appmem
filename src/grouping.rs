//! Turns a flat process list into per-app groups with totals.

use std::collections::HashMap;
use std::path::Path;

use crate::attribution::{Bucket, attribute};
use crate::model::{ProcInfo, Snapshot};

pub struct AppMeta {
    pub name: String,
    pub bundle_id: Option<String>,
}

/// Looks up display names and bundle ids for `.app` bundles.
pub trait BundleInfo {
    fn meta(&mut self, bundle: &Path) -> AppMeta;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub key: Bucket,
    pub name: String,
    pub bundle_id: Option<String>,
    /// Sorted by footprint, largest first.
    pub procs: Vec<ProcInfo>,
    pub total: u64,
    /// True when the total includes memory that could not be attributed to a process.
    pub estimated: bool,
}

/// Groups readable processes per app, sorted by total with System always last.
pub fn build_groups(snap: &Snapshot, bundles: &mut dyn BundleInfo) -> Vec<Group> {
    let paths: HashMap<i32, &Path> =
        snap.procs.iter().filter_map(|p| Some((p.pid, p.path.as_deref()?))).collect();

    let mut groups: HashMap<Bucket, Group> = HashMap::new();
    let mut listed: u64 = 0;
    for proc in &snap.procs {
        let Some(footprint) = proc.footprint else { continue };
        let responsible = proc
            .responsible_pid
            .filter(|&pid| pid != proc.pid)
            .and_then(|pid| paths.get(&pid).copied());
        let key = attribute(proc, responsible, snap.current_uid);
        let group = groups.entry(key.clone()).or_insert_with(|| new_group(key, bundles));
        group.procs.push(proc.clone());
        group.total += footprint;
        listed += footprint;
    }

    let system = groups.entry(Bucket::System).or_insert_with(|| new_group(Bucket::System, bundles));
    let remainder = snap.stats.used.saturating_sub(snap.stats.wired).saturating_sub(listed);
    system.total += remainder;
    system.estimated = remainder > 0;

    let mut groups: Vec<Group> = groups.into_values().collect();
    for group in &mut groups {
        group.procs.sort_by(|a, b| b.footprint.cmp(&a.footprint).then(a.pid.cmp(&b.pid)));
    }
    groups.sort_by(|a, b| {
        (a.key == Bucket::System)
            .cmp(&(b.key == Bucket::System))
            .then(b.total.cmp(&a.total))
            .then(a.name.cmp(&b.name))
    });
    groups
}

fn new_group(key: Bucket, bundles: &mut dyn BundleInfo) -> Group {
    let (name, bundle_id) = match &key {
        Bucket::App(bundle) => {
            let meta = bundles.meta(bundle);
            (meta.name, meta.bundle_id)
        }
        Bucket::Simulator => ("Simulator".to_string(), Some("com.apple.iphonesimulator".to_string())),
        Bucket::CommandLine => ("Command-line tools".to_string(), None),
        Bucket::System => ("System".to_string(), None),
    };
    Group { key, name, bundle_id, procs: Vec::new(), total: 0, estimated: false }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Pressure, ProcInfo, Snapshot, SystemStats};

    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * MB;

    struct FakeBundles;

    impl BundleInfo for FakeBundles {
        fn meta(&mut self, bundle: &Path) -> AppMeta {
            let name = bundle.file_name().unwrap().to_string_lossy();
            AppMeta { name: name.trim_end_matches(".app").to_string(), bundle_id: Some(format!("test.{name}")) }
        }
    }

    fn proc(pid: i32, path: &str, footprint: Option<u64>) -> ProcInfo {
        ProcInfo {
            pid,
            ppid: 1,
            uid: 501,
            name: format!("p{pid}"),
            path: Some(path.into()),
            footprint,
            responsible_pid: None,
        }
    }

    fn snap(procs: Vec<ProcInfo>, used: u64, wired: u64) -> Snapshot {
        Snapshot {
            procs,
            stats: SystemStats { total: 8 * GB, used, wired, swap_used: 0, pressure: Pressure::Normal },
            current_uid: 501,
            self_pid: 9999,
        }
    }

    const CHROME: &str = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
    const SYS: &str = "/usr/libexec/sysd";

    #[test]
    fn groups_chrome_helpers_together() {
        let s = snap(
            vec![proc(1, CHROME, Some(100 * MB)), proc(2, CHROME, Some(200 * MB)), proc(3, CHROME, Some(300 * MB))],
            600 * MB,
            0,
        );
        let groups = build_groups(&s, &mut FakeBundles);
        let chrome = &groups[0];
        assert_eq!(chrome.name, "Google Chrome");
        assert_eq!(chrome.total, 600 * MB);
        let order: Vec<i32> = chrome.procs.iter().map(|p| p.pid).collect();
        assert_eq!(order, vec![3, 2, 1]);
    }

    #[test]
    fn system_is_last_even_when_biggest() {
        let s = snap(vec![proc(1, SYS, Some(3 * GB)), proc(2, CHROME, Some(GB))], 4 * GB, 0);
        let groups = build_groups(&s, &mut FakeBundles);
        assert_eq!(groups.last().unwrap().key, Bucket::System);
        assert_eq!(groups[0].name, "Google Chrome");
    }

    #[test]
    fn remainder_adds_to_system() {
        let s = snap(vec![proc(1, CHROME, Some(3 * GB + GB / 2)), proc(2, SYS, Some(GB / 2))], 6 * GB, GB);
        let groups = build_groups(&s, &mut FakeBundles);
        let system = groups.last().unwrap();
        assert_eq!(system.total, GB / 2 + GB);
        assert!(system.estimated);
    }

    #[test]
    fn remainder_saturates_at_zero() {
        let s = snap(vec![proc(1, CHROME, Some(8 * GB)), proc(2, SYS, Some(GB))], 6 * GB, GB);
        let groups = build_groups(&s, &mut FakeBundles);
        let system = groups.last().unwrap();
        assert_eq!(system.total, GB);
        assert!(!system.estimated);
    }

    #[test]
    fn unreadable_procs_are_not_listed() {
        let s = snap(vec![proc(1, CHROME, Some(GB)), proc(2, CHROME, None)], GB, 0);
        let groups = build_groups(&s, &mut FakeBundles);
        assert!(groups.iter().all(|g| g.procs.iter().all(|p| p.pid != 2)));
    }

    #[test]
    fn system_group_always_present() {
        let s = snap(vec![proc(1, CHROME, Some(GB))], GB, 0);
        let groups = build_groups(&s, &mut FakeBundles);
        assert_eq!(groups.last().unwrap().key, Bucket::System);
    }

    #[test]
    fn responsible_lookup_uses_snapshot() {
        let zed = proc(10, "/Applications/Zed.app/Contents/MacOS/zed", Some(GB));
        let mut ra = proc(11, "/Users/u/.cargo/bin/rust-analyzer", Some(GB));
        ra.responsible_pid = Some(10);
        let groups = build_groups(&snap(vec![zed, ra], 2 * GB, 0), &mut FakeBundles);
        assert_eq!(groups[0].name, "Zed");
        assert_eq!(groups[0].procs.len(), 2);
    }

    #[test]
    fn named_special_groups() {
        let cli = proc(1, "/opt/homebrew/bin/node", Some(GB));
        let sim = proc(2, "/Applications/Xcode.app/Contents/Developer/Applications/Simulator.app/Contents/MacOS/Simulator", Some(GB));
        let groups = build_groups(&snap(vec![cli, sim], 2 * GB, 0), &mut FakeBundles);
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert!(names.contains(&"Command-line tools"));
        let sim = groups.iter().find(|g| g.key == Bucket::Simulator).unwrap();
        assert_eq!(sim.name, "Simulator");
        assert_eq!(sim.bundle_id.as_deref(), Some("com.apple.iphonesimulator"));
    }
}
