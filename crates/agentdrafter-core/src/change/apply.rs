//! Applies a change set to a drawing set's own files.
//!
//! The operations are applied in order to the records held in memory: the
//! set's rows file and the templates they name. Every sheet an operation
//! reaches is then read from the edited records and built into a DWG in
//! memory, as `agentdrafter build` would build it. A sheet that cannot take
//! its part fails, and so does every operation that reaches it; a template
//! edit reaches every sheet built from the template, so it fails with any
//! of them, and takes those sheets' other operations with it (Zach's
//! decision of 2026-11-11). The operations left are applied again from the
//! records as they were, until every sheet they reach takes its part. Only
//! then is anything written, all of it together or none of it
//! (`commit.rs`).

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use super::commit::{commit, Replacement};
use super::operations::{op_error, read_change_set, AddTo, Body, Kind, Operation, Target};
use super::MARK_FILE;
use super::{
    mark_report, quoted_list, read_mark, ChangeReport, ChangeRequest, Fill, OperationOutcome,
    RowValueSet, SheetChange,
};
use crate::build::{is_same_file, prepare_sheet, PreparedSheet};
use crate::description::{
    mark_text, read_file, read_set_settings, rows_at, template_at, template_path, AttributeSet,
    Listed, MarkFailure, ObjectList, RecordFile, Row,
};
use crate::error::{Error, ErrorCode};
use crate::set::{Notice, SET_SETTINGS_FILE};
use crate::settings::Settings;
use crate::units::Unit;

/// The most IDs an error lists as the choices on a sheet.
const MOST_LISTED: usize = 60;

/// A drawing set's files as they are on disk before the change set.
struct SetFiles {
    folder: PathBuf,
    units: Unit,
    output: PathBuf,
    rows_path: PathBuf,
    rows_text: String,
    rows: Vec<Row>,
    templates: Vec<TemplateFile>,
    /// For each row, its template's place in `templates`.
    template_of: Vec<usize>,
}

struct TemplateFile {
    /// Its path as the first row that names it writes it.
    written: String,
    path: PathBuf,
    text: Result<String, Error>,
    /// The rows of the sheets built from it.
    sheets: Vec<usize>,
}

impl SetFiles {
    fn load(folder: &Path) -> Result<Self, Error> {
        let settings = read_set_settings(&folder.join(SET_SETTINGS_FILE))?;
        let rows_path = folder.join(&settings.rows);
        let rows_text = read_file(&rows_path, "the drawing set's rows")?;
        let rows = rows_at(&rows_text, &rows_path)?;
        let mut templates: Vec<TemplateFile> = Vec::new();
        let mut template_of = Vec::with_capacity(rows.len());
        for (k, row) in rows.iter().enumerate() {
            let path = template_path(folder, row);
            let same = templates.iter().position(|t| one_file(&t.path, &path));
            let t = match same {
                Some(t) => t,
                None => {
                    templates.push(TemplateFile {
                        written: row.template.clone(),
                        text: read_file(&path, "the template"),
                        path,
                        sheets: Vec::new(),
                    });
                    templates.len() - 1
                }
            };
            templates[t].sheets.push(k);
            template_of.push(t);
        }
        Ok(Self {
            folder: folder.to_owned(),
            units: settings.units,
            output: folder.join(&settings.output),
            rows_path,
            rows_text,
            rows,
            templates,
            template_of,
        })
    }

    /// The row of the sheet named `name`, in any case.
    fn sheet(&self, name: &str) -> Option<usize> {
        let folded = name.to_lowercase();
        self.rows
            .iter()
            .position(|r| r.sheet.to_lowercase() == folded)
    }

    fn sheet_names(&self) -> String {
        self.rows
            .iter()
            .map(|r| r.sheet.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The template the rows write as `written`, or name by another path
    /// to the same file.
    fn template(&self, written: &str) -> Option<usize> {
        let wanted = self.folder.join(written);
        self.templates
            .iter()
            .position(|t| t.written == written || one_file(&t.path, &wanted))
    }

    fn name(&self, k: usize) -> &str {
        &self.rows[k].sheet
    }

    fn names(&self, sheets: &[usize]) -> Vec<String> {
        sheets.iter().map(|&k| self.rows[k].sheet.clone()).collect()
    }

    fn dwg(&self, k: usize) -> PathBuf {
        self.output.join(format!("{}.dwg", self.rows[k].sheet))
    }
}

/// Whether two paths name one file: the file system says, so `..`, links
/// and, where the file system ignores it, case are worked out as it works
/// them out. Paths to no file are one only when they are written the same.
fn one_file(a: &Path, b: &Path) -> bool {
    a == b || is_same_file(a, b)
}

/// Which record an object is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileRef {
    Rows,
    Template(usize),
}

/// The change set's operations applied to the records in memory.
struct Attempt<'a> {
    files: &'a SetFiles,
    rows: RecordFile,
    templates: Vec<Option<Result<RecordFile, Error>>>,
    /// The operation that last set each row value: the row, the value's
    /// name and the operation.
    values: Vec<(usize, String, usize)>,
}

impl<'a> Attempt<'a> {
    fn new(files: &'a SetFiles) -> Result<Self, Error> {
        Ok(Self {
            files,
            rows: RecordFile::from_text(&files.rows_path, files.rows_text.clone())?,
            templates: files.templates.iter().map(|_| None).collect(),
            values: Vec::new(),
        })
    }

    fn template(&mut self, t: usize) -> Result<&mut RecordFile, Error> {
        let file = &self.files.templates[t];
        let slot = self.templates[t].get_or_insert_with(|| {
            file.text
                .clone()
                .and_then(|text| RecordFile::from_text(&file.path, text))
        });
        slot.as_mut().map_err(|e| e.clone())
    }

    fn file(&mut self, file: FileRef) -> Result<&mut RecordFile, Error> {
        match file {
            FileRef::Rows => Ok(&mut self.rows),
            FileRef::Template(t) => self.template(t),
        }
    }

    /// The objects on sheet `k`: its template's, then its row's own, each
    /// with where it is written.
    fn objects(&mut self, k: usize) -> Result<Vec<(FileRef, ObjectList, usize, Listed)>, Error> {
        let t = self.files.template_of[k];
        let mut found: Vec<(FileRef, ObjectList, usize, Listed)> = self
            .template(t)?
            .objects(ObjectList::Template)
            .into_iter()
            .enumerate()
            .map(|(i, l)| (FileRef::Template(t), ObjectList::Template, i, l))
            .collect();
        found.extend(
            self.rows
                .objects(ObjectList::Row(k))
                .into_iter()
                .enumerate()
                .map(|(i, l)| (FileRef::Rows, ObjectList::Row(k), i, l)),
        );
        Ok(found)
    }

    /// The sheets an edit to `file` reaches.
    fn reach(&self, file: FileRef, k: usize) -> Vec<usize> {
        match file {
            FileRef::Rows => vec![k],
            FileRef::Template(t) => self.files.templates[t].sheets.clone(),
        }
    }
}

/// An object a target names.
struct Found {
    row: usize,
    file: FileRef,
    list: ObjectList,
    index: usize,
    /// Its ID on the sheet named.
    id: String,
}

/// What applying one operation did.
struct Applied {
    /// The sheets it reaches.
    sheets: Vec<usize>,
    outcome: Result<Done, Error>,
    /// Whether its error is the change set's, not one sheet's, so the
    /// whole change set is refused.
    refuse: bool,
}

#[derive(Clone)]
struct Done {
    record: PathBuf,
    id: Option<String>,
    before: Option<String>,
    after: Option<String>,
    row_value: Option<RowValueSet>,
    ids_written: Vec<String>,
    message: String,
}

pub(super) fn apply(request: &ChangeRequest, settings: &Settings) -> Result<ChangeReport, Error> {
    let files = SetFiles::load(&request.set)?;
    let change_set = read_change_set(&request.change_set, files.units)?;
    let mark = read_mark(&files.folder)?;
    let ops = &change_set.operations;
    precheck(&files, ops)?;
    let fixes = fixes(&files, &change_set.fixes, &mark)?;

    let n = ops.len();
    let mut included = vec![true; n];
    let mut op_sheets: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut op_errors: Vec<Option<Error>> = vec![None; n];
    let mut done: Vec<Option<Done>> = vec![None; n];
    let mut failed: BTreeMap<usize, Error> = BTreeMap::new();
    let mut notices: BTreeMap<usize, Vec<Notice>> = BTreeMap::new();
    let mut touched: BTreeSet<usize> = fixes.iter().copied().collect();
    let (attempt, prepared) = loop {
        let mut at = Attempt::new(&files)?;
        for op in ops.iter().filter(|op| included[op.index]) {
            let applied = apply_op(&mut at, op);
            if applied.refuse {
                return Err(applied.outcome.err().expect("a refusal is an error"));
            }
            op_sheets[op.index].clone_from(&applied.sheets);
            match applied.outcome {
                Ok(d) => done[op.index] = Some(d),
                Err(e) => {
                    for &k in &applied.sheets {
                        failed.entry(k).or_insert_with(|| e.clone());
                    }
                    op_errors[op.index] = Some(e);
                }
            }
        }
        for (i, sheets) in op_sheets.iter().enumerate() {
            if included[i] {
                touched.extend(sheets);
            }
        }

        let mut prepared: BTreeMap<usize, PreparedSheet> = BTreeMap::new();
        let rows = at
            .rows
            .new_text()
            .map(|text| text.unwrap_or_else(|| files.rows_text.clone()))
            .and_then(|text| rows_at(&text, &files.rows_path));
        for &k in &touched {
            if failed.contains_key(&k) {
                continue;
            }
            let mut found = Vec::new();
            let outcome = rows
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|rows| validate(&mut at, rows, k, settings, &mut found));
            notices.insert(k, found);
            match outcome {
                Ok(sheet) => {
                    prepared.insert(k, sheet);
                }
                Err(e) => {
                    let e = attribute(e, &at, ops, k, settings);
                    failed.insert(k, e);
                }
            }
        }

        // An operation that reaches a sheet that failed fails, and every
        // other sheet it reaches fails with it.
        let before = included.clone();
        loop {
            let mut grew = false;
            for op in ops {
                let i = op.index;
                if !included[i] {
                    continue;
                }
                let Some(&cause) = op_sheets[i].iter().find(|k| failed.contains_key(k)) else {
                    continue;
                };
                included[i] = false;
                let cause_error = failed[&cause].clone();
                if op_errors[i].is_none() {
                    op_errors[i] = Some(cause_error.clone());
                }
                for &k in &op_sheets[i] {
                    if let Entry::Vacant(slot) = failed.entry(k) {
                        slot.insert(held_back(&files, op, cause, &cause_error));
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }
        if included == before {
            for k in failed.keys() {
                prepared.remove(k);
            }
            break (at, prepared);
        }
    };

    touched.extend(failed.keys());
    let landed: Vec<usize> = touched
        .iter()
        .copied()
        .filter(|k| !failed.contains_key(k))
        .collect();

    // What would be written.
    let mut records: Vec<(PathBuf, String)> = Vec::new();
    if !landed.is_empty() {
        if let Some(text) = attempt.rows.new_text()? {
            records.push((files.rows_path.clone(), text));
        }
        for (t, file) in attempt.templates.iter().enumerate() {
            if let Some(Ok(file)) = file {
                if let Some(text) = file.new_text()? {
                    records.push((files.templates[t].path.clone(), text));
                }
            }
        }
    }
    let mut dwgs: Vec<(usize, PathBuf)> = Vec::new();
    for &k in &landed {
        let path = files.dwg(k);
        let unchanged = fs::read(&path).is_ok_and(|bytes| bytes == prepared[&k].bytes);
        if !unchanged {
            dwgs.push((k, path));
        }
    }

    // The mark as it stands after the change set.
    let cleared: Vec<usize> = fixes
        .iter()
        .copied()
        .filter(|k| landed.contains(k))
        .collect();
    let mut failures: Vec<MarkFailure> = mark
        .iter()
        .filter(|f| {
            !cleared
                .iter()
                .any(|&k| files.name(k).eq_ignore_ascii_case(&f.sheet))
        })
        .cloned()
        .collect();
    if !landed.is_empty() {
        for (&k, error) in &failed {
            let part: Vec<String> = ops
                .iter()
                .filter(|op| op_sheets[op.index].contains(&k))
                .map(|op| op.sent.to_string())
                .collect();
            if part.is_empty() {
                continue;
            }
            failures.push(MarkFailure {
                sheet: files.name(k).to_owned(),
                code: code_text(error),
                message: error.message.clone(),
                landed_on: files.names(&landed),
                operations: part,
            });
        }
    }

    // The records, the DWGs and the mark land together, or nothing does.
    if !request.dry_run {
        let mut replacements: Vec<Replacement> = records
            .iter()
            .map(|(path, text)| Replacement {
                path: path.clone(),
                bytes: Some(text.as_bytes().to_vec()),
                what: "the record",
            })
            .collect();
        replacements.extend(dwgs.iter().map(|(k, path)| Replacement {
            path: path.clone(),
            bytes: Some(prepared[k].bytes.clone()),
            what: "the DWG",
        }));
        if failures != mark {
            replacements.push(Replacement {
                path: files.folder.join(MARK_FILE),
                bytes: (!failures.is_empty()).then(|| mark_text(&failures).into_bytes()),
                what: "the set's inconsistent mark",
            });
        }
        if !replacements.is_empty() {
            commit(&replacements)?;
        }
    }

    let written: BTreeSet<usize> = if request.dry_run {
        BTreeSet::new()
    } else {
        dwgs.iter().map(|(k, _)| *k).collect()
    };
    let sheets: Vec<SheetChange> = touched
        .iter()
        .map(|&k| SheetChange {
            sheet: files.name(k).to_owned(),
            template: files.rows[k].template.clone(),
            ok: !failed.contains_key(&k),
            operations: (0..n).filter(|&i| op_sheets[i].contains(&k)).collect(),
            fixed: cleared.contains(&k),
            dwg_written: written.contains(&k),
            result: prepared.get(&k).map(|p| p.report.clone()),
            error: failed.get(&k).cloned(),
            notices: notices.remove(&k).unwrap_or_default(),
        })
        .collect();
    let operations: Vec<OperationOutcome> = ops
        .iter()
        .map(|op| {
            let i = op.index;
            let d = done[i].clone();
            OperationOutcome {
                index: i,
                op: op.spec.name.to_owned(),
                ok: included[i],
                message: d.as_ref().map_or_else(
                    || format!("{} {}", op.spec.name, target_words(op)),
                    |d| d.message.clone(),
                ),
                sheets: files.names(&op_sheets[i]),
                record: d.as_ref().map(|d| d.record.display().to_string()),
                id: d.as_ref().and_then(|d| d.id.clone()),
                before: d.as_ref().and_then(|d| d.before.clone()),
                after: d.as_ref().and_then(|d| d.after.clone()),
                row_value: d.as_ref().and_then(|d| d.row_value.clone()),
                ids_written: d.map(|d| d.ids_written).unwrap_or_default(),
                error: if included[i] {
                    None
                } else {
                    op_errors[i].clone()
                },
            }
        })
        .collect();
    let failed_names: Vec<&str> = failed.keys().map(|&k| files.name(k)).collect();
    let message = message(
        request.dry_run,
        &files.names(&landed),
        &failed_names,
        failed
            .values()
            .find(|e| e.code != ErrorCode::SharedEditFailed)
            .or_else(|| failed.values().next()),
        touched.is_empty(),
        failures.iter().map(|f| f.sheet.as_str()).collect(),
    );
    Ok(ChangeReport {
        set: files.folder.display().to_string(),
        units: files.units,
        dry_run: request.dry_run,
        message,
        landed: landed.len(),
        failed: failed.len(),
        sheets,
        operations,
        records: records
            .iter()
            .map(|(path, _)| path.display().to_string())
            .collect(),
        inconsistent: (!failures.is_empty()).then(|| mark_report(&files.folder, &failures)),
    })
}

/// Refuses a change set that names a sheet or a template the set does not
/// hold: there is no sheet to fail, so the change set itself is wrong.
fn precheck(files: &SetFiles, ops: &[Operation]) -> Result<(), Error> {
    for op in ops {
        let (sheet, field) = match &op.body {
            Body::Add {
                to: AddTo::Template(written),
                ..
            } => {
                if files.template(written).is_none() {
                    let templates: Vec<&str> =
                        files.templates.iter().map(|t| t.written.as_str()).collect();
                    return Err(op_error(
                        op.index,
                        op.spec.name,
                        ErrorCode::UnknownName,
                        format!(
                            "no sheet of the set is built from the template `{written}`; its \
                             templates: {}",
                            templates.join(", ")
                        ),
                        "template",
                    ));
                }
                continue;
            }
            Body::Add {
                to: AddTo::Sheet(sheet),
                ..
            } => (sheet, "sheet"),
            Body::Change { target, .. }
            | Body::Move { target, .. }
            | Body::Delete { target }
            | Body::SetAttribute { target, .. } => match target {
                Target::Id { sheet, .. } => (sheet, "target.id"),
                Target::Name {
                    sheet: Some(sheet), ..
                } => (sheet, "target.sheet"),
                Target::Name { sheet: None, .. } => continue,
            },
        };
        if files.sheet(sheet).is_none() {
            return Err(op_error(
                op.index,
                op.spec.name,
                ErrorCode::UnknownName,
                format!(
                    "the set holds no sheet `{sheet}`; its sheets: {}",
                    files.sheet_names()
                ),
                field,
            ));
        }
    }
    Ok(())
}

/// The rows of the sheets `named` under `fixes`, each of which the set's
/// mark holds.
fn fixes(
    files: &SetFiles,
    named: &[(String, usize)],
    mark: &[MarkFailure],
) -> Result<Vec<usize>, Error> {
    let mut marked: Vec<&str> = Vec::new();
    for failure in mark {
        if !marked.contains(&failure.sheet.as_str()) {
            marked.push(&failure.sheet);
        }
    }
    named
        .iter()
        .map(|(sheet, i)| {
            let in_mark = marked.iter().any(|m| m.eq_ignore_ascii_case(sheet));
            match files.sheet(sheet) {
                Some(k) if in_mark => Ok(k),
                _ => Err(Error::new(
                    ErrorCode::BadValue,
                    if marked.is_empty() {
                        format!(
                            "the change set fixes sheet `{sheet}`, but the set is not marked \
                             inconsistent, so there is nothing to fix; leave fixes out"
                        )
                    } else {
                        format!(
                            "the change set fixes sheet `{sheet}`, which the set's \
                             inconsistent mark does not hold; it holds {}",
                            quoted_list(&marked)
                        )
                    },
                )
                .at_field(format!("fixes[{i}]"))),
            }
        })
        .collect()
}

/// Applies one operation to the records in memory.
fn apply_op(at: &mut Attempt, op: &Operation) -> Applied {
    let files = at.files;
    let fail = |sheets: Vec<usize>, e: Error| Applied {
        sheets,
        outcome: Err(e),
        refuse: false,
    };
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    // An error from editing a record, as this operation's.
    let from_record = |e: Error| -> Error {
        let field = e.field.clone().unwrap_or_default();
        let own = field.starts_with("set") || field.starts_with("target");
        let message = match (&e.file, own || field.is_empty()) {
            (_, true) => e.message.clone(),
            (Some(file), false) => format!("{} (at `{field}` in {file})", e.message),
            (None, false) => format!("{} (at `{field}`)", e.message),
        };
        op_err(e.code, message, if own { &field } else { "" })
    };

    if let Body::Add { to, keys } = &op.body {
        let Kind::Add(object) = op.spec.kind else {
            unreachable!("an add is an add")
        };
        let (file, list, sheets, first) = match to {
            AddTo::Sheet(name) => {
                let k = files.sheet(name).expect("checked before");
                (FileRef::Rows, ObjectList::Row(k), vec![k], k)
            }
            AddTo::Template(written) => {
                let t = files.template(written).expect("checked before");
                let sheets = files.templates[t].sheets.clone();
                let first = sheets[0];
                (FileRef::Template(t), ObjectList::Template, sheets, first)
            }
        };
        let mut keys = keys.clone();
        if !keys.contains_key("id") {
            let mut taken: BTreeSet<String> = BTreeSet::new();
            for &k in &sheets {
                match at.objects(k) {
                    Ok(objects) => {
                        taken.extend(objects.into_iter().map(|o| o.3.id.to_lowercase()));
                    }
                    Err(e) => return fail(sheets.clone(), e),
                }
            }
            let id = (1..)
                .map(|n| format!("{object}-{n}"))
                .find(|id| !taken.contains(id))
                .expect("a free number");
            keys.insert("id".into(), serde_json::Value::String(id));
        }
        let record = match at.file(file) {
            Ok(record) => record,
            Err(e) => return fail(sheets, e),
        };
        let index = match record.add(list, object, &keys, op.index) {
            Ok(index) => index,
            Err(e) => return fail(sheets, from_record(e)),
        };
        let listed = &record.objects(list)[index];
        let id = format!("{}/{}", files.name(first), listed.id);
        let after = record.object_text(list, index);
        let path = record.path.clone();
        let place = match file {
            FileRef::Rows => format!("to sheet `{}`, in its row", files.name(first)),
            FileRef::Template(t) => format!(
                "to the template `{}`, so it is on {}",
                files.templates[t].written,
                every_sheet(&files.names(&sheets))
            ),
        };
        return Applied {
            outcome: Ok(Done {
                record: path,
                id: Some(id.clone()),
                before: None,
                after: Some(after),
                row_value: None,
                ids_written: Vec::new(),
                message: format!("adds {} {object}, `{id}`, {place}", article(object)),
            }),
            sheets,
            refuse: false,
        };
    }

    let target = match &op.body {
        Body::Change { target, .. }
        | Body::Move { target, .. }
        | Body::Delete { target }
        | Body::SetAttribute { target, .. } => target,
        Body::Add { .. } => unreachable!("handled above"),
    };
    let found = match resolve(at, target, op) {
        Ok(found) => found,
        Err(applied) => return *applied,
    };
    let Found {
        row: k,
        file,
        list,
        index,
        id,
    } = found;
    let reach = at.reach(file, k);
    let units = files.units;
    let record = match at.file(file) {
        Ok(record) => record,
        Err(e) => return fail(reach, e),
    };
    let path = record.path.clone();
    let before = record.object_text(list, index);
    let template_note = match file {
        FileRef::Template(t) if reach.len() > 1 => format!(
            ", in the template `{}`, so on {}",
            files.templates[t].written,
            every_sheet(&files.names(&reach))
        ),
        FileRef::Template(t) => format!(", in the template `{}`", files.templates[t].written),
        FileRef::Rows => String::new(),
    };
    let mut done = Done {
        record: path,
        id: Some(id.clone()),
        before: Some(before),
        after: None,
        row_value: None,
        ids_written: Vec::new(),
        message: String::new(),
    };
    let mut sheets = reach.clone();
    let mut from_row: Option<(String, String)> = None;
    match &op.body {
        Body::Change { set, .. } => match record.change(list, index, set, op.index) {
            Ok(changed) => {
                let keys: Vec<&str> = set.keys().map(String::as_str).collect();
                done.message = format!("sets {} of `{id}`{template_note}", keys.join(", "));
                if !changed.edited {
                    sheets = vec![k];
                }
                from_row = changed.from_row;
            }
            Err(e) => return fail(reach, from_record(e)),
        },
        Body::Move { by, .. } => match record.move_by(list, index, *by, units, op.index) {
            Ok(()) => {
                let by: Vec<String> = by
                    .iter()
                    .map(|d| crate::description::number_text(*d))
                    .collect();
                done.message = format!(
                    "moves `{id}` by [{}] ({units}){template_note}",
                    by.join(", ")
                );
            }
            Err(e) => return fail(reach, from_record(e)),
        },
        Body::Delete { .. } => {
            let kept = record.delete(list, index, op.index);
            done.ids_written = kept
                .iter()
                .flat_map(|place| {
                    reach
                        .iter()
                        .map(move |&s| format!("{}/{place}", files.name(s)))
                })
                .collect();
            done.message = format!("deletes `{id}`{template_note}");
            return Applied {
                sheets: reach,
                outcome: Ok(done),
                refuse: false,
            };
        }
        Body::SetAttribute { tag, value, .. } => {
            match record.set_attribute(list, index, tag, value, op.index) {
                Ok(AttributeSet::Written) => {
                    done.message =
                        format!("sets the attribute {tag} of `{id}` to `{value}`{template_note}");
                }
                Ok(AttributeSet::FromRow(name)) => {
                    sheets = vec![k];
                    from_row = Some((name, value.clone()));
                }
                Err(e) => {
                    let field = if e.code == ErrorCode::BadValue {
                        "target"
                    } else {
                        ""
                    };
                    return fail(reach, op_err(e.code, e.message, field));
                }
            }
        }
        Body::Add { .. } => unreachable!("handled above"),
    }
    done.after = Some(record.object_text(list, index));
    if done.after == done.before {
        done.after = None;
    }
    if let Some((name, text)) = from_row {
        let was = at.rows.row_value(k, &name).unwrap_or_default();
        if let Err(e) = at.rows.set_row_value(k, &name, &text) {
            return fail(vec![k], from_record(e));
        }
        at.values.retain(|(r, n, _)| !(*r == k && *n == name));
        at.values.push((k, name.clone(), op.index));
        let t = files.template_of[k];
        let fills = match at.template(t) {
            Ok(template) => {
                let listed = template.objects(ObjectList::Template);
                template
                    .row_value_places(&name)
                    .into_iter()
                    .map(|(i, place)| Fill {
                        id: format!("{}/{}", files.name(k), listed[i].id),
                        at: place,
                    })
                    .collect::<Vec<_>>()
            }
            Err(_) => Vec::new(),
        };
        let places: Vec<String> = fills
            .iter()
            .map(|f| format!("{} of `{}`", f.at, f.id))
            .collect();
        let set_what = match &op.body {
            Body::SetAttribute { tag, .. } => format!("the attribute {tag} of `{id}`"),
            _ => format!("the contents of `{id}`"),
        };
        let row_message = format!(
            "sets sheet `{}`'s row value {name} to `{text}`, since its template takes {set_what} \
             from the row; the value fills {}",
            files.name(k),
            places.join(" and ")
        );
        done.message = if done.message.is_empty() {
            row_message
        } else {
            format!("{}; and {row_message}", done.message)
        };
        if !sheets.contains(&k) {
            sheets.push(k);
        }
        if sheets == vec![k] && matches!(file, FileRef::Template(_)) {
            done.record = files.rows_path.clone();
        }
        done.row_value = Some(RowValueSet {
            sheet: files.name(k).to_owned(),
            value: name,
            was,
            now: text,
            fills,
        });
    }
    Applied {
        sheets,
        outcome: Ok(done),
        refuse: false,
    }
}

/// The object `target` names, as the records in memory hold it now.
fn resolve(at: &mut Attempt, target: &Target, op: &Operation) -> Result<Found, Box<Applied>> {
    let files = at.files;
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    let objects_of = |at: &mut Attempt, k: usize| {
        at.objects(k).map_err(|e| {
            Box::new(Applied {
                sheets: vec![k],
                outcome: Err(e),
                refuse: false,
            })
        })
    };
    match target {
        Target::Id { sheet, id } => {
            let k = files.sheet(sheet).expect("checked before");
            let objects = objects_of(at, k)?;
            match objects.iter().find(|o| o.3.id.eq_ignore_ascii_case(id)) {
                Some((file, list, index, listed)) => Ok(Found {
                    row: k,
                    file: *file,
                    list: *list,
                    index: *index,
                    id: format!("{}/{}", files.name(k), listed.id),
                }),
                None => {
                    let ids: Vec<String> = objects
                        .iter()
                        .map(|o| format!("{}/{}", files.name(k), o.3.id))
                        .collect();
                    Err(Box::new(Applied {
                        sheets: vec![k],
                        outcome: Err(op_err(
                            ErrorCode::UnknownName,
                            format!(
                                "no object on sheet `{}` has the ID `{}/{id}`; its IDs: {}",
                                files.name(k),
                                files.name(k),
                                listed(&ids)
                            ),
                            "target.id",
                        )),
                        refuse: false,
                    }))
                }
            }
        }
        Target::Name { name, sheet } => {
            let folded = name.to_lowercase();
            let rows: Vec<usize> = match sheet {
                Some(sheet) => vec![files.sheet(sheet).expect("checked before")],
                None => (0..files.rows.len()).collect(),
            };
            let mut matches = Vec::new();
            let mut names_on = Vec::new();
            for &k in &rows {
                let objects = objects_of(at, k)?;
                for (file, list, index, listed) in objects {
                    if let Some(n) = &listed.name {
                        if n.to_lowercase() == folded {
                            matches.push(Found {
                                row: k,
                                file,
                                list,
                                index,
                                id: format!("{}/{}", files.name(k), listed.id),
                            });
                        } else if sheet.is_some() {
                            names_on.push(n.clone());
                        }
                    }
                }
            }
            match (matches.len(), sheet) {
                (1, _) => Ok(matches.pop().expect("one")),
                (0, Some(_)) => {
                    let k = rows[0];
                    Err(Box::new(Applied {
                        sheets: vec![k],
                        outcome: Err(op_err(
                            ErrorCode::UnknownName,
                            format!(
                                "no object on sheet `{}` has the name `{name}`; its names: {}",
                                files.name(k),
                                if names_on.is_empty() {
                                    "none".to_owned()
                                } else {
                                    listed(&names_on)
                                }
                            ),
                            "target.name",
                        )),
                        refuse: false,
                    }))
                }
                (0, None) => Err(Box::new(Applied {
                    sheets: Vec::new(),
                    outcome: Err(op_err(
                        ErrorCode::UnknownName,
                        format!(
                            "no object on any sheet of the set has the name `{name}`; name it by \
                             its ID, or give its sheet, one of {}",
                            files.sheet_names()
                        ),
                        "target.name",
                    )),
                    refuse: true,
                })),
                (_, _) => {
                    let on: Vec<&str> = matches.iter().map(|m| files.name(m.row)).collect();
                    Err(Box::new(Applied {
                        sheets: Vec::new(),
                        outcome: Err(op_err(
                            ErrorCode::BadValue,
                            format!(
                                "the name `{name}` is on more than one sheet, {}; give the sheet \
                                 too, one of {}",
                                on.join(", "),
                                on.join(", ")
                            ),
                            "target",
                        )),
                        refuse: true,
                    }))
                }
            }
        }
    }
}

/// Reads sheet `k` from the records in memory and builds it, adding what
/// reading noticed to `notices`.
fn validate(
    at: &mut Attempt,
    rows: &[Row],
    k: usize,
    settings: &Settings,
    notices: &mut Vec<Notice>,
) -> Result<PreparedSheet, Error> {
    let files = at.files;
    let t = files.template_of[k];
    let template = &files.templates[t];
    let text = match &at.templates[t] {
        Some(Ok(file)) => match file.new_text()? {
            Some(text) => text,
            None => template.text.clone()?,
        },
        Some(Err(e)) => return Err(e.clone()),
        None => template.text.clone()?,
    };
    let (record, _) = template_at(
        &text,
        &template.path,
        &rows[k],
        &files.rows_path,
        files.units,
        notices,
    )?;
    let dwg = files.dwg(k);
    writable(&dwg)?;
    prepare_sheet(&record, &template.path, &dwg, true, settings)
}

/// Refuses a DWG that is there but cannot be written, such as one open in
/// AutoCAD on Windows, before any record is written, so the sheet fails
/// whole rather than its records landing without its DWG.
fn writable(dwg: &Path) -> Result<(), Error> {
    if !dwg.exists() {
        return Ok(());
    }
    OpenOptions::new()
        .append(true)
        .open(dwg)
        .map(|_| ())
        .map_err(|e| {
            Error::new(
                ErrorCode::WriteFailed,
                format!(
                    "the sheet's DWG cannot be written ({e}); it may be open in AutoCAD. Close \
                     it and send the change set again"
                ),
            )
            .in_file(dwg)
        })
}

/// Places an error from reading sheet `k` with the operation that caused
/// it, when one did: the error names the operation, and its field is in the
/// operation. An error no operation caused is the sheet's own.
fn attribute(
    error: Error,
    at: &Attempt,
    ops: &[Operation],
    k: usize,
    settings: &Settings,
) -> Error {
    let files = at.files;
    let t = files.template_of[k];
    let in_rows = error.file.as_deref().map(Path::new) == Some(files.rows_path.as_path());
    let in_template =
        error.file.as_deref().map(Path::new) == Some(files.templates[t].path.as_path());
    let field = error.field.clone().unwrap_or_default();
    let mut cause: Option<(usize, String)> = None;
    if in_rows {
        if let Some((row, index, rest)) = split_place(&field).filter(|p| p.0 != usize::MAX) {
            if let Some(i) = at.rows.blame(ObjectList::Row(row), index, rest) {
                cause = Some((i, rest.to_owned()));
            }
        } else if let Some(value) = field.strip_prefix(&format!("rows[{}].values.", k)) {
            if let Some((_, _, i)) = at.values.iter().find(|(r, n, _)| *r == k && n == value) {
                cause = Some((*i, String::new()));
            }
        }
    } else if in_template {
        if let Some((usize::MAX, index, rest)) = split_place(&field) {
            if let Some(Some(Ok(template))) = at.templates.get(t) {
                if let Some(i) = template.blame(ObjectList::Template, index, rest) {
                    cause = Some((i, rest.to_owned()));
                }
            }
        }
    }
    if let Some((i, rest)) = cause {
        let op = &ops[i];
        let suffix = op_field(op, &rest, error.code);
        return Error {
            message: format!(
                "operation {i} (`{}`) on sheet `{}`: {}",
                op.spec.name,
                files.name(k),
                error.message
            ),
            field: Some(if suffix.is_empty() {
                format!("operations[{i}]")
            } else {
                format!("operations[{i}].{suffix}")
            }),
            line: None,
            column: None,
            file: None,
            ..error
        };
    }
    // An error the sheet had before the change set is reported as it is,
    // placed in its file as it was.
    if let Ok(Err(was)) = before_change(files, k, settings) {
        if was.code == error.code {
            return Error {
                message: format!(
                    "sheet `{}` cannot be built even before this change set: {}",
                    files.name(k),
                    was.message
                ),
                ..was
            };
        }
    }
    let part: Vec<String> = ops
        .iter()
        .filter(|op| at_reaches(op, k, at))
        .map(|op| op.index.to_string())
        .collect();
    Error {
        message: format!(
            "once operations {} of the change set are applied, sheet `{}` cannot be built: {}",
            part.join(", "),
            files.name(k),
            error.message
        ),
        line: None,
        column: None,
        ..error
    }
}

/// Whether operation `op` reaches sheet `k`, as far as can be told without
/// applying it: an add names its sheet or template, and an edit its
/// target's sheet.
fn at_reaches(op: &Operation, k: usize, at: &Attempt) -> bool {
    let files = at.files;
    match &op.body {
        Body::Add {
            to: AddTo::Sheet(sheet),
            ..
        } => files.sheet(sheet) == Some(k),
        Body::Add {
            to: AddTo::Template(written),
            ..
        } => files.template(written) == Some(files.template_of[k]),
        Body::Change { target, .. }
        | Body::Move { target, .. }
        | Body::Delete { target }
        | Body::SetAttribute { target, .. } => match target {
            Target::Id { sheet, .. }
            | Target::Name {
                sheet: Some(sheet), ..
            } => files
                .sheet(sheet)
                .is_some_and(|s| s == k || files.template_of[s] == files.template_of[k]),
            Target::Name { sheet: None, .. } => true,
        },
    }
}

/// Whether sheet `k` builds from the records as they are on disk: the
/// outcome, or an error when even that cannot be told.
fn before_change(files: &SetFiles, k: usize, settings: &Settings) -> Result<Result<(), Error>, ()> {
    let template = &files.templates[files.template_of[k]];
    let text = template.text.clone().map_err(|_| ())?;
    let outcome = template_at(
        &text,
        &template.path,
        &files.rows[k],
        &files.rows_path,
        files.units,
        &mut Vec::new(),
    )
    .and_then(|(record, _)| {
        let dwg = files.dwg(k);
        writable(&dwg)?;
        prepare_sheet(&record, &template.path, &dwg, true, settings).map(|_| ())
    });
    Ok(outcome)
}

/// Splits a place in a record, `objects[3].line.to` or
/// `rows[1].objects[0].line.to`, into its row (`usize::MAX` for a
/// template's own objects), the object's place in its list and what
/// follows, `line.to`.
fn split_place(field: &str) -> Option<(usize, usize, &str)> {
    fn index(text: &str) -> Option<(usize, &str)> {
        let rest = text.strip_prefix('[')?;
        let end = rest.find(']')?;
        Some((rest[..end].parse().ok()?, &rest[end + 1..]))
    }
    let (row, rest) = match field.strip_prefix("rows") {
        Some(rest) => {
            let (row, rest) = index(rest)?;
            (row, rest.strip_prefix(".objects")?)
        }
        None => (usize::MAX, field.strip_prefix("objects")?),
    };
    let (object, rest) = index(rest)?;
    Some((row, object, rest.strip_prefix('.').unwrap_or(rest)))
}

/// The field within operation `op` that `rest`, a place within the object
/// it edits, such as `line.to[1]`, is about.
fn op_field(op: &Operation, rest: &str, code: ErrorCode) -> String {
    let within = rest.split_once('.').map_or("", |(_, within)| within);
    match &op.body {
        Body::Add { .. } => within.to_owned(),
        Body::Change { set, .. } => {
            let key = within.split(['.', '[']).next().unwrap_or("");
            if set.contains_key(key) {
                format!("set.{within}")
            } else if rest.is_empty() && set.contains_key("contents") {
                "set.contents".to_owned()
            } else {
                "set".to_owned()
            }
        }
        Body::Move { .. } => "by".to_owned(),
        Body::Delete { .. } => String::new(),
        Body::SetAttribute { .. } => {
            if code == ErrorCode::UnknownName {
                "tag".to_owned()
            } else {
                "value".to_owned()
            }
        }
    }
}

/// The error of a sheet that failed only because operation `op`, which it
/// shares with sheet `cause`, could not land there.
fn held_back(files: &SetFiles, op: &Operation, cause: usize, why: &Error) -> Error {
    Error::new(
        ErrorCode::SharedEditFailed,
        format!(
            "operation {} (`{}`) edits a template this sheet is built from, and a template edit \
             lands only if every sheet built from the template takes its whole part; sheet `{}` \
             could not, so it lands on none of them and this sheet was left as it was: {}",
            op.index,
            op.spec.name,
            files.name(cause),
            why.message
        ),
    )
    .at_field(format!("operations[{}]", op.index))
}

/// What a change set did, in words.
fn message(
    dry_run: bool,
    landed: &[String],
    failed: &[&str],
    first_error: Option<&Error>,
    nothing: bool,
    marked: Vec<&str>,
) -> String {
    let landed_refs: Vec<&str> = landed.iter().map(String::as_str).collect();
    let mut text = if nothing {
        "the change set touches no sheet, and nothing was written".to_owned()
    } else if failed.is_empty() {
        format!(
            "the change set {} on every sheet it touches: {}",
            if dry_run { "would land" } else { "landed" },
            quoted_list(&landed_refs)
        )
    } else if landed.is_empty() {
        format!(
            "the change set failed on every sheet it touches, {}, and nothing {}: {}",
            quoted_list(failed),
            if dry_run {
                "would be written"
            } else {
                "was written"
            },
            first_error.map_or("", |e| e.message.as_str())
        )
    } else {
        format!(
            "the change set {} on {} and failed on {}, which {} left as {}; each sheet's \
             outcome is under `sheets`: {}",
            if dry_run { "would land" } else { "landed" },
            quoted_list(&landed_refs),
            quoted_list(failed),
            match (dry_run, failed.len()) {
                (true, 1) => "would be",
                (true, _) => "would be",
                (false, 1) => "was",
                (false, _) => "were",
            },
            if failed.len() == 1 {
                "it was"
            } else {
                "they were"
            },
            first_error.map_or("", |e| e.message.as_str())
        )
    };
    if !marked.is_empty() {
        let mut sheets: Vec<&str> = Vec::new();
        for sheet in marked {
            if !sheets.contains(&sheet) {
                sheets.push(sheet);
            }
        }
        text.push_str(&format!(
            ". The set {} inconsistent until {} {} fixed by a change set that names {} under \
             `fixes`",
            if dry_run { "would be" } else { "is" },
            quoted_list(&sheets),
            if sheets.len() == 1 { "is" } else { "are" },
            if sheets.len() == 1 { "it" } else { "them" },
        ));
    }
    if dry_run {
        format!("a dry run, which wrote nothing: {text}")
    } else {
        text
    }
}

fn target_words(op: &Operation) -> String {
    match &op.body {
        Body::Change { target, .. }
        | Body::Move { target, .. }
        | Body::Delete { target }
        | Body::SetAttribute { target, .. } => target.words(),
        Body::Add {
            to: AddTo::Sheet(s),
            ..
        } => format!("to sheet `{s}`"),
        Body::Add {
            to: AddTo::Template(t),
            ..
        } => format!("to the template `{t}`"),
    }
}

/// `ids` listed for an error, at most [`MOST_LISTED`] of them.
fn listed(ids: &[String]) -> String {
    if ids.len() <= MOST_LISTED {
        ids.join(", ")
    } else {
        format!(
            "{}, and {} more; `agentdrafter query find` lists them all",
            ids[..MOST_LISTED].join(", "),
            ids.len() - MOST_LISTED
        )
    }
}

fn every_sheet(names: &[String]) -> String {
    if names.len() == 1 {
        format!("sheet `{}`", names[0])
    } else {
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        format!("every sheet built from it: {}", quoted_list(&refs))
    }
}

fn article(object: &str) -> &'static str {
    if object.starts_with(['a', 'e', 'i', 'o', 'u']) || object == "mtext" {
        "an"
    } else {
        "a"
    }
}

/// An error's code as it is written in JSON, such as `unknown_name`.
fn code_text(error: &Error) -> String {
    serde_json::to_value(error.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}
