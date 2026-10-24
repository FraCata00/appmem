//! Reads app names and bundle ids from `Info.plist`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::grouping::{AppMeta, BundleInfo};

#[derive(Default)]
pub struct PlistBundles {
    cache: HashMap<PathBuf, (String, Option<String>)>,
}

impl BundleInfo for PlistBundles {
    fn meta(&mut self, bundle: &Path) -> AppMeta {
        let (name, bundle_id) = self
            .cache
            .entry(bundle.to_path_buf())
            .or_insert_with(|| read(bundle))
            .clone();
        AppMeta { name, bundle_id }
    }
}

fn read(bundle: &Path) -> (String, Option<String>) {
    let plist = plist::Value::from_file(bundle.join("Contents/Info.plist")).ok();
    let dict = plist.as_ref().and_then(|v| v.as_dictionary());
    let get = |key: &str| {
        dict.and_then(|d| d.get(key))
            .and_then(|v| v.as_string())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let folder = bundle
        .file_name()
        .map(|n| n.to_string_lossy().trim_end_matches(".app").to_string())
        .unwrap_or_default();
    let name = get("CFBundleDisplayName")
        .or_else(|| get("CFBundleName"))
        .unwrap_or(folder);
    (name, get("CFBundleIdentifier"))
}
