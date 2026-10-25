//! Reads app names and bundle ids from `Info.plist`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::grouping::{AppMeta, BundleInfo};

#[derive(Default)]
pub struct PlistBundles {
    cache: HashMap<PathBuf, AppMeta>,
}

impl BundleInfo for PlistBundles {
    fn meta(&mut self, bundle: &Path) -> AppMeta {
        if let Some(meta) = self.cache.get(bundle) {
            return meta.clone();
        }
        let meta = read(bundle);
        self.cache.insert(bundle.to_path_buf(), meta.clone());
        meta
    }
}

fn read(bundle: &Path) -> AppMeta {
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
    AppMeta {
        name,
        bundle_id: get("CFBundleIdentifier"),
    }
}
