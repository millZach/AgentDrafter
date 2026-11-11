//! Settings: what a machine tells the core about itself, apart from any
//! one request.
//!
//! So far the only setting is where fonts are. AutoCAD's SHX fonts are
//! Autodesk's files and are never in this repository (ADR-0008); on a
//! machine that has AutoCAD they are read from its `Fonts` folder, which a
//! setting names. There is no default folder.

use std::ffi::OsStr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The environment variable that names the font folders: a list of folders
/// separated as the system separates paths, `;` on Windows and `:`
/// elsewhere, such as AutoCAD's `Fonts` folder and the system's fonts
/// folder.
pub const FONTS_VARIABLE: &str = "AGENTDRAFTER_FONTS";

/// What the core needs to know about the machine it runs on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// The folders fonts are read from, searched in order. A font is found
    /// by its file name, ignoring case. A font in none of them is measured
    /// with a look-alike, and the result says so.
    #[serde(default)]
    pub font_folders: Vec<PathBuf>,
}

impl Settings {
    /// The settings the environment gives: the font folders that
    /// [`FONTS_VARIABLE`] names, or none.
    pub fn from_env() -> Self {
        Self::from_variable(std::env::var_os(FONTS_VARIABLE).as_deref())
    }

    /// The settings `value` gives when it is the value of
    /// [`FONTS_VARIABLE`]. Empty entries are left out.
    pub fn from_variable(value: Option<&OsStr>) -> Self {
        let font_folders = value
            .map(|v| {
                std::env::split_paths(v)
                    .filter(|p| !p.as_os_str().is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Self { font_folders }
    }
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
