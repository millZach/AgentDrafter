//! Applies a change set to a drawing set's own files.
//!
//! The operations are applied in order to the records held in memory: the
//! set's rows file and the templates they name. Every sheet an operation
//! reaches is then read from the edited records and built into a DWG in
//! memory, as `agentdrafter build` would build it. A sheet that cannot take
//! its part fails, and so does every operation that reaches it; a template
//! edit reaches every sheet built from the template, so it fails with any
//! of them, and takes those sheets' other operations with it (Zach's
//! decision of 2026-11-11). An edit to a block library reaches every sheet
//! whose template names the library, in the same way. The operations left are applied again from the
//! records as they were, until every sheet they reach takes its part. Only
//! then is anything written, all of it together or none of it, with the
//! change set's entry in the journal (`commit.rs`, `journal.rs`).

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::commit::Replacement;
use super::journal::{commit_change_set, HowChanged, Landing, ObjectChanged, SheetFailed};
use super::operations::{
    op_error, read_change_set, AddTo, BlockOp, Body, Import, Kind, Operation, Table, TableOp,
    Target,
};
use super::MARK_FILE;
use super::{
    mark_report, quoted_list, read_mark, relative_to, AttributeRemoved, ChangeReport,
    ChangeRequest, Fill, OperationOutcome, RowValueSet, SheetChange,
};
use crate::build::{is_same_file, prepare_sheet, PreparedSheet};
use crate::description::blocks::tag_in_capitals;
use crate::description::{
    article, library_named, mark_text, one_name, read_file, read_set_settings, rows_at,
    template_at, template_in, template_path, AttributeSet, Disk, GivenValue, Listed, MarkFailure,
    ObjectList, ReadFiles, RecordFile, Row, ID_SEPARATOR,
};
use crate::dwg::BuildMark;
use crate::error::{Error, ErrorCode};
use crate::import::{drawing_unit, is_dxf, plan, write_plan, BlocksImported, Existing};
use crate::set::{set_in_mark, Notice, SET_SETTINGS_FILE};
use crate::settings::Settings;
use crate::units::Unit;

/// The most IDs an error lists as the choices on a sheet.
const MOST_LISTED: usize = 60;

/// A drawing set's files as they are on disk before the change set.
struct SetFiles {
    folder: PathBuf,
    units: Unit,
    output: PathBuf,
    /// The build mark the set's DWGs carry, naming the set as `build`
    /// names it.
    mark: BuildMark,
    rows_path: PathBuf,
    rows_text: String,
    rows: Vec<Row>,
    templates: Vec<TemplateFile>,
    /// For each row, its template's place in `templates`.
    template_of: Vec<usize>,
    /// The block libraries the templates name, each once.
    libraries: Vec<LibraryOfSet>,
    /// For each template, the place in `libraries` of the library it names.
    library_of: Vec<Option<usize>>,
}

/// A block library the set's templates name.
struct LibraryOfSet {
    /// Its path relative to the set's folder, with `/` between folders,
    /// as the first template that names it reaches it, such as
    /// `blocks.yaml`: how a block operation names it.
    written: String,
    /// Every path the templates reach it by, relative to the set's folder.
    /// More than one when templates name one file by different paths,
    /// such as a link and the file it links to.
    spellings: Vec<String>,
    /// Its path as the first template that names it reaches it, joined
    /// onto that template's folder as the reader joins it.
    path: PathBuf,
    /// The templates that name it.
    templates: Vec<usize>,
    /// The rows of the sheets built from those templates, in order.
    sheets: Vec<usize>,
}

struct TemplateFile {
    /// Its path as the first row that names it writes it.
    written: String,
    /// Every path the rows reach it by, as they write them, each with the
    /// first sheet that writes it. More than one when rows name one file by
    /// different paths, such as a link and the file it links to.
    paths: Vec<(String, usize)>,
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
                Some(t) => {
                    if !templates[t].paths.iter().any(|(p, _)| *p == row.template) {
                        templates[t].paths.push((row.template.clone(), k));
                    }
                    t
                }
                None => {
                    templates.push(TemplateFile {
                        written: row.template.clone(),
                        paths: vec![(row.template.clone(), k)],
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
        let mut libraries: Vec<LibraryOfSet> = Vec::new();
        let mut library_of = Vec::with_capacity(templates.len());
        for (t, template) in templates.iter().enumerate() {
            let Some(named) = template.text.as_ref().ok().and_then(|t| library_named(t)) else {
                library_of.push(None);
                continue;
            };
            let path = template.path.parent().unwrap_or(Path::new("")).join(&named);
            let written = relative_to(folder, &path);
            let same = libraries
                .iter()
                .position(|l| l.spellings.contains(&written) || one_file(&l.path, &path));
            let l = match same {
                Some(l) => {
                    if !libraries[l].spellings.contains(&written) {
                        libraries[l].spellings.push(written);
                    }
                    l
                }
                None => {
                    libraries.push(LibraryOfSet {
                        written: written.clone(),
                        spellings: vec![written],
                        path,
                        templates: Vec::new(),
                        sheets: Vec::new(),
                    });
                    libraries.len() - 1
                }
            };
            libraries[l].templates.push(t);
            libraries[l].sheets.extend(&template.sheets);
            libraries[l].sheets.sort_unstable();
            library_of.push(Some(l));
        }
        Ok(Self {
            folder: folder.to_owned(),
            units: settings.units,
            output: folder.join(&settings.output),
            mark: BuildMark {
                set: set_in_mark(folder, &settings.output),
            },
            rows_path,
            rows_text,
            rows,
            templates,
            template_of,
            libraries,
            library_of,
        })
    }

    /// The block library a block operation names by `given`, its path in
    /// the set, or, when it gives none, the one library every template
    /// names: its place in `libraries`, or the code and message of why
    /// there is none.
    fn library(&self, given: Option<&str>) -> Result<usize, (ErrorCode, String)> {
        let listed = || {
            self.libraries
                .iter()
                .map(|l| format!("`{}`", l.written))
                .collect::<Vec<_>>()
                .join(", ")
        };
        if self.libraries.is_empty() {
            return Err((
                ErrorCode::UnknownName,
                "no template of the set names a block library, so there is none to edit; a \
                 template names its library under block_library"
                    .to_owned(),
            ));
        }
        match given {
            Some(given) => {
                let wanted = self.folder.join(given);
                let written = relative_to(&self.folder, &wanted);
                self.libraries
                    .iter()
                    .position(|l| l.spellings.contains(&written) || one_file(&l.path, &wanted))
                    .ok_or_else(|| {
                        (
                            ErrorCode::UnknownName,
                            format!(
                                "no template of the set names the block library `{given}`; the \
                                 set's block libraries, by their paths in the set: {}",
                                listed()
                            ),
                        )
                    })
            }
            None if self.libraries.len() == 1 => Ok(0),
            None => Err((
                ErrorCode::MissingKey,
                format!(
                    "missing key `library`: the set's templates name more than one block \
                     library, so name the one to edit by its path in the set: {}",
                    listed()
                ),
            )),
        }
    }

    /// Whether `path` is the block library `l`.
    fn is_library(&self, l: usize, path: &Path) -> bool {
        let library = &self.libraries[l];
        library.spellings.contains(&relative_to(&self.folder, path))
            || one_file(&library.path, path)
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
    libraries: Vec<Option<Result<RecordFile, Error>>>,
    /// The operation that last wrote each block of a library: the
    /// library, the block's name and the operation.
    blocks: Vec<(usize, String, usize)>,
}

impl<'a> Attempt<'a> {
    fn new(files: &'a SetFiles) -> Result<Self, Error> {
        Ok(Self {
            files,
            rows: RecordFile::from_text(&files.rows_path, files.rows_text.clone())?,
            templates: files.templates.iter().map(|_| None).collect(),
            values: Vec::new(),
            libraries: files.libraries.iter().map(|_| None).collect(),
            blocks: Vec::new(),
        })
    }

    fn library(&mut self, l: usize) -> Result<&mut RecordFile, Error> {
        let path = &self.files.libraries[l].path;
        let slot = self.libraries[l].get_or_insert_with(|| {
            read_file(path, "the block library").and_then(|text| RecordFile::from_text(path, text))
        });
        slot.as_mut().map_err(|e| e.clone())
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
    /// The inserts a block operation brought into line.
    inserts: Vec<String>,
    attributes_removed: Vec<AttributeRemoved>,
    /// What an import_blocks imported.
    imported: Option<BlocksImported>,
    message: String,
}

/// Each object the operations that landed added, changed, moved or
/// deleted, on each sheet they reached, for the change set's journal entry:
/// an object of a template on every sheet built from it, and each place a
/// row's value fills.
fn objects_changed(
    ops: &[Operation],
    included: &[bool],
    done: &[Option<Done>],
    op_sheets: &[Vec<usize>],
    files: &SetFiles,
) -> Vec<ObjectChanged> {
    let mut objects: Vec<ObjectChanged> = Vec::new();
    let mut add = |id: String, how: HowChanged| {
        match objects.iter_mut().find(|o| o.id == id) {
            // An object added, then changed, was added; one changed, then
            // deleted, was deleted.
            Some(seen) if seen.how == HowChanged::Added && how == HowChanged::Changed => {}
            Some(seen) => seen.how = how,
            None => objects.push(ObjectChanged { id, how }),
        }
    };
    for op in ops {
        let i = op.index;
        let Some(d) = done[i].as_ref().filter(|_| included[i]) else {
            continue;
        };
        let how = match op.spec.name {
            "delete" => HowChanged::Deleted,
            name if name.starts_with("add_") || name == "insert_block" => HowChanged::Added,
            _ => HowChanged::Changed,
        };
        if let Some(id) = &d.id {
            // The ID on the sheet named; the same object on every sheet the
            // operation reaches carries it after that sheet's name.
            let local = id.split_once(ID_SEPARATOR).map_or(id.as_str(), |(_, l)| l);
            for &k in &op_sheets[i] {
                add(format!("{}{ID_SEPARATOR}{local}", files.name(k)), how);
            }
        }
        for fill in d.row_value.iter().flat_map(|row| &row.fills) {
            add(fill.id.clone(), HowChanged::Changed);
        }
        // Each insert of a block an operation redefined or brought into
        // line draws as the block now does.
        for id in &d.inserts {
            add(id.clone(), HowChanged::Changed);
        }
    }
    objects
}

/// Applies the change set. `attempted` is given what it did to its sheets
/// before its files are put in place, so a commit that fails is recorded as
/// a failed attempt with the sheets it touched.
pub(super) fn apply(
    request: &ChangeRequest,
    settings: &Settings,
    attempted: &mut Option<Landing>,
) -> Result<ChangeReport, Error> {
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
        for (l, file) in attempt.libraries.iter().enumerate() {
            if let Some(Ok(file)) = file {
                if let Some(text) = file.new_text()? {
                    records.push((files.libraries[l].path.clone(), text));
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

    // The records, the DWGs and the mark land together, with the change
    // set's undo state and its entry in the journal, or nothing does. A
    // change set that failed on every sheet it touched changed nothing, and
    // is recorded as a failed attempt.
    let mut journal_entry = None;
    if !request.dry_run && (!landed.is_empty() || failed.is_empty()) {
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
        let landing = Landing {
            objects: objects_changed(ops, &included, &done, &op_sheets, &files),
            landed_on: files.names(&landed),
            failed_on: failed
                .iter()
                .map(|(&k, e)| SheetFailed {
                    sheet: files.name(k).to_owned(),
                    code: code_text(e),
                    message: e.message.clone(),
                })
                .collect(),
        };
        *attempted = Some(landing.clone());
        journal_entry = Some(commit_change_set(
            &files.folder,
            &request.change_set,
            landing,
            replacements,
        )?);
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
                ids_written: d
                    .as_ref()
                    .map(|d| d.ids_written.clone())
                    .unwrap_or_default(),
                inserts: d.as_ref().map(|d| d.inserts.clone()).unwrap_or_default(),
                imported: d.as_ref().and_then(|d| d.imported.clone()),
                attributes_removed: d.map(|d| d.attributes_removed).unwrap_or_default(),
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
        journal_entry,
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
        checks: None,
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
            }
            | Body::Table {
                to: AddTo::Sheet(sheet),
                ..
            } => (sheet, "sheet"),
            Body::Table {
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
            Body::Block { library, .. } => {
                if let Err((code, message)) = files.library(library.as_deref()) {
                    return Err(op_error(op.index, op.spec.name, code, message, "library"));
                }
                continue;
            }
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
        if let Some(refusal) = many_paths(files, file, op, "template") {
            return refusal;
        }
        let mut keys = keys.clone();
        // An `id` sent as `null` is a bare null, which leaves the id out
        // (rule 8 of the YAML subset), so the object is given one, as an
        // object added without an `id` is.
        if keys.get("id").is_none_or(serde_json::Value::is_null) {
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
                inserts: Vec::new(),
                attributes_removed: Vec::new(),
                imported: None,
                message: format!("adds {} {object}, `{id}`, {place}", article(object)),
            }),
            sheets,
            refuse: false,
        };
    }

    if let Body::Table {
        op: table_op,
        table,
        to,
        name,
        keys,
        new_name,
    } = &op.body
    {
        return apply_table(
            at,
            op,
            *table_op,
            *table,
            to,
            name,
            keys,
            new_name.as_deref(),
        );
    }

    if let Body::Block {
        library,
        import: Some(import),
        ..
    } = &op.body
    {
        return apply_import(at, op, library.as_deref(), import);
    }

    if let Body::Block {
        op: block_op,
        library,
        name,
        base_point,
        objects,
        ..
    } = &op.body
    {
        return apply_block(
            at,
            op,
            *block_op,
            library.as_deref(),
            name.as_deref(),
            base_point.as_ref(),
            objects.as_deref().unwrap_or_default(),
        );
    }

    let target = match &op.body {
        Body::Change { target, .. }
        | Body::Move { target, .. }
        | Body::Delete { target }
        | Body::SetAttribute { target, .. } => target,
        Body::Add { .. } | Body::Table { .. } | Body::Block { .. } => {
            unreachable!("handled above")
        }
    };
    let found = match resolve(at, target, op) {
        Ok(found) => found,
        Err(applied) => return *applied,
    };
    // An object on a locked layer is not edited, as AutoCAD will not edit
    // it (Zach, 2026-11-13); adding one onto it is allowed. A move or a
    // delete always edits it; a change or an attribute is judged once it is
    // known whether it edits the object or only a row's value.
    let locked = |id: &str, layer: &str| {
        op_err(
            ErrorCode::Locked,
            format!(
                "`{id}` sits on layer `{layer}`, which is locked; unlock it first with \
                 change_layer, set: {{\"locked\": false}}, in this change set or an earlier one"
            ),
            "target",
        )
    };
    let target_locked = locked_layer(at, &found);
    if let (Some(layer), Body::Move { .. } | Body::Delete { .. }) = (&target_locked, &op.body) {
        let reach = at.reach(found.file, found.row);
        return fail(reach, locked(&found.id, layer));
    }
    let Found {
        row: k,
        file,
        list,
        index,
        id,
    } = found;
    let reach = at.reach(file, k);
    let units = files.units;
    // The block a set_attribute's insert places, with the attributes it
    // carries as its library holds it now, so that the operation can tell
    // which of a tag the block defines more than once it sets.
    let carried = match &op.body {
        Body::SetAttribute { .. } => {
            let block = at
                .file(file)
                .ok()
                .and_then(|r| r.object_value(list, index, "block"));
            match (block, files.library_of[files.template_of[k]]) {
                (Some(block), Some(l)) => at
                    .library(l)
                    .ok()
                    .and_then(|r| r.block_carried(&block))
                    .map(|carried| (block, carried)),
                _ => None,
            }
        }
        _ => None,
    };
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
        inserts: Vec::new(),
        attributes_removed: Vec::new(),
        imported: None,
        message: String::new(),
    };
    let mut sheets = reach.clone();
    let mut from_row: Option<(String, String)> = None;
    // The attribute a set_attribute sets, in words, for its message.
    let mut attribute_set = String::new();
    // Whether the operation edits the object itself, not only a row value.
    let mut edits_object = true;
    // A move or a delete always edits the object's file; a change and an
    // attribute are checked once it is known whether they did.
    if matches!(op.body, Body::Move { .. } | Body::Delete { .. }) {
        if let Some(refusal) = many_paths(files, file, op, "target") {
            return refusal;
        }
    }
    match &op.body {
        Body::Change { set, .. } => match record.change(list, index, set, op.index) {
            Ok(changed) => {
                let keys: Vec<&str> = set.keys().map(String::as_str).collect();
                done.message = format!("sets {} of `{id}`{template_note}", keys.join(", "));
                edits_object = changed.edited;
                if !changed.edited {
                    sheets = vec![k];
                } else if let Some(refusal) = many_paths(files, file, op, "target") {
                    return refusal;
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
        Body::SetAttribute {
            tag,
            occurrence,
            value,
            ..
        } => {
            let capitals = tag_in_capitals(tag);
            let item = match set_which(
                &capitals,
                *occurrence,
                carried.as_ref(),
                record,
                list,
                index,
            ) {
                Ok(item) => item,
                Err((code, message)) => return fail(reach, op_err(code, message, "occurrence")),
            };
            attribute_set = attribute_words(tag, item);
            match record.set_attribute(list, index, &capitals, item, value, op.index) {
                Ok(AttributeSet::Written) => {
                    if let Some(refusal) = many_paths(files, file, op, "target") {
                        return refusal;
                    }
                    done.message =
                        format!("sets {attribute_set} of `{id}` to `{value}`{template_note}");
                }
                Ok(AttributeSet::FromRow(name)) => {
                    edits_object = false;
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
        Body::Add { .. } | Body::Table { .. } | Body::Block { .. } => {
            unreachable!("handled above")
        }
    }
    if let (Some(layer), true) = (&target_locked, edits_object) {
        return fail(reach, locked(&id, layer));
    }
    done.after = Some(record.object_text(list, index));
    if done.after == done.before {
        done.after = None;
    }
    if let Some((name, text)) = from_row {
        // A row's value fills every place on the sheet its template takes
        // it, and each is edited: none may be on a locked layer.
        let t = files.template_of[k];
        if let Ok(template) = at.template(t) {
            let listed = template.objects(ObjectList::Template);
            for (i, _) in template.row_value_places(&name) {
                let layer = template
                    .object_value(ObjectList::Template, i, "layer")
                    .unwrap_or_else(|| "0".to_owned());
                if template
                    .table_value(Table::Layer.key(), &layer, "locked")
                    .as_deref()
                    == Some("true")
                {
                    let filled = format!("{}/{}", files.name(k), listed[i].id);
                    return fail(vec![k], locked(&filled, &layer));
                }
            }
        }
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
            Body::SetAttribute { .. } => format!("{attribute_set} of `{id}`"),
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

/// The layer the object `found` sits on, when that layer is locked in its
/// sheet's template; an object with no layer sits on layer `0`.
fn locked_layer(at: &mut Attempt, found: &Found) -> Option<String> {
    let layer = at
        .file(found.file)
        .ok()?
        .object_value(found.list, found.index, "layer")
        .unwrap_or_else(|| "0".to_owned());
    let t = at.files.template_of[found.row];
    let locked = at
        .template(t)
        .ok()?
        .table_value(Table::Layer.key(), &layer, "locked");
    (locked.as_deref() == Some("true")).then_some(layer)
}

/// Applies a table operation: adds, changes, renames or deletes the entry
/// `name` of `table` in the template `to` names, reaching every sheet built
/// from it. A rename rewrites what names the entry, in the template and in
/// those sheets' rows; a delete is refused while anything uses it.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn apply_table(
    at: &mut Attempt,
    op: &Operation,
    table_op: TableOp,
    table: Table,
    to: &AddTo,
    name: &str,
    keys: &serde_json::Map<String, serde_json::Value>,
    new_name: Option<&str>,
) -> Applied {
    let files = at.files;
    let t = match to {
        AddTo::Template(written) => files.template(written).expect("checked before"),
        AddTo::Sheet(sheet) => files.template_of[files.sheet(sheet).expect("checked before")],
    };
    let sheets = files.templates[t].sheets.clone();
    let fail = |code: ErrorCode, message: String, field: &str| Applied {
        sheets: sheets.clone(),
        outcome: Err(op_error(op.index, op.spec.name, code, message, field)),
        refuse: false,
    };
    if let Some(refusal) = many_paths(files, FileRef::Template(t), op, "template") {
        return refusal;
    }
    let word = table.word();
    let written = files.templates[t].written.clone();
    let key = table.key();
    let built_in = table
        .built_in()
        .iter()
        .find(|b| b.eq_ignore_ascii_case(name));
    let record = match at.template(t) {
        Ok(record) => record,
        Err(e) => return fail(e.code, e.message, ""),
    };
    let path = record.path.clone();
    let names = record.table_names(key);
    let exists = names.iter().any(|n| n == name);
    let choices = || {
        let mut all: Vec<String> = table.built_in().iter().map(|b| (*b).to_owned()).collect();
        all.extend(
            names
                .iter()
                .filter(|n| !table.built_in().contains(&n.as_str()))
                .cloned(),
        );
        all.join(", ")
    };
    let before = record.table_entry_text(key, name);
    let message = match table_op {
        TableOp::Add => {
            if let Some(same) = names
                .iter()
                .map(String::as_str)
                .chain(table.built_in().iter().copied())
                .find(|n| n.eq_ignore_ascii_case(name))
            {
                return fail(
                    ErrorCode::DuplicateName,
                    format!(
                        "the template `{written}` has a {word} `{same}` already, and AutoCAD \
                         ignores case in names; change it with change_{} or give another name",
                        op.spec.name.trim_start_matches("add_")
                    ),
                    "name",
                );
            }
            if let Err(e) = record.table_add(key, table.entry_keys(), name, keys) {
                return fail(e.code, e.message, "");
            }
            format!("adds the {word} `{name}` to the template `{written}`")
        }
        TableOp::Change => {
            if table == Table::Linetype {
                if let Some(own) = built_in {
                    return fail(
                        ErrorCode::BadValue,
                        format!(
                            "`{own}` is AutoCAD's own linetype, which every drawing has and no \
                             template defines, so it is not changed"
                        ),
                        "name",
                    );
                }
            }
            if !exists && built_in.is_none() {
                return fail(
                    ErrorCode::UnknownName,
                    format!(
                        "the template `{written}` has no {word} `{name}`; its {word}s: {}",
                        choices()
                    ),
                    "name",
                );
            }
            // `Standard`, written for the first time, takes its font from
            // the drawing's own, so that setting its width factor alone
            // still makes a whole style.
            let mut keys = keys.clone();
            if !exists && table == Table::TextStyle && !keys.contains_key("font") {
                keys.insert(
                    "font".into(),
                    serde_json::Value::String(crate::text::STANDARD_FONT.to_owned()),
                );
            }
            let keys = &keys;
            if let Err(e) = record.table_change(key, table.entry_keys(), name, keys) {
                return fail(e.code, e.message, "");
            }
            let set: Vec<&str> = keys.keys().map(String::as_str).collect();
            format!(
                "sets {} of the {word} `{name}` in the template `{written}`",
                set.join(", ")
            )
        }
        TableOp::Rename | TableOp::Delete => {
            if let Some(own) = built_in {
                return fail(
                    ErrorCode::BadValue,
                    format!(
                        "`{own}` is every drawing's own {word}, and is never renamed or deleted"
                    ),
                    "name",
                );
            }
            if !exists {
                return fail(
                    ErrorCode::UnknownName,
                    format!(
                        "the template `{written}` has no {word} `{name}`; its {word}s: {}",
                        choices()
                    ),
                    "name",
                );
            }
            let by = table.named_by();
            if table_op == TableOp::Rename {
                let to = new_name.expect("a rename has a new name");
                if let Some(same) = names
                    .iter()
                    .map(String::as_str)
                    .chain(table.built_in().iter().copied())
                    .find(|n| n.eq_ignore_ascii_case(to) && *n != name)
                {
                    return fail(
                        ErrorCode::DuplicateName,
                        format!(
                            "the template `{written}` has a {word} `{same}` already, and \
                             AutoCAD ignores case in names"
                        ),
                        "to",
                    );
                }
                if let Err(e) = record.table_rename(key, name, to) {
                    return fail(e.code, e.message, "");
                }
                if table == Table::Linetype {
                    record.table_rename_values(Table::Layer.key(), "linetype", name, to);
                }
                let mut count = record.rename_values(ObjectList::Template, by, name, to, op.index);
                for &k in &sheets {
                    count += at
                        .rows
                        .rename_values(ObjectList::Row(k), by, name, to, op.index);
                }
                format!(
                    "renames the {word} `{name}` to `{to}` in the template `{written}`, and \
                     {count} object{} that named it",
                    if count == 1 { "" } else { "s" }
                )
            } else {
                let mut users: Vec<String> = record
                    .object_users(ObjectList::Template, by, name)
                    .into_iter()
                    .map(|id| format!("{}/{id}", files.name(sheets[0])))
                    .collect();
                for &k in &sheets {
                    users.extend(
                        at.rows
                            .object_users(ObjectList::Row(k), by, name)
                            .into_iter()
                            .map(|id| format!("{}/{id}", files.name(k))),
                    );
                }
                let template = at.template(t).expect("read above");
                let layers = if table == Table::Linetype {
                    template.table_users(Table::Layer.key(), "linetype", name)
                } else {
                    Vec::new()
                };
                if !users.is_empty() || !layers.is_empty() {
                    let mut what = Vec::new();
                    if !layers.is_empty() {
                        let s = if layers.len() == 1 { "" } else { "s" };
                        what.push(format!("the layer{s} {}", layers.join(", ")));
                    }
                    if !users.is_empty() {
                        let s = if users.len() == 1 { "" } else { "s" };
                        what.push(format!("the object{s} {}", listed(&users)));
                    }
                    return fail(
                        ErrorCode::InUse,
                        format!(
                            "the {word} `{name}` is in use, so it is not deleted: {}. Move them \
                             to another {word}, or delete them, first",
                            what.join(" and ")
                        ),
                        "name",
                    );
                }
                if let Err(e) = template.table_delete(key, name) {
                    return fail(e.code, e.message, "");
                }
                format!("deletes the {word} `{name}` from the template `{written}`")
            }
        }
    };
    let after_name = new_name.unwrap_or(name);
    let after = at
        .template(t)
        .ok()
        .and_then(|record| record.table_entry_text(key, after_name));
    Applied {
        outcome: Ok(Done {
            record: path,
            id: None,
            before,
            after,
            row_value: None,
            ids_written: Vec::new(),
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            message: format!("{message}, so on {}", every_sheet(&files.names(&sheets))),
        }),
        sheets,
        refuse: false,
    }
}

/// Applies a block operation to the block library `library` names: adds
/// the block `name`, redefines it, or brings the inserts of it, or of every
/// block of the library, into line. It reaches every sheet whose template
/// names the library.
fn apply_block(
    at: &mut Attempt,
    op: &Operation,
    block_op: BlockOp,
    library: Option<&str>,
    name: Option<&str>,
    base_point: Option<&Value>,
    objects: &[Value],
) -> Applied {
    let files = at.files;
    let l = files.library(library).expect("checked before");
    let sheets = files.libraries[l].sheets.clone();
    let written = files.libraries[l].written.clone();
    let fail = |e: Error| Applied {
        sheets: sheets.clone(),
        outcome: Err(e),
        refuse: false,
    };
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    if let Some(refusal) = library_many_paths(files, l, op) {
        return refusal;
    }
    let record = match at.library(l) {
        Ok(record) => record,
        Err(e) => return fail(e),
    };
    let path = record.path.clone();
    let names = record.block_names();
    let unknown = |name: &str| {
        op_err(
            ErrorCode::UnknownName,
            format!(
                "the block library `{written}` has no block `{name}`; its blocks: {}",
                names.join(", ")
            ),
            "name",
        )
    };
    let mut before = None;
    let to_sync: Vec<String> = match (block_op, name) {
        (BlockOp::Add, Some(name)) => {
            if let Some(same) = names.iter().find(|n| one_name(n, name)) {
                return fail(op_err(
                    ErrorCode::DuplicateName,
                    format!(
                        "the block library `{written}` has a block `{same}` already, and AutoCAD \
                         ignores case in names; change it with redefine_block or give another \
                         name"
                    ),
                    "name",
                ));
            }
            if let Err(e) = record.block_set(name, base_point, objects) {
                return fail(op_err(e.code, e.message, ""));
            }
            at.blocks.push((l, name.to_owned(), op.index));
            Vec::new()
        }
        (BlockOp::Redefine, Some(name)) => {
            if !names.iter().any(|n| n == name) {
                return fail(unknown(name));
            }
            before = record.block_text(name);
            if let Err(e) = record.block_set(name, base_point, objects) {
                return fail(op_err(e.code, e.message, ""));
            }
            at.blocks.push((l, name.to_owned(), op.index));
            vec![name.to_owned()]
        }
        (BlockOp::Sync, Some(name)) => {
            if !names.iter().any(|n| n == name) {
                return fail(unknown(name));
            }
            vec![name.to_owned()]
        }
        (BlockOp::Sync, None) => names.clone(),
        (BlockOp::Add | BlockOp::Redefine, None) => unreachable!("read with a name"),
        (BlockOp::Import, _) => unreachable!("an import is applied by apply_import"),
    };
    let synced = if block_op == BlockOp::Add {
        Synced::default()
    } else {
        match sync(at, l, &to_sync, op) {
            Ok(synced) => synced,
            Err(applied) => return *applied,
        }
    };
    let after = name.and_then(|name| {
        at.library(l)
            .ok()
            .and_then(|record| record.block_text(name))
    });
    let reached = match synced.inserts.len() {
        1 => ", reaching 1 insert".to_owned(),
        n => format!(", reaching {n} inserts"),
    };
    let what = match (block_op, name) {
        (BlockOp::Add, Some(name)) => {
            format!("adds the block `{name}` to the block library `{written}`")
        }
        (BlockOp::Redefine, Some(name)) => {
            format!("redefines the block `{name}` in the block library `{written}`{reached}")
        }
        (BlockOp::Sync, Some(name)) => format!(
            "brings the inserts of the block `{name}` of the block library `{written}` into line \
             with its definition{reached}"
        ),
        _ => format!(
            "brings the inserts of every block of the block library `{written}` into line with \
             their definitions{reached}"
        ),
    };
    let removed = match synced.removed.as_slice() {
        [] if block_op == BlockOp::Add => String::new(),
        [] => "; no insert gives a value for a tag its block no longer carries".to_owned(),
        list => {
            let each: Vec<String> = list
                .iter()
                .map(|r| {
                    let place = match r.ids.as_slice() {
                        [] => format!(
                            "`{}` in `{}`",
                            r.field,
                            relative_to(&files.folder, Path::new(&r.record))
                        ),
                        ids => quoted_list(&ids.iter().map(String::as_str).collect::<Vec<_>>()),
                    };
                    match r.occurrence {
                        Some(n) => format!("{} occurrence {n} (`{}`) of {place}", r.tag, r.value),
                        None => format!("{} (`{}`) of {place}", r.tag, r.value),
                    }
                })
                .collect();
            let s = if list.len() == 1 { "" } else { "s" };
            format!(
                "; removes {} attribute value{s} its block no longer carries: {}",
                list.len(),
                each.join(", ")
            )
        }
    };
    let names_of = files.names(&sheets);
    let on = if names_of.is_empty() {
        String::new()
    } else {
        let refs: Vec<&str> = names_of.iter().map(String::as_str).collect();
        format!(
            ", so on every sheet whose template names it: {}",
            quoted_list(&refs)
        )
    };
    Applied {
        outcome: Ok(Done {
            record: path,
            id: None,
            before,
            after,
            row_value: None,
            ids_written: Vec::new(),
            inserts: synced.inserts,
            attributes_removed: synced.removed,
            imported: None,
            message: format!("{what}{removed}{on}"),
        }),
        sheets,
        refuse: false,
    }
}

/// Applies an import_blocks operation: the blocks of the drawing `import`
/// names, planned and written into the block library `library` names as
/// `agentdrafter import-blocks` writes them, then, for each block it
/// replaced, every insert brought into line as redefine_block brings them.
fn apply_import(
    at: &mut Attempt,
    op: &Operation,
    library: Option<&str>,
    import: &Import,
) -> Applied {
    let files = at.files;
    let l = files.library(library).expect("checked before");
    let sheets = files.libraries[l].sheets.clone();
    let written = files.libraries[l].written.clone();
    let fail = |e: Error, field: &str| Applied {
        sheets: sheets.clone(),
        outcome: Err(op_error(op.index, op.spec.name, e.code, e.message, field)),
        refuse: false,
    };
    if let Some(refusal) = library_many_paths(files, l, op) {
        return refusal;
    }
    let from = files.folder.join(&import.from);
    let in_from = |e: Error| Error::new(e.code, format!("`{}`: {}", import.from, e.message));
    let dxf = match is_dxf(&from) {
        Ok(dxf) => dxf,
        Err(e) => return fail(e, "from"),
    };
    let drawing = match crate::dwg::read_blocks(&from, dxf) {
        Ok(drawing) => drawing,
        Err(e) => return fail(in_from(e), "from"),
    };
    let units = match drawing_unit(&drawing, import.units, "units") {
        Ok(units) => units,
        Err(e) => return fail(in_from(e), "units"),
    };
    if units != files.units {
        return fail(
            Error::new(
                ErrorCode::BadValue,
                format!(
                    "`{}` is in {units}, and the drawing set in {}; blocks are imported into a \
                     set's library in the set's unit, since converting them from one unit to \
                     another is not offered yet",
                    import.from, files.units
                ),
            ),
            "from",
        );
    }
    let record = match at.library(l) {
        Ok(record) => record,
        Err(e) => return fail(e, ""),
    };
    let path = record.path.clone();
    let existing = match record.current_text().and_then(|text| Existing::read(&text)) {
        Ok(existing) => existing,
        Err(e) => return fail(e, ""),
    };
    let planned = match plan(
        &drawing,
        &existing,
        &import.blocks,
        import.replace,
        "replace",
    ) {
        Ok(planned) => planned,
        Err(e) => {
            let field = if e.code == ErrorCode::UnknownName {
                "blocks"
            } else {
                ""
            };
            return fail(e, field);
        }
    };
    if let Err(e) = write_plan(record, &planned) {
        return fail(e, "");
    }
    for block in &planned.blocks {
        at.blocks.push((l, block.name.clone(), op.index));
    }
    let replaced = planned.replaced();
    let synced = if replaced.is_empty() {
        Synced::default()
    } else {
        match sync(at, l, &replaced, op) {
            Ok(synced) => synced,
            Err(applied) => return *applied,
        }
    };
    let imported = planned.imported;
    let names = |blocks: &[crate::import::ImportedBlock]| {
        blocks
            .iter()
            .map(|b| format!("`{}`", b.name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut what = vec![];
    if !imported.blocks_added.is_empty() {
        what.push(format!("adds {}", names(&imported.blocks_added)));
    }
    if !imported.blocks_replaced.is_empty() {
        let reached = match synced.inserts.len() {
            1 => "1 insert".to_owned(),
            n => format!("{n} inserts"),
        };
        what.push(format!(
            "replaces {}, reaching {reached}",
            names(&imported.blocks_replaced)
        ));
    }
    if what.is_empty() {
        what.push("imports no block".to_owned());
    }
    let left_out: Vec<String> = imported
        .not_carried
        .iter()
        .map(|n| format!("{} {} in `{}`", n.count, n.object_type, n.block))
        .collect();
    let left_out = if left_out.is_empty() {
        String::new()
    } else {
        format!(
            "; leaves out what the core cannot carry: {}",
            left_out.join(", ")
        )
    };
    let names_of = files.names(&sheets);
    let on = if names_of.is_empty() {
        String::new()
    } else {
        let refs: Vec<&str> = names_of.iter().map(String::as_str).collect();
        format!(
            ", so on every sheet whose template names it: {}",
            quoted_list(&refs)
        )
    };
    Applied {
        outcome: Ok(Done {
            record: path,
            id: None,
            before: None,
            after: None,
            row_value: None,
            ids_written: Vec::new(),
            inserts: synced.inserts,
            attributes_removed: synced.removed,
            imported: Some(imported),
            message: format!(
                "imports from `{}` into the block library `{written}`: {}{left_out}{on}",
                import.from,
                what.join("; ")
            ),
        }),
        sheets,
        refuse: false,
    }
}

/// The attribute a set_attribute sets, in words: `the attribute REV`, or
/// for item `item` of a tag the block defines more than once, counted from
/// 0, `occurrence 2 of the attribute NOTE`. A tag written in lowercase is
/// named in capitals, saying so.
fn attribute_words(tag: &str, item: Option<usize>) -> String {
    let capitals = tag_in_capitals(tag);
    let written = if capitals == tag {
        String::new()
    } else {
        format!(" (written `{tag}`, taken in capitals, as AutoCAD keeps a tag)")
    };
    match item {
        Some(i) => format!("occurrence {} of the attribute {capitals}{written}", i + 1),
        None => format!("the attribute {capitals}{written}"),
    }
}

/// Which value of the attribute `tag`, in capitals, a set_attribute with
/// `occurrence` sets on the insert of `list` at `index` in `record`: none
/// for the tag's single value, or the item of its list, counted from 0,
/// when its block, `carried` with the attributes it carries and their
/// defaults, defines the tag more than once. Without `occurrence` that is
/// an error listing each occurrence with its value; so is an occurrence the
/// block does not define. A tag the block does not carry is left for the
/// sheet's reading to refuse.
fn set_which(
    tag: &str,
    occurrence: Option<usize>,
    carried: Option<&(String, Vec<(String, String)>)>,
    record: &RecordFile,
    list: ObjectList,
    index: usize,
) -> Result<Option<usize>, (ErrorCode, String)> {
    let Some((block, carried)) = carried else {
        return Ok(None);
    };
    let defaults: Vec<&str> = carried
        .iter()
        .filter(|(t, _)| t == tag)
        .map(|(_, default)| default.as_str())
        .collect();
    let given = record
        .inserts(list)
        .into_iter()
        .find(|insert| insert.index == index)
        .and_then(|insert| insert.values.into_iter().find(|v| v.tag == tag));
    let listing = || {
        let each: Vec<String> = defaults
            .iter()
            .enumerate()
            .map(|(i, default)| {
                let written = match &given {
                    Some(GivenValue {
                        items: Some(items), ..
                    }) => items.get(i).cloned().flatten(),
                    Some(g) if i == 0 && !g.null => Some(g.text.clone()),
                    _ => None,
                };
                match written {
                    Some(value) => format!("{} `{value}`", i + 1),
                    None => format!("{} `{default}` (its default)", i + 1),
                }
            })
            .collect();
        each.join(", ")
    };
    match (defaults.len(), occurrence) {
        (0, _) | (1, None | Some(1)) => Ok(None),
        (1, Some(o)) => Err((
            ErrorCode::BadValue,
            format!(
                "block `{block}` defines the attribute `{tag}` once, so its occurrence is 1 or \
                 left out, not {o}"
            ),
        )),
        (n, None) => Err((
            ErrorCode::MissingKey,
            format!(
                "block `{block}` defines the attribute `{tag}` {n} times; name the one to set \
                 with occurrence, counted from 1 in the order the block defines them: {}",
                listing()
            ),
        )),
        (n, Some(o)) if o > n => Err((
            ErrorCode::BadValue,
            format!(
                "block `{block}` defines the attribute `{tag}` {n} times, so its occurrence is \
                 1 to {n}, not {o}: {}",
                listing()
            ),
        )),
        (_, Some(o)) => Ok(Some(o - 1)),
    }
}

/// What bringing inserts into line did.
#[derive(Default)]
struct Synced {
    /// The inserts reached, by ID, in the set's order of sheets.
    inserts: Vec<String>,
    removed: Vec<AttributeRemoved>,
}

/// Brings every insert of each of `blocks`, of the block library `l`, into
/// line with the block's definition as the library holds it now, as
/// AutoCAD's ATTSYNC does: each value an insert gives for a tag its block
/// no longer carries is removed, in the templates that name the library,
/// in the rows of the sheets built from them, and in the library's own
/// blocks. A tag the block now defines fewer times than an insert's list
/// gives values for loses the list's surplus items; a tag it now defines
/// once takes the list's first item alone, and a tag it now defines more
/// than once takes a single value as its list's first item. A new tag
/// needs nothing written, since an insert that gives no value takes the
/// definition's default.
fn sync(
    at: &mut Attempt,
    l: usize,
    blocks: &[String],
    op: &Operation,
) -> Result<Synced, Box<Applied>> {
    let files = at.files;
    let library = &files.libraries[l];
    let fail = |e: Error| {
        Box::new(Applied {
            sheets: library.sheets.clone(),
            outcome: Err(op_error(op.index, op.spec.name, e.code, e.message, "")),
            refuse: false,
        })
    };
    let record = at.library(l).map_err(fail)?;
    let carried: Vec<(String, Vec<String>)> = blocks
        .iter()
        .map(|b| (b.clone(), record.block_carried_tags(b).unwrap_or_default()))
        .collect();
    // How many times the block `block` now carries `tag`; none when the
    // block is not brought into line.
    let count = |block: &str, tag: &str| {
        carried
            .iter()
            .find(|(b, _)| b == block)
            .map(|(_, tags)| tags.iter().filter(|t| *t == tag).count())
    };
    // What fitting `given` to the block `block` takes away, and how many
    // times the block carries its tag; none when it fits as it is.
    let unfit = |block: &str, given: &GivenValue| {
        let n = count(block, &given.tag)?;
        given.unfitted(n).map(|gone| (n, gone))
    };
    // The blocks an insert on a sheet reaches through: these, and every
    // block of the library that nests one of them, however deep, as the
    // library holds them now. An insert of any of them draws as the
    // definitions now do; only a direct insert of one of these has values
    // to bring into line.
    let nesting = record.block_inserts();
    let mut reaching: Vec<&str> = blocks.iter().map(String::as_str).collect();
    loop {
        let before = reaching.len();
        for (outer, insert) in &nesting {
            if reaching.contains(&insert.block.as_str()) && !reaching.contains(&outer.as_str()) {
                reaching.push(outer);
            }
        }
        if reaching.len() == before {
            break;
        }
    }
    let reaches = |block: &str| reaching.contains(&block);
    let mut reached: Vec<(usize, String)> = Vec::new();
    let mut removed = Vec::new();

    // The library's own blocks, whose definitions insert these.
    removed.extend(fit_library_inserts(record, blocks).map_err(fail)?);

    // Each template that names the library: its inserts are on every
    // sheet built from it.
    for &t in &library.templates {
        let sheets = &files.templates[t].sheets;
        let template = at.template(t).map_err(fail)?;
        let listed = template.objects(ObjectList::Template);
        let mut edits = Vec::new();
        for insert in template.inserts(ObjectList::Template) {
            if !reaches(&insert.block) {
                continue;
            }
            let id = &listed[insert.index].id;
            let ids: Vec<String> = sheets
                .iter()
                .map(|&k| format!("{}/{id}", files.name(k)))
                .collect();
            reached.extend(
                sheets
                    .iter()
                    .map(|&k| (k, format!("{}/{id}", files.name(k)))),
            );
            for given in insert.values {
                if let Some((n, gone)) = unfit(&insert.block, &given) {
                    edits.push((insert.index, given, n, gone, ids.clone()));
                }
            }
        }
        if edits.is_empty() {
            continue;
        }
        if let Some(refusal) = many_paths(files, FileRef::Template(t), op, "") {
            return Err(Box::new(refusal));
        }
        let template = at.template(t).map_err(fail)?;
        let path = template.path.display().to_string();
        for (index, given, n, gone, ids) in edits {
            template
                .fit_attribute(ObjectList::Template, index, &given.key, n, op.index)
                .map_err(fail)?;
            let field = ObjectList::Template.place(index);
            removed.extend(removals(&path, &field, &ids, &given.tag, gone));
        }
    }

    // Each sheet's row's own inserts.
    let rows_path = files.rows_path.display().to_string();
    for &k in &library.sheets {
        let list = ObjectList::Row(k);
        let listed = at.rows.objects(list);
        let mut edits = Vec::new();
        for insert in at.rows.inserts(list) {
            if !reaches(&insert.block) {
                continue;
            }
            let id = format!("{}/{}", files.name(k), listed[insert.index].id);
            reached.push((k, id.clone()));
            for given in insert.values {
                if let Some((n, gone)) = unfit(&insert.block, &given) {
                    edits.push((insert.index, given, n, gone, id.clone()));
                }
            }
        }
        for (index, given, n, gone, id) in edits {
            at.rows
                .fit_attribute(list, index, &given.key, n, op.index)
                .map_err(fail)?;
            let field = list.place(index);
            removed.extend(removals(&rows_path, &field, &[id], &given.tag, gone));
        }
    }
    reached.sort_by_key(|(k, _)| *k);
    Ok(Synced {
        inserts: reached.into_iter().map(|(_, id)| id).collect(),
        removed,
    })
}

/// Brings each insert among the objects of the blocks of the block
/// library `record` that places one of `blocks` into line with that
/// block's definition as `record` writes it now, as an attsync does: a
/// value it gives for a tag the block no longer carries, or more values
/// than the block carries the tag, is removed. Gives each value removed.
pub(crate) fn fit_library_inserts(
    record: &mut RecordFile,
    blocks: &[String],
) -> Result<Vec<AttributeRemoved>, Error> {
    let carried: Vec<(String, Vec<String>)> = blocks
        .iter()
        .map(|b| (b.clone(), record.block_carried_tags(b).unwrap_or_default()))
        .collect();
    let library_path = record.path.display().to_string();
    let nested: Vec<(String, usize, GivenValue, usize, Removed)> = record
        .block_inserts()
        .into_iter()
        .filter_map(|(outer, insert)| {
            let (_, tags) = carried.iter().find(|(b, _)| *b == insert.block)?;
            let index = insert.index;
            let fitted: Vec<_> = insert
                .values
                .into_iter()
                .filter_map(|given| {
                    let n = tags.iter().filter(|t| **t == given.tag).count();
                    let gone = given.unfitted(n)?;
                    Some((outer.clone(), index, given, n, gone))
                })
                .collect();
            Some(fitted)
        })
        .flatten()
        .collect();
    let mut removed = Vec::new();
    for (outer, index, given, n, gone) in nested {
        record.fit_block_attribute(&outer, index, &given.key, n)?;
        let field = format!("blocks.{outer}.objects[{index}]");
        removed.extend(removals(&library_path, &field, &[], &given.tag, gone));
    }
    Ok(removed)
}

/// What fitting an insert's value to its block takes away: the whole
/// value, or items of its list, each with its occurrence, counted from 1.
type Removed = Vec<(Option<usize>, String)>;

/// The removals `gone` from the value an insert gives for `tag`, the insert
/// being at `field` in `record` with the IDs `ids`.
fn removals(
    record: &str,
    field: &str,
    ids: &[String],
    tag: &str,
    gone: Removed,
) -> impl Iterator<Item = AttributeRemoved> {
    let (record, field, ids, tag) = (
        record.to_owned(),
        field.to_owned(),
        ids.to_vec(),
        tag.to_owned(),
    );
    gone.into_iter()
        .map(move |(occurrence, value)| AttributeRemoved {
            record: record.clone(),
            field: field.clone(),
            ids: ids.clone(),
            tag: tag.clone(),
            occurrence,
            value,
        })
}

/// The refusal of block operation `op` on the library `l` when the set's
/// templates reach it by more than one path, as [`many_paths`] refuses an
/// edit to a template so reached.
fn library_many_paths(files: &SetFiles, l: usize, op: &Operation) -> Option<Applied> {
    let library = &files.libraries[l];
    if library.spellings.len() < 2 {
        return None;
    }
    let listed: Vec<String> = library
        .spellings
        .iter()
        .map(|path| format!("`{path}`"))
        .collect();
    let error = op_error(
        op.index,
        op.spec.name,
        ErrorCode::BadValue,
        format!(
            "it edits a block library the templates reach by more than one path to one file: {}. \
             A change set writes a library through one path, so a sheet that reaches it by \
             another would not follow; write one path to the library in every template that \
             names it, then send the change set again",
            listed.join(", ")
        ),
        "library",
    );
    Some(Applied {
        sheets: Vec::new(),
        outcome: Err(error),
        refuse: true,
    })
}

/// The refusal of operation `op`, which edits `file`, when `file` is a
/// template the rows reach by more than one path. A change set writes a
/// template through one path, so a sheet that reaches it by another, such
/// as a link to it, would keep its old record beside a rebuilt DWG; the
/// whole change set is refused instead, before anything is written. `field`
/// is the operation's key that names the template or its object.
fn many_paths(files: &SetFiles, file: FileRef, op: &Operation, field: &str) -> Option<Applied> {
    let FileRef::Template(t) = file else {
        return None;
    };
    let paths = &files.templates[t].paths;
    if paths.len() < 2 {
        return None;
    }
    let listed: Vec<String> = paths
        .iter()
        .map(|(path, k)| format!("`{path}` (sheet `{}`)", files.name(*k)))
        .collect();
    let error = op_error(
        op.index,
        op.spec.name,
        ErrorCode::BadValue,
        format!(
            "it edits a template the rows reach by more than one path to one file: {}. A change \
             set writes a template through one path, so a sheet that reaches it by another \
             would not follow; write one path to the template in every row that names it, then \
             send the change set again",
            listed.join(", ")
        ),
        field,
    );
    Some(Applied {
        sheets: Vec::new(),
        outcome: Err(error),
        refuse: true,
    })
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
    // The block library the template names, as the change set edited it.
    let mut edited = Edited { library: None };
    if let Some(l) = files.library_of[t] {
        match &at.libraries[l] {
            Some(Ok(file)) => {
                if let Some(text) = file.new_text()? {
                    edited.library = Some((l, files, text));
                }
            }
            Some(Err(e)) => return Err(e.clone()),
            None => {}
        }
    }
    let (record, _) = template_in(
        &edited,
        &text,
        &template.path,
        &rows[k],
        &files.rows_path,
        files.units,
        notices,
    )?;
    let dwg = files.dwg(k);
    writable(&dwg)?;
    prepare_sheet(&record, &template.path, &dwg, &files.mark, true, settings)
}

/// The files a sheet is read from while a change set is applied: the block
/// library it edited, as edited, and every other file as it is on disk.
struct Edited<'a> {
    /// The library, the set's files and the library's new text.
    library: Option<(usize, &'a SetFiles, String)>,
}

impl ReadFiles for Edited<'_> {
    fn text(&self, path: &Path) -> io::Result<String> {
        if let Some((l, files, text)) = &self.library {
            if files.is_library(*l, path) {
                return Ok(text.clone());
            }
        }
        Disk.text(path)
    }
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
    let in_library = files.library_of[t].filter(|&l| {
        error
            .file
            .as_deref()
            .is_some_and(|file| files.is_library(l, Path::new(file)))
    });
    if let (Some(l), Some(rest)) = (in_library, field.strip_prefix("blocks.")) {
        // The block the error is in: the longest of the library's names
        // the place starts with, since a name may hold a dot, as `A.B`,
        // followed by nothing or by one of a block's own keys, so that a
        // name such as `A.objects[0].line` is not taken for a place in `A`.
        let names = match &at.libraries[l] {
            Some(Ok(library)) => library.block_names(),
            _ => Vec::new(),
        };
        let within = |name: &str| -> Option<String> {
            let after = rest.strip_prefix(name)?;
            if after.is_empty() {
                return Some(String::new());
            }
            let within = after.strip_prefix('.')?;
            let key = within.split(['.', '[']).next().unwrap_or_default();
            ["base_point", "objects"]
                .contains(&key)
                .then(|| within.to_owned())
        };
        let block = names
            .iter()
            .filter(|name| within(name).is_some())
            .max_by_key(|name| name.len());
        // The last operation that wrote that block.
        if let Some(block) = block {
            cause = at
                .blocks
                .iter()
                .rev()
                .find(|(lib, name, _)| *lib == l && name == block)
                .and_then(|(_, name, i)| within(name).map(|within| (*i, within)));
        }
    } else if in_rows {
        if let Some((list @ ObjectList::Row(_), index, rest)) = split_place(&field) {
            if let Some(i) = at.rows.blame(list, index, rest) {
                cause = Some((i, rest.to_owned()));
            }
        } else if let Some(value) = field.strip_prefix(&format!("rows[{}].values.", k)) {
            if let Some((_, _, i)) = at.values.iter().find(|(r, n, _)| *r == k && n == value) {
                cause = Some((*i, String::new()));
            }
        }
    } else if in_template {
        if let Some((ObjectList::Template, index, rest)) = split_place(&field) {
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
        }
        | Body::Table {
            to: AddTo::Template(written),
            ..
        } => files.template(written) == Some(files.template_of[k]),
        Body::Table {
            to: AddTo::Sheet(sheet),
            ..
        } => files
            .sheet(sheet)
            .is_some_and(|s| files.template_of[s] == files.template_of[k]),
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
        Body::Block { library, .. } => {
            files.library(library.as_deref()).ok() == files.library_of[files.template_of[k]]
        }
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
        prepare_sheet(&record, &template.path, &dwg, &files.mark, true, settings).map(|_| ())
    });
    Ok(outcome)
}

/// Splits a place in a record, `objects[3].line.to` or
/// `rows[1].objects[0].line.to`, into the list the object is in, its place
/// in the list and what follows, `line.to`.
fn split_place(field: &str) -> Option<(ObjectList, usize, &str)> {
    fn index(text: &str) -> Option<(usize, &str)> {
        let rest = text.strip_prefix('[')?;
        let end = rest.find(']')?;
        Some((rest[..end].parse().ok()?, &rest[end + 1..]))
    }
    let (list, rest) = match field.strip_prefix("rows") {
        Some(rest) => {
            let (row, rest) = index(rest)?;
            (ObjectList::Row(row), rest.strip_prefix(".objects")?)
        }
        None => (ObjectList::Template, field.strip_prefix("objects")?),
    };
    let (object, rest) = index(rest)?;
    Some((list, object, rest.strip_prefix('.').unwrap_or(rest)))
}

/// The field within operation `op` that `rest`, a place within the object
/// it edits, such as `line.to[1]`, is about.
fn op_field(op: &Operation, rest: &str, code: ErrorCode) -> String {
    let within = rest.split_once('.').map_or("", |(_, within)| within);
    match &op.body {
        Body::Add { .. } => within.to_owned(),
        Body::Table {
            op: TableOp::Change,
            keys,
            ..
        } => {
            let key = rest.rsplit('.').next().unwrap_or("");
            if keys.contains_key(key) {
                format!("set.{key}")
            } else {
                "set".to_owned()
            }
        }
        Body::Table {
            op: TableOp::Rename,
            ..
        } => "to".to_owned(),
        Body::Table { .. } => {
            let key = rest.rsplit('.').next().unwrap_or("");
            if key.is_empty() {
                "name".to_owned()
            } else {
                key.to_owned()
            }
        }
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
        // A place within the block it wrote, such as `objects[0].line.layer`,
        // is that place in the operation.
        Body::Block { .. } => rest.to_owned(),
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
    let (edits, lands) = match op.body {
        Body::Block { .. } => (
            "the block library this sheet's template names",
            "an edit to a block library lands only if every sheet whose template names it",
        ),
        _ => (
            "a template this sheet is built from",
            "a template edit lands only if every sheet built from the template",
        ),
    };
    Error::new(
        ErrorCode::SharedEditFailed,
        format!(
            "operation {} (`{}`) edits {edits}, and {lands} takes its whole part; sheet `{}` \
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
        Body::Table {
            table,
            name,
            to: AddTo::Template(t),
            ..
        } => format!("the {} `{name}` of the template `{t}`", table.word()),
        Body::Table {
            table,
            name,
            to: AddTo::Sheet(s),
            ..
        } => format!("the {} `{name}` of sheet `{s}`'s template", table.word()),
        Body::Block {
            import: Some(import),
            ..
        } => format!("the blocks of `{}`", import.from),
        Body::Block {
            name: Some(name), ..
        } => format!("the block `{name}`"),
        Body::Block { name: None, .. } => "every block of the block library".to_owned(),
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

/// An error's code as it is written in JSON, such as `unknown_name`.
fn code_text(error: &Error) -> String {
    serde_json::to_value(error.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}
