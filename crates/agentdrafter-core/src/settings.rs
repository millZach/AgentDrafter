//! Settings: what a machine tells the core about itself, apart from any
//! one request.
//!
//! The settings say where fonts and plot styles are. AutoCAD's SHX fonts
//! and its CTB plot styles are Autodesk's files and are never in this
//! repository (ADR-0008); on a machine that has AutoCAD they are read from
//! its `Fonts` and `Plot Styles` folders, which settings name. There is no
//! default folder.

use std::ffi::OsStr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The environment variable that names the font folders: a list of folders
/// separated as the system separates paths, `;` on Windows and `:`
/// elsewhere, such as AutoCAD's `Fonts` folder and the system's fonts
/// folder.
pub const FONTS_VARIABLE: &str = "AGENTDRAFTER_FONTS";

/// The environment variable that names the plot style folders: a list of
/// folders separated as the system separates paths, such as AutoCAD's
/// `Plot Styles` folder.
pub const PLOT_STYLES_VARIABLE: &str = "AGENTDRAFTER_PLOT_STYLES";

/// What the core needs to know about the machine it runs on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// The folders fonts are read from, searched in order. A font is found
    /// by its file name, ignoring case. A font in none of them is measured
    /// with a look-alike, and the result says so.
    #[serde(default)]
    pub font_folders: Vec<PathBuf>,
    /// The folders CTB plot styles are read from, searched in order. A plot
    /// style is found by its file name, ignoring case.
    #[serde(default)]
    pub plot_style_folders: Vec<PathBuf>,
}

impl Settings {
    /// The settings the environment gives: the font folders that
    /// [`FONTS_VARIABLE`] names and the plot style folders that
    /// [`PLOT_STYLES_VARIABLE`] names, or none.
    pub fn from_env() -> Self {
        Self {
            plot_style_folders: folders(std::env::var_os(PLOT_STYLES_VARIABLE).as_deref()),
            ..Self::from_variable(std::env::var_os(FONTS_VARIABLE).as_deref())
        }
    }

    /// The settings `value` gives when it is the value of
    /// [`FONTS_VARIABLE`]. Empty entries are left out.
    pub fn from_variable(value: Option<&OsStr>) -> Self {
        Self {
            font_folders: folders(value),
            plot_style_folders: Vec::new(),
        }
    }
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
}
