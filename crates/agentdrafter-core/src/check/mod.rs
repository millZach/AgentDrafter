//! The checker: re-reads a drawing set's saved files, or one sheet
//! description, and reports each check a sheet fails.
//!
//! It is separate from whatever made a change (`docs/plan.md`): it reads
//! the records on disk, as a build reads them, and never another module's
//! records in memory, so whatever made a change never marks its own work. A
//! change set's reply carries its report on the sheets the change set
//! landed on, read back once they were written.
//!
//! The default checks are the ones a reviewer would otherwise catch by eye
//! ([`Rule`]): the exact paper size, nothing off the sheet, no text
//! overflowing its cell, and no text sitting on other text. Each sheet is
//! drawn through the one drawing pipeline (ADR-0011), as `query extents`
//! draws it, so text is measured as its font draws it. A failed check is
//! reported, not refused: it does not stop a change set from landing, and
//! the reviewer can accept a set after a warning that lists them.

mod ink;
mod rules;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::description::{read_sheet_record, SavedSet, SheetRecord};
use crate::error::{Error, ErrorCode};
use crate::model::Paper;
use crate::query::{Extents, WrittenAt};
use crate::settings::Settings;
use crate::text::fonts::FontSubstitution;
use crate::text::placement::SheetFonts;
use crate::units::Unit;

/// How far past a limit counts, in millimetres on paper: a sheet is drawn
/// at its paper's size, so a length on it is a length on paper. Within it,
/// a sheet is its paper's size, an object is on the paper, a text is in its
/// cell and two texts do not sit on each other.
pub const CHECK_TOLERANCE_MM: f64 = 0.01;

/// A request to check a drawing set, or one sheet description.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckRequest {
    /// A drawing set's folder, which holds its settings, or one sheet
    /// description.
    pub source: PathBuf,
    /// Only the sheets of these names, in any case. Without them, every
    /// sheet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sheets: Vec<String>,
}

/// A rule a sheet is checked against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    /// The sheet's extents are its declared paper's size, within the
    /// tolerance. A sheet that declares no paper fails it once, saying so.
    PaperSize,
    /// Each object draws on the paper, whose lower left corner is at the
    /// origin, within the tolerance.
    OffSheet,
    /// Each text and attribute of an insert whose block declares its cell
    /// draws within the cell, placed by the insert.
    TextOverflow,
    /// No two texts on a sheet, the text inserts draw among them, have
    /// drawn boxes that overlap by more than the tolerance.
    TextOnText,
    /// No line touches a text's letters as they print, by more than the
    /// tolerance, but the lines of the text's own insert.
    TextOnLine,
}

impl Rule {
    /// The rule's name as the reply writes it, such as `paper_size`.
    pub const fn name(self) -> &'static str {
        match self {
            Rule::PaperSize => "paper_size",
            Rule::OffSheet => "off_sheet",
            Rule::TextOverflow => "text_overflow",
            Rule::TextOnText => "text_on_text",
            Rule::TextOnLine => "text_on_line",
        }
    }
}

/// What the checker found on the sheets it checked.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CheckReport {
    /// The drawing set's folder, or the sheet description, checked.
    pub source: String,
    /// The declared unit every length in the report is in.
    pub units: Unit,
    /// The sheets checked, by name, in the set's order.
    pub sheets: Vec<String>,
    /// What was found, in words.
    pub message: String,
    /// Whether every check passed on every sheet checked.
    pub passed: bool,
    /// How far past a limit counts, in the declared unit:
    /// [`CHECK_TOLERANCE_MM`] on paper.
    pub tolerance: f64,
    /// How many checks failed; 0 when every check passed.
    pub count: usize,
    /// Each failed check, sheet by sheet in the set's order.
    pub findings: Vec<Finding>,
    /// The text styles whose fonts are not in the font folders, and the
    /// look-alike each was measured with: the text checks rest on it.
    pub font_substitutions: Vec<FontSubstitution>,
    /// The drawing set's inconsistent mark, when a change set landed on
    /// some of its sheets and failed on others: the checker reads the
    /// records as they are.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inconsistent: Option<crate::change::Inconsistent>,
}

/// One failed check: the sheet, the rule, the object and what is wrong, in
/// numbers in the declared unit.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Finding {
    pub sheet: String,
    pub rule: Rule,
    /// What is wrong, in words and numbers.
    pub message: String,
    /// The object that fails the check. None for a check of the whole
    /// sheet: `paper_size`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<CheckedObject>,
    /// For `text_on_text`, the text the object sits on: the one drawn
    /// before it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<CheckedObject>,
    /// The box round what is drawn: the sheet's extents for `paper_size`,
    /// the object's for `off_sheet`, its hatches' boundaries among it, the
    /// text's in its block's own coordinates for `text_overflow`, and for
    /// `text_on_text` the line's that sits deepest on the other text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drawn: Option<Extents>,
    /// For `text_on_text`, the box round what the other text's line draws.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_drawn: Option<Extents>,
    /// The paper declared, for `paper_size` and `off_sheet`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paper: Option<Paper>,
    /// For `text_overflow`, the cell, in its block's own coordinates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell: Option<Extents>,
    /// For `paper_size`, how much wider and taller the sheet draws than its
    /// paper, `[width, height]`: less than 0 where it is narrower or
    /// shorter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub off_by: Option<[f64; 2]>,
    /// For `off_sheet` and `text_overflow`, how far what is drawn reaches
    /// past each side of the paper or the cell; 0 on a side it keeps
    /// within. A cell's are in its block's own coordinates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub past: Option<Sides>,
    /// For `text_on_text`, how far the two drawn boxes overlap: how far one
    /// would move to clear the other.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlap: Option<f64>,
    /// For `text_on_line`, a point where the line touches the text's
    /// letters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<[f64; 2]>,
}

/// How far something reaches past each side of a box.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Sides {
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
    pub top: f64,
}

/// The object a check names: one of the sheet's objects, by its ID and
/// its name, and where it is written; for text an insert draws, which of
/// its block's objects.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CheckedObject {
    /// The object's ID, such as `DEMO-001/heading`.
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The object's type, such as `insert`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub written_at: WrittenAt,
    /// When the check is about text an insert draws: which of its block's
    /// objects draws it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_block: Option<BlockPart>,
    /// What the text says, when the check is about text: a text's or an
    /// mtext's contents, or an attribute's value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// One object of an insert's block, as its block library defines it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BlockPart {
    pub block: String,
    /// The block library that defines the block.
    pub file: String,
    /// The object's place in the block definition, such as
    /// `blocks.TITLE_BLOCK.objects[13]`.
    pub field: String,
    /// The object's type: `attdef` for an attribute, `text` or `mtext`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The attribute's tag, for an attribute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

/// Checks a drawing set or a sheet description, reading fonts from the
/// font folders the environment names ([`Settings::from_env`]). See
/// [`check_with`].
pub fn check(request: &CheckRequest) -> Result<CheckReport, Error> {
    check_with(request, &Settings::from_env())
}

/// Checks a drawing set, or one sheet description, from its saved files,
/// reading fonts from the font folders `settings` names to measure text.
///
/// A sheet that cannot be read stops the check with its error, placed in
/// its file: a report that left a sheet out could say it passed.
pub fn check_with(request: &CheckRequest, settings: &Settings) -> Result<CheckReport, Error> {
    let source = request.source.as_path();
    let (units, records) = load(source, &request.sheets)?;
    let tolerance = Unit::Mm.convert(CHECK_TOLERANCE_MM, units);
    let mut findings = Vec::new();
    let mut substitutions: Vec<FontSubstitution> = Vec::new();
    for record in &records {
        let fonts = SheetFonts::find(&record.sheet, settings);
        for substitution in fonts.substitutions() {
            if !substitutions.contains(&substitution) {
                substitutions.push(substitution);
            }
        }
        findings.extend(rules::check_sheet(record, &fonts, units, tolerance));
    }
    let sheets: Vec<String> = records.iter().map(|r| r.name.clone()).collect();
    Ok(CheckReport {
        source: source.display().to_string(),
        units,
        message: message(&sheets, &findings),
        sheets,
        passed: findings.is_empty(),
        tolerance,
        count: findings.len(),
        findings,
        font_substitutions: substitutions,
        inconsistent: if source.is_dir() {
            crate::change::inconsistent(source)
        } else {
            None
        },
    })
}

/// Reads the sheets to check from their saved files, with the declared
/// unit: every sheet of the drawing set whose folder is `source`, or the
/// one sheet description `source` is; or only those `wanted` names, in any
/// case. A sheet that cannot be read stops the check.
fn load(source: &Path, wanted: &[String]) -> Result<(Unit, Vec<SheetRecord>), Error> {
    let named = |name: &str| wanted.is_empty() || wanted.iter().any(|w| same_name(w, name));
    let missing = |names: &[&str]| {
        wanted
            .iter()
            .find(|w| !names.iter().any(|name| same_name(w, name)))
            .map(|w| no_such_sheet(w, names))
    };
    if !source.is_dir() {
        let record = read_sheet_record(source)?;
        if let Some(error) = missing(&[record.name.as_str()]) {
            return Err(error);
        }
        return Ok((record.sheet.units, vec![record]));
    }
    let set = SavedSet::read(source)?;
    if let Some(error) = missing(&set.names()) {
        return Err(error);
    }
    let records = set
        .rows
        .iter()
        .filter(|row| named(&row.sheet))
        .map(|row| {
            set.sheet(row).map_err(|e| Error {
                message: format!(
                    "sheet `{}` cannot be read, and the checker checks every sheet it is asked \
                     to: {}",
                    row.sheet, e.message
                ),
                ..e
            })
        })
        .collect::<Result<_, _>>()?;
    Ok((set.units, records))
}

/// Whether two sheet names are one: names that differ only in case are.
fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// The error for a sheet to check that is not among `names`, the sheets
/// there are.
fn no_such_sheet(sheet: &str, names: &[&str]) -> Error {
    Error::new(
        ErrorCode::UnknownName,
        format!(
            "there is no sheet `{sheet}` to check; the sheets: {}",
            names.join(", ")
        ),
    )
    .at_field("sheets")
}

/// What the checker found, in words.
fn message(sheets: &[String], findings: &[Finding]) -> String {
    let checked = match sheets {
        [one] => format!("sheet `{one}`"),
        all => format!("the {} sheets checked", all.len()),
    };
    if findings.is_empty() {
        return format!("every check passes on {checked}");
    }
    // Each sheet with a failed check, and the rules it failed, in order.
    let mut failed: Vec<(&str, Vec<&str>)> = Vec::new();
    for finding in findings {
        let rule = finding.rule.name();
        match failed.iter_mut().find(|(sheet, _)| *sheet == finding.sheet) {
            Some((_, rules)) if rules.contains(&rule) => {}
            Some((_, rules)) => rules.push(rule),
            None => failed.push((&finding.sheet, vec![rule])),
        }
    }
    let count = if findings.len() == 1 {
        "1 check failed".to_owned()
    } else {
        format!("{} checks failed", findings.len())
    };
    if let ([_], [(_, rules)]) = (sheets, failed.as_slice()) {
        return format!("{count} on {checked}: {}", rules.join(", "));
    }
    let on = if failed.len() == sheets.len() {
        format!("each of {checked}")
    } else {
        format!("{} of {checked}", failed.len())
    };
    let listed: Vec<String> = failed
        .iter()
        .map(|(sheet, rules)| format!("`{sheet}` ({})", rules.join(", ")))
        .collect();
    format!("{count} on {on}: {}", listed.join(", "))
}
