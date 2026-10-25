//! Saved preferences: per-app memory alerts, in `~/.config/appmem/config.toml`.
//!
//! The file is a small TOML subset, written and read by hand to avoid a dependency:
//!
//! ```toml
//! [alerts]
//! "com.apple.Safari" = 2
//! ```
//!
//! Values are gigabytes.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use crate::format::{GIB, gb_to_bytes};

const ALERTS: &str = "alerts";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    /// Alert thresholds in bytes, keyed by [`crate::grouping::alert_key`].
    alerts: BTreeMap<String, u64>,
}

impl Config {
    pub fn has_alerts(&self) -> bool {
        !self.alerts.is_empty()
    }

    pub fn alert(&self, key: &str) -> Option<u64> {
        self.alerts.get(key).copied()
    }

    /// Sets or, with `None`, removes the alert for `key`.
    pub fn set_alert(&mut self, key: &str, bytes: Option<u64>) {
        match bytes {
            Some(bytes) => self.alerts.insert(key.to_string(), bytes),
            None => self.alerts.remove(key),
        };
    }

    /// Reads what it understands and ignores the rest, so a hand-edited typo never stops appmem.
    pub fn parse(text: &str) -> Config {
        let mut config = Config::default();
        let mut section = String::new();
        for line in text.lines() {
            let line = line.trim();
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.trim().to_string();
                continue;
            }
            if section != ALERTS {
                continue;
            }
            let Some((key, rest)) = parse_key(line) else { continue };
            let value = rest
                .trim_start()
                .strip_prefix('=')
                .map(|v| v.split('#').next().unwrap_or("").trim());
            let Some(gb) = value.and_then(|v| v.parse::<f64>().ok()) else {
                continue;
            };
            if let Some(bytes) = gb_to_bytes(gb) {
                config.alerts.insert(key, bytes);
            }
        }
        config
    }

    pub fn to_text(&self) -> String {
        let mut text = String::from("# appmem preferences. Alerts are in GB.\n\n[alerts]\n");
        for (key, bytes) in &self.alerts {
            let escaped = key.replace('\\', "\\\\").replace('"', "\\\"");
            text.push_str(&format!("\"{escaped}\" = {}\n", *bytes as f64 / GIB as f64));
        }
        text
    }

    /// A missing or unreadable file is an empty config.
    pub fn load(path: &Path) -> Config {
        std::fs::read_to_string(path)
            .map(|t| Config::parse(&t))
            .unwrap_or_default()
    }

    /// Writes through a temporary file so a crash never leaves half a config.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(&tmp, path)
    }
}

/// Parses a leading `"quoted key"`, returning it unescaped and the rest of the line.
fn parse_key(line: &str) -> Option<(String, &str)> {
    let body = line.strip_prefix('"')?;
    let mut key = String::new();
    let mut chars = body.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => key.push(chars.next()?.1),
            '"' => return Some((key, &body[i + 1..])),
            _ => key.push(c),
        }
    }
    None
}

/// `$XDG_CONFIG_HOME/appmem/config.toml`, else `~/.config/appmem/config.toml`.
pub fn config_path() -> Option<PathBuf> {
    config_path_from(std::env::var("XDG_CONFIG_HOME").ok(), std::env::var("HOME").ok())
}

fn config_path_from(xdg: Option<String>, home: Option<String>) -> Option<PathBuf> {
    let base = match xdg.filter(|x| !x.is_empty()) {
        Some(xdg) => PathBuf::from(xdg),
        None => PathBuf::from(home.filter(|h| !h.is_empty())?).join(".config"),
    };
    Some(base.join("appmem/config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    #[test]
    fn parses_thresholds_in_gigabytes() {
        let config = Config::parse("[alerts]\n\"com.apple.Safari\" = 2\n\"com.google.Chrome\" = 1.5\n");
        assert_eq!(config.alert("com.apple.Safari"), Some(2 * GB));
        assert_eq!(config.alert("com.google.Chrome"), Some(GB + GB / 2));
        assert_eq!(config.alert("org.mozilla.firefox"), None);
    }

    #[test]
    fn skips_comments_other_sections_and_bad_lines() {
        let text = "# appmem\n[other]\n\"x\" = 9\n[alerts]\n\"a\" = nope\nbroken\n\"b\" = -1\n\"c\" = 0.5 # half\n";
        let config = Config::parse(text);
        assert_eq!(config.alert("x"), None);
        assert_eq!(config.alert("a"), None);
        assert_eq!(config.alert("b"), None);
        assert_eq!(config.alert("c"), Some(GB / 2));
    }

    #[test]
    fn round_trips_through_text() {
        let mut config = Config::default();
        config.set_alert("com.apple.Safari", Some(2 * GB));
        config.set_alert("command-line-tools", Some(GB / 4));
        let again = Config::parse(&config.to_text());
        assert_eq!(again, config);
    }

    #[test]
    fn keys_with_quotes_are_escaped() {
        let mut config = Config::default();
        config.set_alert("/Applications/My \"Odd\" App.app", Some(GB));
        assert_eq!(Config::parse(&config.to_text()), config);
    }

    #[test]
    fn removing_an_alert() {
        let mut config = Config::default();
        config.set_alert("a", Some(GB));
        config.set_alert("a", None);
        assert_eq!(config.alert("a"), None);
    }

    #[test]
    fn saves_and_loads_from_disk() {
        let dir = std::env::temp_dir().join(format!("appmem-config-test-{}", std::process::id()));
        let path = dir.join("nested/config.toml");
        let mut config = Config::default();
        config.set_alert("com.apple.Safari", Some(2 * GB));
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path), config);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_file_is_empty_config() {
        assert_eq!(
            Config::load(Path::new("/nonexistent/appmem/config.toml")),
            Config::default()
        );
    }

    #[test]
    fn config_path_honours_xdg() {
        assert_eq!(
            config_path_from(Some("/x/cfg".into()), Some("/Users/u".into())),
            Some(PathBuf::from("/x/cfg/appmem/config.toml"))
        );
        assert_eq!(
            config_path_from(None, Some("/Users/u".into())),
            Some(PathBuf::from("/Users/u/.config/appmem/config.toml"))
        );
        assert_eq!(config_path_from(Some("".into()), None), None);
    }
}
