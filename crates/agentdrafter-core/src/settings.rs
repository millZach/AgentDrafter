//! Settings: what a machine tells the core about itself, apart from any
//! one request.
//!
//! The settings say where fonts and plot styles are. AutoCAD's SHX fonts
//! and its CTB plot styles are Autodesk's files and are never in this
//! repository (ADR-0008); on a machine that has AutoCAD they are read from
//! its `Fonts` and `Plot Styles` folders, which settings name. TrueType
//! fonts are also looked for in the system's own font folders, after the
//! folders named, as AutoCAD finds them ([`system_font_folders`]), unless
//! [`SYSTEM_FONTS_VARIABLE`] turns them off. There is no other default
//! folder: SHX fonts and plot styles are read only from the folders named.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The environment variable that names the font folders: a list of folders
/// separated as the system separates paths, `;` on Windows and `:`
/// elsewhere, such as AutoCAD's `Fonts` folder.
pub const FONTS_VARIABLE: &str = "AGENTDRAFTER_FONTS";

/// The environment variable that turns the system's font folders off: `0`
/// (or `false`, `no` or `off`), and TrueType fonts are read only from the
/// font folders named, so that what is measured does not depend on the
/// fonts the machine has installed. Any other value, or none, leaves them
/// on.
pub const SYSTEM_FONTS_VARIABLE: &str = "AGENTDRAFTER_SYSTEM_FONTS";

/// The environment variable that names the plot style folders: a list of
/// folders separated as the system separates paths, such as AutoCAD's
/// `Plot Styles` folder.
pub const PLOT_STYLES_VARIABLE: &str = "AGENTDRAFTER_PLOT_STYLES";

/// What the core needs to know about the machine it runs on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// The folders fonts are read from, searched in order. A font is found
    /// by its file name, ignoring case. A font in none of them, nor for a
    /// TrueType font in the system's font folders, is measured with a
    /// look-alike, and the result says so.
    #[serde(default)]
    pub font_folders: Vec<PathBuf>,
    /// The system's own font folders, searched after `font_folders`, each
    /// with the folders inside it, for TrueType fonts only: AutoCAD's SHX
    /// fonts are never among the system's. A font found in one says so.
    #[serde(default)]
    pub system_font_folders: Vec<PathBuf>,
    /// The folders CTB plot styles are read from, searched in order. A plot
    /// style is found by its file name, ignoring case.
    #[serde(default)]
    pub plot_style_folders: Vec<PathBuf>,
}

impl Settings {
    /// The settings the environment gives: the font folders that
    /// [`FONTS_VARIABLE`] names, the system's font folders unless
    /// [`SYSTEM_FONTS_VARIABLE`] turns them off, and the plot style folders
    /// that [`PLOT_STYLES_VARIABLE`] names.
    pub fn from_env() -> Self {
        let system = system_fonts_on(std::env::var_os(SYSTEM_FONTS_VARIABLE).as_deref());
        Self {
            system_font_folders: if system {
                system_font_folders(std::env::consts::OS, |name| std::env::var_os(name))
            } else {
                Vec::new()
            },
            plot_style_folders: folders(std::env::var_os(PLOT_STYLES_VARIABLE).as_deref()),
            ..Self::from_variable(std::env::var_os(FONTS_VARIABLE).as_deref())
        }
    }

    /// The settings `value` gives when it is the value of
    /// [`FONTS_VARIABLE`]. Empty entries are left out.
    pub fn from_variable(value: Option<&OsStr>) -> Self {
        Self {
            font_folders: folders(value),
            system_font_folders: Vec::new(),
            plot_style_folders: Vec::new(),
        }
    }

    /// These settings without the system's font folders, as a command's
    /// `no_system_fonts` asks.
    pub fn without_system_fonts(mut self) -> Self {
        self.system_font_folders.clear();
        self
    }
}

/// Whether `value`, the value of [`SYSTEM_FONTS_VARIABLE`], leaves the
/// system's font folders on: anything but `0`, `false`, `no` or `off`,
/// whatever its case, or no value at all.
fn system_fonts_on(value: Option<&OsStr>) -> bool {
    let Some(value) = value.and_then(OsStr::to_str) else {
        return true;
    };
    !["0", "false", "no", "off"]
        .iter()
        .any(|off| value.trim().eq_ignore_ascii_case(off))
}

/// The system's own font folders on the system `os` names, as
/// [`std::env::consts::OS`] does, with `env` giving the environment's
/// variables: the folders TrueType fonts are installed in, searched in this
/// order, each with the folders inside it.
///
/// - Windows: `%WINDIR%\Fonts`, then the user's own,
///   `%LOCALAPPDATA%\Microsoft\Windows\Fonts`.
/// - macOS: `/System/Library/Fonts`, `/Library/Fonts`, `~/Library/Fonts`.
/// - Linux and the other Unix systems: `/usr/share/fonts`,
///   `/usr/local/share/fonts`, `~/.local/share/fonts`, `~/.fonts`.
///
/// A folder whose variable is not set is left out. A system with no file
/// system of its own, such as WebAssembly in a browser, has none.
pub fn system_font_folders(os: &str, env: impl Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let set = |name: &str| env(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let home = |below: &[&str]| {
        set("HOME").map(|home| below.iter().fold(home, |path, part| path.join(part)))
    };
    let folders = match os {
        "windows" => vec![
            set("WINDIR")
                .or_else(|| set("SystemRoot"))
                .map(|w| w.join("Fonts")),
            set("LOCALAPPDATA").map(|l| l.join("Microsoft").join("Windows").join("Fonts")),
        ],
        "macos" => vec![
            Some(PathBuf::from("/System/Library/Fonts")),
            Some(PathBuf::from("/Library/Fonts")),
            home(&["Library", "Fonts"]),
        ],
        "linux" | "freebsd" | "openbsd" | "netbsd" | "dragonfly" | "solaris" | "illumos" => vec![
            Some(PathBuf::from("/usr/share/fonts")),
            Some(PathBuf::from("/usr/local/share/fonts")),
            home(&[".local", "share", "fonts"]),
            home(&[".fonts"]),
        ],
        _ => Vec::new(),
    };
    folders.into_iter().flatten().collect()
}

/// The folders a variable's value lists, empty entries left out.
fn folders(value: Option<&OsStr>) -> Vec<PathBuf> {
    value
        .map(|v| {
            std::env::split_paths(v)
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use super::*;

    #[test]
    fn the_variable_lists_folders_as_the_system_separates_paths() {
        let joined: OsString =
            std::env::join_paths([Path::new("first"), Path::new(""), Path::new("second")]).unwrap();
        let settings = Settings::from_variable(Some(&joined));
        assert_eq!(
            settings.font_folders,
            [PathBuf::from("first"), PathBuf::from("second")]
        );
        assert_eq!(Settings::from_variable(None), Settings::default());
    }

    fn env<'a>(vars: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            vars.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn each_system_has_its_own_font_folders_in_order() {
        let windows = system_font_folders(
            "windows",
            env(&[("WINDIR", "W"), ("LOCALAPPDATA", "L"), ("HOME", "H")]),
        );
        assert_eq!(
            windows,
            [
                Path::new("W").join("Fonts"),
                Path::new("L")
                    .join("Microsoft")
                    .join("Windows")
                    .join("Fonts"),
            ]
        );
        let macos = system_font_folders("macos", env(&[("HOME", "H")]));
        assert_eq!(
            macos,
            [
                PathBuf::from("/System/Library/Fonts"),
                PathBuf::from("/Library/Fonts"),
                Path::new("H").join("Library").join("Fonts"),
            ]
        );
        let linux = system_font_folders("linux", env(&[("HOME", "H")]));
        assert_eq!(
            linux,
            [
                PathBuf::from("/usr/share/fonts"),
                PathBuf::from("/usr/local/share/fonts"),
                Path::new("H").join(".local").join("share").join("fonts"),
                Path::new("H").join(".fonts"),
            ]
        );
    }

    #[test]
    fn a_folder_whose_variable_is_not_set_is_left_out() {
        assert_eq!(
            system_font_folders("windows", env(&[("SystemRoot", "S")])),
            [Path::new("S").join("Fonts")]
        );
        assert!(system_font_folders("windows", env(&[])).is_empty());
        assert_eq!(
            system_font_folders("linux", env(&[("HOME", "")])),
            [
                PathBuf::from("/usr/share/fonts"),
                PathBuf::from("/usr/local/share/fonts"),
            ]
        );
        assert!(system_font_folders("unknown", env(&[("HOME", "H")])).is_empty());
    }

    #[test]
    fn the_system_folders_are_off_only_when_the_variable_says_so() {
        assert!(system_fonts_on(None));
        assert!(system_fonts_on(Some(OsStr::new("1"))));
        assert!(system_fonts_on(Some(OsStr::new(""))));
        for off in ["0", "false", "No", "OFF", " 0 "] {
            assert!(!system_fonts_on(Some(OsStr::new(off))), "{off}");
        }
    }

    #[test]
    fn without_system_fonts_keeps_the_folders_named() {
        let settings = Settings {
            font_folders: vec![PathBuf::from("named")],
            system_font_folders: vec![PathBuf::from("system")],
            plot_style_folders: vec![PathBuf::from("styles")],
        };
        let without = settings.clone().without_system_fonts();
        assert_eq!(without.font_folders, settings.font_folders);
        assert_eq!(without.plot_style_folders, settings.plot_style_folders);
        assert!(without.system_font_folders.is_empty());
    }
}
