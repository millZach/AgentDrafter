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
use super::{ChangeReport, ChangeRequest, OperationOutcome, SnapResolved};
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
    /// The journal's newest finished entry, which the steps are worked out
    /// as of.
    pub newest_entry: u64,
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
    /// The newest of them that completes it, as the core checked it: the
    /// step is done while it stays in place.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_by: Option<u64>,
    /// The object that change set added, or the one copy it made, which
    /// later steps name.
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
    /// Whether the change set completes the step, as the core checked it
    /// before it landed: only the one that does names, for later steps,
    /// what the step made. The core writes it; a change set does not send
    /// it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completes: Option<bool>,
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
            "completes": {
                "description": "Written by the core in the journal: whether the change set                                 completes the step. A change set does not send it.",
                "type": "boolean",
                "readOnly": true,
            },
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
        .find(|k| !matches!(k.as_str(), "guide" | "step" | "of" | "completes"))
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
    if map.get("completes").is_some_and(|v| !v.is_boolean()) {
        return Err(wrong("expected true or false", "guide.completes"));
    }
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

/// Checks a guide's keys and their kinds, each refused at its own key, such
/// as `guide.steps[1].points[0].label`.
fn shape(value: &Value) -> Result<(), Error> {
    let wrong = |field: String, what: &str| -> Result<(), Error> {
        Err(Error::new(ErrorCode::WrongType, format!("expected {what}")).at_field(field))
    };
    let keys = |map: &Map<String, Value>,
                field: &str,
                what: &str,
                known: &[&str],
                required: &[&str]|
     -> Result<(), Error> {
        if let Some(k) = map.keys().find(|k| !known.contains(&k.as_str())) {
            return Err(Error::new(
                ErrorCode::UnknownKey,
                format!("unknown key `{k}`; {what} takes {}", known.join(", ")),
            )
            .at_field(format!("{field}.{k}")));
        }
        if let Some(k) = required.iter().find(|k| !map.contains_key(**k)) {
            return Err(Error::new(
                ErrorCode::MissingKey,
                format!("missing key `{k}`; {what} takes {}", known.join(", ")),
            )
            .at_field(format!("{field}.{k}")));
        }
        Ok(())
    };
    let words = |map: &Map<String, Value>, key: &str, field: &str| -> Result<(), Error> {
        match map.get(key) {
            Some(v) if !v.is_string() => wrong(format!("{field}.{key}"), "words"),
            _ => Ok(()),
        }
    };
    let Value::Object(guide) = value else {
        return wrong(
            "guide".to_owned(),
            "a guide, {\"title\", \"sheet\", \"steps\"}",
        );
    };
    keys(
        guide,
        "guide",
        "a guide",
        &["title", "sheet", "steps"],
        &["title", "sheet", "steps"],
    )?;
    words(guide, "title", "guide")?;
    words(guide, "sheet", "guide")?;
    let Some(steps) = guide["steps"].as_array() else {
        return wrong("guide.steps".to_owned(), "a list of steps");
    };
    for (s, step) in steps.iter().enumerate() {
        let here = format!("guide.steps[{s}]");
        let Value::Object(step) = step else {
            return wrong(here, "a step, {\"say\", ...}");
        };
        keys(
            step,
            &here,
            "a step",
            &["say", "title", "tool", "points", "completes", "setting"],
            &["say"],
        )?;
        for key in ["say", "title", "tool"] {
            words(step, key, &here)?;
        }
        if let Some(points) = step.get("points") {
            let Some(points) = points.as_array() else {
                return wrong(format!("{here}.points"), "a list of points");
            };
            for (p, point) in points.iter().enumerate() {
                let at = format!("{here}.points[{p}]");
                let Value::Object(point) = point else {
                    return wrong(at, "a point, {\"at\"} or {\"object\"}, with its \"label\"");
                };
                keys(point, &at, "a point", &["at", "object", "label"], &[])?;
                words(point, "label", &at)?;
            }
        }
        if step.get("completes").is_some_and(|v| !v.is_object()) {
            return wrong(
                format!("{here}.completes"),
                "an operation, as a change set holds one",
            );
        }
        if let Some(setting) = step.get("setting") {
            let at = format!("{here}.setting");
            let Value::Object(setting) = setting else {
                return wrong(at, "a setting, {\"current_layer\"}");
            };
            keys(
                setting,
                &at,
                "a setting",
                &["current_layer"],
                &["current_layer"],
            )?;
            words(setting, "current_layer", &at)?;
        }
    }
    Ok(())
}

/// Reads and checks a guide's shape: everything that can be checked
/// without the set.
fn read_guide(value: &Value) -> Result<GuideGiven, Error> {
    shape(value)?;
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

/// Checks that a step's points are its operation's own, so the drafter who
/// clicks each ring and picks each object lands the step: the objects to
/// pick are the ones its operation moves, copies or erases, and the points
/// to click are where its tool takes the operation's points from. Line's
/// two are the line's ends, Circle's its centre and a point on it,
/// Rectangle's two opposite corners, Text's its position, and Move's and
/// Copy's a base point and each place it goes to, or the base point alone,
/// how far typed. A Rectangle step completes an axis-aligned, closed,
/// four-vertex rectangle.
fn points_agree(s: usize, step: &StepGiven, walked: &StepWalked) -> Result<(), Error> {
    let (Some(tool), Some(op)) = (step.tool.as_deref(), walked.resolved.as_ref()) else {
        return Ok(());
    };
    let tool = tool.to_lowercase();
    let field = format!("guide.steps[{s}].points");
    let wrong = |message: String| -> Result<(), Error> {
        Err(
            Error::new(ErrorCode::BadValue, format!("step {}: {message}", s + 1))
                .at_field(field.clone()),
        )
    };
    let mut clicks: Vec<[f64; 3]> = Vec::new();
    let mut picks: Vec<String> = Vec::new();
    for (given, w) in step.points.iter().zip(&walked.points) {
        match (&given.at, w.at, &w.object) {
            (Some(_), Some(at), _) => clicks.push(at),
            (None, _, Some(id)) => picks.push(id.to_lowercase()),
            // A point that names nothing yet is refused where it is.
            _ => return Ok(()),
        }
    }
    let at = |key: &str| op.get(key).and_then(xyz);
    let point = |p: [f64; 3]| shown(&json!(p));
    let points = |ps: &[[f64; 3]]| {
        ps.iter()
            .map(|&p| point(p))
            .collect::<Vec<_>>()
            .join(" and ")
    };
    if matches!(tool.as_str(), "move" | "copy" | "erase") {
        let mut want: Vec<String> = walked.targets.iter().map(|t| t.to_lowercase()).collect();
        want.sort();
        let mut got = picks.clone();
        got.sort();
        got.dedup();
        if got != want || got.len() != picks.len() {
            return wrong(format!(
                "its objects to pick are {}, but its operation takes {}; a step picks each \
                 object its operation takes, once",
                if picks.is_empty() {
                    "none".to_owned()
                } else {
                    picks.join(", ")
                },
                walked.targets.join(", ")
            ));
        }
    } else if !picks.is_empty() {
        return wrong(format!(
            "{tool} picks no objects: its points are `at`, points to click"
        ));
    }
    match tool.as_str() {
        "line" => {
            let (Some(from), Some(to)) = (at("from"), at("to")) else {
                return Ok(());
            };
            let fits = |c: &[[f64; 3]]| {
                c.len() == 2
                    && ((near(c[0], from) && near(c[1], to))
                        || (near(c[0], to) && near(c[1], from)))
            };
            if !fits(&clicks) {
                return wrong(format!(
                    "Line takes two points, the line's ends, and its operation draws from {} to \
                     {}; its points are {}",
                    point(from),
                    point(to),
                    points(&clicks)
                ));
            }
        }
        "circle" => {
            let Some(center) = at("center") else {
                return Ok(());
            };
            let radius = op.get("radius").and_then(Value::as_f64);
            let on = |p: [f64; 3]| {
                let d = ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2)).sqrt();
                radius.is_some_and(|r| same(d, r))
            };
            let fits = match clicks.as_slice() {
                [c] => near(*c, center),
                [c, p] => near(*c, center) && on(*p),
                _ => false,
            };
            if !fits {
                return wrong(format!(
                    "Circle takes its centre, {}, then a point on it{}; its points are {}",
                    point(center),
                    radius.map_or(String::new(), |r| format!(", {} from it", shown(&json!(r)))),
                    points(&clicks)
                ));
            }
        }
        "rectangle" => {
            let corners = |a: [f64; 3], b: [f64; 3]| {
                vec![
                    json!(a),
                    json!([b[0], a[1], a[2]]),
                    json!(b),
                    json!([a[0], b[1], a[2]]),
                ]
            };
            let given: Vec<Value> = op
                .get("points")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let vertices: Vec<[f64; 3]> = given.iter().filter_map(|v| xyz(&vertex(v).0)).collect();
            let unbulged = given.iter().all(|v| vertex(v).1 == 0.0);
            let rectangle = op.get("closed").and_then(Value::as_bool) == Some(true)
                && unbulged
                && vertices.len() == 4
                && given.len() == 4
                && !same(vertices[0][0], vertices[2][0])
                && !same(vertices[0][1], vertices[2][1])
                && same_polyline(&corners(vertices[0], vertices[2]), &given, true);
            if !rectangle {
                return Err(Error::new(
                    ErrorCode::BadValue,
                    format!(
                        "step {}: Rectangle lands an axis-aligned rectangle, a closed polyline of \
                         four vertices with no bulges, and its operation is another polyline",
                        s + 1
                    ),
                )
                .at_field(format!("guide.steps[{s}].completes")));
            }
            let fits = match clicks.as_slice() {
                [a, b] => same_polyline(&corners(*a, *b), &given, true),
                _ => false,
            };
            if !fits {
                return wrong(format!(
                    "Rectangle takes two opposite corners, and its operation's are {}; its points \
                     are {}",
                    points(&[vertices[0], vertices[2]]),
                    points(&clicks)
                ));
            }
        }
        "text" => {
            let Some(position) = at("position") else {
                return Ok(());
            };
            if !matches!(clicks.as_slice(), [c] if near(*c, position)) {
                return wrong(format!(
                    "Text takes one point, its position, {}; its points are {}",
                    point(position),
                    points(&clicks)
                ));
            }
        }
        "move" | "copy" => {
            let op = normal(op, &tool);
            let goes: Vec<[f64; 3]> = match (op.get("by").and_then(xyz), op.get("offsets")) {
                (Some(by), _) => vec![by],
                (None, Some(Value::Array(offsets))) => offsets.iter().filter_map(xyz).collect(),
                _ => return Ok(()),
            };
            let fits = match clicks.as_slice() {
                [] | [_] => true,
                [base, to @ ..] => {
                    to.len() == goes.len()
                        && to.iter().zip(&goes).all(|(p, by)| {
                            near([p[0] - base[0], p[1] - base[1], p[2] - base[2]], *by)
                        })
                }
            };
            if !fits {
                return wrong(format!(
                    "{} takes a base point, then where it goes, and its operation goes {}; its \
                     points are {}",
                    if tool == "move" { "Move" } else { "Copy" },
                    points(&goes),
                    points(&clicks)
                ));
            }
        }
        _ => {
            if !clicks.is_empty() {
                return wrong("Erase takes objects to pick, and no points".to_owned());
            }
        }
    }
    Ok(())
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
    let mut given = read_guide(&request.guide)?;
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
        points_agree(s, step, &walked[s])?;
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
    // A setting's layer is kept as the sheet names it, so the drafting app's
    // Layer box shows it.
    for (step, walked) in given.steps.iter_mut().zip(&walked) {
        if let (Some(setting), Some(layer)) = (&mut step.setting, &walked.layer) {
            setting.current_layer.clone_from(layer);
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

/// What the journal holds of one step of a guide.
#[derive(Clone, Debug, Default)]
struct StepLanded {
    /// The change sets in place sent for it, the oldest first.
    entries: Vec<u64>,
    /// The newest of them that completes it.
    completed_by: Option<u64>,
    /// The object that one added on the guide's sheet.
    made: Option<String>,
}

/// For each step of guide `number`, the change sets in place sent for it,
/// the newest that completes it and the object that one added on sheet
/// `sheet`. A change set sent for the step that does not complete it, such
/// as a line drawn to the wrong place, names nothing for later steps.
fn landed_for(entries: &[journal::Entry], number: u64, sheet: &str) -> BTreeMap<u64, StepLanded> {
    let fates = fates(entries);
    let mut out: BTreeMap<u64, StepLanded> = BTreeMap::new();
    let prefix = format!("{sheet}{ID_SEPARATOR}").to_lowercase();
    for entry in in_place(entries, &fates) {
        let Some(mark) = entry.guide.filter(|m| m.guide == number) else {
            continue;
        };
        let slot = out.entry(mark.step).or_default();
        slot.entries.push(entry.entry);
        if mark.completes != Some(true) {
            continue;
        }
        slot.completed_by = Some(entry.entry);
        slot.made = entry
            .objects
            .iter()
            .find(|o| o.how == HowChanged::Added && o.id.to_lowercase().starts_with(&prefix))
            .map(|o| o.id.clone());
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
        // A commit still pending is not in place yet: as of the newest
        // finished entry, the steps say what is.
        newest_entry: entries
            .iter()
            .filter(|e| e.finished())
            .map(|e| e.entry)
            .max()
            .unwrap_or(0),
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
        .filter_map(|(&k, l)| l.made.clone().map(|id| (k, id)))
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
            let landed = landed.get(&number).cloned().unwrap_or_default();
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
                landed: landed.entries,
                completed_by: landed.completed_by,
                made: landed.made,
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
    /// The guide's sheet, as the set names it.
    sheet: String,
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
    let Some((mark, stored)) = marked_step(&request.set, &request.change_set)? else {
        return Ok(None);
    };
    let folder = &request.set;
    let step = mark.step;
    let mut change_set = request.change_set.clone();
    change_set["guide"] = serde_json::to_value(mark).expect("a mark serializes");
    let entries = journal::read_entries(folder)?;
    let sheet = stored.sheet.clone();
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
        sheet,
        expected_op: wanted.operation.clone(),
        expected,
    }))
}

/// `guided`'s change set, marked with whether it completes its step, as
/// the core checked it before it landed, for its journal entry.
pub(super) fn mark_completes(guided: &mut Guided, completes: bool) {
    guided.mark.completes = Some(completes);
    guided.request.change_set["guide"]["completes"] = Value::Bool(completes);
}

/// Checks that a change set's `guide`, when it has one, names an operation
/// step of a guide the set in `folder` holds, as a change set or a proposal
/// sent for one must: refused whole otherwise.
pub(super) fn check_mark(folder: &Path, change_set: &Value) -> Result<(), Error> {
    marked_step(folder, change_set).map(|_| ())
}

/// The step `change_set`'s `guide` names, with `of` filled in, and its
/// guide; none when it names none.
fn marked_step(folder: &Path, change_set: &Value) -> Result<Option<(GuideMark, Stored)>, Error> {
    let Some(mark) = change_set.get("guide") else {
        return Ok(None);
    };
    let (number, step, of) = read_mark(mark)?;
    if mark.get("completes").is_some() {
        return Err(Error::new(
            ErrorCode::UnknownKey,
            "`completes` is the core's to write, once it has checked the change set against              the step; a change set sends the guide and the step, and `of`",
        )
        .at_field("guide.completes"));
    }
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
        completes: None,
    };
    Ok(Some((mark, stored)))
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

/// The operation `sent` with each snapped point `snapped` says it took
/// written as its coordinates.
pub(super) fn resolved(sent: &Value, snapped: &[SnapResolved]) -> Value {
    let mut out = sent.clone();
    for snapped in snapped {
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

/// The keys whose words name something, a layer or a style, which match in
/// any case, as AutoCAD's names do. Any other words, such as a text's
/// contents, match exactly.
const NAME_KEYS: [&str; 13] = [
    "layer",
    "linetype",
    "style",
    "color",
    "lineweight",
    "justify",
    "block",
    "space",
    "text_style",
    "dimension_style",
    "table_style",
    "multileader_style",
    "snap",
];

/// Whether `got` is `want`: numbers and points within [`same`], a point of
/// two coordinates as one with z 0, words exactly, or in any case for a
/// name (`name`), and of a mapping only the keys `want` gives.
fn matches(want: &Value, got: &Value, name: bool) -> bool {
    match (want, got) {
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => same(a, b),
            _ => false,
        },
        (Value::Array(a), Value::Array(b)) => {
            if let (Some(a), Some(b)) = (xyz(want), xyz(got)) {
                near(a, b)
            } else {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| matches(x, y, name))
            }
        }
        (Value::String(a), Value::String(b)) if name => a.trim().eq_ignore_ascii_case(b.trim()),
        (Value::Object(a), Value::Object(b)) => a.iter().all(|(k, v)| {
            b.get(k)
                .is_some_and(|w| matches(v, w, NAME_KEYS.contains(&k.as_str())))
        }),
        (a, b) => a == b,
    }
}

/// A point, `[x, y]` or `[x, y, z]`, as three coordinates.
fn xyz(value: &Value) -> Option<[f64; 3]> {
    let items = value.as_array()?;
    if !(2..=3).contains(&items.len()) {
        return None;
    }
    let mut out = [0.0; 3];
    for (i, item) in items.iter().enumerate() {
        out[i] = item.as_f64()?;
    }
    Some(out)
}

/// Whether two points are one, within [`same`].
fn near(a: [f64; 3], b: [f64; 3]) -> bool {
    (0..3).all(|i| same(a[i], b[i]))
}

/// A polyline's vertex: its point and its bulge.
fn vertex(v: &Value) -> (Value, f64) {
    match v.get("point") {
        Some(p) => (
            p.clone(),
            v.get("bulge").and_then(Value::as_f64).unwrap_or(0.0),
        ),
        None => (v.clone(), 0.0),
    }
}

/// Whether polyline vertices `got` draw the polyline `want` does: the same
/// vertices in the same order, either way round, a closed one from any of
/// them, each segment's bulge with it, turned the other way when the
/// polyline runs the other way. A bow-tie is not a rectangle.
fn same_polyline(want: &[Value], got: &[Value], closed: bool) -> bool {
    let n = want.len();
    if n != got.len() {
        return false;
    }
    if n == 0 {
        return true;
    }
    let w: Vec<(Value, f64)> = want.iter().map(vertex).collect();
    let g: Vec<(Value, f64)> = got.iter().map(vertex).collect();
    // An open polyline's last vertex starts no segment.
    let counts = |i: usize| closed || i + 1 < n;
    let starts: Vec<usize> = if closed { (0..n).collect() } else { vec![0] };
    let forwards = starts.iter().any(|&s| {
        (0..n).all(|i| {
            let (gp, gb) = &g[(s + i) % n];
            matches(&w[i].0, gp, false) && (!counts(i) || same(w[i].1, *gb))
        })
    });
    if forwards {
        return true;
    }
    // Run the other way from vertex `s`: its k-th vertex is `s - k`, and
    // its k-th segment the one from `s - k - 1`, its bulge turned.
    let ends: Vec<usize> = if closed {
        (0..n).collect()
    } else {
        vec![n - 1]
    };
    ends.iter().any(|&s| {
        (0..n).all(|k| {
            let i = (s + n - k) % n;
            let j = (s + 2 * n - k - 1) % n;
            matches(&w[k].0, &g[i].0, false) && (!counts(k) || same(w[k].1, -g[j].1))
        })
    })
}

/// `op`, resolved, as the check compares it: a move or a copy given from
/// and to written as how far it goes.
fn normal(op: &Value, name: &str) -> Value {
    let mut op = op.clone();
    if name == "move" || name == "copy" {
        if let (Some(from), Some(to)) = (op.get("from").and_then(xyz), op.get("to").and_then(xyz)) {
            let by = json!([to[0] - from[0], to[1] - from[1], to[2] - from[2]]);
            if let Value::Object(map) = &mut op {
                map.remove("from");
                map.remove("to");
                map.insert("by".to_owned(), by);
            }
        }
    }
    op
}

/// The keys the check passes over: what names the operation and where it
/// lands, which the sheets reached, the targets' IDs and a given ID
/// compare instead.
const PASSED_OVER: [&str; 5] = ["op", "sheet", "target", "targets", "id"];

/// An ID's part after its sheet's name, in small letters, to compare.
fn local(id: &str) -> String {
    id.split_once(ID_SEPARATOR)
        .map_or(id, |(_, l)| l)
        .to_lowercase()
}

/// The objects an operation moved, copied, deleted or changed, by their
/// IDs' own parts, sorted: none for an add.
fn targets_of(outcome: &OperationOutcome) -> Vec<String> {
    let mut out: Vec<String> = if outcome.op == "copy" {
        outcome.copies.iter().map(|c| local(&c.source)).collect()
    } else if outcome.op.starts_with("add_") {
        Vec::new()
    } else {
        outcome.id.iter().map(|id| local(id)).collect()
    };
    out.sort();
    out.dedup();
    out
}

/// The same, as the reply names them.
fn targets_named(outcome: &OperationOutcome) -> String {
    let mut out: Vec<String> = if outcome.op == "copy" {
        outcome.copies.iter().map(|c| c.source.clone()).collect()
    } else {
        outcome.id.iter().cloned().collect()
    };
    out.dedup();
    if out.is_empty() {
        "nothing".to_owned()
    } else {
        out.iter()
            .map(|id| format!("`{id}`"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The sheets an operation reached, sorted in small letters.
fn sheets_of(outcome: &OperationOutcome) -> Vec<String> {
    let mut out: Vec<String> = outcome.sheets.iter().map(|s| s.to_lowercase()).collect();
    out.sort();
    out
}

/// The change set `request` sent for a guide's step, checked against the
/// step's own operation, as `report` says it landed.
pub(super) fn checked(guided: &Guided, report: &ChangeReport) -> GuideChecked {
    let mark = guided.mark;
    let here = |id: &str| {
        let own = id.split_once(ID_SEPARATOR).map_or(id, |(_, l)| l);
        format!("{}{ID_SEPARATOR}{own}", guided.sheet)
    };
    let made = report.operations.iter().find(|o| o.ok).and_then(|o| {
        if o.op.starts_with("add_") {
            o.id.as_deref().map(here)
        } else if let [one] = o.copies.as_slice() {
            Some(here(&one.id))
        } else {
            None
        }
    });
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
    if sheets_of(got) != sheets_of(want) {
        let named = |o: &OperationOutcome| {
            o.sheets
                .iter()
                .map(|s| format!("`{s}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        return verdict(
            false,
            format!(
                "That landed on {}; the step wants {}.",
                named(got),
                named(want)
            ),
        );
    }
    if targets_of(got) != targets_of(want) {
        return verdict(
            false,
            format!(
                "That went to {}; the step wants {}.",
                targets_named(got),
                targets_named(want)
            ),
        );
    }
    if want.op.starts_with("add_") && sent_want.get("id").is_some() {
        let (w, g) = (want.id.as_deref(), got.id.as_deref());
        if w.map(local) != g.map(local) {
            return verdict(
                false,
                format!(
                    "Its `id` is `{}`; the step wants `{}`.",
                    g.unwrap_or_default(),
                    w.unwrap_or_default()
                ),
            );
        }
    }
    let sent_got = guided
        .request
        .change_set
        .get("operations")
        .and_then(|o| o.get(0))
        .cloned()
        .unwrap_or(Value::Null);
    let want_op = normal(&resolved(&sent_want, &want.snapped), &want.op);
    let got_op = normal(&resolved(&sent_got, &got.snapped), &got.op);
    let polyline = want.op == "add_polyline";
    let closed = want_op.get("closed").and_then(Value::as_bool) == Some(true);
    let differs = |got_op: &Value| -> Option<String> {
        let want_map = want_op.as_object()?;
        want_map
            .iter()
            .filter(|(k, _)| !PASSED_OVER.contains(&k.as_str()))
            .find(|(k, v)| {
                let Some(g) = got_op.get(k.as_str()) else {
                    return true;
                };
                match (v, g) {
                    (Value::Array(w), Value::Array(g)) if polyline && k.as_str() == "points" => {
                        !same_polyline(w, g, closed)
                    }
                    _ => !matches(v, g, NAME_KEYS.contains(&k.as_str())),
                }
            })
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
            let went = if want_op.get(&key).and_then(xyz).is_some() {
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
