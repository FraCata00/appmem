//! Decides which app a process belongs to.
//!
//! Rules, in order (see the design spec):
//! 1. Simulator runtimes and Simulator.app go to Simulator.
//! 2. The app of the responsible process, if it is an app or the Simulator.
//!    System processes are responsible for themselves, so they never land here.
//! 3. Apple system paths go to System, except Apple's own apps in
//!    `/System/Applications` (Safari, Notes…), which are apps like any other.
//! 4. The outermost `.app` bundle in the executable path.
//! 5. Other processes of the current user go to Command-line tools.
//! 6. Everything else goes to System.

use std::path::{Component, Path, PathBuf};

use crate::model::ProcInfo;

const SYSTEM_PREFIXES: [&str; 5] = ["/System/", "/usr/", "/bin/", "/sbin/", "/Library/Apple/"];
const APPLE_APP_DIRS: [&str; 2] = [
    "/System/Applications/",
    "/System/Volumes/Preboot/Cryptexes/App/System/Applications/",
];

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Bucket {
    /// Path of the outermost `.app` bundle.
    App(PathBuf),
    Simulator,
    CommandLine,
    System,
}

/// Simulator, system and bundle rules applied to a single executable path.
pub fn classify_path(path: &Path) -> Option<Bucket> {
    let text = path.to_string_lossy();
    if text.contains(".simruntime/") || text.contains("/CoreSimulator/") {
        return Some(Bucket::Simulator);
    }
    let apple_app = APPLE_APP_DIRS.iter().any(|dir| text.starts_with(dir));
    if !apple_app && SYSTEM_PREFIXES.iter().any(|prefix| text.starts_with(prefix)) {
        return Some(Bucket::System);
    }

    let mut outermost: Option<PathBuf> = None;
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component);
        if let Component::Normal(name) = component {
            let name = name.to_string_lossy();
            if name == "Simulator.app" {
                return Some(Bucket::Simulator);
            }
            if outermost.is_none() && name.ends_with(".app") {
                outermost = Some(prefix.clone());
            }
        }
    }
    outermost.map(Bucket::App)
}

pub fn attribute(proc: &ProcInfo, responsible_path: Option<&Path>, current_uid: u32) -> Bucket {
    let own = proc.path.as_deref().and_then(classify_path);
    if own == Some(Bucket::Simulator) {
        return Bucket::Simulator;
    }
    if let Some(responsible) = responsible_path.and_then(classify_path)
        && matches!(responsible, Bucket::App(_) | Bucket::Simulator)
    {
        return responsible;
    }
    if let Some(bucket) = own {
        return bucket;
    }
    if proc.uid == current_uid {
        Bucket::CommandLine
    } else {
        Bucket::System
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProcInfo;
    use std::path::Path;

    fn p(path: &str, uid: u32) -> ProcInfo {
        ProcInfo {
            pid: 100,
            ppid: 1,
            uid,
            name: Path::new(path).file_name().unwrap().to_string_lossy().into_owned(),
            path: Some(path.into()),
            footprint: Some(1),
            responsible_pid: None,
        }
    }

    #[test]
    fn chrome_renderer_goes_to_chrome() {
        let r = p(
            "/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Versions/130.0/Helpers/Google Chrome Helper (Renderer).app/Contents/MacOS/Google Chrome Helper (Renderer)",
            501,
        );
        assert_eq!(
            attribute(&r, None, 501),
            Bucket::App("/Applications/Google Chrome.app".into())
        );
    }

    #[test]
    fn bundle_path_with_spaces() {
        let r = p("/Applications/My App 🚀.app/Contents/MacOS/x", 501);
        assert_eq!(
            attribute(&r, None, 501),
            Bucket::App("/Applications/My App 🚀.app".into())
        );
    }

    #[test]
    fn finder_is_system() {
        let f = p("/System/Library/CoreServices/Finder.app/Contents/MacOS/Finder", 501);
        assert_eq!(attribute(&f, None, 501), Bucket::System);
    }

    #[test]
    fn simruntime_process_is_simulator() {
        let s = p(
            "/Library/Developer/CoreSimulator/Volumes/iOS_22A/Library/Developer/CoreSimulator/Profiles/Runtimes/iOS 18.0.simruntime/Contents/Resources/RuntimeRoot/Applications/MobileSafari.app/MobileSafari",
            501,
        );
        assert_eq!(attribute(&s, None, 501), Bucket::Simulator);
    }

    #[test]
    fn simulator_app_inside_xcode_is_simulator() {
        let s = p(
            "/Applications/Xcode.app/Contents/Developer/Applications/Simulator.app/Contents/MacOS/Simulator",
            501,
        );
        assert_eq!(attribute(&s, None, 501), Bucket::Simulator);
    }

    #[test]
    fn responsible_app_wins_over_cli() {
        let cli = p("/Users/u/.cargo/bin/rust-analyzer", 501);
        let zed = Path::new("/Applications/Zed.app/Contents/MacOS/zed");
        assert_eq!(
            attribute(&cli, Some(zed), 501),
            Bucket::App("/Applications/Zed.app".into())
        );
    }

    #[test]
    fn system_daemon_responsible_for_itself_is_system() {
        let d = p("/usr/libexec/sharingd", 501);
        assert_eq!(
            attribute(&d, Some(Path::new("/usr/libexec/sharingd")), 501),
            Bucket::System
        );
    }

    #[test]
    fn system_app_bundles_are_apps() {
        let safari = p(
            "/System/Volumes/Preboot/Cryptexes/App/System/Applications/Safari.app/Contents/MacOS/Safari",
            501,
        );
        assert_eq!(
            attribute(&safari, None, 501),
            Bucket::App("/System/Volumes/Preboot/Cryptexes/App/System/Applications/Safari.app".into())
        );
        let notes = p("/System/Applications/Notes.app/Contents/MacOS/Notes", 501);
        assert_eq!(
            attribute(&notes, None, 501),
            Bucket::App("/System/Applications/Notes.app".into())
        );
    }

    #[test]
    fn webkit_service_goes_to_responsible_app() {
        let gpu = p(
            "/System/Volumes/Preboot/Cryptexes/Incoming/OS/System/Library/Frameworks/WebKit.framework/Versions/A/XPCServices/com.apple.WebKit.GPU.xpc/Contents/MacOS/com.apple.WebKit.GPU",
            501,
        );
        let safari =
            Path::new("/System/Volumes/Preboot/Cryptexes/App/System/Applications/Safari.app/Contents/MacOS/Safari");
        assert_eq!(
            attribute(&gpu, Some(safari), 501),
            Bucket::App("/System/Volumes/Preboot/Cryptexes/App/System/Applications/Safari.app".into())
        );
    }

    #[test]
    fn system_responsible_is_ignored() {
        let cli = p("/opt/homebrew/bin/node", 501);
        let finder = Path::new("/System/Library/CoreServices/Finder.app/Contents/MacOS/Finder");
        assert_eq!(attribute(&cli, Some(finder), 501), Bucket::CommandLine);
    }

    #[test]
    fn user_cli_without_app_is_command_line() {
        assert_eq!(
            attribute(&p("/opt/homebrew/bin/node", 501), None, 501),
            Bucket::CommandLine
        );
    }

    #[test]
    fn other_user_cli_is_system() {
        assert_eq!(attribute(&p("/opt/homebrew/bin/node", 0), None, 501), Bucket::System);
    }

    #[test]
    fn unknown_path_own_user_is_command_line() {
        let mut u = p("/x", 501);
        u.path = None;
        assert_eq!(attribute(&u, None, 501), Bucket::CommandLine);
    }
}
