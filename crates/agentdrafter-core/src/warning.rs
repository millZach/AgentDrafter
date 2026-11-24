//! Warnings: what a reply says at its top, beside `ok`, so that an agent
//! reads it without looking inside each sheet's result.
//!
//! So far there is one kind: a look-alike stood in for a font
//! (`font_look_alike`). A reply in which one did lists each font once, with
//! the look-alike and every sheet it was used on, under `warnings`; a reply
//! in which none did leaves `warnings` out, so it reads as it did before.
//! Each sheet's own result still lists its `font_substitutions` as before.
//! The warnings are worked out here, from the command's result, so every
//! interface sends the same ones (ADR-0007).

use std::path::Path;

use serde::Serialize;

use crate::build::BuildReport;
use crate::change::{AcceptReport, ChangeReport};
use crate::check::CheckReport;
use crate::display_list::DisplayListReport;
use crate::model::UnresolvedXref;
use crate::original::ReadReport;
use crate::plot::{PlotReport, PlotSetReport};
use crate::query::QueryReport;
use crate::render::RenderReport;
use crate::set::SetReport;
use crate::text::fonts::FontSubstitution;
use crate::text::measure::TextMeasurement;

/// What a warning is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningCode {
    /// A font was in no font folder, or could not be read, and a look-alike
    /// was measured and drawn in its place.
    FontLookAlike,
    /// An external reference (xref) could not be found or loaded from disk.
    UnresolvedXref,
}

/// One warning at the top of a reply: a font a look-alike stood in for, or an
/// external reference that could not be found or loaded.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Warning {
    /// `font_look_alike` or `unresolved_xref`.
    pub code: WarningCode,
    /// The font that was asked for, as the text style names it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub font: String,
    /// The look-alike used in its place.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub look_alike: String,
    /// The external reference block name, for `unresolved_xref`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xref: Option<String>,
    /// The external reference path that was looked for, for `unresolved_xref`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Why the external reference could not be loaded, for `unresolved_xref`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Every sheet it was used on, in the order the reply gives them;
    /// empty for `measure-text`, which measures no sheet.
    pub sheets: Vec<String>,
}

/// The warnings of one reply, gathered as its result is read: one per
/// font, whatever its case, each sheet named once.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Warnings(Vec<Warning>);

impl Warnings {
    /// Notes that `look_alike` stood in for `font`, on `sheet` if a sheet
    /// was measured.
    pub(crate) fn look_alike(&mut self, font: &str, look_alike: &str, sheet: Option<&str>) {
        let at = match self.0.iter().position(|w| {
            w.code == WarningCode::FontLookAlike
                && w.font.eq_ignore_ascii_case(font)
                && w.look_alike.eq_ignore_ascii_case(look_alike)
        }) {
            Some(at) => at,
            None => {
                self.0.push(Warning {
                    code: WarningCode::FontLookAlike,
                    font: font.to_owned(),
                    look_alike: look_alike.to_owned(),
                    xref: None,
                    path: None,
                    reason: None,
                    sheets: Vec::new(),
                });
                self.0.len() - 1
            }
        };
        if let Some(sheet) = sheet {
            let sheets = &mut self.0[at].sheets;
            if !sheets.iter().any(|s| s == sheet) {
                sheets.push(sheet.to_owned());
            }
        }
    }

    /// Notes an unresolved external reference on `sheet`.
    pub(crate) fn unresolved_xref(&mut self, xref: &UnresolvedXref, sheet: Option<&str>) {
        let at = match self.0.iter().position(|w| {
            w.code == WarningCode::UnresolvedXref
                && w.xref
                    .as_deref()
                    .is_some_and(|n| n.eq_ignore_ascii_case(&xref.name))
                && w.path.as_deref() == Some(xref.path.as_str())
        }) {
            Some(at) => at,
            None => {
                self.0.push(Warning {
                    code: WarningCode::UnresolvedXref,
                    font: String::new(),
                    look_alike: String::new(),
                    xref: Some(xref.name.clone()),
                    path: Some(xref.path.clone()),
                    reason: Some(xref.reason.clone()),
                    sheets: Vec::new(),
                });
                self.0.len() - 1
            }
        };
        if let Some(sheet) = sheet {
            let sheets = &mut self.0[at].sheets;
            if !sheets.iter().any(|s| s == sheet) {
                sheets.push(sheet.to_owned());
            }
        }
    }

    /// Notes every unresolved external reference on `sheet`.
    pub(crate) fn unresolved_xrefs(&mut self, sheet: &str, unresolved: &[UnresolvedXref]) {
        for xref in unresolved {
            self.unresolved_xref(xref, Some(sheet));
        }
    }

    /// Notes each of a sheet's font substitutions.
    pub(crate) fn sheet(&mut self, sheet: &str, substitutions: &[FontSubstitution]) {
        for substitution in substitutions {
            self.look_alike(&substitution.font, &substitution.used, Some(sheet));
        }
    }

    /// Notes another reply's warnings, in their order.
    pub(crate) fn extend(&mut self, other: &Warnings) {
        for warning in &other.0 {
            match warning.code {
                WarningCode::FontLookAlike => {
                    if warning.sheets.is_empty() {
                        self.look_alike(&warning.font, &warning.look_alike, None);
                    }
                    for sheet in &warning.sheets {
                        self.look_alike(&warning.font, &warning.look_alike, Some(sheet));
                    }
                }
                WarningCode::UnresolvedXref => {
                    let xref = UnresolvedXref {
                        name: warning.xref.clone().unwrap_or_default(),
                        path: warning.path.clone().unwrap_or_default(),
                        kind: crate::model::XrefKind::Attach,
                        reason: warning.reason.clone().unwrap_or_default(),
                    };
                    if warning.sheets.is_empty() {
                        self.unresolved_xref(&xref, None);
                    }
                    for sheet in &warning.sheets {
                        self.unresolved_xref(&xref, Some(sheet));
                    }
                }
            }
        }
    }

    pub(crate) fn into_vec(self) -> Vec<Warning> {
        self.0
    }
}

/// A command's result, which may hold what its reply warns of.
pub(crate) trait Warns {
    /// Notes what the result warns of in `warnings`.
    fn warn(&self, warnings: &mut Warnings);

    /// The warnings at the top of the result's reply.
    fn warnings(&self) -> Vec<Warning> {
        let mut warnings = Warnings::default();
        self.warn(&mut warnings);
        warnings.into_vec()
    }
}

/// The name a sheet description or a DWG built, rendered or plotted on its
/// own goes by: its file's name without its extension, as `check` and
/// `display-list` name it.
fn own_sheet(source: &str) -> String {
    Path::new(source)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl Warns for BuildReport {
    fn warn(&self, warnings: &mut Warnings) {
        warnings.sheet(&own_sheet(&self.description), &self.font_substitutions);
    }
}

impl Warns for SetReport {
    fn warn(&self, warnings: &mut Warnings) {
        for outcome in &self.sheets {
            if let Some(result) = &outcome.result {
                warnings.sheet(&outcome.sheet, &result.font_substitutions);
            }
        }
    }
}

impl Warns for RenderReport {
    fn warn(&self, warnings: &mut Warnings) {
        let sheet = own_sheet(&self.source);
        warnings.sheet(&sheet, &self.font_substitutions);
        warnings.unresolved_xrefs(&sheet, &self.unresolved_xrefs);
    }
}

impl Warns for PlotReport {
    fn warn(&self, warnings: &mut Warnings) {
        let sheet = own_sheet(&self.source);
        warnings.sheet(&sheet, &self.font_substitutions);
        warnings.unresolved_xrefs(&sheet, &self.unresolved_xrefs);
    }
}

impl Warns for PlotSetReport {
    fn warn(&self, warnings: &mut Warnings) {
        for sheet in &self.sheets {
            if let Some(result) = &sheet.result {
                warnings.sheet(&sheet.sheet, &result.font_substitutions);
                warnings.unresolved_xrefs(&sheet.sheet, &result.unresolved_xrefs);
            }
        }
    }
}

impl Warns for DisplayListReport {
    fn warn(&self, warnings: &mut Warnings) {
        warnings.sheet(&self.sheet, &self.font_substitutions);
        warnings.unresolved_xrefs(&self.sheet, &self.unresolved_xrefs);
    }
}

impl Warns for ReadReport {
    fn warn(&self, warnings: &mut Warnings) {
        warnings.unresolved_xrefs(&self.sheet, &self.unresolved_xrefs);
    }
}

impl Warns for QueryReport {
    fn warn(&self, warnings: &mut Warnings) {
        for sheet in &self.sheets {
            warnings.unresolved_xrefs(sheet, &self.unresolved_xrefs);
        }
    }
}

impl Warns for CheckReport {
    fn warn(&self, warnings: &mut Warnings) {
        warnings.extend(&self.warnings);
    }
}

impl Warns for ChangeReport {
    fn warn(&self, warnings: &mut Warnings) {
        for sheet in &self.sheets {
            if let Some(result) = &sheet.result {
                warnings.sheet(&sheet.sheet, &result.font_substitutions);
            }
        }
        if let Some(checks) = self.checks.as_ref().and_then(|c| c.result.as_ref()) {
            checks.warn(warnings);
        }
    }
}

impl Warns for crate::change::ProposeReport {
    fn warn(&self, warnings: &mut Warnings) {
        self.would.warn(warnings);
    }
}

impl Warns for crate::change::KeepReport {
    fn warn(&self, warnings: &mut Warnings) {
        self.change.warn(warnings);
    }
}

impl Warns for AcceptReport {
    fn warn(&self, warnings: &mut Warnings) {
        self.checks.warn(warnings);
    }
}

impl Warns for TextMeasurement {
    fn warn(&self, warnings: &mut Warnings) {
        if self.font.substituted {
            warnings.look_alike(&self.font.requested, &self.font.used, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use serde_json::{json, Value};

    use super::*;
    use crate::commands::find_command;
    use crate::interface::Interface;
    use crate::settings::Settings;
    use crate::text::truetype::tests::made_up_font;

    const HERSHEY: &str = "Hershey Roman Simplex";

    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("agentdrafter-warning-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Runs `command` of the registry as every interface does, and gives
    /// its reply as JSON text.
    fn run(command: &str, input: &Value, settings: &Settings) -> String {
        let reply = find_command(command)
            .unwrap()
            .run(input.clone(), settings, Interface::Cli);
        serde_json::to_string(&reply).unwrap()
    }

    #[test]
    fn every_command_that_measures_or_draws_text_warns_of_a_look_alike_at_its_top() {
        let dir = scratch("every");
        // A made-up TrueType font, in a folder inside a made-up system folder.
        let system = dir.join("system");
        fs::create_dir_all(system.join("truetype")).unwrap();
        fs::write(
            system.join("truetype").join("MadeUpSans.ttf"),
            made_up_font(&[('A', 600, [0, 0, 500, 700]), ('B', 650, [0, 0, 550, 700])]),
        )
        .unwrap();
        let sheet = dir.join("label.yaml");
        fs::write(
            &sheet,
            "units: mm\n\
             text_styles:\n  label: {font: madeupsans.ttf}\n\
             objects:\n  - text: {contents: AB, position: [0, 0, 0], height: 5, style: label}\n",
        )
        .unwrap();
        let settings = Settings {
            system_font_folders: vec![system],
            ..Settings::default()
        };
        let source = sheet.display().to_string();
        let calls = [
            ("build", json!({"source": source, "overwrite": true})),
            (
                "render",
                json!({"source": source, "overwrite": true, "width": 64, "height": 64}),
            ),
            ("plot", json!({"source": source, "overwrite": true})),
            ("display-list", json!({"source": source})),
            ("check", json!({"source": source})),
            (
                "measure-text",
                json!({"contents": "AB", "font": "MadeUpSans.ttf", "height": 5}),
            ),
        ];
        for (name, input) in &calls {
            // Found in the system's folder: nothing to warn of.
            let found: Value = serde_json::from_str(&run(name, input, &settings)).unwrap();
            assert_eq!(found["ok"], true, "{name}: {found}");
            assert!(found.get("warnings").is_none(), "{name}: {found}");

            // Without the system's folders, a look-alike, warned of at the top.
            let mut without = input.clone();
            without["no_system_fonts"] = json!(true);
            let text = run(name, &without, &settings);
            assert!(
                text.starts_with(r#"{"ok":true,"warnings":[{"code":"font_look_alike","#),
                "{name}: {text}"
            );
            let reply: Value = serde_json::from_str(&text).unwrap();
            let (font, sheets) = if *name == "measure-text" {
                ("MadeUpSans.ttf", json!([]))
            } else {
                ("madeupsans.ttf", json!(["label"]))
            };
            assert_eq!(
                reply["warnings"],
                json!([{"code": "font_look_alike", "font": font, "look_alike": HERSHEY, "sheets": sheets}]),
                "{name}"
            );
            // The same as the environment's opt-out gives.
            let off = settings.clone().without_system_fonts();
            assert_eq!(
                serde_json::from_str::<Value>(&run(name, input, &off)).unwrap()["warnings"],
                reply["warnings"],
                "{name}"
            );
        }
        // `no_system_fonts` is true or false.
        let wrong = json!({"contents": "AB", "font": "a.ttf", "height": 5, "no_system_fonts": 1});
        let reply: Value = serde_json::from_str(&run("measure-text", &wrong, &settings)).unwrap();
        assert_eq!(reply["error"]["code"], "usage", "{reply}");
        assert_eq!(reply["error"]["field"], "no_system_fonts", "{reply}");
        let _ = fs::remove_dir_all(&dir);
    }

    fn substitution(font: &str, used: &str) -> FontSubstitution {
        FontSubstitution {
            style: "Notes".into(),
            font: font.into(),
            used: used.into(),
            why: "it is not in the font folders".into(),
            source: None,
        }
    }

    #[test]
    fn one_warning_per_font_names_every_sheet_once() {
        let mut warnings = Warnings::default();
        warnings.sheet(
            "S-1",
            &[substitution("romans.shx", "Hershey Roman Simplex")],
        );
        warnings.sheet(
            "S-2",
            &[
                substitution("ROMANS.SHX", "Hershey Roman Simplex"),
                substitution("arial.ttf", "Hershey Roman Simplex"),
            ],
        );
        warnings.sheet(
            "S-1",
            &[substitution("romans.shx", "Hershey Roman Simplex")],
        );
        let all = serde_json::to_value(warnings.into_vec()).unwrap();
        assert_eq!(
            all,
            serde_json::json!([
                {
                    "code": "font_look_alike",
                    "font": "romans.shx",
                    "look_alike": "Hershey Roman Simplex",
                    "sheets": ["S-1", "S-2"],
                },
                {
                    "code": "font_look_alike",
                    "font": "arial.ttf",
                    "look_alike": "Hershey Roman Simplex",
                    "sheets": ["S-2"],
                },
            ])
        );
    }

    #[test]
    fn a_measurement_of_no_sheet_names_none() {
        let mut warnings = Warnings::default();
        warnings.look_alike("arial.ttf", "LiberationSans-Regular.ttf", None);
        let all = warnings.into_vec();
        assert_eq!(all.len(), 1);
        assert!(all[0].sheets.is_empty());
    }
}
