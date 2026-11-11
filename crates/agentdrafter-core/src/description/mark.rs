//! Reads and writes the mark of an inconsistent drawing set: the file
//! `crate::change` keeps in the set's folder while a change set has landed
//! on some of the set's sheets and failed on others.
//!
//! It is written in the same strict YAML as every file of a set, so it reads
//! as they do. Each failure names the sheet, why it failed, the sheets the
//! change set landed on, and the operations that were the sheet's part, each
//! as the JSON it was sent as.

use super::emit::{scalar_text, Context};
use super::strict_yaml::{self, index_path, Node};
use super::{scalar, sequence, Fields};
use crate::error::{Error, ErrorCode};

/// The keys of one failure in the mark.
const FAILURE_KEYS: [&str; 5] = ["sheet", "code", "message", "landed_on", "operations"];

/// One sheet a change set failed on while it landed on others.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MarkFailure {
    pub sheet: String,
    /// The failure's error code, such as `unknown_name`.
    pub code: String,
    pub message: String,
    /// The sheets the change set landed on.
    pub landed_on: Vec<String>,
    /// The operations that were the sheet's part, each as JSON text.
    pub operations: Vec<String>,
}

/// The text of a mark holding `failures`.
pub(crate) fn mark_text(failures: &[MarkFailure]) -> String {
    let line = |text: &str| scalar_text(&text.replace(['\n', '\r'], " "), Context::Block, 0, "\n");
    let flow = |text: &str| scalar_text(text, Context::Flow, 0, "\n");
    let mut out = String::from(
        "# AgentDrafter marks this drawing set inconsistent: a change set landed on some of\n\
         # its sheets and failed on others, which were left as they were. Each failure\n\
         # below stays until a later change set names its sheet under `fixes` and lands\n\
         # on it, or until the change set that marked it is undone.\n\
         failures:\n",
    );
    for failure in failures {
        out.push_str(&format!("  - sheet: {}\n", line(&failure.sheet)));
        out.push_str(&format!("    code: {}\n", line(&failure.code)));
        out.push_str(&format!("    message: {}\n", line(&failure.message)));
        let landed: Vec<String> = failure.landed_on.iter().map(|s| flow(s)).collect();
        out.push_str(&format!("    landed_on: [{}]\n", landed.join(", ")));
        if failure.operations.is_empty() {
            out.push_str("    operations: []\n");
        } else {
            out.push_str("    operations:\n");
            for operation in &failure.operations {
                out.push_str(&format!("      - {}\n", line(operation)));
            }
        }
    }
    out
}

/// Reads the text of a mark.
pub(crate) fn parse_mark(text: &str) -> Result<Vec<MarkFailure>, Error> {
    let Some(root) = strict_yaml::read(text)? else {
        return Ok(Vec::new());
    };
    let fields = Fields::of(&root, "", &["failures"], "the inconsistent set's mark")?;
    let Some(list) = fields.get("failures") else {
        return Ok(Vec::new());
    };
    let items = sequence(list, "failures", "a list of failures")?;
    items
        .iter()
        .enumerate()
        .map(|(i, node)| failure(node, &index_path("failures", i)))
        .collect()
}

fn failure(node: &Node, path: &str) -> Result<MarkFailure, Error> {
    let fields = Fields::of(node, path, &FAILURE_KEYS, "a failure")?;
    let text = |key: &str| -> Result<String, Error> {
        scalar(fields.require(key)?, &fields.path_of(key), "text").map(str::to_owned)
    };
    let texts = |key: &str| -> Result<Vec<String>, Error> {
        let field = fields.path_of(key);
        sequence(fields.require(key)?, &field, "a list")?
            .iter()
            .enumerate()
            .map(|(i, item)| scalar(item, &index_path(&field, i), "text").map(str::to_owned))
            .collect()
    };
    let failure = MarkFailure {
        sheet: text("sheet")?,
        code: text("code")?,
        message: text("message")?,
        landed_on: texts("landed_on")?,
        operations: texts("operations")?,
    };
    if failure.sheet.is_empty() {
        return Err(Error::new(ErrorCode::BadValue, "a failure names its sheet").at_field(path));
    }
    Ok(failure)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mark_reads_back_as_written() {
        let failures = vec![
            MarkFailure {
                sheet: "DEMO-002".into(),
                code: "unknown_name".into(),
                message: "operation 1 (`add_line`): unknown layer `nope`; layers: 0, notes".into(),
                landed_on: vec!["DEMO-001".into(), "DEMO-003".into()],
                operations: vec![r#"{"op":"add_line","sheet":"DEMO-002","layer":"nope"}"#.into()],
            },
            MarkFailure {
                sheet: "A: B".into(),
                code: "bad_value".into(),
                message: "two\nlines".into(),
                landed_on: vec![],
                operations: vec![],
            },
        ];
        let text = mark_text(&failures);
        let mut back = parse_mark(&text).unwrap();
        assert_eq!(back[1].message, "two lines");
        back[1].message = "two\nlines".into();
        assert_eq!(back, failures);
    }
}
