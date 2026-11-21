//! A guide's step as the drafter does it (#157): what the step wants next,
//! from the drawing tool's own state, where its ring goes, and whether a
//! point the drafter took, or an object they picked, is the one the step
//! wants. Plain computation, tested natively; `web` feeds it the step the
//! page runs, as the core's `guides` lists it, and each click and pick.
//!
//! The step's points are in the order its tool takes them: the objects to
//! pick, then the points to click. A point taken is the step's when it is
//! within a millionth of where the core worked the step's point out to be,
//! as the core checks a change set's points; one that is not gets a note
//! saying where it went and what the step wants, and the drafter decides
//! whether to take it back. Nothing here takes back anything on its own.
//!
//! Whether the change set the tool lands completes the step is the core's
//! check, in `change`'s reply (ADR-0007); this module only says, as each
//! click lands, what the drafter is doing against what the step asks.

use serde::{Deserialize, Serialize};

use crate::tools::{snap_label, Taken, ToolKind, Tools};

/// One step of a guide, as the core's `guides` lists it: its tool and its
/// points, each worked out on the set as it stands.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct StepPlan {
    /// The drawing tool, as the page names it; none for a setting step.
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub points: Vec<PlanPoint>,
    /// The operation that completes it, with each `made_by` named.
    #[serde(default)]
    pub operation: Option<serde_json::Value>,
}

/// A point to click or an object to pick, as `guides` lists it.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct PlanPoint {
    #[serde(default)]
    pub label: Option<String>,
    /// Where it is; none while it waits on an earlier step.
    #[serde(default)]
    pub at: Option<[f64; 3]>,
    /// The object to pick, by ID.
    #[serde(default)]
    pub object: Option<String>,
}

impl PlanPoint {
    /// What it is, in the guide's words.
    pub fn words(&self) -> String {
        self.label.clone().unwrap_or_else(|| match &self.object {
            Some(id) => id.rsplit('/').next().unwrap_or(id).to_owned(),
            None => "the ring".to_owned(),
        })
    }
}

impl StepPlan {
    /// Its tool.
    pub fn kind(&self) -> Option<ToolKind> {
        self.tool.as_deref().and_then(crate::tools::tool_for)
    }

    /// Its points to click, by their places among its points.
    fn clicks(&self) -> Vec<usize> {
        (0..self.points.len())
            .filter(|&i| self.points[i].object.is_none())
            .collect()
    }

    /// Its objects to pick, by their places among its points.
    fn picks(&self) -> Vec<usize> {
        (0..self.points.len())
            .filter(|&i| self.points[i].object.is_some())
            .collect()
    }
}

/// What the step wants next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "phase", content = "point", rename_all = "snake_case")]
pub enum Phase {
    /// Its tool started: it is not running yet, or another tool is.
    Start,
    /// The object at this place among the step's points picked.
    Pick(usize),
    /// The point at this place among the step's points clicked.
    Click(usize),
    /// Its tool has every point and object the step gives: Enter, or for a
    /// tool that goes on, its last point typed or clicked.
    Finish,
    /// The text typed.
    Type,
}

/// What the step wants next, from the tool running.
pub fn phase(step: &StepPlan, tools: &Tools) -> Phase {
    let Some(kind) = step.kind() else {
        return Phase::Start;
    };
    if tools.running() != Some(kind) {
        return Phase::Start;
    }
    let state = tools.state(false);
    if state.typing_text {
        return Phase::Type;
    }
    if state.selecting {
        let picked: Vec<String> = state
            .picked
            .iter()
            .filter_map(|&p| id_at(tools, p))
            .collect();
        return step
            .picks()
            .into_iter()
            .find(|&i| {
                let want = step.points[i].object.as_deref().unwrap_or_default();
                !picked.iter().any(|id| id.eq_ignore_ascii_case(want))
            })
            .map_or(Phase::Finish, Phase::Pick);
    }
    match step.clicks().get(state.points.len()) {
        Some(&i) => Phase::Click(i),
        None => Phase::Finish,
    }
}

/// The ID of the object at `place` on the sheet the tools work on.
fn id_at(tools: &Tools, place: usize) -> Option<String> {
    tools.context().objects.get(place)?.id.clone()
}

/// The place of the object of ID `id` on the sheet the tools work on.
pub fn place_of(tools: &Tools, id: &str) -> Option<usize> {
    tools
        .context()
        .objects
        .iter()
        .position(|o| o.id.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(id)))
}

/// Where the ring goes: the point to click, or the object to pick.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Ring {
    /// Its place among the step's points.
    pub point: usize,
    pub label: String,
    /// The point to click, in drawing units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<[f64; 2]>,
    /// The object to pick, by its place in the sheet's objects.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<usize>,
}

/// The ring the step shows now: before its tool starts, its first point or
/// object; then each in turn; none once it wants Enter or text, or while a
/// point waits on an earlier step.
pub fn ring(step: &StepPlan, tools: &Tools) -> Option<Ring> {
    let point = match phase(step, tools) {
        Phase::Start => (!step.points.is_empty()).then_some(0)?,
        Phase::Pick(i) | Phase::Click(i) => i,
        Phase::Finish | Phase::Type => return None,
    };
    let given = &step.points[point];
    let label = given.words();
    match &given.object {
        Some(id) => Some(Ring {
            point,
            label,
            at: None,
            object: Some(place_of(tools, id)?),
        }),
        None => given.at.map(|at| Ring {
            point,
            label,
            at: Some([at[0], at[1]]),
            object: None,
        }),
    }
}

/// What the drafter did against what the step wants, at the moment they did
/// it, and what taking it back means.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Note {
    pub words: String,
    /// What Undo it takes back: `point`, the point just taken, or `pick`,
    /// the object just picked.
    pub take_back: &'static str,
    /// The object just picked, by its place, for `pick`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<usize>,
}

/// Whether two coordinates are one point, as the core checks a change
/// set's points against a step's: within a millionth of the larger, or of
/// 1.
fn same(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0)
}

/// Where a point went, in words: its snap and object, or its coordinates.
fn went(tools: &Tools, taken: &Taken) -> String {
    match &taken.snap {
        Some(s) => {
            let object = tools
                .context()
                .objects
                .get(s.object)
                .map_or_else(|| "?".to_owned(), |o| o.words());
            format!("the {} of {object}", snap_label(s.snap).to_lowercase())
        }
        None => format!(
            "{}, {}",
            crate::tools::f4(taken.at[0]),
            crate::tools::f4(taken.at[1])
        ),
    }
}

/// A point the drafter took, `taken`, while the step was at `before`: a
/// note when it is not the point the step wants, or when the step had
/// every point it gives and the tool took another.
pub fn after_point(
    step: &StepPlan,
    before: Phase,
    tools: &Tools,
    taken: &Taken,
    points_before: usize,
) -> Option<Note> {
    let kind = step.kind()?;
    if tools.running() != Some(kind) {
        return None;
    }
    // A point the tool did not take, such as the same point twice, is
    // nothing to say.
    if tools.state(false).points.len() <= points_before {
        return None;
    }
    match before {
        Phase::Click(i) => {
            let want = &step.points[i];
            let at = want.at?;
            if same(taken.at[0], at[0]) && same(taken.at[1], at[1]) {
                return None;
            }
            Some(Note {
                words: format!(
                    "That went to {}; the step wants {}.",
                    went(tools, taken),
                    want.words()
                ),
                take_back: "point",
                place: None,
            })
        }
        Phase::Finish => {
            let last = step.clicks().last().map(|&i| step.points[i].words());
            Some(Note {
                words: match last {
                    Some(last) => format!("That would draw on past {last}; the step ends there."),
                    None => "That point is more than the step asks for.".to_owned(),
                },
                take_back: "point",
                place: None,
            })
        }
        _ => None,
    }
}

/// An object the drafter picked, at `place`, while the step was at
/// `before`: a note when it is not one the step wants.
pub fn after_pick(step: &StepPlan, before: Phase, tools: &Tools, place: usize) -> Option<Note> {
    let kind = step.kind()?;
    if tools.running() != Some(kind) || !matches!(before, Phase::Pick(_) | Phase::Finish) {
        return None;
    }
    let state = tools.state(false);
    // Taken out again, it is nothing to say.
    if !state.picked.contains(&place) {
        return None;
    }
    let id = id_at(tools, place)?;
    let wanted = step.picks().into_iter().any(|i| {
        step.points[i]
            .object
            .as_deref()
            .is_some_and(|w| w.eq_ignore_ascii_case(&id))
    });
    if wanted {
        return None;
    }
    let want = match before {
        Phase::Pick(i) => step.points[i].words(),
        _ => step
            .picks()
            .iter()
            .map(|&i| step.points[i].words())
            .collect::<Vec<_>>()
            .join(", "),
    };
    let object = tools
        .context()
        .objects
        .get(place)
        .map_or_else(|| "?".to_owned(), |o| o.words());
    Some(Note {
        words: format!("That picked {object}; the step wants {want}."),
        take_back: "pick",
        place: Some(place),
    })
}

/// The text height a Text step's operation gives, which the tool takes up
/// when the step starts, so the text the drafter types is the step's.
pub fn text_height(step: &StepPlan) -> Option<f64> {
    if step.kind() != Some(ToolKind::Text) {
        return None;
    }
    step.operation
        .as_ref()?
        .get("height")?
        .as_f64()
        .filter(|h| *h > 0.0)
}

#[cfg(test)]
#[path = "guide_tests.rs"]
mod tests;
