//! What the drafter is looking at: the sheet open in the viewer, its layout,
//! and the objects selected on it (#156, drafting app build 8).
//!
//! The viewer's page records it with `look-at` as the drafter moves about,
//! and an agent reads it with `looking-at`, through any interface, so it can
//! answer "what is this?" and act on "move this". It is kept in the set's
//! folder, beside the journal, in [`LOOKING_AT_FILE`], since the agent
//! asking runs in another process than the page's server: its own MCP
//! server, the command line or the Python library. It is not one of the
//! set's records: a change set, an undo and an accept never read or write
//! it, and the journal never sees it.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::change::now;
use crate::description::SavedSet;
use crate::error::{Error, ErrorCode};
use crate::reply::Reply;
use crate::set::SET_SETTINGS_FILE;

/// Where a drawing set keeps what the drafter is looking at, under its
/// folder.
pub const LOOKING_AT_FILE: &str = ".agentdrafter/looking-at.json";

/// The most objects a selection holds.
pub const MOST_SELECTED: usize = 1000;

/// The longest an object's ID or a layout's name may be, in characters.
const LONGEST_NAME: usize = 1024;

/// `look-at`: what the drafter is looking at now, as the viewer's page says
/// it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LookAtRequest {
    /// The drawing set's folder.
    pub set: PathBuf,
    /// The sheet open, by its name, in any case.
    pub sheet: String,
    /// The layout of it shown, for a sheet drawn in several.
    #[serde(default)]
    pub layout: Option<String>,
    /// The objects selected on it, by their IDs.
    #[serde(default)]
    pub selected: Vec<String>,
}

/// `looking-at`: the drawing set to ask about.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LookingAtRequest {
    /// The drawing set's folder.
    pub set: PathBuf,
}

/// What the drafter is looking at, as the viewer last recorded it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LookingAtReport {
    /// The set, as the request named it.
    pub set: PathBuf,
    /// The sheet open, by its name as the set gives it; none before the
    /// viewer has recorded one.
    pub sheet: Option<String>,
    /// The layout shown, when the viewer named one.
    pub layout: Option<String>,
    /// The objects selected, by their IDs; empty when nothing is.
    pub selected: Vec<String>,
    /// When it was recorded, in UTC; none before it was.
    pub at: Option<String>,
    /// What it says, in words.
    pub message: String,
}

/// The record in [`LOOKING_AT_FILE`].
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Record {
    sheet: String,
    #[serde(default)]
    layout: Option<String>,
    #[serde(default)]
    selected: Vec<String>,
    at: String,
}

/// Records what the drafter is looking at in the set's folder, replacing
/// what was there. The sheet must be one of the set's; the selection is
/// taken as the page gives it, since the page picks only what it draws.
pub fn look_at(request: &LookAtRequest) -> Reply<LookingAtReport> {
    Reply::from(record(request))
}

/// What the drafter is looking at in the set, as the viewer last recorded
/// it.
pub fn looking_at(request: &LookingAtRequest) -> Reply<LookingAtReport> {
    Reply::from(read(request))
}

fn record(request: &LookAtRequest) -> Result<LookingAtReport, Error> {
    // The input is checked whole before the set is read.
    if request.selected.len() > MOST_SELECTED {
        return Err(Error::usage(format!(
            "a selection holds at most {MOST_SELECTED} objects, not {}",
            request.selected.len()
        ))
        .at_field("selected"));
    }
    let named = |text: &str| !text.trim().is_empty() && text.chars().count() <= LONGEST_NAME;
    if let Some(bad) = request.selected.iter().find(|id| !named(id)) {
        return Err(Error::usage(format!(
            "each object selected is named by its ID, of 1 to {LONGEST_NAME} characters, not \
             `{bad}`"
        ))
        .at_field("selected"));
    }
    if request
        .layout
        .as_deref()
        .is_some_and(|layout| !named(layout))
    {
        return Err(Error::usage(format!(
            "a layout is named by 1 to {LONGEST_NAME} characters"
        ))
        .at_field("layout"));
    }
    let set = SavedSet::read(drawing_set(&request.set)?)?;
    let sheet = set
        .names()
        .into_iter()
        .find(|name| name.eq_ignore_ascii_case(request.sheet.trim()))
        .ok_or_else(|| {
            Error::new(
                ErrorCode::UnknownName,
                format!(
                    "there is no sheet `{}` in the set; the sheets: {}",
                    request.sheet,
                    set.names().join(", ")
                ),
            )
            .at_field("sheet")
        })?
        .to_owned();
    let mut selected: Vec<String> = Vec::with_capacity(request.selected.len());
    for id in &request.selected {
        if !selected.iter().any(|s| s.eq_ignore_ascii_case(id)) {
            selected.push(id.clone());
        }
    }
    let record = Record {
        sheet,
        layout: request.layout.clone(),
        selected,
        at: now(),
    };
    write(&request.set, &record)?;
    Ok(report(&request.set, Some(record)))
}

fn read(request: &LookingAtRequest) -> Result<LookingAtReport, Error> {
    let folder = drawing_set(&request.set)?;
    let path = folder.join(LOOKING_AT_FILE);
    let record = match fs::read_to_string(&path) {
        Ok(text) => Some(serde_json::from_str::<Record>(&text).map_err(|e| {
            Error::new(
                ErrorCode::ReadFailed,
                format!(
                    "what the drafter is looking at, {}, cannot be read: {e}",
                    path.display()
                ),
            )
            .in_file(&path)
        })?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(Error::new(
                ErrorCode::ReadFailed,
                format!("{} cannot be read: {e}", path.display()),
            )
            .in_file(&path))
        }
    };
    Ok(report(&request.set, record))
}

/// The set's folder, refused unless it is a drawing set's.
fn drawing_set(set: &Path) -> Result<&Path, Error> {
    let settings = set.join(SET_SETTINGS_FILE);
    if !settings.is_file() {
        return Err(Error::new(
            ErrorCode::ReadFailed,
            format!(
                "`{}` is not a drawing set's folder: it holds no {SET_SETTINGS_FILE}",
                set.display()
            ),
        )
        .in_file(&settings));
    }
    Ok(set)
}

/// Writes the record beside where it goes and moves it into place, so it
/// is never read in part.
fn write(set: &Path, record: &Record) -> Result<(), Error> {
    let path = set.join(LOOKING_AT_FILE);
    let failed = |e: std::io::Error| {
        Error::new(
            ErrorCode::WriteFailed,
            format!(
                "what the drafter is looking at cannot be written to {}: {e}",
                path.display()
            ),
        )
        .in_file(&path)
    };
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder).map_err(failed)?;
    }
    let mut text = serde_json::to_string_pretty(record).expect("a record serializes to JSON");
    text.push('\n');
    let part = path.with_extension("json.part");
    let written = fs::File::create(&part)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .and_then(|()| fs::rename(&part, &path));
    if let Err(e) = written {
        let _ = fs::remove_file(&part);
        return Err(failed(e));
    }
    Ok(())
}

fn report(set: &Path, record: Option<Record>) -> LookingAtReport {
    let Some(record) = record else {
        return LookingAtReport {
            set: set.to_owned(),
            sheet: None,
            layout: None,
            selected: Vec::new(),
            at: None,
            message: "the viewer has not said what the drafter is looking at: no sheet of the \
                      set has been open in it"
                .to_owned(),
        };
    };
    let on = match &record.layout {
        Some(layout) => format!("sheet {}, layout {layout}", record.sheet),
        None => format!("sheet {}", record.sheet),
    };
    let selected = match record.selected.as_slice() {
        [] => "nothing selected".to_owned(),
        [one] => format!("{one} selected"),
        more => format!("{} objects selected", more.len()),
    };
    LookingAtReport {
        set: set.to_owned(),
        message: format!("the drafter has {on} open, with {selected}"),
        sheet: Some(record.sheet),
        layout: record.layout,
        selected: record.selected,
        at: Some(record.at),
    }
}
