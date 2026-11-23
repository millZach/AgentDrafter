//! Pure-Rust desktop shell helpers: path inspection, persisted Recents store,
//! window URL and title construction, and launch CLI argument parsing.
//!
//! Kept free of Tauri/GTK/WebView dependencies so every invariant is unit-tested
//! directly on every platform.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use agentdrafter_core::SET_SETTINGS_FILE;
use serde::{Deserialize, Serialize};

/// Maximum number of recent drawings and drawing sets kept on disk.
pub const MAX_RECENTS: usize = 20;

/// What kind of drawing source the desktop shell opens (Zach's A1, 2026-11-22).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// A drawing set folder holding `set.yaml`.
    Set,
    /// A plant `.dwg` original or revision (#70/#71).
    Dwg,
    /// A single `.yaml` sheet description.
    Sheet,
}

/// Result of inspecting a user-selected folder or file path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectedPath {
    pub kind: SourceKind,
    pub path: String,
    pub name: String,
    pub detail: String,
}

/// One persisted entry in the desktop app's Recent drawings & sets list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentEntry {
    pub path: String,
    pub kind: SourceKind,
    pub name: String,
    pub opened_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
}

/// Current wall-clock time in Unix milliseconds.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Normalizes a path for display and storage without introducing Windows `\\?\`
/// verbatim prefixes that confuse users or downstream tools.
fn clean_path(path: &Path) -> PathBuf {
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    #[cfg(windows)]
    {
        let s = resolved.to_string_lossy();
        if let Some(stripped) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{stripped}"));
        }
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(stripped);
        }
    }
    resolved
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

/// Classifies a folder or file selected by the drafter or passed on the command
/// line as a drawing set (`set.yaml`), a plant `.dwg`, or a single `.yaml`
/// sheet description — or returns a clear, human-readable error.
pub fn inspect_path(raw: &Path) -> Result<InspectedPath, String> {
    if !raw.exists() {
        return Err(format!(
            "Could not find `{}`: check that the folder or file still exists.",
            raw.display()
        ));
    }
    let path = clean_path(raw);
    if path.is_dir() {
        let settings = path.join(SET_SETTINGS_FILE);
        if settings.is_file() {
            let name = display_name(&path);
            return Ok(InspectedPath {
                kind: SourceKind::Set,
                path: path.display().to_string(),
                name,
                detail: format!("Drawing set folder · {SET_SETTINGS_FILE}"),
            });
        }
        return Err(format!(
            "Not a drawing set: `{}` has no `{SET_SETTINGS_FILE}`. Pick a drawing set folder, a \
             plant `.dwg` file, or a `.yaml` sheet description.",
            display_name(&path)
        ));
    }
    if path.is_file() {
        let file_name = display_name(&path);
        // Picking `set.yaml` directly opens its enclosing drawing set folder.
        if file_name.eq_ignore_ascii_case(SET_SETTINGS_FILE) {
            if let Some(parent) = path.parent() {
                let set_name = display_name(parent);
                return Ok(InspectedPath {
                    kind: SourceKind::Set,
                    path: parent.display().to_string(),
                    name: set_name,
                    detail: format!("Drawing set folder · {SET_SETTINGS_FILE}"),
                });
            }
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        return match ext.as_deref() {
            Some("dwg") => Ok(InspectedPath {
                kind: SourceKind::Dwg,
                path: path.display().to_string(),
                name: file_name,
                detail: "Plant DWG · original read-only".to_owned(),
            }),
            Some("yaml" | "yml") => Ok(InspectedPath {
                kind: SourceKind::Sheet,
                path: path.display().to_string(),
                name: file_name,
                detail: "Single sheet description · YAML".to_owned(),
            }),
            _ => Err(format!(
                "Unsupported file `{file_name}`: AgentDrafter opens drawing set folders \
                 (`{SET_SETTINGS_FILE}`), plant `.dwg` files, and single `.yaml` sheet descriptions."
            )),
        };
    }
    Err(format!(
        "`{}` is neither a folder nor a regular file.",
        path.display()
    ))
}

/// Loads persisted recents from `file`. Returns an empty list if missing or
/// unreadable/corrupt so the Start screen always opens cleanly.
pub fn load_recents(file: &Path) -> Vec<RecentEntry> {
    let Ok(bytes) = fs::read(file) else {
        return Vec::new();
    };
    let mut entries: Vec<RecentEntry> = serde_json::from_slice(&bytes).unwrap_or_default();
    entries.truncate(MAX_RECENTS);
    entries
}

/// Atomically writes `entries` to `file` (creating parent directories if needed).
pub fn save_recents(file: &Path, entries: &[RecentEntry]) -> Result<(), String> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "could not create app data folder `{}`: {e}",
                parent.display()
            )
        })?;
    }
    let json = serde_json::to_vec_pretty(entries)
        .map_err(|e| format!("could not serialize recents: {e}"))?;
    let tmp = file.with_extension(format!("json.tmp.{}", std::process::id()));
    fs::write(&tmp, &json).map_err(|e| format!("could not write temporary recents file: {e}"))?;
    if let Err(e) = fs::rename(&tmp, file) {
        // On Windows `rename` can fail if destination exists; remove and retry.
        let _ = fs::remove_file(file);
        fs::rename(&tmp, file).map_err(|e2| {
            let _ = fs::remove_file(&tmp);
            format!(
                "could not save recents (`{}`): {e}; retry: {e2}",
                file.display()
            )
        })?;
    }
    Ok(())
}

fn same_recent_path(a: &str, b: &str) -> bool {
    #[cfg(windows)]
    {
        a.eq_ignore_ascii_case(b)
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

/// Inserts `entry` at the front of `entries`, removing any duplicate for the
/// same path and capping the list at [`MAX_RECENTS`].
pub fn record_recent(entries: &mut Vec<RecentEntry>, entry: RecentEntry) {
    entries.retain(|existing| !same_recent_path(&existing.path, &entry.path));
    entries.insert(0, entry);
    entries.truncate(MAX_RECENTS);
}

/// Removes any entry matching `path` from `entries`.
pub fn remove_recent(entries: &mut Vec<RecentEntry>, path: &str) {
    entries.retain(|existing| !same_recent_path(&existing.path, path));
}

/// Percent-encodes a query parameter value according to RFC 3986 unreserved bytes.
pub fn percent_encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        let unreserved = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if unreserved {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Builds the local server URL loaded in a desktop `WebviewWindow`.
///
/// The per-session bearer token is always placed in the URL fragment (`#token=…`),
/// never in the query string.
pub fn window_url(base_url: &str, token: &str, source: Option<&str>, start: bool) -> String {
    let base = base_url.trim_end_matches('/');
    let query = match source.filter(|s| !s.trim().is_empty()) {
        Some(path) => format!("desktop=1&source={}", percent_encode_query_value(path)),
        None if start => "desktop=1&start=1".to_owned(),
        None => "desktop=1".to_owned(),
    };
    format!("{base}/?{query}#token={token}")
}

/// Formats the native desktop window title.
pub fn window_title(name: Option<&str>) -> String {
    match name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(n) => format!("AgentDrafter — {n}"),
        None => "AgentDrafter".to_owned(),
    }
}

/// Parses an optional folder or file path passed on the desktop binary's
/// command line (`agentdrafter-desktop <path>`). Ignores flags and macOS
/// Finder `-psn_...` arguments.
pub fn parse_launch_path(args: &[OsString]) -> Option<PathBuf> {
    for arg in args.iter().skip(1) {
        let text = arg.to_string_lossy();
        if text.is_empty() || text.starts_with('-') {
            continue;
        }
        let path = PathBuf::from(arg);
        if path.is_absolute() {
            return Some(path);
        }
        return Some(
            std::env::current_dir()
                .map(|cwd| cwd.join(&path))
                .unwrap_or(path),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agentdrafter-desktop-state-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn inspect_path_classifies_sets_dwgs_sheets_and_rejects_invalid_paths() {
        let dir = temp_dir("inspect");

        // Drawing set folder with set.yaml.
        let set_dir = dir.join("demo-set");
        fs::create_dir_all(&set_dir).unwrap();
        fs::write(set_dir.join("set.yaml"), "unit: in\n").unwrap();
        let set_info = inspect_path(&set_dir).unwrap();
        assert_eq!(set_info.kind, SourceKind::Set);
        assert_eq!(set_info.name, "demo-set");

        // Picking set.yaml directly maps to its parent drawing set folder.
        let via_yaml = inspect_path(&set_dir.join("set.yaml")).unwrap();
        assert_eq!(via_yaml.kind, SourceKind::Set);
        assert_eq!(via_yaml.name, "demo-set");
        assert_eq!(via_yaml.path, set_info.path);

        // Folder without set.yaml gives a friendly error.
        let empty_dir = dir.join("misc-notes");
        fs::create_dir_all(&empty_dir).unwrap();
        let err = inspect_path(&empty_dir).unwrap_err();
        assert!(err.contains("set.yaml"), "{err}");

        // Plant DWG (any case).
        let dwg = dir.join("DP-P-101.DWG");
        fs::write(&dwg, b"AC1032").unwrap();
        let dwg_info = inspect_path(&dwg).unwrap();
        assert_eq!(dwg_info.kind, SourceKind::Dwg);
        assert_eq!(dwg_info.name, "DP-P-101.DWG");

        // Single YAML sheet description.
        let yaml = dir.join("geometry.yaml");
        fs::write(&yaml, "units: in\n").unwrap();
        let sheet_info = inspect_path(&yaml).unwrap();
        assert_eq!(sheet_info.kind, SourceKind::Sheet);
        assert_eq!(sheet_info.name, "geometry.yaml");

        // Unsupported file extension.
        let txt = dir.join("readme.txt");
        fs::write(&txt, "hi").unwrap();
        let err = inspect_path(&txt).unwrap_err();
        assert!(err.contains("Unsupported file"), "{err}");

        // Nonexistent path.
        let err = inspect_path(&dir.join("missing.dwg")).unwrap_err();
        assert!(err.contains("Could not find"), "{err}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn recents_store_persists_dedupes_caps_and_removes() {
        let dir = temp_dir("recents");
        let file = dir.join("nested").join("recents.json");

        assert!(load_recents(&file).is_empty());

        let mut list = Vec::new();
        for i in 0..(MAX_RECENTS + 5) {
            record_recent(
                &mut list,
                RecentEntry {
                    path: format!("/drawings/set-{i}"),
                    kind: SourceKind::Set,
                    name: format!("set-{i}"),
                    opened_at_ms: i as u64,
                    subtitle: None,
                },
            );
        }
        assert_eq!(list.len(), MAX_RECENTS);
        assert_eq!(list[0].name, format!("set-{}", MAX_RECENTS + 4));

        // Re-recording an existing entry moves it to front without duplicating.
        record_recent(
            &mut list,
            RecentEntry {
                path: "/drawings/set-10".to_owned(),
                kind: SourceKind::Set,
                name: "set-10".to_owned(),
                opened_at_ms: 9999,
                subtitle: Some("8 sheets".to_owned()),
            },
        );
        assert_eq!(list.len(), MAX_RECENTS);
        assert_eq!(list[0].path, "/drawings/set-10");
        assert_eq!(
            list.iter().filter(|e| e.path == "/drawings/set-10").count(),
            1
        );

        save_recents(&file, &list).unwrap();
        let loaded = load_recents(&file);
        assert_eq!(loaded, list);

        remove_recent(&mut list, "/drawings/set-10");
        assert_eq!(list.len(), MAX_RECENTS - 1);
        assert!(list.iter().all(|e| e.path != "/drawings/set-10"));

        // Corrupt JSON falls back cleanly to empty list.
        fs::write(&file, b"{not valid json").unwrap();
        assert!(load_recents(&file).is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn window_url_encodes_paths_and_keeps_token_in_fragment() {
        let token = "ab".repeat(32);
        let start = window_url("http://127.0.0.1:51234", &token, None, true);
        assert_eq!(
            start,
            format!("http://127.0.0.1:51234/?desktop=1&start=1#token={token}")
        );

        let win_path = r"C:\Users\drafter\Demo Plant\DP-P-101 #1&2.dwg";
        let url = window_url("http://127.0.0.1:51234/", &token, Some(win_path), false);
        assert!(url.starts_with("http://127.0.0.1:51234/?desktop=1&source="));
        assert!(url.ends_with(&format!("#token={token}")));
        let query_part = url.split('#').next().unwrap();
        assert!(
            !query_part.contains(&token),
            "token must never appear in query"
        );
        assert!(!query_part.contains(' '), "spaces must be percent-encoded");
        assert!(
            query_part.contains("%5C"),
            "backslashes must be percent-encoded"
        );
        assert!(
            query_part.contains("%23"),
            "# in path must be percent-encoded"
        );
        assert!(
            query_part.contains("%26"),
            "& in path must be percent-encoded"
        );
    }

    #[test]
    fn window_title_and_cli_arg_parsing() {
        assert_eq!(window_title(None), "AgentDrafter");
        assert_eq!(window_title(Some("   ")), "AgentDrafter");
        assert_eq!(
            window_title(Some("demo-plant")),
            "AgentDrafter — demo-plant"
        );

        let empty: Vec<OsString> = vec!["agentdrafter-desktop".into()];
        assert_eq!(parse_launch_path(&empty), None);

        let flags_only: Vec<OsString> = vec![
            "agentdrafter-desktop".into(),
            "-psn_0_12345".into(),
            "--devtools".into(),
        ];
        assert_eq!(parse_launch_path(&flags_only), None);

        let with_abs: Vec<OsString> = vec![
            "agentdrafter-desktop".into(),
            "-psn_0_12345".into(),
            "/tmp/demo-plant".into(),
        ];
        assert_eq!(
            parse_launch_path(&with_abs),
            Some(PathBuf::from("/tmp/demo-plant"))
        );
    }
}
