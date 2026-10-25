//! Turns a flat process list into per-app groups with totals.

use std::collections::HashMap;
use std::path::Path;

use crate::attribution::{Bucket, attribute};
use crate::config::Config;
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
    let paths: HashMap<i32, &Path> = snap
        .procs
        .iter()
        .filter_map(|p| Some((p.pid, p.path.as_deref()?)))
        .collect();

    let mut groups: HashMap<Bucket, Group> = HashMap::new();
    let mut listed: u64 = 0;
    let mut unreadable = false;
    for proc in &snap.procs {
        let Some(footprint) = proc.footprint else {
            unreadable = true;
            continue;
        };
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

    let system = groups
        .entry(Bucket::System)
        .or_insert_with(|| new_group(Bucket::System, bundles));
    // Only estimate when something couldn't be read: otherwise the gap is
    // shared or kernel memory that belongs to no process in particular.
    let remainder = if unreadable {
        snap.stats.app_footprint.saturating_sub(listed)
    } else {
        0
    };
    system.total += remainder;
    system.estimated = remainder > 0;

    let mut groups: Vec<Group> = groups.into_values().collect();
    for group in &mut groups {
        group
            .procs
            .sort_by(|a, b| b.footprint.cmp(&a.footprint).then(a.pid.cmp(&b.pid)));
    }
    groups.sort_by(|a, b| {
        (a.key == Bucket::System)
            .cmp(&(b.key == Bucket::System))
            .then(b.total.cmp(&a.total))
            .then(a.name.cmp(&b.name))
    });
    groups
}

/// Name a group's alert is saved under: the bundle id, which survives moving or
/// renaming the app. `None` for System, which can't have an alert.
pub fn alert_key(group: &Group) -> Option<String> {
    match &group.key {
        Bucket::System => None,
        Bucket::CommandLine => Some("command-line-tools".to_string()),
        Bucket::App(bundle) => Some(
            group
                .bundle_id
                .clone()
                .unwrap_or_else(|| bundle.to_string_lossy().into_owned()),
        ),
        Bucket::Simulator => group.bundle_id.clone(),
    }
}

/// True when the group uses more memory than its saved alert.
pub fn over_alert(group: &Group, config: &Config) -> bool {
    alert_key(group)
        .and_then(|key| config.alert(&key))
        .is_some_and(|limit| group.total > limit)
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
    Group {
        key,
        name,
        bundle_id,
        procs: Vec::new(),
        total: 0,
        estimated: false,
    }
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
            AppMeta {
                name: name.trim_end_matches(".app").to_string(),
                bundle_id: Some(format!("test.{name}")),
            }
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

    fn snap(procs: Vec<ProcInfo>, app_footprint: u64) -> Snapshot {
        Snapshot {
            procs,
            stats: SystemStats {
                total: 8 * GB,
                used: app_footprint / 2,
                wired: GB,
                swap_used: 0,
                app_footprint,
                pressure: Pressure::Normal,
            },
            current_uid: 501,
            self_pid: 9999,
        }
    }

    const CHROME: &str = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
    const SYS: &str = "/usr/libexec/sysd";

    #[test]
    fn groups_chrome_helpers_together() {
        let s = snap(
            vec![
                proc(1, CHROME, Some(100 * MB)),
                proc(2, CHROME, Some(200 * MB)),
                proc(3, CHROME, Some(300 * MB)),
            ],
            600 * MB,
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
        let s = snap(vec![proc(1, SYS, Some(3 * GB)), proc(2, CHROME, Some(GB))], 4 * GB);
        let groups = build_groups(&s, &mut FakeBundles);
        assert_eq!(groups.last().unwrap().key, Bucket::System);
        assert_eq!(groups[0].name, "Google Chrome");
    }

    #[test]
    fn remainder_adds_to_system() {
        // Footprints are compared with app memory counted uncompressed, not with "used".
        let s = snap(
            vec![
                proc(1, CHROME, Some(3 * GB + GB / 2)),
                proc(2, SYS, Some(GB / 2)),
                proc(3, SYS, None),
            ],
            5 * GB,
        );
        let groups = build_groups(&s, &mut FakeBundles);
        let system = groups.last().unwrap();
        assert_eq!(system.total, GB / 2 + GB);
        assert!(system.estimated);
    }

    #[test]
    fn no_estimate_when_every_process_is_readable() {
        let s = snap(vec![proc(1, CHROME, Some(GB)), proc(2, SYS, Some(GB))], 5 * GB);
        let system = build_groups(&s, &mut FakeBundles).pop().unwrap();
        assert_eq!(system.total, GB);
        assert!(!system.estimated);
    }

    #[test]
    fn remainder_saturates_at_zero() {
        let s = snap(
            vec![
                proc(1, CHROME, Some(8 * GB)),
                proc(2, SYS, Some(GB)),
                proc(3, SYS, None),
            ],
            6 * GB,
        );
        let groups = build_groups(&s, &mut FakeBundles);
        let system = groups.last().unwrap();
        assert_eq!(system.total, GB);
        assert!(!system.estimated);
    }

    #[test]
    fn unreadable_procs_are_not_listed() {
        let s = snap(vec![proc(1, CHROME, Some(GB)), proc(2, CHROME, None)], GB);
        let groups = build_groups(&s, &mut FakeBundles);
        assert!(groups.iter().all(|g| g.procs.iter().all(|p| p.pid != 2)));
    }

    #[test]
    fn system_group_always_present() {
        let s = snap(vec![proc(1, CHROME, Some(GB))], GB);
        let groups = build_groups(&s, &mut FakeBundles);
        assert_eq!(groups.last().unwrap().key, Bucket::System);
    }

    #[test]
    fn responsible_lookup_uses_snapshot() {
        let zed = proc(10, "/Applications/Zed.app/Contents/MacOS/zed", Some(GB));
        let mut ra = proc(11, "/Users/u/.cargo/bin/rust-analyzer", Some(GB));
        ra.responsible_pid = Some(10);
        let groups = build_groups(&snap(vec![zed, ra], 2 * GB), &mut FakeBundles);
        assert_eq!(groups[0].name, "Zed");
        assert_eq!(groups[0].procs.len(), 2);
    }

    #[test]
    fn named_special_groups() {
        let cli = proc(1, "/opt/homebrew/bin/node", Some(GB));
        let sim = proc(
            2,
            "/Applications/Xcode.app/Contents/Developer/Applications/Simulator.app/Contents/MacOS/Simulator",
            Some(GB),
        );
        let groups = build_groups(&snap(vec![cli, sim], 2 * GB), &mut FakeBundles);
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert!(names.contains(&"Command-line tools"));
        let sim = groups.iter().find(|g| g.key == Bucket::Simulator).unwrap();
        assert_eq!(sim.name, "Simulator");
        assert_eq!(sim.bundle_id.as_deref(), Some("com.apple.iphonesimulator"));
    }

    fn plain(key: Bucket, name: &str, bundle_id: Option<&str>, total: u64) -> Group {
        Group {
            key,
            name: name.into(),
            bundle_id: bundle_id.map(Into::into),
            procs: vec![],
            total,
            estimated: false,
        }
    }

    #[test]
    fn alert_keys() {
        let safari = plain(
            Bucket::App("/Applications/Safari.app".into()),
            "Safari",
            Some("com.apple.Safari"),
            0,
        );
        assert_eq!(alert_key(&safari).as_deref(), Some("com.apple.Safari"));
        let no_id = plain(Bucket::App("/Applications/Tool.app".into()), "Tool", None, 0);
        assert_eq!(alert_key(&no_id).as_deref(), Some("/Applications/Tool.app"));
        let cli = plain(Bucket::CommandLine, "Command-line tools", None, 0);
        assert_eq!(alert_key(&cli).as_deref(), Some("command-line-tools"));
        let sim = plain(Bucket::Simulator, "Simulator", Some("com.apple.iphonesimulator"), 0);
        assert_eq!(alert_key(&sim).as_deref(), Some("com.apple.iphonesimulator"));
        assert_eq!(alert_key(&plain(Bucket::System, "System", None, 0)), None);
    }

    #[test]
    fn over_alert_only_above_threshold() {
        let mut config = crate::config::Config::default();
        config.set_alert("com.apple.Safari", Some(2 * GB));
        let at = plain(
            Bucket::App("/Applications/Safari.app".into()),
            "Safari",
            Some("com.apple.Safari"),
            2 * GB,
        );
        let above = Group {
            total: 2 * GB + 1,
            ..at.clone()
        };
        assert!(!over_alert(&at, &config));
        assert!(over_alert(&above, &config));
        let other = Group {
            bundle_id: Some("x".into()),
            ..above
        };
        assert!(!over_alert(&other, &config));
    }
}
