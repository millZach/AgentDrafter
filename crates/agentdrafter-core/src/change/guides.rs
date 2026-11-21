//! Guides: the assistant teaches as steps on the drafter's own drawing
//! (#157, drafting app build 9).
//!
//! A guide is data: a title, the sheet it runs on, and its steps. Each step
//! says what to say, the drawing tool to use, the points to click, each a
//! point or a snapped point as an operation takes one, or an object to
//! pick, and the operation that completes it; or it is a setting step,
//! such as making a layer current, with no operation. A later step may name
//! what an earlier step adds, `{"made_by": 2}`, wherever it names an
//! object: in a point's snap, in an object to pick and in its operation.
//!
//! `guide` checks a guide as it is given, on the set as it is: its sheet,
//! each point, worked out on the sheet as the steps before it leave it, and
//! each operation, applied in memory in order, as a dry run applies one.
//! It records it beside the journal, in [`GUIDES_FOLDER`], one file each,
//! numbered from 1 on their own, as a proposal is. It writes nothing else.
//!
//! The drafting app runs it. Each change set the drafter lands for a step
//! names it under the change set's `guide`, `{"guide": 1, "step": 2}`: its
//! journal entry is marked with the step, and the reply checks the change
//! set against the step's operation (`guide` in `change`'s reply), so the
//! page can say when the drafter did something else, and offer to undo it.
//! The guide never undoes anything itself.
//!
//! `guides` lists the guides with each step worked out on the set as it
//! stands: where each point is, the operation with each `made_by` named by
//! the object the step's change set made, and the change sets in place
//! that were sent for it.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::apply::{self, walk_guide, StepWalked, WalkStep};
use super::journal::{self, fates, in_place, now, HowChanged};
use super::proposals::{newest, numbers, text_given, who};
use super::recover::{refuse_unfinished, Unfinished};
use super::{ChangeReport, ChangeRequest, OperationOutcome};
use crate::description::{read_set_settings, ID_SEPARATOR};
use crate::error::{Error, ErrorCode};
use crate::interface::Interface;
use crate::set::SET_SETTINGS_FILE;
use crate::settings::Settings;

/// The folder, relative to a drawing set's folder, that holds its guides,
/// one file each, `<number>.json`, beside the journal.
pub const GUIDES_FOLDER: &str = ".agentdrafter/guides";

/// The file a process holds locked while it numbers and writes a guide.
const GUIDES_LOCK: &str = "guides.lock";

/// The most steps a guide may have.
pub const MOST_STEPS: usize = 40;

/// The most characters a guide's or a step's title may hold.
pub const LONGEST_TITLE: usize = 120;

/// The most characters a step's words may hold.
pub const LONGEST_SAY: usize = 1000;

/// The most characters a point's label may hold.
pub const LONGEST_LABEL: usize = 80;

/// The drawing tools a step may name, each with the operation the drafting
/// app's tool sends, which completes it.
pub const TOOLS: [(&str, &str); 7] = [
    ("line", "add_line"),
    ("circle", "add_circle"),
    ("rectangle", "add_polyline"),
    ("text", "add_text"),
    ("move", "move"),
    ("copy", "copy"),
    ("erase", "delete"),
];

/// A guide, as an agent gives it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuideGiven {
    /// What it teaches, in a few words.
    pub title: String,
    /// The sheet it runs on, by its name.
    pub sheet: String,
    /// Its steps, in order.
    pub steps: Vec<StepGiven>,
}

/// One step of a guide.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepGiven {
    /// What the step says to the drafter.
    pub say: String,
    /// The step in a few words, for the list of steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The drawing tool it uses, for an operation step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// The points to click, and the objects to pick, in the order the tool
    /// takes them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<GuidePoint>,
    /// The operation that completes it, as a change set holds one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completes: Option<Value>,
    /// What a setting step sets, in place of an operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setting: Option<Setting>,
}

/// A point to click, `at`, or an object to pick, `object`, with the words
/// its ring's tag shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuidePoint {
    /// A point, `[x, y]` or `[x, y, z]`, or a snapped point, `{"snap": {...}}`,
    /// whose object may be `made_by` an earlier step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Value>,
    /// An object to pick, as a target names one, or `{"made_by": N}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<Value>,
    /// What the point is, in the drafter's words, such as `TB1-1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// What a setting step sets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Setting {
    /// The layer to make current.
    pub current_layer: String,
}

/// A guide, as its file holds it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    guide: u64,
    by: String,
    at: String,
    base_entry: u64,
    title: String,
    sheet: String,
    steps: Vec<StepGiven>,
}

/// A request to give a guide.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuideRequest {
    /// The drawing set's folder.
    pub set: PathBuf,
    /// The guide.
    pub guide: Value,
    /// Who gives it, in the caller's words; with none, the interface it
    /// came through.
    #[serde(
        default,
        deserialize_with = "text_given",
        skip_serializing_if = "Option::is_none"
    )]
    pub by: Option<String>,
}

/// A guide recorded.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GuideReport {
    /// The drawing set's folder.
    pub set: String,
    /// The guide's number, which a change set sent for one of its steps
    /// names.
    pub guide: u64,
    pub by: String,
    pub at: String,
    pub title: String,
    /// The sheet it runs on, as the set names it.
    pub sheet: String,
    /// How many steps it has.
    pub steps: usize,
    /// Its file.
    pub file: String,
    /// What was recorded, in words.
    pub message: String,
}

/// A request to list a drawing set's guides.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuidesRequest {
    /// The drawing set's folder.
    pub set: PathBuf,
}

/// A drawing set's guides.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GuidesReport {
    pub set: String,
    /// The folder the guides are kept in.
    pub folder: String,
    pub message: String,
    /// Each guide, the oldest first.
    pub guides: Vec<GuideListed>,
}

/// One guide, worked out on the set as it stands.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GuideListed {
    pub guide: u64,
    pub by: String,
    pub at: String,
    /// The journal's newest entry when it was given.
    pub base_entry: u64,
    pub title: String,
    pub sheet: String,
    pub steps: Vec<StepListed>,
    /// Why its steps cannot be worked out now, such as its sheet gone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One step of a guide, worked out on the set as it stands.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StepListed {
    /// Its number, counted from 1.
    pub step: u64,
    pub say: String,
    /// Its title, or its words when it has none.
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setting: Option<Setting>,
    /// Its operation, as the guide gives it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completes: Option<Value>,
    /// The steps whose objects it names, `made_by`: skipping one skips it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<u64>,
    pub points: Vec<PointListed>,
    /// Its operation with each `made_by` named by the object's ID, ready to
    /// send; none while a step it needs has made nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<Value>,
    /// The change sets in place that were sent for it, the oldest first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub landed: Vec<u64>,
    /// The object the newest of them added, which later steps name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub made: Option<String>,
}

/// One point of a step, worked out on the set as it stands.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PointListed {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Where it is, in the set's declared unit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<[f64; 3]>,
    /// The snap it names: the object, the snap and which.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snap: Option<SnapNamed>,
    /// The object to pick, by ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    /// The step it waits on, while that step has made nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waits_on: Option<u64>,
    /// Why it names nothing now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The snap a point names, worked out.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SnapNamed {
    pub id: String,
    pub snap: crate::geometry::SnapKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub which: Option<String>,
}

/// The step a change set was sent for, as its journal entry records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuideMark {
    pub guide: u64,
    pub step: u64,
    pub of: u64,
}

/// A change set sent for a guide's step, checked against the step's
/// operation: `change`'s reply holds it under `guide`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GuideChecked {
    pub guide: u64,
    pub step: u64,
    pub of: u64,
    /// Whether the change set does what the step's operation does.
    pub completes: bool,
    /// What it did against what the step wants, in words.
    pub message: String,
    /// The object it added, which later steps name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub made: Option<String>,
}

/// The schema of a change set's `guide`.
pub(crate) fn mark_schema() -> Value {
    json!({
        "description": "The guide's step this change set is sent for, as the drafting app \
                        sends it while a guide runs: its journal entry is marked with the step, \
                        and the reply says whether it completes it (`guide`). `of`, the guide's \
                        steps, is filled in when left out.",
        "type": "object",
        "additionalProperties": false,
        "required": ["guide", "step"],
        "properties": {
            "guide": { "description": "The guide, by its number.", "type": "integer", "minimum": 1 },
            "step": { "description": "The step, counted from 1.", "type": "integer", "minimum": 1 },
            "of": { "description": "How many steps the guide has.", "type": "integer", "minimum": 1 },
        },
    })
}

/// Reads a change set's `guide`, `{"guide": 1, "step": 2}`, with `of`
/// when it is given.
pub(super) fn read_mark(value: &Value) -> Result<(u64, u64, Option<u64>), Error> {
    let wrong = |message: &str, field: &str| {
        Error::new(ErrorCode::WrongType, message.to_owned()).at_field(field.to_owned())
    };
    let Value::Object(map) = value else {
        return Err(wrong(
            "expected the guide's step, {\"guide\": 1, \"step\": 2}",
            "guide",
        ));
    };
    if let Some(key) = map
        .keys()
        .find(|k| !matches!(k.as_str(), "guide" | "step" | "of"))
    {
        return Err(Error::new(
            ErrorCode::UnknownKey,
            format!("unknown key `{key}`; a change set's guide takes guide, step and of"),
        )
        .at_field(format!("guide.{key}")));
    }
    let number = |key: &str| -> Result<Option<u64>, Error> {
        match map.get(key) {
            None => Ok(None),
            Some(v) => match v.as_u64() {
                Some(n) if n >= 1 => Ok(Some(n)),
                _ => Err(wrong(
                    "expected a whole number from 1",
                    &format!("guide.{key}"),
                )),
            },
        }
    };
    let missing = |key: &str| {
        Error::new(
            ErrorCode::MissingKey,
            format!("missing key `{key}`; a change set's guide names the guide and its step"),
        )
        .at_field(format!("guide.{key}"))
    };
    let guide = number("guide")?.ok_or_else(|| missing("guide"))?;
    let step = number("step")?.ok_or_else(|| missing("step"))?;
    Ok((guide, step, number("of")?))
}

/// The step a change set, normalised, was sent for: its `guide` with `of`.
pub(super) fn mark_of(change_set: &Value) -> Option<GuideMark> {
    serde_json::from_value(change_set.get("guide")?.clone()).ok()
}

/// `value` with each `{"made_by": N, ...}` in it naming the object step N
/// made by its ID, `{"id": ..., ...}`; or the first step that has made
/// none, as `made` holds them.
pub(super) fn substituted(value: &Value, made: &BTreeMap<u64, String>) -> Result<Value, u64> {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                if k == "made_by" {
                    let n = v.as_u64().unwrap_or(0);
                    let id = made.get(&n).ok_or(n)?;
                    out.insert("id".to_owned(), Value::String(id.clone()));
                } else {
                    out.insert(k.clone(), substituted(v, made)?);
                }
            }
            Ok(Value::Object(out))
        }
        Value::Array(items) => items
            .iter()
            .map(|v| substituted(v, made))
            .collect::<Result<_, _>>()
            .map(Value::Array),
        other => Ok(other.clone()),
    }
}

/// Each `made_by` in `value`, with its place, such as `completes.target`.
fn made_by_in(value: &Value, field: &str, out: &mut Vec<(Value, String)>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let at = format!("{field}.{k}");
                if k == "made_by" {
                    out.push((v.clone(), at));
                } else {
                    made_by_in(v, &at, out);
                }
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                made_by_in(v, &format!("{field}[{i}]"), out);
            }
        }
        _ => {}
    }
}

/// The steps a step names under `made_by`, each once, in order.
fn needs_of(step: &StepGiven) -> Vec<u64> {
    let mut found = Vec::new();
    for point in &step.points {
        for v in point.at.iter().chain(point.object.iter()) {
            made_by_in(v, "", &mut found);
        }
    }
    if let Some(op) = &step.completes {
        made_by_in(op, "", &mut found);
    }
    let mut needs: Vec<u64> = found.iter().filter_map(|(v, _)| v.as_u64()).collect();
    needs.sort_unstable();
    needs.dedup();
    needs
}

/// Reads and checks a guide's shape: everything that can be checked
/// without the set.
fn read_guide(value: &Value) -> Result<GuideGiven, Error> {
    let guide: GuideGiven = serde_json::from_value(value.clone()).map_err(|e| {
        Error::new(
            ErrorCode::WrongType,
            format!(
                "expected a guide, {{\"title\", \"sheet\", \"steps\": [{{\"say\", \"tool\", \
                 \"points\", \"completes\"}} or {{\"say\", \"setting\"}}]}}: {e}"
            ),
        )
        .at_field("guide")
    })?;
    let long = |text: &str, most: usize, field: String| -> Result<(), Error> {
        if text.trim().is_empty() || text.chars().count() > most {
            return Err(Error::new(
                ErrorCode::BadValue,
                format!("expected words, up to {most} characters"),
            )
            .at_field(field));
        }
        Ok(())
    };
    long(&guide.title, LONGEST_TITLE, "guide.title".to_owned())?;
    if guide.steps.is_empty() || guide.steps.len() > MOST_STEPS {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!("a guide has from 1 to {MOST_STEPS} steps"),
        )
        .at_field("guide.steps"));
    }
    for (s, step) in guide.steps.iter().enumerate() {
        let here = format!("guide.steps[{s}]");
        long(&step.say, LONGEST_SAY, format!("{here}.say"))?;
        if let Some(title) = &step.title {
            long(title, LONGEST_TITLE, format!("{here}.title"))?;
        }
        match (&step.completes, &step.setting) {
            (Some(_), Some(_)) | (None, None) => {
                return Err(Error::new(
                    ErrorCode::Usage,
                    "a step has either `completes`, the operation that completes it, or \
                     `setting`, what a setting step sets",
                )
                .at_field(here));
            }
            (None, Some(setting)) => {
                if step.tool.is_some() || !step.points.is_empty() {
                    return Err(Error::new(
                        ErrorCode::Usage,
                        "a setting step names no tool and no points: the drafter sets it in \
                         the ribbon",
                    )
                    .at_field(here));
                }
                long(
                    &setting.current_layer,
                    LONGEST_TITLE,
                    format!("{here}.setting.current_layer"),
                )?;
            }
            (Some(op), None) => {
                let Some(tool) = &step.tool else {
                    return Err(Error::new(
                        ErrorCode::MissingKey,
                        format!(
                            "missing key `tool`; a step with an operation names the drawing \
                             tool the drafter does it with: {}",
                            tool_names()
                        ),
                    )
                    .at_field(format!("{here}.tool")));
                };
                if !TOOLS.iter().any(|(t, _)| t.eq_ignore_ascii_case(tool)) {
                    return Err(Error::new(
                        ErrorCode::UnknownName,
                        format!("unknown tool `{tool}`; the tools: {}", tool_names()),
                    )
                    .at_field(format!("{here}.tool")));
                }
                if !op.is_object() {
                    return Err(Error::new(
                        ErrorCode::WrongType,
                        "expected an operation, as a change set holds one",
                    )
                    .at_field(format!("{here}.completes")));
                }
            }
        }
        for (p, point) in step.points.iter().enumerate() {
            let at = format!("{here}.points[{p}]");
            if point.at.is_some() == point.object.is_some() {
                return Err(Error::new(
                    ErrorCode::Usage,
                    "a point has either `at`, a point to click, or `object`, an object to pick",
                )
                .at_field(at));
            }
            if let Some(label) = &point.label {
                long(label, LONGEST_LABEL, format!("{at}.label"))?;
            }
        }
        let mut found = Vec::new();
        for (p, point) in step.points.iter().enumerate() {
            for v in point.at.iter().chain(point.object.iter()) {
                made_by_in(v, &format!("{here}.points[{p}]"), &mut found);
            }
        }
        if let Some(op) = &step.completes {
            made_by_in(op, &format!("{here}.completes"), &mut found);
        }
        for (v, field) in found {
            let earlier = v.as_u64().filter(|&n| n >= 1 && n <= s as u64);
            let Some(n) = earlier else {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "`made_by` names an earlier step, by its number, from 1 to {s}: what \
                         that step's operation adds"
                    ),
                )
                .at_field(field));
            };
            let source = &guide.steps[n as usize - 1];
            if source.completes.is_none() {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!("`made_by` names step {n}, a setting step, which adds nothing"),
                )
                .at_field(field));
            }
        }
    }
    Ok(guide)
}

fn tool_names() -> String {
    TOOLS.iter().map(|(t, _)| *t).collect::<Vec<_>>().join(", ")
}

/// The steps of `guide` as the walk takes them.
fn walk_steps(guide: &[StepGiven]) -> Vec<WalkStep<'_>> {
    guide
        .iter()
        .map(|step| WalkStep {
            points: &step.points,
            operation: step.completes.as_ref(),
            layer: step.setting.as_ref().map(|s| s.current_layer.as_str()),
        })
        .collect()
}

/// Records a guide for the drawing set in `request.set`, once it is
/// checked on the set as it is: its sheet, each point and each operation,
/// in order. A guide the set would refuse is not recorded.
pub fn guide_with(
    request: &GuideRequest,
    settings: &Settings,
    via: Interface,
) -> Result<GuideReport, Error> {
    let by = who(request.by.as_deref(), via)?;
    let folder = &request.set;
    read_set_settings(&folder.join(SET_SETTINGS_FILE))?;
    refuse_unfinished(folder, Unfinished::Proposals)?;
    let given = read_guide(&request.guide)?;
    let (sheet, walked) = walk_guide(
        folder,
        &given.sheet,
        &walk_steps(&given.steps),
        None,
        settings,
    )?;
    // Each step's operation is the one its tool sends, and a step names
    // only what an earlier step's operation adds.
    for (s, step) in given.steps.iter().enumerate() {
        if let (Some(tool), Some(_)) = (&step.tool, &step.completes) {
            let wants = TOOLS
                .iter()
                .find(|(t, _)| t.eq_ignore_ascii_case(tool))
                .map(|(_, op)| *op)
                .expect("the tool was checked");
            let name = walked[s].op_name;
            if name != Some(wants) {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "step {}'s tool is {tool}, which lands `{wants}`, but its operation is \
                         `{}`; a step is completed by the operation its tool sends",
                        s + 1,
                        name.unwrap_or_default()
                    ),
                )
                .at_field(format!("guide.steps[{s}].completes.op")));
            }
        }
        for n in needs_of(step) {
            if walked[n as usize - 1].made.is_none() {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "step {} names what step {n} makes, `made_by: {n}`, but step {n}'s \
                         operation adds no object",
                        s + 1
                    ),
                )
                .at_field(format!("guide.steps[{s}]")));
            }
        }
    }
    let base_entry = newest(&journal::read_entries(folder)?);
    let store = folder.join(GUIDES_FOLDER);
    let failed = |what: &str, path: &Path, e: std::io::Error| {
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not {what}: {e}; the guide was not recorded"),
        )
        .in_file(path)
    };
    fs::create_dir_all(&store).map_err(|e| failed("make the guides' folder", &store, e))?;
    let lock_path = store.join(GUIDES_LOCK);
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| failed("open the guides' lock", &lock_path, e))?;
    lock.lock()
        .map_err(|e| failed("take the guides' lock", &lock_path, e))?;
    let number = numbers(&store)?.last().copied().unwrap_or(0) + 1;
    let stored = Stored {
        guide: number,
        by: by.clone(),
        at: now(),
        base_entry,
        title: given.title.trim().to_owned(),
        sheet: sheet.clone(),
        steps: given.steps,
    };
    let path = store.join(format!("{number}.json"));
    let mut text = serde_json::to_string_pretty(&stored).expect("a guide serializes to JSON");
    text.push('\n');
    let part = store.join(format!("{number}.json.part"));
    let written = fs::File::create(&part)
        .and_then(|mut file| {
            file.write_all(text.as_bytes())?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&part, &path));
    if let Err(e) = written {
        let _ = fs::remove_file(&part);
        return Err(failed("write the guide", &path, e));
    }
    drop(lock);
    let steps = stored.steps.len();
    Ok(GuideReport {
        set: folder.display().to_string(),
        guide: number,
        message: format!(
            "guide {number} recorded, by {by}: \"{}\", {steps} step{} on sheet `{sheet}`. \
             Nothing has changed: the drafter starts it in the drafting app, and each step \
             lands as their own change set",
            stored.title,
            if steps == 1 { "" } else { "s" }
        ),
        by,
        at: stored.at,
        title: stored.title,
        sheet,
        steps,
        file: path.display().to_string(),
    })
}

/// Guide `number` of the set in `folder`.
fn read(folder: &Path, number: u64) -> Result<Stored, Error> {
    let path = folder.join(GUIDES_FOLDER).join(format!("{number}.json"));
    let text = fs::read_to_string(&path).map_err(|e| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("could not read guide {number}: {e}"),
        )
        .in_file(&path)
    })?;
    serde_json::from_str(&text).map_err(|e| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("guide {number} cannot be read: {e}"),
        )
        .in_file(&path)
    })
}

/// For each step of guide `number`, the change sets in place sent for it,
/// the oldest first, and the object the newest added on sheet `sheet`.
fn landed_for(
    entries: &[journal::Entry],
    number: u64,
    sheet: &str,
) -> BTreeMap<u64, (Vec<u64>, Option<String>)> {
    let fates = fates(entries);
    let mut out: BTreeMap<u64, (Vec<u64>, Option<String>)> = BTreeMap::new();
    let prefix = format!("{sheet}{ID_SEPARATOR}").to_lowercase();
    for entry in in_place(entries, &fates) {
        let Some(mark) = entry.guide.filter(|m| m.guide == number) else {
            continue;
        };
        let added = entry
            .objects
            .iter()
            .find(|o| o.how == HowChanged::Added && o.id.to_lowercase().starts_with(&prefix))
            .map(|o| o.id.clone());
        let slot = out.entry(mark.step).or_default();
        slot.0.push(entry.entry);
        if added.is_some() {
            slot.1 = added;
        }
    }
    out
}

/// Lists the drawing set's guides, the oldest first, each step worked out
/// on the set as it stands.
pub fn guides_with(request: &GuidesRequest, settings: &Settings) -> Result<GuidesReport, Error> {
    let folder = &request.set;
    read_set_settings(&folder.join(SET_SETTINGS_FILE))?;
    refuse_unfinished(folder, Unfinished::Proposals)?;
    let entries = journal::read_entries(folder)?;
    let store = folder.join(GUIDES_FOLDER);
    let mut listed = Vec::new();
    for number in numbers(&store)? {
        listed.push(listed_guide(
            folder,
            &entries,
            read(folder, number)?,
            settings,
        ));
    }
    let message = match listed.len() {
        0 => "no guide has been given".to_owned(),
        1 => "1 guide has been given".to_owned(),
        n => format!("{n} guides have been given"),
    };
    Ok(GuidesReport {
        set: folder.display().to_string(),
        folder: store.display().to_string(),
        message,
        guides: listed,
    })
}

fn listed_guide(
    folder: &Path,
    entries: &[journal::Entry],
    stored: Stored,
    settings: &Settings,
) -> GuideListed {
    let landed = landed_for(entries, stored.guide, &stored.sheet);
    let made: BTreeMap<u64, String> = landed
        .iter()
        .filter_map(|(&k, (_, id))| id.clone().map(|id| (k, id)))
        .collect();
    let walked = walk_guide(
        folder,
        &stored.sheet,
        &walk_steps(&stored.steps),
        Some(&made),
        settings,
    );
    let (walked, error) = match walked {
        Ok((_, walked)) => (walked, None),
        Err(e) => (
            vec![StepWalked::default(); stored.steps.len()],
            Some(e.message),
        ),
    };
    let steps = stored
        .steps
        .iter()
        .zip(walked)
        .enumerate()
        .map(|(s, (step, walked))| {
            let number = s as u64 + 1;
            let (landed, made) = landed.get(&number).cloned().unwrap_or_default();
            let points = step
                .points
                .iter()
                .enumerate()
                .map(|(p, given)| {
                    let w = walked.points.get(p).cloned().unwrap_or_default();
                    PointListed {
                        label: given.label.clone(),
                        at: w.at,
                        snap: w.snapped.map(|s| SnapNamed {
                            id: s.id,
                            snap: s.snap,
                            which: s.which,
                        }),
                        object: w.object,
                        waits_on: w.waits_on,
                        error: w.error,
                    }
                })
                .collect();
            StepListed {
                step: number,
                say: step.say.clone(),
                title: step.title.clone().unwrap_or_else(|| step.say.clone()),
                tool: step.tool.as_ref().map(|t| t.to_lowercase()),
                setting: step.setting.clone(),
                completes: step.completes.clone(),
                needs: needs_of(step),
                points,
                operation: walked.operation,
                landed,
                made,
            }
        })
        .collect();
    GuideListed {
        guide: stored.guide,
        by: stored.by,
        at: stored.at,
        base_entry: stored.base_entry,
        title: stored.title,
        sheet: stored.sheet,
        steps,
        error,
    }
}

/// A change set sent for a guide's step, ready to apply: the request with
/// its `guide` given `of`, and what the step's own operation would do on
/// the set as it is, to check the change set against.
pub(super) struct Guided {
    pub request: ChangeRequest,
    pub mark: GuideMark,
    /// The step's operation, each `made_by` named, as the dry run sent it.
    expected_op: Option<Value>,
    expected: Result<ChangeReport, Error>,
}

/// When `request`'s change set names a guide's step, its `guide`, checks
/// the step is one of the guide's and works out what the step's operation
/// would do on the set as it is now, as a dry run.
pub(super) fn prepare(
    request: &ChangeRequest,
    settings: &Settings,
) -> Result<Option<Guided>, Error> {
    let Some(mark) = request.change_set.get("guide") else {
        return Ok(None);
    };
    let (number, step, of) = read_mark(mark)?;
    let folder = &request.set;
    let store = folder.join(GUIDES_FOLDER);
    if !numbers(&store)?.contains(&number) {
        return Err(Error::new(
            ErrorCode::UnknownName,
            format!("the set has no guide {number}"),
        )
        .at_field("guide.guide"));
    }
    let stored = read(folder, number)?;
    let steps = stored.steps.len() as u64;
    if step > steps {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!("guide {number} has {steps} steps, so no step {step}"),
        )
        .at_field("guide.step"));
    }
    if of.is_some_and(|of| of != steps) {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!("guide {number} has {steps} steps, not {}", of.unwrap_or(0)),
        )
        .at_field("guide.of"));
    }
    if stored.steps[step as usize - 1].completes.is_none() {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "step {step} of guide {number} is a setting step, which the drafting app sets; \
                 no change set completes it"
            ),
        )
        .at_field("guide.step"));
    }
    let mark = GuideMark {
        guide: number,
        step,
        of: steps,
    };
    let mut change_set = request.change_set.clone();
    change_set["guide"] = serde_json::to_value(mark).expect("a mark serializes");
    let entries = journal::read_entries(folder)?;
    let listed = listed_guide(folder, &entries, stored, settings);
    let wanted = &listed.steps[step as usize - 1];
    let expected = match &wanted.operation {
        Some(op) => apply::apply(
            &ChangeRequest {
                set: folder.clone(),
                change_set: json!({ "operations": [op] }),
                dry_run: true,
            },
            settings,
            &mut None,
            None,
            None,
            false,
        ),
        None => Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "it names what step {} makes, and no change set in place made it",
                wanted.needs.first().copied().unwrap_or(0)
            ),
        )),
    };
    Ok(Some(Guided {
        request: ChangeRequest {
            change_set,
            ..request.clone()
        },
        mark,
        expected_op: wanted.operation.clone(),
        expected,
    }))
}

/// How far two coordinates may be apart and still be one point: a
/// millionth of the larger, or of 1.
fn same(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0)
}

/// A number or a point in words, as a record writes it.
fn shown(value: &Value) -> String {
    match value {
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(shown).collect::<Vec<_>>().join(", ")
        ),
        Value::Number(n) => n
            .as_f64()
            .map(|f| {
                let r = (f * 1e6).round() / 1e6 + 0.0;
                format!("{r}")
            })
            .unwrap_or_else(|| n.to_string()),
        Value::String(s) => format!("`{s}`"),
        other => other.to_string(),
    }
}

/// The operation `sent` with each snapped point the reply `outcome` says it
/// took written as its coordinates.
fn resolved(sent: &Value, outcome: &OperationOutcome) -> Value {
    let mut out = sent.clone();
    for snapped in &outcome.snapped {
        let at = json!([snapped.at.x, snapped.at.y, snapped.at.z]);
        if let Some(slot) = place(&mut out, &snapped.field) {
            *slot = at;
        }
    }
    out
}

/// The value at `field`, such as `points[0].point`, in `value`.
fn place<'a>(value: &'a mut Value, field: &str) -> Option<&'a mut Value> {
    let mut here = value;
    for part in field.split('.') {
        let (key, rest) = match part.find('[') {
            Some(i) => (&part[..i], &part[i..]),
            None => (part, ""),
        };
        if !key.is_empty() {
            here = here.get_mut(key)?;
        }
        for index in rest.split(['[', ']']).filter(|s| !s.is_empty()) {
            here = here.get_mut(index.parse::<usize>().ok()?)?;
        }
    }
    Some(here)
}

/// Whether `got` is `want`: numbers and points within [`same`], a point of
/// two coordinates as one with z 0, words in any case, and of a mapping
/// only the keys `want` gives.
fn matches(want: &Value, got: &Value) -> bool {
    match (want, got) {
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => same(a, b),
            _ => false,
        },
        (Value::Array(a), Value::Array(b)) => {
            let numbers = |v: &[Value]| v.iter().all(Value::is_number);
            if numbers(a) && numbers(b) && (2..=3).contains(&a.len()) && (2..=3).contains(&b.len())
            {
                (0..3).all(|i| {
                    let c = |v: &[Value]| v.get(i).and_then(Value::as_f64).unwrap_or(0.0);
                    same(c(a), c(b))
                })
            } else {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| matches(x, y))
            }
        }
        (Value::String(a), Value::String(b)) => a.trim().eq_ignore_ascii_case(b.trim()),
        (Value::Object(a), Value::Object(b)) => a
            .iter()
            .all(|(k, v)| b.get(k).is_some_and(|w| matches(v, w))),
        (a, b) => a == b,
    }
}

/// `op`, resolved, as the check compares it: a move given from and to
/// written as how far it goes, and a closed polyline's points in order.
fn normal(op: &Value, name: &str) -> Value {
    let mut op = op.clone();
    if name == "move" || name == "copy" {
        if let (Some(Value::Array(from)), Some(Value::Array(to))) = (op.get("from"), op.get("to")) {
            let c = |v: &Vec<Value>, i: usize| v.get(i).and_then(Value::as_f64).unwrap_or(0.0);
            let by = json!([
                c(to, 0) - c(from, 0),
                c(to, 1) - c(from, 1),
                c(to, 2) - c(from, 2)
            ]);
            if let Value::Object(map) = &mut op {
                map.remove("from");
                map.remove("to");
                map.insert("by".to_owned(), by);
            }
        }
    }
    if name == "add_polyline" {
        if let Some(Value::Array(points)) = op.get_mut("points") {
            let key = |v: &Value| -> Vec<f64> {
                let p = v.get("point").unwrap_or(v);
                (0..3)
                    .map(|i| p.get(i).and_then(Value::as_f64).unwrap_or(0.0))
                    .collect()
            };
            points.sort_by(|a, b| {
                key(a)
                    .partial_cmp(&key(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
    }
    op
}

/// The keys the check passes over: what names the operation and where it
/// lands, which the targets' IDs and the sheets reached compare instead.
const PASSED_OVER: [&str; 5] = ["op", "sheet", "target", "targets", "id"];

/// The change set `request` sent for a guide's step, checked against the
/// step's own operation, as `report` says it landed.
pub(super) fn checked(guided: &Guided, report: &ChangeReport) -> GuideChecked {
    let mark = guided.mark;
    let made = report
        .operations
        .iter()
        .find(|o| o.ok && o.op.starts_with("add_"))
        .and_then(|o| o.id.clone());
    let verdict = |completes: bool, message: String| GuideChecked {
        guide: mark.guide,
        step: mark.step,
        of: mark.of,
        completes,
        message,
        made: made.clone(),
    };
    let expected = match &guided.expected {
        Ok(expected) => expected,
        Err(e) => {
            return verdict(
                false,
                format!(
                    "step {} of {} cannot be checked now: {}",
                    mark.step, mark.of, e.message
                ),
            )
        }
    };
    if report.landed == 0 {
        return verdict(
            false,
            format!(
                "That did not land, so step {} of {} waits.",
                mark.step, mark.of
            ),
        );
    }
    let (Some(want), Some(sent_want)) = (expected.operations.first(), guided.expected_op.clone())
    else {
        return verdict(
            false,
            "The step's own operation would not land now.".to_owned(),
        );
    };
    let ops = &report.operations;
    if ops.len() != 1 {
        return verdict(
            false,
            format!(
                "That was {} operations; the step wants one, `{}`.",
                ops.len(),
                want.op
            ),
        );
    }
    let got = &ops[0];
    if got.op != want.op {
        return verdict(
            false,
            format!("That was `{}`; the step wants `{}`.", got.op, want.op),
        );
    }
    if !want.op.starts_with("add_") {
        if let (Some(w), Some(g)) = (&want.id, &got.id) {
            if !w.eq_ignore_ascii_case(g) {
                return verdict(false, format!("That went to `{g}`; the step wants `{w}`."));
            }
        }
    }
    let sent_got = guided
        .request
        .change_set
        .get("operations")
        .and_then(|o| o.get(0))
        .cloned()
        .unwrap_or(Value::Null);
    let want_op = normal(&resolved(&sent_want, want), &want.op);
    let got_op = normal(&resolved(&sent_got, got), &got.op);
    let differs = |got_op: &Value| -> Option<String> {
        let want_map = want_op.as_object()?;
        want_map
            .iter()
            .filter(|(k, _)| !PASSED_OVER.contains(&k.as_str()))
            .find(|(k, v)| !got_op.get(k.as_str()).is_some_and(|g| matches(v, g)))
            .map(|(k, _)| k.clone())
    };
    let mut first = differs(&got_op);
    if first.is_some() && want.op == "add_line" {
        // A line drawn the other way round is the same line.
        let mut swapped = got_op.clone();
        if let Value::Object(map) = &mut swapped {
            let from = map.remove("from");
            let to = map.remove("to");
            if let (Some(from), Some(to)) = (from, to) {
                map.insert("from".to_owned(), to);
                map.insert("to".to_owned(), from);
            }
        }
        if differs(&swapped).is_none() {
            first = None;
        }
    }
    match first {
        None => verdict(
            true,
            format!("That completes step {} of {}.", mark.step, mark.of),
        ),
        Some(key) => {
            let at = |outcome: &OperationOutcome, op: &Value| -> String {
                let value = op
                    .get(&key)
                    .map(shown)
                    .unwrap_or_else(|| "not given".to_owned());
                match outcome.snapped.iter().find(|s| s.field == key) {
                    Some(s) => format!("the {} of `{}`, {value}", snap_words(s.snap), s.id),
                    None => value,
                }
            };
            // A point goes somewhere; anything else is something.
            let went = if want_op.get(&key).is_some_and(Value::is_array) {
                "went to"
            } else {
                "is"
            };
            verdict(
                false,
                format!(
                    "Its `{key}` {went} {}; the step wants {}.",
                    at(got, &got_op),
                    at(want, &want_op)
                ),
            )
        }
    }
}

/// A snap in words, such as `midpoint`.
fn snap_words(kind: crate::geometry::SnapKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.replace('_', " ")))
        .unwrap_or_default()
}
