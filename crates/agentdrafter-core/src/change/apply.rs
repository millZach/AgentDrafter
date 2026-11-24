//! Applies a change set to a drawing set's own files.
//!
//! The operations are applied in order to the records held in memory: the
//! set's rows files and the templates they name, each edit to a row in the
//! rows file the row is in. Every sheet an operation
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

use super::array;
use super::break_join::{compute_break, compute_join, BreakSpec};
use super::commit::Replacement;
use super::explode::compute_explode;
use super::fillet_chamfer::{
    compute_chamfer_polyline, compute_chamfer_two, compute_fillet_polyline, compute_fillet_two,
    ChamferSpec,
};
use super::from_dimension::dimension_from;
use super::journal::{commit_change_set, HowChanged, Keeping, Landing, ObjectChanged, SheetFailed};
use super::offset::{compute_offset, object_to_record_keys, OffsetBy, OffsetSide};
use super::operations::{
    op_error, read_change_set, AddTo, BlockOp, Body, CornerMode, DrawOrderPlace, FromOp, Import,
    Kind, Motion, OpBreakBy, OpOffsetBy, OpOffsetSide, OpPoint, Operation, Pattern, PointSnap,
    Table, TableOp, Target, TextToFrontWhich, TransformBy, ROW_VALUE_KEY,
};
use super::replace::{TextMatchKind, TextMatcher, TextScope};
use super::stretch::{compute_stretch, StretchAction, StretchBox};
use super::trim_extend::{compute_extend, compute_trim, ExtendEnd, TrimKeep};
use super::MARK_FILE;
use super::{
    mark_report, quoted_list, read_mark, relative_to, AttributeRemoved, AttributeUpdated,
    ChangeReport, ChangeRequest, Copied, Fill, OperationOutcome, RowValueSet, SheetChange,
    SnapResolved, TextReplaced,
};
use crate::build::{is_same_file, prepare_sheet, PreparedSheet};
use crate::description::blocks::tag_in_capitals;
use crate::description::parts::{self, Unsplit};
use crate::description::SheetRecord;
use crate::description::{
    article, library_named, mark_text, one_name, read_file, read_rows_files_in, read_set_settings,
    rows_of, shown_mtext, template_at, template_in, template_path, AttributeSet, Disk, FromRow,
    GivenValue, How, LeaderLineEdit, Listed, MarkFailure, Matched, ObjectList, ReadFiles,
    RecordFile, Resolved, Row, RowsFile, TableEdit, Transform, COPY_OF_KEY, ID_SEPARATOR,
    LONGEST_ID,
};
use crate::dwg::BuildMark;
use crate::error::{Error, ErrorCode};
use crate::geometry::curve::Curve;
use crate::geometry::{Reader, SnapRef};
use crate::import::{drawing_unit, is_dxf, plan, write_plan, BlocksImported, Existing};
use crate::model::Object;
use crate::set::{set_in_mark, sheet_mark, Notice, SET_SETTINGS_FILE};
use crate::settings::Settings;
use crate::text::placement::SheetFonts;
use crate::units::Unit;

/// The most IDs an error lists as the choices on a sheet.
const MOST_LISTED: usize = 60;

/// A drawing set's files as they are on disk before the change set.
struct SetFiles {
    folder: PathBuf,
    units: Unit,
    output: PathBuf,
    /// The set as the build marks of its DWGs name it, as `build` names it
    /// names it.
    marked: Option<String>,
    /// The set's rows files, in the order it builds their rows.
    rows_files: Vec<RowsFile>,
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
        let rows_files = read_rows_files_in(&Disk, folder, &settings)?;
        let rows = rows_of(&rows_files)?;
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
            marked: set_in_mark(folder, &settings.output),
            rows_files,
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

    /// The build mark sheet `k`'s DWG carries, as `build` writes it.
    fn mark(&self, k: usize) -> BuildMark {
        sheet_mark(self.marked.as_deref(), &self.rows_files, &self.rows[k])
    }

    /// Where sheet `k`'s own objects are written: its row's rows file, and
    /// the list of its row there.
    fn row_list(&self, k: usize) -> (FileRef, ObjectList) {
        let row = &self.rows[k];
        (FileRef::Rows(row.file_index), ObjectList::Row(row.index))
    }

    /// The path of the rows file sheet `k`'s row is in.
    fn rows_path(&self, k: usize) -> &Path {
        &self.rows[k].file
    }
}

/// The place of sheet `k`'s rows file among the set's, and the list of its
/// row's own objects there.
fn row_place(files: &SetFiles, k: usize) -> (usize, ObjectList) {
    let row = &files.rows[k];
    (row.file_index, ObjectList::Row(row.index))
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
    /// The rows file at this place among the set's.
    Rows(usize),
    Template(usize),
}

/// The change set's operations applied to the records in memory.
struct Attempt<'a> {
    files: &'a SetFiles,
    /// Each of the set's rows files, in the order of `files.rows_files`.
    rows: Vec<RecordFile>,
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
            rows: files
                .rows_files
                .iter()
                .map(|f| RecordFile::from_text(&f.path, f.text.clone()))
                .collect::<Result<_, _>>()?,
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
            FileRef::Rows(f) => Ok(&mut self.rows[f]),
            FileRef::Template(t) => self.template(t),
        }
    }

    /// The rows file sheet `k`'s row is in, as edited, and the list of its
    /// row's own objects there.
    fn row_record(&mut self, k: usize) -> (&mut RecordFile, ObjectList) {
        let row = &self.files.rows[k];
        (&mut self.rows[row.file_index], ObjectList::Row(row.index))
    }

    /// The set's rows as the records stand now: each rows file's edited
    /// text, or its text as it was.
    fn rows_now(&self) -> Result<Vec<Row>, Error> {
        let files = self
            .files
            .rows_files
            .iter()
            .zip(&self.rows)
            .map(|(file, record)| {
                Ok(RowsFile {
                    text: record.new_text()?.unwrap_or_else(|| file.text.clone()),
                    ..file.clone()
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        rows_of(&files)
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
        let (file, list) = self.files.row_list(k);
        let FileRef::Rows(f) = file else {
            unreachable!("a row's objects are in its rows file")
        };
        found.extend(
            self.rows[f]
                .objects(list)
                .into_iter()
                .enumerate()
                .map(|(i, l)| (file, list, i, l)),
        );
        Ok(found)
    }

    /// The sheets an edit to `file` reaches.
    fn reach(&self, file: FileRef, k: usize) -> Vec<usize> {
        match file {
            FileRef::Rows(_) => vec![k],
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
    /// The row values matched to a list of parts it sets.
    row_values: Vec<RowValueSet>,
    ids_written: Vec<String>,
    /// The inserts a block operation brought into line.
    inserts: Vec<String>,
    attributes_removed: Vec<AttributeRemoved>,
    /// What an import_blocks imported.
    imported: Option<BlocksImported>,
    /// Each snapped point it took, and the point it was worked out to.
    snapped: Vec<SnapResolved>,
    /// Each copy a copy or an array made.
    copies: Vec<Copied>,
    /// The sheets each copy is on, in the order of `copies`.
    copy_sheets: Vec<Vec<usize>>,
    /// Each object a rotate, a scale or a mirror in place transformed, by
    /// its ID on the sheet named, and the sheets it is on.
    transformed: Vec<(String, Vec<usize>)>,
    /// The row values a set created, giving an object its own.
    created_row_values: Vec<String>,
    /// Each text, mtext or attribute value changed by `replace_text`.
    replacements: Vec<TextReplaced>,
    /// Each insert attribute changed by `set_attributes`.
    attributes_set: Vec<AttributeUpdated>,
    /// The ID of the second object created when `trim` or `break` splits an
    /// object in two.
    split_id: Option<String>,
    /// Objects deleted by `join` or `explode`, each with the sheets it sits on.
    deleted: Vec<(String, Vec<usize>)>,
    /// Objects created by `explode`, each with the sheets it sits on.
    created: Vec<(String, Vec<usize>)>,
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
        let how = match (op.spec.kind, &op.body) {
            (Kind::Delete | Kind::Explode, _) => HowChanged::Deleted,
            (Kind::Add(..) | Kind::Offset, _) => HowChanged::Added,
            (
                Kind::Fillet,
                Body::Fillet {
                    mode: CornerMode::TwoObjects { .. },
                    ..
                },
            )
            | (
                Kind::Chamfer,
                Body::Chamfer {
                    mode: CornerMode::TwoObjects { .. },
                    ..
                },
            ) => HowChanged::Added,
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
        for fill in d
            .row_value
            .iter()
            .chain(&d.row_values)
            .flat_map(|row| &row.fills)
        {
            add(fill.id.clone(), HowChanged::Changed);
        }
        // Each insert of a block an operation redefined or brought into
        // line draws as the block now does.
        for id in &d.inserts {
            add(id.clone(), HowChanged::Changed);
        }
        // Each object transformed in place is changed, on every sheet it is
        // on.
        for (id, on) in &d.transformed {
            let local = id.split_once(ID_SEPARATOR).map_or(id.as_str(), |(_, l)| l);
            for &k in on {
                add(
                    format!("{}{ID_SEPARATOR}{local}", files.name(k)),
                    HowChanged::Changed,
                );
            }
        }
        for r in &d.replacements {
            add(r.id.clone(), HowChanged::Changed);
        }
        for a in &d.attributes_set {
            add(a.id.clone(), HowChanged::Changed);
        }
        if let Some(split) = &d.split_id {
            let local = split
                .split_once(ID_SEPARATOR)
                .map_or(split.as_str(), |(_, l)| l);
            for &k in &op_sheets[i] {
                add(
                    format!("{}{ID_SEPARATOR}{local}", files.name(k)),
                    HowChanged::Added,
                );
            }
        }
        for (del_id, on) in &d.deleted {
            let local = del_id
                .split_once(ID_SEPARATOR)
                .map_or(del_id.as_str(), |(_, l)| l);
            for &k in on {
                add(
                    format!("{}{ID_SEPARATOR}{local}", files.name(k)),
                    HowChanged::Deleted,
                );
            }
        }
        for (new_id, on) in &d.created {
            let local = new_id
                .split_once(ID_SEPARATOR)
                .map_or(new_id.as_str(), |(_, l)| l);
            for &k in on {
                add(
                    format!("{}{ID_SEPARATOR}{local}", files.name(k)),
                    HowChanged::Added,
                );
            }
        }
        // Each copy is added, on every sheet it is on.
        for (copy, on) in d.copies.iter().zip(&d.copy_sheets) {
            let local = copy
                .id
                .split_once(ID_SEPARATOR)
                .map_or(copy.id.as_str(), |(_, l)| l);
            for &k in on {
                add(
                    format!("{}{ID_SEPARATOR}{local}", files.name(k)),
                    HowChanged::Added,
                );
            }
        }
    }
    objects
}

/// What a dry run would write, kept for a preview to draw from: each record
/// it would write with its new text, and each object it would add, change
/// or delete on each sheet it would land on, as its journal entry would
/// name them.
#[derive(Default)]
pub(super) struct Kept {
    pub records: Vec<(PathBuf, String)>,
    pub objects: Vec<ObjectChanged>,
}

/// Applies the change set. `attempted` is given what it did to its sheets
/// before its files are put in place, so a commit that fails is recorded as
/// a failed attempt with the sheets it touched. A dry run gives `kept`
/// what it would write, when it is asked for.
pub(super) fn apply(
    request: &ChangeRequest,
    settings: &Settings,
    attempted: &mut Option<Landing>,
    kept: Option<&mut Kept>,
    keeping: Option<&Keeping>,
    whole: bool,
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
    let (attempt, mut prepared) = loop {
        let mut at = Attempt::new(&files)?;
        for op in ops.iter().filter(|op| included[op.index]) {
            let applied = apply_op(&mut at, op, settings);
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
        let rows = at.rows_now();
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
    let mut landed: Vec<usize> = touched
        .iter()
        .copied()
        .filter(|k| !failed.contains_key(k))
        .collect();
    // A proposal is kept whole or not at all (`whole`): when any sheet it
    // reaches would refuse it, the others are left as they are too, held
    // back, nothing lands and no operation does, and a keep is recorded as
    // a failed attempt. Its dry run, and its preview, say so beforehand.
    if whole && !failed.is_empty() && !landed.is_empty() {
        let refusing: Vec<String> = failed
            .keys()
            .map(|&k| format!("`{}`", files.name(k)))
            .collect();
        let why = match refusing.as_slice() {
            [one] => format!("{one} would refuse it"),
            [rest @ .., last] => format!("{} and {last} would refuse it", rest.join(", ")),
            [] => unreachable!("a sheet refuses it"),
        };
        let held = format!(
            "left as it was: a proposal is kept on every sheet it reaches or on none, and {why}"
        );
        for k in landed.drain(..) {
            // Held back: nothing of it is built or written.
            prepared.remove(&k);
            failed.insert(k, Error::new(ErrorCode::SharedEditFailed, held.clone()));
        }
        // And no operation lands: each that would have is held back too.
        for op in ops {
            if !std::mem::replace(&mut included[op.index], false) {
                continue;
            }
            done[op.index] = None;
            op_errors[op.index] = Some(op_error(
                op.index,
                op.spec.name,
                ErrorCode::SharedEditFailed,
                held.clone(),
                "",
            ));
        }
    }

    // What would be written.
    let mut records: Vec<(PathBuf, String)> = Vec::new();
    if !landed.is_empty() {
        for (file, record) in files.rows_files.iter().zip(&attempt.rows) {
            if let Some(text) = record.new_text()? {
                records.push((file.path.clone(), text));
            }
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

    if let Some(kept) = kept.filter(|_| request.dry_run) {
        kept.records.clone_from(&records);
        kept.objects = objects_changed(ops, &included, &done, &op_sheets, &files);
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
            keeping,
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
                record: d
                    .as_ref()
                    .filter(|d| !d.record.as_os_str().is_empty())
                    .map(|d| d.record.display().to_string()),
                id: d.as_ref().and_then(|d| d.id.clone()),
                before: d.as_ref().and_then(|d| d.before.clone()),
                after: d.as_ref().and_then(|d| d.after.clone()),
                row_value: d.as_ref().and_then(|d| d.row_value.clone()),
                row_values: d.as_ref().map(|d| d.row_values.clone()).unwrap_or_default(),
                ids_written: d
                    .as_ref()
                    .map(|d| d.ids_written.clone())
                    .unwrap_or_default(),
                inserts: d.as_ref().map(|d| d.inserts.clone()).unwrap_or_default(),
                imported: d.as_ref().and_then(|d| d.imported.clone()),
                snapped: d.as_ref().map(|d| d.snapped.clone()).unwrap_or_default(),
                // A copy of an operation held back never landed.
                copies: d
                    .as_ref()
                    .filter(|_| included[i])
                    .map(|d| d.copies.clone())
                    .unwrap_or_default(),
                transformed: d
                    .as_ref()
                    .filter(|_| included[i])
                    .map(|d| d.transformed.iter().map(|(id, _)| id.clone()).collect())
                    .unwrap_or_default(),
                // Nor did a row value of an operation held back.
                created_row_values: d
                    .as_ref()
                    .filter(|_| included[i])
                    .map(|d| d.created_row_values.clone())
                    .unwrap_or_default(),
                replacements: d
                    .as_ref()
                    .filter(|_| included[i])
                    .map(|d| d.replacements.clone())
                    .unwrap_or_default(),
                attributes_set: d
                    .as_ref()
                    .filter(|_| included[i])
                    .map(|d| d.attributes_set.clone())
                    .unwrap_or_default(),
                split_id: d
                    .as_ref()
                    .filter(|_| included[i])
                    .and_then(|d| d.split_id.clone()),
                deleted: d
                    .as_ref()
                    .filter(|_| included[i])
                    .map(|d| d.deleted.iter().map(|(id, _)| id.clone()).collect())
                    .unwrap_or_default(),
                created: d
                    .as_ref()
                    .filter(|_| included[i])
                    .map(|d| d.created.iter().map(|(id, _)| id.clone()).collect())
                    .unwrap_or_default(),
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
        guide: None,
    })
}

/// Refuses a change set that names a sheet or a template the set does not
/// hold: there is no sheet to fail, so the change set itself is wrong.
fn precheck(files: &SetFiles, ops: &[Operation]) -> Result<(), Error> {
    for op in ops {
        if let Body::AddRevisionCloud {
            to,
            around,
            draw_order,
            ..
        } = &op.body
        {
            if let Some(AddTo::Template(written)) = to {
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
            }
            if let Some(AddTo::Sheet(sheet)) = to {
                if files.sheet(sheet).is_none() {
                    return Err(op_error(
                        op.index,
                        op.spec.name,
                        ErrorCode::UnknownName,
                        format!(
                            "the set holds no sheet `{sheet}`; its sheets: {}",
                            files.sheet_names()
                        ),
                        "sheet",
                    ));
                }
            }
            let extra = draw_order_ref_target(draw_order.as_ref(), "draw_order");
            for (target, key) in around
                .iter()
                .map(|(t, k)| (t, k.as_str()))
                .chain(extra.iter().map(|(t, k)| (*t, k.as_str())))
            {
                let (sheet, field) = match target {
                    Target::Id { sheet, .. } => (sheet, format!("{key}.id")),
                    Target::Name {
                        sheet: Some(sheet), ..
                    } => (sheet, format!("{key}.sheet")),
                    Target::Name { sheet: None, .. } => continue,
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
                        &field,
                    ));
                }
            }
            continue;
        }
        if let Body::DrawOrder { targets, order } = &op.body {
            let extra = draw_order_ref_target(Some(order), "order");
            for (target, key) in targets
                .iter()
                .map(|(t, k)| (t, k.as_str()))
                .chain(extra.iter().map(|(t, k)| (*t, k.as_str())))
            {
                let (sheet, field) = match target {
                    Target::Id { sheet, .. } => (sheet, format!("{key}.id")),
                    Target::Name {
                        sheet: Some(sheet), ..
                    } => (sheet, format!("{key}.sheet")),
                    Target::Name { sheet: None, .. } => continue,
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
                        &field,
                    ));
                }
            }
            continue;
        }
        if let Body::Add {
            draw_order: Some(place),
            ..
        } = &op.body
        {
            if let Some((target, key)) = draw_order_ref_target(Some(place), "draw_order") {
                let check = match target {
                    Target::Id { sheet, .. } => Some((sheet, format!("{key}.id"))),
                    Target::Name {
                        sheet: Some(sheet), ..
                    } => Some((sheet, format!("{key}.sheet"))),
                    Target::Name { sheet: None, .. } => None,
                };
                if let Some((sheet, field)) = check {
                    if files.sheet(sheet).is_none() {
                        return Err(op_error(
                            op.index,
                            op.spec.name,
                            ErrorCode::UnknownName,
                            format!(
                                "the set holds no sheet `{sheet}`; its sheets: {}",
                                files.sheet_names()
                            ),
                            &field,
                        ));
                    }
                }
            }
        }
        if let Body::Copy { targets, .. } | Body::Transform { targets, .. } = &op.body {
            let path = match &op.body {
                Body::Copy {
                    pattern: Pattern::Path { path, .. },
                    ..
                } => Some((path, "path".to_owned())),
                _ => None,
            };
            let named = targets.iter().map(|(t, key)| (t, key.clone())).chain(path);
            for (target, key) in named {
                let (sheet, field) = match target {
                    Target::Id { sheet, .. } => (sheet, format!("{key}.id")),
                    Target::Name {
                        sheet: Some(sheet), ..
                    } => (sheet, format!("{key}.sheet")),
                    Target::Name { sheet: None, .. } => continue,
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
                        &field,
                    ));
                }
            }
            continue;
        }
        let (sheet, field) = match &op.body {
            Body::Add {
                to: AddTo::Template(written),
                ..
            }
            | Body::TextToFront {
                to: AddTo::Template(written),
                ..
            }
            | Body::Stretch {
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
            | Body::TextToFront {
                to: AddTo::Sheet(sheet),
                ..
            }
            | Body::Stretch {
                to: AddTo::Sheet(sheet),
                ..
            }
            | Body::Table {
                to: AddTo::Sheet(sheet),
                ..
            }
            | Body::SetPageSetup {
                to: AddTo::Sheet(sheet),
                ..
            } => (sheet, "sheet"),
            Body::Table {
                to: AddTo::Template(written),
                ..
            }
            | Body::SetPageSetup {
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
            | Body::SetAttribute { target, .. }
            | Body::SetBlockState { target, .. }
            | Body::SetField { target, .. }
            | Body::Offset { target, .. }
            | Body::Break { target, .. }
            | Body::TableEdit { target, .. }
            | Body::LeaderLine { target, .. }
            | Body::DimensionFrom { target, .. } => match target {
                Target::Id { sheet, .. } => (sheet, "target.id"),
                Target::Name {
                    sheet: Some(sheet), ..
                } => (sheet, "target.sheet"),
                Target::Name { sheet: None, .. } => continue,
            },
            Body::Copy { .. }
            | Body::Transform { .. }
            | Body::AddRevisionCloud { .. }
            | Body::DrawOrder { .. } => continue,
            Body::ReplaceText { sheets, .. } | Body::SetAttributes { sheets, .. } => {
                if let Some(names) = sheets {
                    for (i, sheet) in names.iter().enumerate() {
                        if files.sheet(sheet).is_none() {
                            return Err(op_error(
                                op.index,
                                op.spec.name,
                                ErrorCode::UnknownName,
                                format!(
                                    "the set holds no sheet `{sheet}`; its sheets: {}",
                                    files.sheet_names()
                                ),
                                &format!("sheets[{i}]"),
                            ));
                        }
                    }
                }
                continue;
            }
            Body::Trim {
                target, boundaries, ..
            }
            | Body::Extend {
                target, boundaries, ..
            } => {
                let all = std::iter::once((target, "target"))
                    .chain(boundaries.iter().map(|(t, f)| (t, f.as_str())));
                for (t, key) in all {
                    let (sheet, field) = match t {
                        Target::Id { sheet, .. } => (sheet, format!("{key}.id")),
                        Target::Name {
                            sheet: Some(sheet), ..
                        } => (sheet, format!("{key}.sheet")),
                        Target::Name { sheet: None, .. } => continue,
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
                            &field,
                        ));
                    }
                }
                continue;
            }
            Body::Fillet { mode, .. } | Body::Chamfer { mode, .. } => {
                let targets: Vec<(&Target, &str)> = match mode {
                    CornerMode::Polyline { target } => vec![(target, "target")],
                    CornerMode::TwoObjects { first, second, .. } => {
                        vec![(first, "first"), (second, "second")]
                    }
                };
                for (t, key) in targets {
                    let (sheet, field) = match t {
                        Target::Id { sheet, .. } => (sheet, format!("{key}.id")),
                        Target::Name {
                            sheet: Some(sheet), ..
                        } => (sheet, format!("{key}.sheet")),
                        Target::Name { sheet: None, .. } => continue,
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
                            &field,
                        ));
                    }
                }
                continue;
            }
            Body::Join { targets, .. } | Body::Explode { targets, .. } => {
                for (t, key) in targets {
                    let (sheet, field) = match t {
                        Target::Id { sheet, .. } => (sheet, format!("{key}.id")),
                        Target::Name {
                            sheet: Some(sheet), ..
                        } => (sheet, format!("{key}.sheet")),
                        Target::Name { sheet: None, .. } => continue,
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
                            &field,
                        ));
                    }
                }
                continue;
            }
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
fn apply_op(at: &mut Attempt, op: &Operation, settings: &Settings) -> Applied {
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

    if let Body::Add {
        to,
        keys,
        snaps,
        draw_order,
    } = &op.body
    {
        let Kind::Add(object, _) = op.spec.kind else {
            unreachable!("an add is an add")
        };
        let (file, list, sheets, first) = match to {
            AddTo::Sheet(name) => {
                let k = files.sheet(name).expect("checked before");
                let (file, list) = files.row_list(k);
                (file, list, vec![k], k)
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
        let insert_at =
            match resolve_add_insert_at(at, op, file, list, &sheets, draw_order.as_ref()) {
                Ok(pos) => pos,
                Err(applied) => return *applied,
            };
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
        // Each snapped point is written as the coordinates it names on the
        // sheets the object is added to, as they stand now.
        let snaps: Vec<&PointSnap> = snaps.iter().collect();
        let snapped = match snapped_points(at, op, &sheets, &snaps, settings) {
            Ok(snapped) => snapped,
            Err(e) => return fail(sheets, e),
        };
        let mut whole = serde_json::Value::Object(keys);
        for (snap, (point, _)) in snaps.iter().zip(&snapped) {
            if let Some(slot) = whole.pointer_mut(&snap.pointer) {
                *slot = coordinates(*point);
            }
        }
        let serde_json::Value::Object(keys) = whole else {
            unreachable!("a mapping stays one")
        };
        let snapped: Vec<SnapResolved> = snapped.into_iter().map(|(_, r)| r).collect();
        let record = match at.file(file) {
            Ok(record) => record,
            Err(e) => return fail(sheets, e),
        };
        let (index, kept) = match record.add_at(list, object, &keys, insert_at, op.index) {
            Ok(res) => res,
            Err(e) => return fail(sheets, from_record(e)),
        };
        let listed = &record.objects(list)[index];
        let id = format!("{}/{}", files.name(first), listed.id);
        let after = record.object_text(list, index);
        let path = record.path.clone();
        let ids_written: Vec<String> = kept
            .iter()
            .flat_map(|place| {
                sheets
                    .iter()
                    .map(move |&s| format!("{}/{place}", files.name(s)))
            })
            .collect();
        let place = match file {
            FileRef::Rows(_) => format!("to sheet `{}`, in its row", files.name(first)),
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
                row_values: Vec::new(),
                ids_written,
                inserts: Vec::new(),
                attributes_removed: Vec::new(),
                imported: None,
                copies: Vec::new(),
                copy_sheets: Vec::new(),
                transformed: Vec::new(),
                created_row_values: Vec::new(),
                replacements: Vec::new(),
                attributes_set: Vec::new(),
                split_id: None,
                deleted: Vec::new(),
                created: Vec::new(),
                message: format!(
                    "adds {} {object}, `{id}`, {place}{}",
                    article(object),
                    snapped_words(&snapped)
                ),
                snapped,
            }),
            sheets,
            refuse: false,
        };
    }

    if let Body::AddRevisionCloud {
        to,
        around,
        margin,
        keys,
        draw_order,
    } = &op.body
    {
        return apply_revision_cloud_around(
            at,
            op,
            to.as_ref(),
            around,
            margin.as_ref(),
            keys,
            draw_order.as_ref(),
            settings,
        );
    }

    if let Body::DrawOrder { targets, order } = &op.body {
        return apply_draw_order(at, op, targets, order);
    }

    if let Body::TextToFront { to, which } = &op.body {
        return apply_text_to_front(at, op, to, *which);
    }

    if let Body::ReplaceText {
        find,
        replace,
        matching,
        case_sensitive,
        sheets,
        scopes,
        tag,
        block,
    } = &op.body
    {
        return apply_replace_text(
            at,
            op,
            find,
            replace,
            *matching,
            *case_sensitive,
            sheets.as_deref(),
            scopes,
            tag.as_deref(),
            block.as_deref(),
        );
    }

    if let Body::SetAttributes {
        selection,
        sheets,
        tag,
        occurrence,
        value,
    } = &op.body
    {
        return apply_set_attributes(
            at,
            op,
            selection,
            sheets.as_deref(),
            tag,
            *occurrence,
            value,
            settings,
        );
    }

    if let Body::Offset {
        target,
        by,
        id,
        name,
        layer,
        erase_source,
    } = &op.body
    {
        return apply_offset(
            at,
            op,
            target,
            by,
            id.as_deref(),
            name.as_deref(),
            layer.as_deref(),
            *erase_source,
            settings,
        );
    }

    if let Body::Trim {
        target,
        boundaries,
        keep,
        new_id,
    } = &op.body
    {
        return apply_trim(
            at,
            op,
            target,
            boundaries,
            *keep,
            new_id.as_deref(),
            settings,
        );
    }

    if let Body::Extend {
        target,
        boundaries,
        end,
    } = &op.body
    {
        return apply_extend(at, op, target, boundaries, *end, settings);
    }

    if let Body::Fillet { mode, radius } = &op.body {
        return apply_corner(at, op, mode, CornerOp::Fillet(*radius));
    }

    if let Body::Chamfer { mode, spec } = &op.body {
        return apply_corner(at, op, mode, CornerOp::Chamfer(*spec));
    }

    if let Body::Break { target, by, new_id } = &op.body {
        return apply_break(at, op, target, by, new_id.as_deref(), settings);
    }

    if let Body::Join { targets, tolerance } = &op.body {
        return apply_join(at, op, targets, *tolerance);
    }

    if let Body::Stretch {
        to,
        space,
        targets,
        corners,
        motion,
    } = &op.body
    {
        return apply_stretch(
            at,
            op,
            to,
            space.as_deref(),
            targets.as_deref(),
            corners,
            motion,
            settings,
        );
    }

    if let Body::Explode { targets, id_prefix } = &op.body {
        return apply_explode(at, op, targets, id_prefix.as_deref(), settings);
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

    if let Body::SetPageSetup { to, layout, set } = &op.body {
        return apply_page_setup(at, op, to, layout.as_deref(), set);
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

    if let Body::DimensionFrom {
        from,
        target,
        point,
        id,
        name,
    } = &op.body
    {
        return apply_dimension_from(at, op, *from, target, point, id, name, settings);
    }

    if let Body::Copy { targets, pattern } = &op.body {
        return apply_copy(at, op, targets, pattern, settings);
    }
    if let Body::Transform { targets, by } = &op.body {
        return apply_transform(at, op, targets, by, settings);
    }

    let target = match &op.body {
        Body::Change { target, .. }
        | Body::Move { target, .. }
        | Body::Delete { target }
        | Body::SetAttribute { target, .. }
        | Body::SetBlockState { target, .. }
        | Body::SetField { target, .. }
        | Body::TableEdit { target, .. }
        | Body::LeaderLine { target, .. } => target,
        Body::Add { .. }
        | Body::AddRevisionCloud { .. }
        | Body::DrawOrder { .. }
        | Body::TextToFront { .. }
        | Body::ReplaceText { .. }
        | Body::SetAttributes { .. }
        | Body::Offset { .. }
        | Body::Trim { .. }
        | Body::Extend { .. }
        | Body::Fillet { .. }
        | Body::Chamfer { .. }
        | Body::Break { .. }
        | Body::Join { .. }
        | Body::Stretch { .. }
        | Body::Explode { .. }
        | Body::SetPageSetup { .. }
        | Body::Table { .. }
        | Body::Block { .. }
        | Body::DimensionFrom { .. }
        | Body::Copy { .. }
        | Body::Transform { .. } => {
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
    // change_viewport changes a viewport alone, and neither it nor change
    // changes the scale or the view centre of a locked viewport unless it
    // unlocks it too, as AutoCAD's display lock keeps a view from being
    // zoomed or panned.
    if let Body::Change { set, viewport, .. } = &op.body {
        let record = match at.file(file) {
            Ok(record) => record,
            Err(e) => return fail(reach, e),
        };
        let kind = record
            .objects(list)
            .get(index)
            .map(|l| l.kind.clone())
            .unwrap_or_default();
        if *viewport && kind != "viewport" {
            return fail(
                reach,
                op_err(
                    ErrorCode::BadValue,
                    format!(
                        "change_viewport changes a viewport, and `{id}` is {} {kind}; change sets \
                         its keys",
                        article(&kind)
                    ),
                    "target",
                ),
            );
        }
        let locked_now = record.object_value(list, index, "locked").as_deref() == Some("true");
        let view = ["scale", "view_center"]
            .into_iter()
            .find(|key| set.contains_key(*key));
        let unlocks = set.get("locked") == Some(&Value::Bool(false));
        if let (true, true, Some(key), false) = (kind == "viewport", locked_now, view, unlocks) {
            return fail(
                reach,
                op_err(
                    ErrorCode::Locked,
                    format!(
                        "viewport `{id}` is locked, which keeps its scale and its view centre \
                         as they are; send \"locked\": false with the change to unlock it"
                    ),
                    key,
                ),
            );
        }
    }
    // The block a set_attribute's insert places, with the attributes it
    // carries as its library holds it now, so that the operation can tell
    // which of a tag the block defines more than once it sets.
    let carried = match &op.body {
        Body::SetAttribute { .. } | Body::SetField { tag: Some(_), .. } => {
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
    // A move from a base point to a second point, and a leader line's
    // snapped points, are worked out on the sheets the object is on, as
    // they stand now, before it is edited.
    let mut snapped: Vec<SnapResolved> = Vec::new();
    let moved_by = match &op.body {
        Body::Move { motion, .. } => match motion {
            Motion::By(by) => Some(*by),
            Motion::Between { from, to } => {
                let snaps: Vec<&PointSnap> = [&**from, &**to]
                    .into_iter()
                    .filter_map(|p| match p {
                        OpPoint::Snap(snap) => Some(snap),
                        OpPoint::At(_) => None,
                    })
                    .collect();
                let mut found = match snapped_points(at, op, &reach, &snaps, settings) {
                    Ok(found) => found.into_iter(),
                    Err(e) => return fail(reach, e),
                };
                let mut point_of = |p: &OpPoint| match p {
                    OpPoint::At(at) => *at,
                    OpPoint::Snap(_) => {
                        let (point, resolved) = found.next().expect("one for each snap");
                        snapped.push(resolved);
                        point
                    }
                };
                let (a, b) = (point_of(from), point_of(to));
                Some([b[0] - a[0], b[1] - a[1], b[2] - a[2]])
            }
        },
        _ => None,
    };
    // A change_viewport's snapped view centre, worked out as a move's
    // points are and written as coordinates.
    let change_set = match &op.body {
        Body::Change { set, snaps, .. } if !snaps.is_empty() => {
            let only: Vec<&PointSnap> = snaps.iter().map(|(_, s)| s).collect();
            let found = match snapped_points(at, op, &reach, &only, settings) {
                Ok(found) => found,
                Err(e) => return fail(reach, e),
            };
            let mut set = set.clone();
            for ((key, _), (point, resolved)) in snaps.iter().zip(found) {
                set.insert(key.clone(), coordinates(point));
                snapped.push(resolved);
            }
            Some(set)
        }
        // A clip_viewport's keys, worked out from the viewport as it
        // stands, so that model space stays where it showed.
        Body::Change {
            clip: Some(clip), ..
        } => match clipped_keys(at, op, &reach, &id, clip, units) {
            Ok(set) => Some(set),
            Err(e) => return fail(reach, e),
        },
        _ => None,
    };
    let line_edit = match &op.body {
        Body::LeaderLine {
            edit: LeaderLineEdit::Add { points },
            snaps,
            ..
        } => {
            let only: Vec<&PointSnap> = snaps.iter().map(|(_, s)| s).collect();
            let found = match snapped_points(at, op, &reach, &only, settings) {
                Ok(found) => found,
                Err(e) => return fail(reach, e),
            };
            let mut points = points.clone();
            for ((place, _), (point, resolved)) in snaps.iter().zip(found) {
                points[*place] = point;
                snapped.push(resolved);
            }
            Some(LeaderLineEdit::Add { points })
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
        FileRef::Rows(_) => String::new(),
    };
    let mut done = Done {
        record: path,
        id: Some(id.clone()),
        before: Some(before),
        after: None,
        row_value: None,
        row_values: Vec::new(),
        ids_written: Vec::new(),
        inserts: Vec::new(),
        attributes_removed: Vec::new(),
        imported: None,
        copies: Vec::new(),
        copy_sheets: Vec::new(),
        transformed: Vec::new(),
        created_row_values: Vec::new(),
        replacements: Vec::new(),
        attributes_set: Vec::new(),
        split_id: None,
        deleted: Vec::new(),
        created: Vec::new(),
        snapped: Vec::new(),
        message: String::new(),
    };
    let mut sheets = reach.clone();
    let mut from_row: Option<(FromRow, String)> = None;
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
        Body::Change { set, .. } => match record.change(
            list,
            index,
            change_set.as_ref().unwrap_or(set),
            units,
            op.index,
        ) {
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
            Err(e) => {
                // A change of nothing but contents the template takes from
                // the row sets this sheet's row values alone, so a refusal
                // of it reaches this sheet alone (ADR-0005).
                let row_alone = set.keys().all(|key| key == "contents")
                    && record.takes_from_row(list, index, "contents");
                return fail(if row_alone { vec![k] } else { reach }, from_record(e));
            }
        },
        Body::Move { .. } => {
            let by = moved_by.expect("a move's distance");
            match record.move_by(list, index, by, units, op.index) {
                Ok(()) => {
                    let by: Vec<String> = by
                        .iter()
                        .map(|d| crate::description::number_text(*d))
                        .collect();
                    done.message = format!(
                        "moves `{id}` by [{}] ({units}){template_note}{}",
                        by.join(", "),
                        snapped_words(&snapped)
                    );
                }
                Err(e) => return fail(reach, from_record(e)),
            }
        }
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
                Ok(AttributeSet::FromRow(written)) => {
                    edits_object = false;
                    sheets = vec![k];
                    from_row = Some((written, value.clone()));
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
        Body::SetBlockState { state, .. } => {
            if let Err(e) = record.set_block_state(list, index, state, op.index) {
                let field = if e.code == ErrorCode::BadValue {
                    "target"
                } else {
                    "state"
                };
                return fail(reach, op_err(e.code, e.message, field));
            }
            if let Some(refusal) = many_paths(files, file, op, "target") {
                return refusal;
            }
            let summary = state
                .iter()
                .map(|(k, v)| {
                    if v.is_null() {
                        format!("{k}=default")
                    } else {
                        format!("{k}={v}")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            done.message = format!("sets block state ({summary}) of `{id}`{template_note}");
        }
        Body::SetField {
            tag,
            occurrence,
            field,
            value,
            ..
        } => {
            let slot = match tag {
                Some(t) => {
                    let capitals = tag_in_capitals(t);
                    match set_which(
                        &capitals,
                        *occurrence,
                        carried.as_ref(),
                        record,
                        list,
                        index,
                    ) {
                        Ok(item) => item,
                        Err((code, message)) => {
                            return fail(reach, op_err(code, message, "occurrence"))
                        }
                    }
                }
                None => None,
            };
            let fallback_text = if field.is_none() && value.is_none() {
                Some(crate::model::UNEVALUATED_FIELD.to_owned())
            } else {
                None
            };
            if let Err(e) = record.set_field(
                list,
                index,
                tag.as_deref(),
                slot,
                field.as_ref(),
                value.as_deref(),
                fallback_text.as_deref(),
                op.index,
            ) {
                let field_name = e.field.as_deref().unwrap_or("target");
                return fail(reach, op_err(e.code, e.message, field_name));
            }
            if let Some(refusal) = many_paths(files, file, op, "target") {
                return refusal;
            }
            done.message = match (field.is_some(), tag) {
                (true, Some(t)) => {
                    format!("sets field on attribute `{t}` of `{id}`{template_note}")
                }
                (true, None) => format!("sets field of `{id}`{template_note}"),
                (false, Some(t)) => {
                    format!("clears field on attribute `{t}` of `{id}`{template_note}")
                }
                (false, None) => format!("clears field of `{id}`{template_note}"),
            };
        }
        Body::TableEdit { edit, .. } => match record.table_edit(list, index, edit, op.index) {
            Ok(edited) => {
                let mut words = format!("{} of `{id}`", edited.words);
                for note in &edited.notes {
                    words.push_str("; ");
                    words.push_str(note);
                }
                edits_object = edited.edited;
                if edited.edited {
                    done.message = format!("{words}{template_note}");
                    if let Some(refusal) = many_paths(files, file, op, "target") {
                        return refusal;
                    }
                } else {
                    sheets = vec![k];
                    if edited.from_row.is_none() {
                        done.message = format!("{words}, which it gives already");
                    }
                }
                from_row = edited
                    .from_row
                    .map(|(name, text)| (FromRow::Whole(name), text));
            }
            Err(e) => {
                let field = e.field.clone().unwrap_or_default();
                return fail(reach, op_err(e.code, e.message, &field));
            }
        },
        Body::LeaderLine { edit, .. } => {
            let edit = line_edit.as_ref().unwrap_or(edit);
            match record.leader_line_edit(list, index, edit, units, op.index) {
                Ok(words) => {
                    if let Some(refusal) = many_paths(files, file, op, "target") {
                        return refusal;
                    }
                    done.message = format!(
                        "{words} of `{id}`{template_note}{}",
                        snapped_words(&snapped)
                    );
                }
                Err(e) => {
                    let field = e.field.clone().unwrap_or_default();
                    return fail(reach, op_err(e.code, e.message, &field));
                }
            }
        }
        Body::Add { .. }
        | Body::AddRevisionCloud { .. }
        | Body::DrawOrder { .. }
        | Body::TextToFront { .. }
        | Body::ReplaceText { .. }
        | Body::SetAttributes { .. }
        | Body::Offset { .. }
        | Body::Trim { .. }
        | Body::Extend { .. }
        | Body::Fillet { .. }
        | Body::Chamfer { .. }
        | Body::Break { .. }
        | Body::Join { .. }
        | Body::Stretch { .. }
        | Body::Explode { .. }
        | Body::SetPageSetup { .. }
        | Body::Table { .. }
        | Body::Block { .. }
        | Body::DimensionFrom { .. }
        | Body::Copy { .. }
        | Body::Transform { .. } => {
            unreachable!("handled above")
        }
    }
    if let (Some(layer), true) = (&target_locked, edits_object) {
        return fail(reach, locked(&id, layer));
    }
    done.snapped = snapped;
    done.after = Some(record.object_text(list, index));
    if done.after == done.before {
        done.after = None;
    }
    // A row value named alone, of a value the template does not take from
    // the row, names nothing to set.
    if let (Some(asked), None) = (&op.row_value, &from_row) {
        return fail(
            reach,
            op_err(
                ErrorCode::BadValue,
                format!(
                    "row_value names `{asked}`, but the template does not take this value of \
                     `{id}` from the row: it is written where the object is, so send the value \
                     without row_value"
                ),
                ROW_VALUE_KEY,
            ),
        );
    }
    if let Some((written, text)) = from_row {
        // An operation that edits its object as well as a row's value
        // reaches every sheet the object is on, so a refusal of it holds
        // them all back; one that sets the row's value alone reaches its
        // sheet.
        let refused_on = if edits_object { reach.clone() } else { vec![k] };
        let is_mtext = record
            .objects(list)
            .get(index)
            .is_some_and(|object| object.kind == "mtext");
        let set_what = match &op.body {
            Body::SetAttribute { .. } => format!("{attribute_set} of `{id}`"),
            Body::TableEdit {
                edit: TableEdit::SetCell { cell: [r, c], .. },
                ..
            } => format!("the text of cell [{r}, {c}] of `{id}`"),
            Body::Change { set, .. } if set.contains_key("content") => {
                format!("the text of the content of `{id}`")
            }
            Body::Change { set, .. } if set.contains_key("annotation") => {
                format!("the text of the annotation of `{id}`")
            }
            _ => format!("the contents of `{id}`"),
        };
        // The field of the text sent, for a refusal of it.
        let text_field = match &op.body {
            Body::SetAttribute { .. } => "value",
            _ => "set.contents",
        };
        // Each row value to set, and its text; and the parts the text was
        // matched to, when it was.
        let (sets, matched) = match (&written, &op.row_value) {
            (FromRow::Whole(name), None) => (vec![(name.clone(), text.clone())], None),
            (FromRow::Whole(name), Some(asked)) if asked == name => {
                (vec![(name.clone(), text.clone())], None)
            }
            (FromRow::Whole(name), Some(asked)) => {
                return fail(
                    refused_on,
                    op_err(
                        ErrorCode::BadValue,
                        format!(
                            "row_value names `{asked}`, but the template takes {set_what} \
                             whole from the row's value {name}; name {name}, or leave \
                             row_value out"
                        ),
                        ROW_VALUE_KEY,
                    ),
                );
            }
            (FromRow::Parts(held), Some(asked)) => {
                let names = parts::names(held);
                if !names.contains(asked) {
                    return fail(
                        refused_on,
                        op_err(
                            ErrorCode::UnknownName,
                            format!(
                                "row_value names `{asked}`, which is not one of the row values \
                                 of the parts {} the template joins {set_what} from; they are {}",
                                parts::pattern(held),
                                names.join(", ")
                            ),
                            ROW_VALUE_KEY,
                        ),
                    );
                }
                (vec![(asked.clone(), text.clone())], None)
            }
            (FromRow::Parts(held), None) => {
                let pattern = parts::pattern(held);
                // An mtext's text is matched as it shows: without the line
                // breaks it ends with, and CR LF as LF.
                let matched = if is_mtext {
                    parts::split(&parts::shown_mtext_parts(held), &shown_mtext(&text))
                } else {
                    parts::split(held, &text)
                };
                match matched {
                    Ok(values) => (values, Some(pattern)),
                    Err(Unsplit::Differs) => {
                        return fail(
                            refused_on,
                            op_err(
                                ErrorCode::BadValue,
                                format!(
                                    "`{text}` does not match the parts {pattern} the template \
                                     joins {set_what} from: its fixed text differs. Send the \
                                     whole text with the parts' fixed text as it is, or set one \
                                     row value with row_value, one of {}",
                                    parts::names(held).join(", ")
                                ),
                                text_field,
                            ),
                        );
                    }
                    Err(Unsplit::Ambiguous(first, second)) => {
                        let each = |values: &[(String, String)]| {
                            values
                                .iter()
                                .map(|(name, value)| format!("{name} `{value}`"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        return fail(
                            refused_on,
                            op_err(
                                ErrorCode::BadValue,
                                format!(
                                    "`{text}` splits more than one way into the parts {pattern} \
                                     the template joins {set_what} from, such as {} or {}; set \
                                     each row value with row_value, one of {}",
                                    each(&first),
                                    each(&second),
                                    parts::names(held).join(", ")
                                ),
                                text_field,
                            ),
                        );
                    }
                    Err(Unsplit::Unclear) => {
                        return fail(
                            refused_on,
                            op_err(
                                ErrorCode::BadValue,
                                format!(
                                    "`{text}` cannot be told to split one way only into the \
                                     parts {pattern} the template joins {set_what} from: they \
                                     put row values side by side, and the text splits too many \
                                     ways to try. Set each row value with row_value, one of {}",
                                    parts::names(held).join(", ")
                                ),
                                text_field,
                            ),
                        );
                    }
                }
            }
        };
        let t = files.template_of[k];
        let counts = carried_by_blocks(at, t);
        let carries = |block: &str, tag: &str| {
            counts
                .iter()
                .find(|(b, _)| b == block)
                .map(|(_, tags)| tags.iter().filter(|t| *t == tag).count())
        };
        // A value a copy shares with the object it is a copy of, or with
        // another copy of it, becomes the copy's own when an edit changes
        // it, so that the edit sets this copy alone; the others keep the
        // value as they show it (Zach, 2026-11-21). Only an object written
        // with `copy_of` is a copy: the first object of the copies sets the
        // shared value where it is, so the copies not set since follow it,
        // and so does every other place that shares it, as every row value
        // is set (decision 5). Of the values a text matched to its parts
        // sets, only those whose text changes are taken.
        // Of the values a whole text matched to its parts sets, those whose
        // text the edit does not change are dropped: they are neither held
        // to a locked layer, nor made a copy's own, nor written. Each is
        // compared as the matcher reads it: the parts joined with the
        // sheet's row values as they stand, split as the text sent is, so
        // an mtext's last value written as a block of lines, whose last
        // line break the text shows without, is the same.
        let mut sets = sets;
        if let (FromRow::Parts(held), Some(_)) = (&written, &matched) {
            let row = files.rows[k].index;
            let (rows, _) = at.row_record(k);
            let joined: String = held
                .iter()
                .map(|part| match part {
                    parts::Part::Text(text) => text.clone(),
                    parts::Part::Row(name) => rows.row_value(row, name).unwrap_or_default(),
                })
                .collect();
            let now = if is_mtext {
                parts::split(&parts::shown_mtext_parts(held), &shown_mtext(&joined))
            } else {
                parts::split(held, &joined)
            };
            if let Ok(now) = now {
                sets.retain(|(name, value)| {
                    now.iter().find(|(n, _)| n == name).map(|(_, v)| v) != Some(value)
                });
            }
        }
        let mut created: Vec<String> = Vec::new();
        let mut refused_on = refused_on;
        if let FileRef::Template(_) = file {
            let changing: Vec<String> = {
                let row = files.rows[k].index;
                let (rows, _) = at.row_record(k);
                sets.iter()
                    .filter(|(name, value)| rows.row_value(row, name).as_deref() != Some(value))
                    .map(|(name, _)| name.clone())
                    .collect()
            };
            let shared: Vec<String> = match at.template(t) {
                Ok(template) => {
                    let listed = template.objects(ObjectList::Template);
                    let first_of = |i: usize| {
                        template
                            .object_value(ObjectList::Template, i, COPY_OF_KEY)
                            .map(|c| c.to_lowercase())
                    };
                    match first_of(index) {
                        // The object it is a copy of, or another copy of it.
                        Some(first) => {
                            let kin = |i: usize| {
                                listed[i].id.to_lowercase() == first
                                    || first_of(i).as_deref() == Some(first.as_str())
                            };
                            changing
                                .into_iter()
                                .filter(|name| {
                                    template
                                        .row_value_places(name, &carries)
                                        .iter()
                                        .any(|(i, _)| *i != index && kin(*i))
                                })
                                .collect()
                        }
                        None => Vec::new(),
                    }
                }
                Err(_) => Vec::new(),
            };
            if !shared.is_empty() {
                if let Some(refusal) = many_paths(files, file, op, "target") {
                    return refusal;
                }
                let built_from = files.templates[t].sheets.clone();
                let local = id.split_once(ID_SEPARATOR).map_or(id.as_str(), |(_, l)| l);
                // Its own value's name: the value's and the object's ID,
                // joined by `-`, `_` for a character a value's name does not
                // take, and a number after it when the name is taken.
                let mut taken: BTreeSet<String> = BTreeSet::new();
                for &s in &built_from {
                    let (rows, l) = at.row_record(s);
                    if let ObjectList::Row(r) = l {
                        taken.extend(rows.row_value_names(r));
                    }
                }
                let renamed: Vec<(String, String)> = shared
                    .iter()
                    .map(|old| {
                        let base: String = format!("{old}-{local}")
                            .chars()
                            .map(|c| {
                                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                                    c
                                } else {
                                    '_'
                                }
                            })
                            .collect();
                        let new = std::iter::once(base.clone())
                            .chain((2..).map(|n| format!("{base}-{n}")))
                            .find(|n| !taken.contains(n))
                            .expect("a free name");
                        taken.insert(new.clone());
                        (old.clone(), new)
                    })
                    .collect();
                match at.template(t) {
                    Ok(template) => {
                        if let Err(e) = template.rename_row_values(list, index, &renamed, op.index)
                        {
                            return fail(reach, from_record(e));
                        }
                        done.after = Some(template.object_text(list, index));
                    }
                    Err(e) => return fail(reach, e),
                }
                // On every sheet built from the template, the new value is
                // what the object showed; on this sheet it is then set.
                for &s in &built_from {
                    let (rows, l) = at.row_record(s);
                    let ObjectList::Row(r) = l else {
                        continue;
                    };
                    for (old, new) in &renamed {
                        if let Some(text) = rows.row_value(r, old) {
                            if let Err(e) = rows.add_row_value(r, new, &text) {
                                return fail(reach, from_record(e));
                            }
                        }
                    }
                }
                for (name, _) in sets.iter_mut() {
                    if let Some((_, new)) = renamed.iter().find(|(old, _)| old == name) {
                        new.clone_into(name);
                    }
                }
                created = renamed.into_iter().map(|(_, new)| new).collect();
                sheets.clone_from(&reach);
                refused_on.clone_from(&reach);
            }
        }
        // A row's value fills every place on the sheet its template takes
        // it, and each is edited: none may be on a locked layer.
        if let Ok(template) = at.template(t) {
            let listed = template.objects(ObjectList::Template);
            for (name, _) in &sets {
                for (i, _) in template.row_value_places(name, &carries) {
                    let layer = template
                        .object_value(ObjectList::Template, i, "layer")
                        .unwrap_or_else(|| "0".to_owned());
                    if template
                        .table_value(Table::Layer.key(), &layer, "locked")
                        .as_deref()
                        == Some("true")
                    {
                        let filled = format!("{}/{}", files.name(k), listed[i].id);
                        return fail(refused_on, locked(&filled, &layer));
                    }
                }
            }
        }
        let mut set_values: Vec<RowValueSet> = Vec::new();
        for (name, value) in sets {
            let row = files.rows[k].index;
            let (record, _) = at.row_record(k);
            let was = record.row_value(row, &name).unwrap_or_default();
            if let Err(e) = record.set_row_value(row, &name, &value) {
                return fail(refused_on, from_record(e));
            }
            at.values.retain(|(r, n, _)| !(*r == k && *n == name));
            at.values.push((k, name.clone(), op.index));
            let fills = match at.template(t) {
                Ok(template) => {
                    let listed = template.objects(ObjectList::Template);
                    template
                        .row_value_places(&name, &carries)
                        .into_iter()
                        .map(|(i, place)| Fill {
                            id: format!("{}/{}", files.name(k), listed[i].id),
                            at: place,
                        })
                        .collect::<Vec<_>>()
                }
                Err(_) => Vec::new(),
            };
            set_values.push(RowValueSet {
                sheet: files.name(k).to_owned(),
                value: name,
                was,
                now: value,
                fills,
            });
        }
        let places = |set: &RowValueSet| {
            set.fills
                .iter()
                .map(|f| format!("{} of `{}`", f.at, f.id))
                .collect::<Vec<String>>()
                .join(" and ")
        };
        let row_message = if set_values.is_empty() {
            format!(
                "changes no row value of sheet `{}`: the text sent is what the parts {} its                  template joins {set_what} from show already",
                files.name(k),
                matched.as_deref().unwrap_or_default()
            )
        } else {
            match (&written, &matched) {
                (FromRow::Whole(_), _) => {
                    let set = &set_values[0];
                    format!(
                    "sets sheet `{}`'s row value {} to `{}`, since its template takes {set_what} \
                     from the row; the value fills {}",
                    files.name(k),
                    set.value,
                    set.now,
                    places(set)
                )
                }
                (FromRow::Parts(held), None) => {
                    let set = &set_values[0];
                    format!(
                        "sets sheet `{}`'s row value {} to `{}`, one of the parts {} its template \
                     joins {set_what} from; the value fills {}",
                        files.name(k),
                        set.value,
                        set.now,
                        parts::pattern(held),
                        places(set)
                    )
                }
                (FromRow::Parts(_), Some(pattern)) => {
                    let each: Vec<String> = set_values
                        .iter()
                        .map(|set| format!("{} to `{}`", set.value, set.now))
                        .collect();
                    let fills: Vec<String> = set_values
                        .iter()
                        .map(|set| format!("{} fills {}", set.value, places(set)))
                        .collect();
                    format!(
                        "sets sheet `{}`'s row {} {}, matched to the parts {pattern} its template \
                     joins {set_what} from; {}",
                        files.name(k),
                        if set_values.len() == 1 {
                            "value"
                        } else {
                            "values"
                        },
                        join_and(&each),
                        fills.join("; ")
                    )
                }
            }
        };
        done.message = if done.message.is_empty() {
            row_message
        } else {
            format!("{}; and {row_message}", done.message)
        };
        if !created.is_empty() {
            let names: Vec<String> = created.iter().map(|n| format!("`{n}`")).collect();
            done.message.push_str(&format!(
                "; `{id}` shared the value with its copies, so it takes its own row value {}, \
                 written in the row of every sheet built from the template, set to what it \
                 showed there, and to the new value on sheet `{}`; the others keep theirs",
                join_and(&names),
                files.name(k)
            ));
            done.created_row_values = created;
        }
        if !sheets.contains(&k) {
            sheets.push(k);
        }
        if sheets == vec![k] && matches!(file, FileRef::Template(_)) {
            done.record = files.rows_path(k).to_owned();
        }
        if matched.is_some() {
            done.row_values = set_values;
        } else {
            done.row_value = set_values.pop();
        }
    }
    Applied {
        sheets,
        outcome: Ok(done),
        refuse: false,
    }
}

/// The block each insert of the template `t` places, each once, with the
/// tags it carries, in order, as its library holds it now: how many times
/// a block carries a tag says whether a list given for it is a list of
/// parts.
fn carried_by_blocks(at: &mut Attempt, t: usize) -> Vec<(String, Vec<String>)> {
    let blocks: Vec<String> = match at.template(t) {
        Ok(template) => template
            .inserts(ObjectList::Template)
            .into_iter()
            .map(|insert| insert.block)
            .collect(),
        Err(_) => return Vec::new(),
    };
    let Some(l) = at.files.library_of[t] else {
        return Vec::new();
    };
    let Ok(library) = at.library(l) else {
        return Vec::new();
    };
    let mut carried: Vec<(String, Vec<String>)> = Vec::new();
    for block in blocks {
        if carried.iter().any(|(b, _)| *b == block) {
            continue;
        }
        if let Some(tags) = library.block_carried_tags(&block) {
            carried.push((block, tags));
        }
    }
    carried
}

/// Items in words, the last after `and`: `A`, `A and B`, `A, B and C`.
fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
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

/// Applies `set_page_setup` (`PAGESETUP`) to a template's top-level or named
/// layout `page_setup`.
fn apply_page_setup(
    at: &mut Attempt,
    op: &Operation,
    to: &AddTo,
    layout: Option<&str>,
    set: &serde_json::Map<String, serde_json::Value>,
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
    let written = files.templates[t].written.clone();
    let record = match at.template(t) {
        Ok(record) => record,
        Err(e) => return fail(e.code, e.message, ""),
    };
    let path = record.path.clone();
    let target_label = match record.set_page_setup(layout, set) {
        Ok(label) => label,
        Err(e) => {
            let field = e.field.as_deref().unwrap_or("");
            return fail(e.code, e.message, field);
        }
    };
    Applied {
        outcome: Ok(Done {
            record: path,
            id: None,
            before: None,
            after: None,
            row_value: None,
            row_values: Vec::new(),
            ids_written: Vec::new(),
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed: Vec::new(),
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set: Vec::new(),
            split_id: None,
            deleted: Vec::new(),
            created: Vec::new(),
            snapped: Vec::new(),
            message: format!(
                "updates {target_label} in template `{written}`, so on {}",
                every_sheet(&files.names(&sheets))
            ),
        }),
        sheets,
        refuse: false,
    }
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
                // A dimension style draws its text in a text style.
                if table == Table::TextStyle {
                    record.table_rename_values(Table::DimensionStyle.key(), "text_style", name, to);
                }
                let mut count = record.rename_values(ObjectList::Template, by, name, to, op.index);
                // A layer frozen in a viewport stays frozen there.
                if table == Table::Layer {
                    count += record.rename_in_lists(
                        ObjectList::Template,
                        FROZEN_LAYERS,
                        name,
                        to,
                        op.index,
                    );
                }
                for &k in &sheets {
                    let (f, list) = row_place(files, k);
                    count += at.rows[f].rename_values(list, by, name, to, op.index);
                }
                if table == Table::Layer {
                    for &k in &sheets {
                        let (f, list) = row_place(files, k);
                        count +=
                            at.rows[f].rename_in_lists(list, FROZEN_LAYERS, name, to, op.index);
                    }
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
                // A viewport that freezes the layer uses it too.
                if table == Table::Layer {
                    users.extend(
                        record
                            .list_users(ObjectList::Template, FROZEN_LAYERS, name)
                            .into_iter()
                            .map(|id| format!("{}/{id}", files.name(sheets[0]))),
                    );
                }
                for &k in &sheets {
                    let (f, list) = row_place(files, k);
                    users.extend(
                        at.rows[f]
                            .object_users(list, by, name)
                            .into_iter()
                            .map(|id| format!("{}/{id}", files.name(k))),
                    );
                }
                if table == Table::Layer {
                    for &k in &sheets {
                        let (f, list) = row_place(files, k);
                        users.extend(
                            at.rows[f]
                                .list_users(list, FROZEN_LAYERS, name)
                                .into_iter()
                                .map(|id| format!("{}/{id}", files.name(k))),
                        );
                    }
                }
                let template = at.template(t).expect("read above");
                let layers = if table == Table::Linetype {
                    template.table_users(Table::Layer.key(), "linetype", name)
                } else {
                    Vec::new()
                };
                let styles = if table == Table::TextStyle {
                    template.table_users(Table::DimensionStyle.key(), "text_style", name)
                } else {
                    Vec::new()
                };
                if !users.is_empty() || !layers.is_empty() || !styles.is_empty() {
                    let mut what = Vec::new();
                    if !layers.is_empty() {
                        let s = if layers.len() == 1 { "" } else { "s" };
                        what.push(format!("the layer{s} {}", layers.join(", ")));
                    }
                    if !styles.is_empty() {
                        let s = if styles.len() == 1 { "" } else { "s" };
                        what.push(format!("the dimension style{s} {}", styles.join(", ")));
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
            row_values: Vec::new(),
            ids_written: Vec::new(),
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed: Vec::new(),
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set: Vec::new(),
            split_id: None,
            deleted: Vec::new(),
            created: Vec::new(),
            snapped: Vec::new(),
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
    // The tags each block to bring into line carries just before this
    // operation, as the change set has built the library so far.
    let carried_before: Vec<(String, Vec<String>)> = match (block_op, name) {
        (BlockOp::Redefine | BlockOp::Sync, Some(name)) => record
            .block_carried_tags(name)
            .map(|tags| vec![(name.to_owned(), tags)])
            .unwrap_or_default(),
        (BlockOp::Sync, None) => names
            .iter()
            .map(|n| (n.clone(), record.block_carried_tags(n).unwrap_or_default()))
            .collect(),
        _ => Vec::new(),
    };
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
        match sync(at, l, &to_sync, &carried_before, op) {
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
            row_values: Vec::new(),
            ids_written: Vec::new(),
            inserts: synced.inserts,
            attributes_removed: synced.removed,
            imported: None,
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed: Vec::new(),
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set: Vec::new(),
            split_id: None,
            deleted: Vec::new(),
            created: Vec::new(),
            snapped: Vec::new(),
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
    // The tags each block it replaces carries just before the import, as
    // the change set has built the library so far.
    let carried_before: Vec<(String, Vec<String>)> = planned
        .replaced()
        .into_iter()
        .map(|b| {
            let tags = record.block_carried_tags(&b).unwrap_or_default();
            (b, tags)
        })
        .collect();
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
        match sync(at, l, &replaced, &carried_before, op) {
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
            row_values: Vec::new(),
            ids_written: Vec::new(),
            inserts: synced.inserts,
            attributes_removed: synced.removed,
            imported: Some(imported),
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed: Vec::new(),
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set: Vec::new(),
            split_id: None,
            deleted: Vec::new(),
            created: Vec::new(),
            snapped: Vec::new(),
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
///
/// `before` gives the tags each block carried, in order, just before the
/// operation, as the change set had built the library so far: before a
/// redefinition or an import its old definition, and for an attsync the
/// definition in force.
fn sync(
    at: &mut Attempt,
    l: usize,
    blocks: &[String],
    before: &[(String, Vec<String>)],
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
    // A list given for a tag the block carries once now is a list of parts
    // (#102) only when the block carried the tag once just before the
    // operation, as `before` gives it, too; otherwise it is a repeated
    // tag's values, which keep their first.
    let once_before = |block: &str, tag: &str| {
        before
            .iter()
            .find(|(b, _)| b == block)
            .is_some_and(|(_, tags)| tags.iter().filter(|t| *t == tag).count() == 1)
    };
    // What fitting `given` to the block `block` takes away, and how many
    // times the block carries its tag; none when it fits as it is.
    let unfit = |block: &str, given: &GivenValue| {
        let n = count(block, &given.tag)?;
        let mut given = given.clone();
        given.parts = given.parts && once_before(block, &given.tag);
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
    for &k in &library.sheets {
        let rows_path = files.rows_path(k).display().to_string();
        let (f, list) = row_place(files, k);
        let listed = at.rows[f].objects(list);
        let mut edits = Vec::new();
        for insert in at.rows[f].inserts(list) {
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
            at.rows[f]
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
                .filter_map(|mut given| {
                    // A block's insert takes no value from a row, so no
                    // list of parts.
                    given.parts = false;
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

/// Adds a dimension `from` the one `target` names, to `point`, written
/// where that one is, with the id and the name given: AutoCAD's
/// DIMBASELINE and DIMCONTINUE.
#[allow(clippy::too_many_arguments)]
fn apply_dimension_from(
    at: &mut Attempt,
    op: &Operation,
    from: FromOp,
    target: &Target,
    point: &OpPoint,
    id: &Option<String>,
    name: &Option<String>,
    settings: &Settings,
) -> Applied {
    let files = at.files;
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    let fail = |sheets: Vec<usize>, e: Error| Applied {
        sheets,
        outcome: Err(e),
        refuse: false,
    };
    let found = match resolve(at, target, op) {
        Ok(found) => found,
        Err(applied) => return *applied,
    };
    let reach = at.reach(found.file, found.row);
    if let Some(refusal) = many_paths(files, found.file, op, "target") {
        return refusal;
    }
    // The dimension it is added from, as the sheet it is on stands now.
    let rows = match at.rows_now() {
        Ok(rows) => rows,
        Err(e) => return fail(reach, e),
    };
    let record = match sheet_record(at, &rows, found.row, &mut Vec::new()) {
        Ok(record) => record,
        Err(e) => return fail(reach, e),
    };
    let local = found
        .id
        .split_once(ID_SEPARATOR)
        .map_or(found.id.as_str(), |(_, local)| local);
    let in_row = matches!(found.file, FileRef::Rows(_));
    let base = record
        .objects
        .iter()
        .position(|o| o.in_row == in_row && o.id.eq_ignore_ascii_case(local))
        .and_then(|i| record.sheet.objects.get(i));
    let Some(crate::model::Object::Dimension(base)) = base else {
        let what = base.map_or("object", |o| o.type_name());
        return fail(
            reach,
            op_err(
                ErrorCode::BadValue,
                format!(
                    "`{}` is {} {what}; a dimension is added from a dimension",
                    found.id,
                    article(what)
                ),
                "target",
            ),
        );
    };
    let Some(style) = record.sheet.dimension_style(&base.dimension_style) else {
        return fail(
            reach,
            op_err(
                ErrorCode::UnknownName,
                format!(
                    "`{}` draws with the dimension style `{}`, which its sheet does not have",
                    found.id, base.dimension_style
                ),
                "target",
            ),
        );
    };
    let mut snapped: Vec<SnapResolved> = Vec::new();
    let xyz = match point {
        OpPoint::At(p) => *p,
        OpPoint::Snap(snap) => match snapped_points(at, op, &reach, &[snap], settings) {
            Ok(mut found) => {
                let (p, resolved) = found.remove(0);
                snapped.push(resolved);
                p
            }
            Err(e) => return fail(reach, e),
        },
    };
    let to = crate::model::Point3::new(xyz[0], xyz[1], xyz[2]);
    let mut keys = match dimension_from(base, &style, from, to) {
        Ok(keys) => keys,
        Err(why) => return fail(reach, op_err(ErrorCode::BadValue, why, "target")),
    };
    let new_id = match id {
        Some(id) => id.clone(),
        None => {
            let mut taken: BTreeSet<String> = BTreeSet::new();
            for &k in &reach {
                match at.objects(k) {
                    Ok(objects) => {
                        taken.extend(objects.into_iter().map(|o| o.3.id.to_lowercase()));
                    }
                    Err(e) => return fail(reach.clone(), e),
                }
            }
            (1..)
                .map(|n| format!("dimension-{n}"))
                .find(|id| !taken.contains(id))
                .expect("a free number")
        }
    };
    keys.insert("id".into(), serde_json::Value::String(new_id));
    if let Some(name) = name {
        keys.insert("name".into(), serde_json::Value::String(name.clone()));
    }
    let first = reach[0];
    let file = match at.file(found.file) {
        Ok(file) => file,
        Err(e) => return fail(reach, e),
    };
    let index = match file.add(found.list, "dimension", &keys, op.index) {
        Ok(index) => index,
        Err(e) => return fail(reach, op_err(e.code, e.message, "point")),
    };
    let listed = &file.objects(found.list)[index];
    let added = format!("{}/{}", files.name(first), listed.id);
    let after = file.object_text(found.list, index);
    let path = file.path.clone();
    let what = match from {
        FromOp::Baseline => "a baseline dimension",
        FromOp::Continued => "a continued dimension",
    };
    let place = match found.file {
        FileRef::Rows(_) => format!("to sheet `{}`, in its row", files.name(first)),
        FileRef::Template(t) => format!(
            "to the template `{}`, so it is on {}",
            files.templates[t].written,
            every_sheet(&files.names(&reach))
        ),
    };
    Applied {
        outcome: Ok(Done {
            record: path,
            id: Some(added.clone()),
            before: None,
            after: Some(after),
            row_value: None,
            row_values: Vec::new(),
            ids_written: Vec::new(),
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed: Vec::new(),
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set: Vec::new(),
            split_id: None,
            deleted: Vec::new(),
            created: Vec::new(),
            message: format!(
                "adds {what}, `{added}`, from `{}`, {place}{}",
                found.id,
                snapped_words(&snapped)
            ),
            snapped,
        }),
        sheets: reach,
        refuse: false,
    }
}

/// `applied`'s error placed at `key` of the operation, such as `targets[1]`
/// or `path`, where [`resolve`] places it at `target`.
fn keyed(mut applied: Applied, op: &Operation, key: &str) -> Applied {
    if let Err(e) = &mut applied.outcome {
        let at = format!("operations[{}].target", op.index);
        if let Some(rest) = e.field.as_deref().and_then(|f| f.strip_prefix(&at)) {
            e.field = Some(format!("operations[{}].{key}{rest}", op.index));
        }
    }
    applied
}

/// The id a copy of `listed` is given before its number: its source's id,
/// or, for one whose ID is its place, such as `objects[2]`, its type.
fn copy_base(listed: &Listed) -> String {
    if listed.written && !listed.id.contains('[') && listed.id.len() + 8 <= LONGEST_ID {
        listed.id.clone()
    } else {
        listed.kind.clone()
    }
}

/// The objects `targets` names, each with its key in the operation, all
/// of one sheet; the sheets they reach; and that sheet. Every object is
/// looked up before any fails the operation, so that a lookup that fails
/// fails every sheet the objects found reach too: a sheet that shares
/// their template is not left to take the change set's other operations
/// without its part of this one (ADR-0005). An object named twice, one on
/// a locked layer, which AutoCAD does not select, and objects of two
/// sheets are refused.
#[allow(clippy::type_complexity)]
fn found_objects<'t>(
    at: &mut Attempt,
    op: &Operation,
    targets: &'t [(Target, String)],
) -> Result<(Vec<(Found, &'t str)>, Vec<usize>, usize), Box<Applied>> {
    let files = at.files;
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    let fail = |sheets: Vec<usize>, e: Error| {
        Box::new(Applied {
            sheets,
            outcome: Err(e),
            refuse: false,
        })
    };
    let (one_sheet, done) = match op.spec.kind {
        Kind::Rotate => ("one rotate turns", "turned"),
        Kind::Scale => ("one scale scales", "scaled"),
        Kind::Mirror => ("one mirror mirrors", "mirrored"),
        _ => ("one copy or array copies", "copied"),
    };
    let mut found: Vec<(Found, &str)> = Vec::with_capacity(targets.len());
    let mut lookup: Option<Applied> = None;
    for (target, key) in targets {
        match resolve(at, target, op) {
            Ok(object) => found.push((object, key)),
            Err(applied) if applied.refuse => return Err(Box::new(keyed(*applied, op, key))),
            Err(applied) => {
                lookup.get_or_insert_with(|| keyed(*applied, op, key));
            }
        }
    }
    let mut reach: BTreeSet<usize> = BTreeSet::new();
    for (f, _) in &found {
        reach.extend(at.reach(f.file, f.row));
    }
    if let Some(mut failed) = lookup {
        reach.extend(failed.sheets.iter().copied());
        failed.sheets = reach.into_iter().collect();
        return Err(Box::new(failed));
    }
    let reach: Vec<usize> = reach.into_iter().collect();
    let k = found[0].0.row;
    if let Some((other, key)) = found.iter().find(|(f, _)| f.row != k) {
        return Err(fail(
            reach.clone(),
            op_err(
                ErrorCode::BadValue,
                format!(
                    "`{}` is on sheet `{}` and `{}` on sheet `{}`; {one_sheet} objects of one \
                     sheet, so send one for each sheet",
                    found[0].0.id,
                    files.name(k),
                    other.id,
                    files.name(other.row)
                ),
                key,
            ),
        ));
    }
    for (i, (f, key)) in found.iter().enumerate() {
        if let Some((_, first)) = found[..i]
            .iter()
            .find(|(g, _)| (g.file, g.list, g.index) == (f.file, f.list, f.index))
        {
            return Err(fail(
                reach,
                op_err(
                    ErrorCode::BadValue,
                    format!(
                        "`{}` is named twice, at {first} and here; name it once",
                        f.id
                    ),
                    key,
                ),
            ));
        }
        if let Some(layer) = locked_layer(at, f) {
            return Err(fail(
                reach,
                op_err(
                    ErrorCode::Locked,
                    format!(
                        "`{}` sits on layer `{layer}`, which is locked, so it is not {done}; \
                         unlock it first with change_layer, set: {{\"locked\": false}}, in \
                         this change set or an earlier one",
                        f.id
                    ),
                    key,
                ),
            ));
        }
        if let Some(refusal) = many_paths(files, f.file, op, key) {
            return Err(Box::new(refusal));
        }
    }
    Ok((found, reach, k))
}

/// What each of `found` draws with that a transform `t` writes as its own:
/// a leader's and a multileader's lengths from its style or the sheet's
/// defaults, which a scaling scales, and a table's size, which a mirror
/// places it by; worked out on sheet `k` as the change set has left it.
fn resolved_for(
    at: &Attempt,
    op: &Operation,
    k: usize,
    found: &[(Found, &str)],
    t: &Transform,
) -> Result<Vec<Resolved>, Error> {
    let scaled = t.factor() != 1.0;
    let mut resolved = vec![Resolved::default(); found.len()];
    if !scaled && !t.mirrors() {
        return Ok(resolved);
    }
    let needs = |o: &Object| match o {
        Object::MLeader(_) | Object::Leader(_) => scaled,
        Object::Table(_) => t.mirrors(),
        _ => false,
    };
    let rows = at.rows_now()?;
    let mut sheet: Option<SheetRecord> = None;
    for (i, (f, _)) in found.iter().enumerate() {
        if sheet.is_none() {
            sheet = Some(
                sheet_record(at, &rows, k, &mut Vec::new()).map_err(|e| Error {
                    message: format!(
                        "operation {} (`{}`): sheet `{}` cannot be read to work out what its \
                     objects draw with: {}",
                        op.index,
                        op.spec.name,
                        at.files.name(k),
                        e.message
                    ),
                    ..e
                })?,
            );
        }
        let record = sheet.as_ref().expect("read above");
        let Some(place) = model_index(record, f) else {
            continue;
        };
        let object = &record.sheet.objects[place];
        if !needs(object) {
            continue;
        }
        resolved[i] = match object {
            Object::MLeader(m) => {
                let style = record.sheet.mleader_style(&m.mleader_style);
                let mut lengths = Vec::new();
                if let Some(style) = style {
                    lengths.push(("arrow_size", m.arrow_size.unwrap_or(style.arrow_size)));
                    lengths.push(("landing", m.landing.unwrap_or(style.landing)));
                    if let crate::model::MLeaderContent::Text(text) = &m.content {
                        lengths.push(("gap", m.gap.unwrap_or(style.landing_gap)));
                        lengths.push(("content.height", text.height.unwrap_or(style.text_height)));
                    }
                }
                Resolved {
                    lengths,
                    size: None,
                }
            }
            Object::Leader(l) => {
                let mut lengths = vec![("arrow_size", l.arrow_size), ("gap", l.gap)];
                if let Some(annotation) = &l.annotation {
                    lengths.push(("annotation.height", annotation.height));
                }
                Resolved {
                    lengths,
                    size: None,
                }
            }
            Object::Table(table) => Resolved {
                lengths: Vec::new(),
                size: Some([table.columns.iter().sum(), table.rows.iter().sum()]),
            },
            _ => Resolved::default(),
        };
    }
    Ok(resolved)
}

/// Turns, scales or mirrors the objects `targets` names in place, as
/// AutoCAD's ROTATE, SCALE and MIRROR with its source erased do (#67): each
/// keeps its ID and is written where it is, a template's object in the
/// template, so on every sheet built from it, all or nothing, and a row's
/// own object in the row.
fn apply_transform(
    at: &mut Attempt,
    op: &Operation,
    targets: &[(Target, String)],
    by: &TransformBy,
    settings: &Settings,
) -> Applied {
    let files = at.files;
    let units = files.units;
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    let fail = |sheets: Vec<usize>, e: Error| Applied {
        sheets,
        outcome: Err(e),
        refuse: false,
    };
    let (found, reach, k) = match found_objects(at, op, targets) {
        Ok(found) => found,
        Err(applied) => return *applied,
    };
    // The points, worked out on the sheets the objects reach as they stand
    // now.
    let mut snapped: Vec<SnapResolved> = Vec::new();
    let mut point = |at: &Attempt, p: &OpPoint| match p {
        OpPoint::At(xyz) => Ok(*xyz),
        OpPoint::Snap(snap) => {
            snapped_points(at, op, &reach, &[snap], settings).map(|mut found| {
                let (xyz, resolved) = found.remove(0);
                snapped.push(resolved);
                xyz
            })
        }
    };
    let t = match by {
        TransformBy::Rotate { base_point, angle } => match point(at, base_point) {
            Ok(base) => Transform::rotate(base, *angle),
            Err(e) => return fail(reach, e),
        },
        TransformBy::Scale { base_point, factor } => match point(at, base_point) {
            Ok(base) => Transform::scale(base, *factor),
            Err(e) => return fail(reach, e),
        },
        TransformBy::Mirror { first, second } => {
            let a = match point(at, first) {
                Ok(a) => a,
                Err(e) => return fail(reach, e),
            };
            let b = match point(at, second) {
                Ok(b) => b,
                Err(e) => return fail(reach, e),
            };
            match Transform::mirror(a, b) {
                Some(t) => t,
                None => return fail(reach, no_mirror_line(op)),
            }
        }
    };
    // What the sheet description cannot hold is refused, never
    // approximated.
    for (f, key) in &found {
        let why = match at.file(f.file) {
            Ok(record) => record.refuses_transform(f.list, f.index, &t, units),
            Err(e) => return fail(reach, e),
        };
        if let Some(why) = why {
            return fail(
                reach,
                op_err(ErrorCode::BadValue, format!("`{}`: {why}", f.id), key),
            );
        }
    }
    let resolved = match resolved_for(at, op, k, &found, &t) {
        Ok(resolved) => resolved,
        Err(e) => return fail(reach, e),
    };
    let one = found.len() == 1;
    let mut before = None;
    let mut after = None;
    let mut transformed: Vec<(String, Vec<usize>)> = Vec::with_capacity(found.len());
    for ((f, _), resolved) in found.iter().zip(&resolved) {
        let on = at.reach(f.file, f.row);
        let record = match at.file(f.file) {
            Ok(record) => record,
            Err(e) => return fail(reach, e),
        };
        if one {
            before = Some(record.object_text(f.list, f.index));
        }
        if let Err(e) = record.transform(f.list, f.index, &t, resolved, units, op.index) {
            return fail(reach, op_err(e.code, e.message, ""));
        }
        if one {
            after = Some(record.object_text(f.list, f.index));
        }
        transformed.push((f.id.clone(), on));
    }
    let what = join_and(
        &found
            .iter()
            .map(|(f, _)| format!("`{}`", f.id))
            .collect::<Vec<_>>(),
    );
    let message = format!(
        "{} {what} {}{}",
        match t.how() {
            How::Rotate(_) => "turns",
            How::Scale(_) => "scales",
            How::Mirror(_) => "mirrors",
        },
        transform_words(&t, by, units),
        where_written(files, &found, k)
    ) + &snapped_words(&snapped);
    Applied {
        outcome: Ok(Done {
            record: record_of(files, &found, k),
            id: one.then(|| found[0].0.id.clone()),
            before,
            after,
            row_value: None,
            row_values: Vec::new(),
            ids_written: Vec::new(),
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            snapped,
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed,
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set: Vec::new(),
            split_id: None,
            deleted: Vec::new(),
            created: Vec::new(),
            message,
        }),
        sheets: reach,
        refuse: false,
    }
}

/// The error of a mirror whose two points are one seen from above.
fn no_mirror_line(op: &Operation) -> Error {
    op_error(
        op.index,
        op.spec.name,
        ErrorCode::BadValue,
        "second_point is first_point seen from above, so the two give no line to mirror \
         across; give two points apart",
        "second_point",
    )
}

/// How a transform moves its objects, in words: `by 90 degrees about
/// [1, 2, 0] (in)`.
fn transform_words(t: &Transform, by: &TransformBy, units: Unit) -> String {
    let point = |p: [f64; 3]| {
        format!(
            "[{}]",
            p.iter()
                .map(|c| crate::description::number_text(*c))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    match (t.how(), by) {
        (How::Rotate(angle), _) => format!(
            "by {} degrees about {} ({units}), in place",
            crate::description::number_text(angle),
            point(t.about())
        ),
        (How::Scale(factor), _) => format!(
            "by {} about {} ({units}), in place",
            crate::description::number_text(factor),
            point(t.about())
        ),
        (How::Mirror(axis), _) => format!(
            "across the line through {} ({units}) at {} degrees, in place, erasing the source",
            point(t.about()),
            crate::description::number_text(axis)
        ),
    }
}

/// Where an operation on `found`, of sheet `k`, writes them, in words: `,
/// in the template ..., so on every sheet built from it: ...` and `, in
/// sheet ...'s row`.
fn where_written(files: &SetFiles, found: &[(Found, &str)], k: usize) -> String {
    let mut written: Vec<String> = Vec::new();
    let in_template: BTreeSet<usize> = found
        .iter()
        .filter_map(|(f, _)| match f.file {
            FileRef::Template(t) => Some(t),
            FileRef::Rows(_) => None,
        })
        .collect();
    for t in in_template {
        let sheets = &files.templates[t].sheets;
        written.push(if sheets.len() > 1 {
            format!(
                "in the template `{}`, so on {}",
                files.templates[t].written,
                every_sheet(&files.names(sheets))
            )
        } else {
            format!("in the template `{}`", files.templates[t].written)
        });
    }
    if found
        .iter()
        .any(|(f, _)| matches!(f.file, FileRef::Rows(_)))
    {
        written.push(format!("in sheet `{}`'s row", files.name(k)));
    }
    format!(", written {}", written.join(" and "))
}

/// The record an operation on `found`, of sheet `k`, writes, as for every
/// edit that writes a row and a template: the template when it is built
/// into more than one sheet, and the rows file when it serves the one
/// sheet alone.
fn record_of(files: &SetFiles, found: &[(Found, &str)], k: usize) -> PathBuf {
    let template = found
        .iter()
        .filter_map(|(f, _)| match f.file {
            FileRef::Template(t) => Some(t),
            FileRef::Rows(_) => None,
        })
        .max_by_key(|&t| files.templates[t].sheets.len());
    let writes_rows = found
        .iter()
        .any(|(f, _)| matches!(f.file, FileRef::Rows(_)));
    match template {
        Some(t) if !writes_rows || files.templates[t].sheets.len() > 1 => {
            files.templates[t].path.clone()
        }
        _ => files.rows_path(k).to_owned(),
    }
}

/// Copies the objects `targets` names to the places `pattern` gives: a
/// copy or an array, AutoCAD's COPY and ARRAY as plain copies (Zach,
/// 2026-11-21). Each copy is an ordinary object, written where its source
/// is, as a dimension added from another is: a template's object's copies
/// in the template, so on every sheet built from it, all or nothing, and a
/// row's own object's in the row. Each is given its source's id and the
/// first number free on every sheet it reaches, and keeps every key of its
/// source, an insert's attribute values among them, but its name.
fn apply_copy(
    at: &mut Attempt,
    op: &Operation,
    targets: &[(Target, String)],
    pattern: &Pattern,
    settings: &Settings,
) -> Applied {
    let files = at.files;
    let units = files.units;
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    let fail = |sheets: Vec<usize>, e: Error| Applied {
        sheets,
        outcome: Err(e),
        refuse: false,
    };
    let (found, reach, k) = match found_objects(at, op, targets) {
        Ok(found) => found,
        Err(applied) => return *applied,
    };

    // The places, worked out on the sheets the copies reach, as they stand
    // now: a snapped point on each, the base point and the path on the
    // sheet the operation names.
    let mut snapped: Vec<SnapResolved> = Vec::new();
    let point = |at: &Attempt, p: &OpPoint, snapped: &mut Vec<SnapResolved>| match p {
        OpPoint::At(xyz) => Ok(*xyz),
        OpPoint::Snap(snap) => {
            snapped_points(at, op, &reach, &[snap], settings).map(|mut found| {
                let (xyz, resolved) = found.remove(0);
                snapped.push(resolved);
                xyz
            })
        }
    };
    let mut path_id = None;
    let mut mirror: Option<(Transform, [[f64; 3]; 2])> = None;
    let places = match pattern {
        Pattern::Mirror { first, second } => {
            let a = match point(at, first, &mut snapped) {
                Ok(a) => a,
                Err(e) => return fail(reach, e),
            };
            let b = match point(at, second, &mut snapped) {
                Ok(b) => b,
                Err(e) => return fail(reach, e),
            };
            match Transform::mirror(a, b) {
                Some(t) => mirror = Some((t, [a, b])),
                None => return fail(reach, no_mirror_line(op)),
            }
            array::offsets(&[[0.0; 3]])
        }
        Pattern::Offsets(motions) => {
            let mut by = Vec::with_capacity(motions.len());
            for motion in motions {
                by.push(match motion {
                    Motion::By(by) => *by,
                    Motion::Between { from, to } => {
                        let a = match point(at, from, &mut snapped) {
                            Ok(a) => a,
                            Err(e) => return fail(reach, e),
                        };
                        let b = match point(at, to, &mut snapped) {
                            Ok(b) => b,
                            Err(e) => return fail(reach, e),
                        };
                        [b[0] - a[0], b[1] - a[1], b[2] - a[2]]
                    }
                });
            }
            array::offsets(&by)
        }
        Pattern::Rectangular {
            rows,
            columns,
            row_spacing,
            column_spacing,
        } => array::rectangular(*rows, *columns, *row_spacing, *column_spacing),
        Pattern::Polar {
            center,
            count,
            angle,
            rotate,
            base_point,
        } => {
            let c = match point(at, center, &mut snapped) {
                Ok(c) => c,
                Err(e) => return fail(reach, e),
            };
            let base = match (rotate, base_point) {
                (true, _) => c,
                (false, Some(p)) => match point(at, p, &mut snapped) {
                    Ok(b) => b,
                    Err(e) => return fail(reach, e),
                },
                (false, None) => match extents_center(at, op, k, &found, settings) {
                    Ok(b) => b,
                    Err(e) => return fail(reach, e),
                },
            };
            array::polar(c, *count, *angle, *rotate, base)
        }
        Pattern::Path {
            path,
            count,
            spacing,
            align,
        } => {
            let (id, curves) = match path_curves(at, op, path, k, &found, settings) {
                Ok(found) => found,
                Err(applied) => return *applied,
            };
            path_id = Some(id);
            match array::along(&curves, *count, *spacing, *align) {
                Ok(places) => places,
                Err(why) => {
                    let key = if spacing.is_some() { "spacing" } else { "path" };
                    return fail(reach, op_err(ErrorCode::BadValue, why, key));
                }
            }
        }
    };

    // A copy that turns is of an object a turn turns: not a table or a
    // viewport.
    if let Some(turning) = places.iter().find(|p| p.turn != 0.0) {
        let key = match pattern {
            Pattern::Polar { .. } => "rotate: false",
            _ => "align: false",
        };
        let turn = Transform::rotate(turning.about, turning.turn);
        for (f, field) in &found {
            let why = match at.file(f.file) {
                Ok(record) => record.refuses_transform(f.list, f.index, &turn, units),
                Err(e) => return fail(reach, e),
            };
            if let Some(why) = why {
                return fail(
                    reach,
                    op_err(
                        ErrorCode::BadValue,
                        format!(
                            "`{}`: {why}; its copies turn as they go round, so give {key} to \
                             place them unturned",
                            f.id,
                        ),
                        field,
                    ),
                );
            }
        }
    }

    // A mirrored copy is of an object the sheet description holds
    // mirrored.
    let mut resolved = vec![Resolved::default(); found.len()];
    if let Some((t, _)) = &mirror {
        for (f, field) in &found {
            let why = match at.file(f.file) {
                Ok(record) => record.refuses_transform(f.list, f.index, t, units),
                Err(e) => return fail(reach, e),
            };
            if let Some(why) = why {
                return fail(
                    reach,
                    op_err(ErrorCode::BadValue, format!("`{}`: {why}", f.id), field),
                );
            }
        }
        resolved = match resolved_for(at, op, k, &found, t) {
            Ok(resolved) => resolved,
            Err(e) => return fail(reach, e),
        };
    }

    // Each copy's ID: its source's id and the first number free on every
    // sheet the copy itself reaches: a template's object's copy on every
    // sheet built from the template, a row's own object's on its sheet.
    let mut taken: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    for &s in &reach {
        match at.objects(s) {
            Ok(objects) => {
                taken.insert(
                    s,
                    objects.into_iter().map(|o| o.3.id.to_lowercase()).collect(),
                );
            }
            Err(e) => return fail(reach, e),
        }
    }
    let mut copies: Vec<Copied> = Vec::with_capacity(places.len() * found.len());
    let mut copy_sheets: Vec<Vec<usize>> = Vec::with_capacity(copies.capacity());
    for place in &places {
        for ((f, _), resolved) in found.iter().zip(&resolved) {
            let on = at.reach(f.file, f.row);
            let record = match at.file(f.file) {
                Ok(record) => record,
                Err(e) => return fail(reach, e),
            };
            let base = copy_base(&record.objects(f.list)[f.index]);
            let local = (1..)
                .map(|n| format!("{base}-{n}"))
                .find(|id| {
                    let id = id.to_lowercase();
                    on.iter().all(|s| !taken[s].contains(&id))
                })
                .expect("a free number");
            for s in &on {
                taken
                    .get_mut(s)
                    .expect("a sheet it reaches")
                    .insert(local.to_lowercase());
            }
            // The first object of the copies: the source's own when it is a
            // copy itself.
            let first = record
                .object_value(f.list, f.index, COPY_OF_KEY)
                .unwrap_or_else(|| record.objects(f.list)[f.index].id.clone());
            let made = record
                .duplicate(f.list, f.index, &local, &first, op.index)
                .and_then(|index| {
                    if place.by != [0.0; 3] {
                        record.move_by(f.list, index, place.by, units, op.index)?;
                    }
                    if place.turn != 0.0 {
                        record.turn_by(f.list, index, place.about, place.turn, units, op.index)?;
                    }
                    if let Some((t, _)) = &mirror {
                        record.transform(f.list, index, t, resolved, units, op.index)?;
                    }
                    Ok(())
                });
            if let Err(e) = made {
                return fail(reach, op_err(e.code, e.message, ""));
            }
            let (offset, row, column, item, along) = match place.at {
                array::At::Offset(i) => (Some(i), None, None, None, None),
                array::At::Grid { row, column } => (None, Some(row), Some(column), None, None),
                array::At::Item(i) => (None, None, None, Some(i), None),
                array::At::Along { item, distance } => {
                    (None, None, None, Some(item), Some(distance))
                }
            };
            // A mirrored copy is placed by its mirror line, not an offset.
            let offset = offset.filter(|_| mirror.is_none());
            let turned = place.turn != 0.0;
            copies.push(Copied {
                id: format!("{}{ID_SEPARATOR}{local}", files.name(k)),
                source: f.id.clone(),
                offset,
                row,
                column,
                item,
                along,
                by: place.by,
                turned: turned.then_some(place.turn),
                about: turned.then_some(place.about),
                mirrored_across: mirror.as_ref().map(|(_, line)| *line),
            });
            copy_sheets.push(on);
        }
    }

    // The message: what it copied, how, where the copies are written, and
    // their IDs.
    let sources: Vec<String> = found.iter().map(|(f, _)| format!("`{}`", f.id)).collect();
    let what = join_and(&sources);
    let numbers = |v: &[f64]| {
        v.iter()
            .map(|d| crate::description::number_text(*d))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let plural = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    let how = match pattern {
        Pattern::Mirror { .. } => {
            let (_, [a, b]) = mirror.expect("a mirror's line");
            format!(
                "across the line through [{}] and [{}] ({units}), keeping {}",
                numbers(&a),
                numbers(&b),
                if found.len() == 1 { "it" } else { "them" }
            )
        }
        Pattern::Offsets(_) if places.len() == 1 => {
            format!("by [{}] ({units})", numbers(&places[0].by))
        }
        Pattern::Offsets(_) => format!("by each of {} offsets ({units})", places.len()),
        Pattern::Rectangular {
            rows,
            columns,
            row_spacing,
            column_spacing,
        } => {
            let mut parts = Vec::new();
            if *rows > 1 {
                parts.push(format!(
                    "{} {} apart",
                    plural(*rows, "row", "rows"),
                    numbers(&[*row_spacing])
                ));
            } else {
                parts.push("1 row".to_owned());
            }
            if *columns > 1 {
                parts.push(format!(
                    "{} {} apart",
                    plural(*columns, "column", "columns"),
                    numbers(&[*column_spacing])
                ));
            } else {
                parts.push("1 column".to_owned());
            }
            format!(
                "in a rectangular array of {} ({units})",
                parts.join(" and ")
            )
        }
        Pattern::Polar {
            count,
            angle,
            rotate,
            ..
        } => format!(
            "in a polar array of {count} items over {} degrees, {} apart, {}",
            numbers(&[*angle]),
            numbers(&[array::polar_step(*count, *angle)]),
            if *rotate {
                "each turned as it goes round"
            } else {
                "unturned"
            }
        ),
        Pattern::Path {
            count,
            spacing,
            align,
            ..
        } => format!(
            "in a path array of {count} items along `{}`, {}, {}",
            path_id.as_deref().unwrap_or_default(),
            match spacing {
                Some(s) => format!("{} ({units}) apart", numbers(&[*s])),
                None => "spaced evenly".to_owned(),
            },
            if *align {
                "each turned with the path"
            } else {
                "unturned"
            }
        ),
    };
    let mut written: Vec<String> = Vec::new();
    let in_template: BTreeSet<usize> = found
        .iter()
        .filter_map(|(f, _)| match f.file {
            FileRef::Template(t) => Some(t),
            FileRef::Rows(_) => None,
        })
        .collect();
    for t in in_template {
        let sheets = &files.templates[t].sheets;
        written.push(if sheets.len() > 1 {
            format!(
                "in the template `{}`, so on {}",
                files.templates[t].written,
                every_sheet(&files.names(sheets))
            )
        } else {
            format!("in the template `{}`", files.templates[t].written)
        });
    }
    if found
        .iter()
        .any(|(f, _)| matches!(f.file, FileRef::Rows(_)))
    {
        written.push(format!("in sheet `{}`'s row", files.name(k)));
    }
    let ids: Vec<String> = copies.iter().map(|c| format!("`{}`", c.id)).collect();
    let verb = match op.spec.kind {
        Kind::Array => "arrays",
        Kind::Mirror => "mirrors",
        _ => "copies",
    };
    let message = format!(
        "{verb} {what} {how}, making {}, written {}: {}{}",
        plural(copies.len(), "copy", "copies"),
        written.join(" and "),
        listed(&ids),
        snapped_words(&snapped)
    );
    // The record, as for every edit that writes a row and a template: the
    // template when it is built into more than one sheet, and the rows file
    // when it serves the one sheet alone.
    let template = found
        .iter()
        .filter_map(|(f, _)| match f.file {
            FileRef::Template(t) => Some(t),
            FileRef::Rows(_) => None,
        })
        .max_by_key(|&t| files.templates[t].sheets.len());
    let writes_rows = found
        .iter()
        .any(|(f, _)| matches!(f.file, FileRef::Rows(_)));
    let record = match template {
        Some(t) if !writes_rows || files.templates[t].sheets.len() > 1 => {
            files.templates[t].path.clone()
        }
        _ => files.rows_path(k).to_owned(),
    };
    Applied {
        outcome: Ok(Done {
            record,
            id: None,
            before: None,
            after: None,
            row_value: None,
            row_values: Vec::new(),
            ids_written: Vec::new(),
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            snapped,
            copies,
            copy_sheets,
            transformed: Vec::new(),
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set: Vec::new(),
            split_id: None,
            deleted: Vec::new(),
            created: Vec::new(),
            message,
        }),
        sheets: reach,
        refuse: false,
    }
}

/// The sheet `k` as the change set has left it so far, with the fonts its
/// text is measured in, for the geometry of its objects.
fn sheet_geometry(
    at: &Attempt,
    op: &Operation,
    k: usize,
    settings: &Settings,
) -> Result<(SheetRecord, SheetFonts), Error> {
    let rows = at.rows_now()?;
    let record = sheet_record(at, &rows, k, &mut Vec::new()).map_err(|e| Error {
        message: format!(
            "operation {} (`{}`): sheet `{}` cannot be read to work out where the copies go: {}",
            op.index,
            op.spec.name,
            at.files.name(k),
            e.message
        ),
        ..e
    })?;
    let fonts = SheetFonts::find(&record.sheet, settings);
    Ok((record, fonts))
}

/// The place in `record`'s objects of the object `found` names.
fn model_index(record: &SheetRecord, found: &Found) -> Option<usize> {
    let local = found
        .id
        .split_once(ID_SEPARATOR)
        .map_or(found.id.as_str(), |(_, local)| local);
    let in_row = matches!(found.file, FileRef::Rows(_));
    record
        .objects
        .iter()
        .position(|o| o.in_row == in_row && o.id.eq_ignore_ascii_case(local))
}

/// Adds a revision cloud framing the combined drawn extents of the objects
/// `around` names, expanded on every side by `margin` (or half of `arc_length`
fn draw_order_ref_target<'a>(
    place: Option<&'a DrawOrderPlace>,
    prefix: &str,
) -> Option<(&'a Target, String)> {
    match place {
        Some(DrawOrderPlace::Above(t)) => Some((t, format!("{prefix}.above"))),
        Some(DrawOrderPlace::Under(t)) => Some((t, format!("{prefix}.under"))),
        _ => None,
    }
}

/// Resolves an `add_*` operation's optional `draw_order` into an insertion
/// index in `(file, list)` (or `None` to append at the end).
fn resolve_add_insert_at(
    at: &mut Attempt,
    op: &Operation,
    file: FileRef,
    list: ObjectList,
    sheets: &[usize],
    place: Option<&DrawOrderPlace>,
) -> Result<Option<usize>, Box<Applied>> {
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    let fail = |e: Error| {
        Box::new(Applied {
            sheets: sheets.to_vec(),
            outcome: Err(e),
            refuse: false,
        })
    };
    match place {
        None | Some(DrawOrderPlace::Front) => Ok(None),
        Some(DrawOrderPlace::Back) => Ok(Some(0)),
        Some(DrawOrderPlace::Above(target)) | Some(DrawOrderPlace::Under(target)) => {
            let is_above = matches!(place, Some(DrawOrderPlace::Above(_)));
            let field = if is_above {
                "draw_order.above"
            } else {
                "draw_order.under"
            };
            let found = match resolve(at, target, op) {
                Ok(found) => found,
                Err(applied) => return Err(Box::new(keyed(*applied, op, field))),
            };
            if (found.file, found.list) != (file, list) {
                return Err(fail(op_err(
                    ErrorCode::BadValue,
                    format!(
                        "`{}` is not in the same record/list where this object is being added; \
                         template objects always draw before a sheet's row objects, so `above` \
                         and `under` must name an object in the same template or row",
                        found.id
                    ),
                    field,
                )));
            }
            Ok(Some(if is_above {
                found.index + 1
            } else {
                found.index
            }))
        }
    }
}

fn apply_draw_order(
    at: &mut Attempt,
    op: &Operation,
    targets: &[(Target, String)],
    order: &DrawOrderPlace,
) -> Applied {
    let files = at.files;
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    let fail = |sheets: Vec<usize>, e: Error| Applied {
        sheets,
        outcome: Err(e),
        refuse: false,
    };
    let mut found: Vec<(Found, &str)> = Vec::with_capacity(targets.len());
    let mut lookup: Option<Applied> = None;
    for (target, key) in targets {
        match resolve(at, target, op) {
            Ok(object) => found.push((object, key)),
            Err(applied) if applied.refuse => return keyed(*applied, op, key),
            Err(applied) => {
                lookup.get_or_insert_with(|| keyed(*applied, op, key));
            }
        }
    }
    let mut reach_set: BTreeSet<usize> = BTreeSet::new();
    for (f, _) in &found {
        reach_set.extend(at.reach(f.file, f.row));
    }
    if let Some(mut failed) = lookup {
        reach_set.extend(failed.sheets.iter().copied());
        failed.sheets = reach_set.into_iter().collect();
        return failed;
    }
    let reach: Vec<usize> = reach_set.into_iter().collect();
    let (file, list) = (found[0].0.file, found[0].0.list);
    for (i, (f, key)) in found.iter().enumerate() {
        if (f.file, f.list) != (file, list) {
            return fail(
                reach,
                op_err(
                    ErrorCode::BadValue,
                    format!(
                        "`{}` and `{}` are not in the same record/list; template objects always \
                         draw before a sheet's row objects, so `draw_order` reorders objects in \
                         one template or one sheet's row",
                        found[0].0.id, f.id
                    ),
                    key,
                ),
            );
        }
        if let Some((_, first)) = found[..i]
            .iter()
            .find(|(g, _)| (g.file, g.list, g.index) == (f.file, f.list, f.index))
        {
            return fail(
                reach,
                op_err(
                    ErrorCode::BadValue,
                    format!(
                        "`{}` is named twice, at {first} and here; name it once",
                        f.id
                    ),
                    key,
                ),
            );
        }
    }
    if let Some(refusal) = many_paths(files, file, op, "target") {
        return refusal;
    }
    let moving_set: BTreeSet<usize> = found.iter().map(|(f, _)| f.index).collect();
    let ref_index = match order {
        DrawOrderPlace::Front | DrawOrderPlace::Back => None,
        DrawOrderPlace::Above(t) | DrawOrderPlace::Under(t) => {
            let field = if matches!(order, DrawOrderPlace::Above(_)) {
                "order.above"
            } else {
                "order.under"
            };
            let rf = match resolve(at, t, op) {
                Ok(rf) => rf,
                Err(applied) => {
                    let mut failed = keyed(*applied, op, field);
                    let mut all: BTreeSet<usize> = reach.iter().copied().collect();
                    all.extend(failed.sheets.iter().copied());
                    failed.sheets = all.into_iter().collect();
                    return failed;
                }
            };
            if (rf.file, rf.list) != (file, list) {
                return fail(
                    reach,
                    op_err(
                        ErrorCode::BadValue,
                        format!(
                            "reference object `{}` is not in the same record/list as `{}`; \
                             template objects always draw before a sheet's row objects",
                            rf.id, found[0].0.id
                        ),
                        field,
                    ),
                );
            }
            if moving_set.contains(&rf.index) {
                return fail(
                    reach,
                    op_err(
                        ErrorCode::BadValue,
                        format!(
                            "reference object `{}` cannot also be one of the objects being \
                             reordered",
                            rf.id
                        ),
                        field,
                    ),
                );
            }
            Some(rf.index)
        }
    };
    let record = match at.file(file) {
        Ok(record) => record,
        Err(e) => return fail(reach, e),
    };
    let count = record.objects(list).len();
    let stay: Vec<usize> = (0..count).filter(|i| !moving_set.contains(i)).collect();
    let moved: Vec<usize> = (0..count).filter(|i| moving_set.contains(i)).collect();
    let insert_pos = match (order, ref_index) {
        (DrawOrderPlace::Front, _) => stay.len(),
        (DrawOrderPlace::Back, _) => 0,
        (DrawOrderPlace::Above(_), Some(r)) => stay
            .iter()
            .position(|&i| i == r)
            .map_or(stay.len(), |p| p + 1),
        (DrawOrderPlace::Under(_), Some(r)) => stay.iter().position(|&i| i == r).unwrap_or(0),
        _ => stay.len(),
    };
    let mut new_order = Vec::with_capacity(count);
    new_order.extend_from_slice(&stay[..insert_pos]);
    new_order.extend_from_slice(&moved);
    new_order.extend_from_slice(&stay[insert_pos..]);
    let kept = match record.reorder(list, &new_order, op.index) {
        Ok(kept) => kept,
        Err(e) => return fail(reach, op_err(e.code, e.message, "")),
    };
    let path = record.path.clone();
    let ids_written: Vec<String> = kept
        .iter()
        .flat_map(|place| {
            reach
                .iter()
                .map(move |&s| format!("{}/{place}", files.name(s)))
        })
        .collect();
    let transformed: Vec<(String, Vec<usize>)> = found
        .iter()
        .map(|(f, _)| (f.id.clone(), reach.clone()))
        .collect();
    let first_id = found[0].0.id.clone();
    let names: Vec<&str> = found.iter().map(|(f, _)| f.id.as_str()).collect();
    Applied {
        outcome: Ok(Done {
            record: path,
            id: Some(first_id),
            before: None,
            after: None,
            row_value: None,
            row_values: Vec::new(),
            ids_written,
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed,
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set: Vec::new(),
            split_id: None,
            deleted: Vec::new(),
            created: Vec::new(),
            snapped: Vec::new(),
            message: format!(
                "moves {} {} in draw order",
                quoted_list(&names),
                order.words()
            ),
        }),
        sheets: reach,
        refuse: false,
    }
}

fn apply_text_to_front(
    at: &mut Attempt,
    op: &Operation,
    to: &AddTo,
    which: TextToFrontWhich,
) -> Applied {
    let files = at.files;
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    let fail = |sheets: Vec<usize>, e: Error| Applied {
        sheets,
        outcome: Err(e),
        refuse: false,
    };
    let (file, list, sheets, first) = match to {
        AddTo::Sheet(name) => {
            let k = files.sheet(name).expect("checked before");
            let (file, list) = files.row_list(k);
            (file, list, vec![k], k)
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
    let record = match at.file(file) {
        Ok(record) => record,
        Err(e) => return fail(sheets, e),
    };
    let objects = record.objects(list);
    let mut stay = Vec::with_capacity(objects.len());
    let mut moved = Vec::with_capacity(objects.len());
    let mut moved_ids = Vec::new();
    for (i, listed) in objects.iter().enumerate() {
        if which.matches_kind(&listed.kind) {
            moved.push(i);
            moved_ids.push(format!("{}/{}", files.name(first), listed.id));
        } else {
            stay.push(i);
        }
    }
    let moved_count = moved.len();
    let mut new_order = stay;
    new_order.extend(moved);
    let kept = match record.reorder(list, &new_order, op.index) {
        Ok(kept) => kept,
        Err(e) => return fail(sheets, op_err(e.code, e.message, "")),
    };
    let path = record.path.clone();
    let ids_written: Vec<String> = kept
        .iter()
        .flat_map(|place| {
            sheets
                .iter()
                .map(move |&s| format!("{}/{place}", files.name(s)))
        })
        .collect();
    let transformed: Vec<(String, Vec<usize>)> = moved_ids
        .into_iter()
        .map(|id| (id, sheets.clone()))
        .collect();
    let place = match file {
        FileRef::Rows(_) => format!("on sheet `{}`", files.name(first)),
        FileRef::Template(t) => format!(
            "in template `{}` ({})",
            files.templates[t].written,
            every_sheet(&files.names(&sheets))
        ),
    };
    Applied {
        outcome: Ok(Done {
            record: path,
            id: None,
            before: None,
            after: None,
            row_value: None,
            row_values: Vec::new(),
            ids_written,
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed,
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set: Vec::new(),
            split_id: None,
            deleted: Vec::new(),
            created: Vec::new(),
            snapped: Vec::new(),
            message: format!(
                "brings {moved_count} {} object{} to the front {place}",
                which.name(),
                if moved_count == 1 { "" } else { "s" }
            ),
        }),
        sheets,
        refuse: false,
    }
}

/// Resolves the target sheet indices for a batch operation (`sheets == None`
/// means every sheet of the drawing set).
fn batch_target_sheets(files: &SetFiles, sheets: Option<&[String]>) -> Vec<usize> {
    match sheets {
        None => (0..files.rows.len()).collect(),
        Some(names) => {
            let mut set = BTreeSet::new();
            for name in names {
                if let Some(k) = files.sheet(name) {
                    set.insert(k);
                }
            }
            set.into_iter().collect()
        }
    }
}

/// Checks whether the object at `(file, list, list_idx)` on sheet `k` sits on
/// a locked layer.
fn batch_object_locked_layer(
    at: &mut Attempt,
    files: &SetFiles,
    k: usize,
    file: FileRef,
    list: ObjectList,
    list_idx: usize,
) -> Option<String> {
    let layer = at
        .file(file)
        .ok()
        .and_then(|r| r.object_value(list, list_idx, "layer"))
        .unwrap_or_else(|| "0".to_owned());
    let t = files.template_of[k];
    let locked = at
        .template(t)
        .ok()
        .and_then(|tmpl| tmpl.table_value(Table::Layer.key(), &layer, "locked"))
        .as_deref()
        == Some("true");
    locked.then_some(layer)
}

/// Refuses a literal edit to an object in shared template `t` when `target_set`
/// names only a strict subset of the sheets built from `t`.
fn check_shared_template_scope(
    files: &SetFiles,
    t: usize,
    target_set: &BTreeSet<usize>,
    obj_id: &str,
    op: &Operation,
) -> Option<Error> {
    let missing: Vec<&str> = files.templates[t]
        .sheets
        .iter()
        .filter(|s| !target_set.contains(s))
        .map(|&s| files.name(s))
        .collect();
    if missing.is_empty() {
        return None;
    }
    Some(op_error(
        op.index,
        op.spec.name,
        ErrorCode::BadValue,
        format!(
            "`{obj_id}` is written literally in shared template `{}`, which also builds {}; \
             include those sheets under `sheets` or omit `sheets` to edit it across the template",
            files.templates[t].written,
            quoted_list(&missing)
        ),
        "sheets",
    ))
}

/// Applies a row-value update derived from `binding` (`{row: NAME}` or list of
/// parts) on sheet `k`, skipping writes for any `(k, row_value)` already
/// updated earlier in the same batch operation so shared row values are never
/// double-replaced. Returns the first row-value name updated (for reporting).
#[allow(clippy::too_many_arguments)]
fn apply_batch_from_row(
    at: &mut Attempt,
    op: &Operation,
    k: usize,
    binding: &FromRow,
    new_text: &str,
    is_mtext: bool,
    obj_id: &str,
    set_what: &str,
    text_field: &str,
    edited_row_values: &mut BTreeSet<(usize, String)>,
    row_value_override: Option<&str>,
) -> Result<Option<String>, Error> {
    let files = at.files;
    let op_err =
        |code, message, field: &str| op_error(op.index, op.spec.name, code, message, field);
    let sets: Vec<(String, String)> = match (binding, row_value_override) {
        (FromRow::Whole(name), None) => vec![(name.clone(), new_text.to_owned())],
        (FromRow::Whole(name), Some(asked)) if asked == name => {
            vec![(name.clone(), new_text.to_owned())]
        }
        (FromRow::Whole(name), Some(asked)) => {
            return Err(op_err(
                ErrorCode::BadValue,
                format!(
                    "row_value names `{asked}`, but the template takes {set_what} whole from the \
                     row's value {name}; name {name}, or leave row_value out"
                ),
                ROW_VALUE_KEY,
            ));
        }
        (FromRow::Parts(held), Some(asked)) => {
            let names = parts::names(held);
            if !names.iter().any(|n| n == asked) {
                return Err(op_err(
                    ErrorCode::UnknownName,
                    format!(
                        "row_value names `{asked}`, which is not one of the row values of the \
                         parts {} the template joins {set_what} from; they are {}",
                        parts::pattern(held),
                        names.join(", ")
                    ),
                    ROW_VALUE_KEY,
                ));
            }
            vec![(asked.to_owned(), new_text.to_owned())]
        }
        (FromRow::Parts(held), None) => {
            let pattern = parts::pattern(held);
            let split = if is_mtext {
                parts::split(&parts::shown_mtext_parts(held), &shown_mtext(new_text))
            } else {
                parts::split(held, new_text)
            };
            let mut values = match split {
                Ok(v) => v,
                Err(Unsplit::Differs) => {
                    return Err(op_err(
                        ErrorCode::BadValue,
                        format!(
                            "`{new_text}` does not match the parts {pattern} the template joins \
                             {set_what} from: its fixed text differs"
                        ),
                        text_field,
                    ));
                }
                Err(Unsplit::Ambiguous(first, second)) => {
                    let each = |vals: &[(String, String)]| {
                        vals.iter()
                            .map(|(n, v)| format!("{n} `{v}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    return Err(op_err(
                        ErrorCode::BadValue,
                        format!(
                            "`{new_text}` splits more than one way into the parts {pattern} the \
                             template joins {set_what} from, such as {} or {}",
                            each(&first),
                            each(&second)
                        ),
                        text_field,
                    ));
                }
                Err(Unsplit::Unclear) => {
                    return Err(op_err(
                        ErrorCode::BadValue,
                        format!(
                            "`{new_text}` cannot be split uniquely into the parts {pattern} the \
                             template joins {set_what} from"
                        ),
                        text_field,
                    ));
                }
            };
            // Drop parts whose value did not change compared to the current row.
            let row = files.rows[k].index;
            let (rows, _) = at.row_record(k);
            let joined: String = held
                .iter()
                .map(|part| match part {
                    parts::Part::Text(text) => text.clone(),
                    parts::Part::Row(name) => rows.row_value(row, name).unwrap_or_default(),
                })
                .collect();
            let now = if is_mtext {
                parts::split(&parts::shown_mtext_parts(held), &shown_mtext(&joined))
            } else {
                parts::split(held, &joined)
            };
            if let Ok(now) = now {
                values.retain(|(name, value)| {
                    now.iter().find(|(n, _)| n == name).map(|(_, v)| v) != Some(value)
                });
            }
            values
        }
    };
    let t = files.template_of[k];
    let counts = carried_by_blocks(at, t);
    let carries = |block: &str, tag: &str| {
        counts
            .iter()
            .find(|(b, _)| b == block)
            .map(|(_, tags)| tags.iter().filter(|t| *t == tag).count())
    };
    // Verify no place filled by any updated row value sits on a locked layer.
    if let Ok(template) = at.template(t) {
        let listed = template.objects(ObjectList::Template);
        for (name, _) in &sets {
            for (i, _) in template.row_value_places(name, &carries) {
                let layer = template
                    .object_value(ObjectList::Template, i, "layer")
                    .unwrap_or_else(|| "0".to_owned());
                if template
                    .table_value(Table::Layer.key(), &layer, "locked")
                    .as_deref()
                    == Some("true")
                {
                    let filled = format!("{}{ID_SEPARATOR}{}", files.name(k), listed[i].id);
                    return Err(op_err(
                        ErrorCode::Locked,
                        format!(
                            "`{filled}` is on layer `{layer}`, which is locked; unlock it first \
                             with change_layer, set: {{\"locked\": false}}"
                        ),
                        text_field,
                    ));
                }
            }
        }
    }
    let first_name = sets.first().map(|(n, _)| n.clone());
    for (name, value) in sets {
        if !edited_row_values.insert((k, name.clone())) {
            continue;
        }
        let row = files.rows[k].index;
        let (record, _) = at.row_record(k);
        record
            .set_row_value(row, &name, &value)
            .map_err(|e| op_err(e.code, format!("{obj_id}: {}", e.message), text_field))?;
        at.values.retain(|(r, n, _)| !(*r == k && *n == name));
        at.values.push((k, name, op.index));
    }
    Ok(first_name)
}

type BatchSnapshot = (
    usize,
    SheetRecord,
    Vec<(FileRef, ObjectList, usize, Listed)>,
);
type TemplatePlaceKey = (usize, usize, Option<(String, Option<usize>)>);

/// Finds and replaces text across texts, mtexts and insert attributes on the
/// targeted sheets (`replace_text` / AutoCAD `FIND`).
#[allow(clippy::too_many_arguments)]
fn apply_replace_text(
    at: &mut Attempt,
    op: &Operation,
    find: &str,
    replace: &str,
    matching: TextMatchKind,
    case_sensitive: bool,
    sheets: Option<&[String]>,
    scopes: &[TextScope],
    tag_filter: Option<&str>,
    block_filter: Option<&str>,
) -> Applied {
    let files = at.files;
    let op_err =
        |code, message, field: &str| op_error(op.index, op.spec.name, code, message, field);
    let fail = |sheets: Vec<usize>, error: Error| Applied {
        sheets,
        outcome: Err(error),
        refuse: false,
    };
    let matcher = match TextMatcher::compile(find, matching, case_sensitive) {
        Ok(m) => m,
        Err(e) => {
            return Applied {
                sheets: Vec::new(),
                outcome: Err(op_err(e.code, e.message, "find")),
                refuse: true,
            };
        }
    };
    let target_sheets = batch_target_sheets(files, sheets);
    let target_set: BTreeSet<usize> = target_sheets.iter().copied().collect();
    let rows_now = match at.rows_now() {
        Ok(r) => r,
        Err(e) => return fail(target_sheets, e),
    };
    // Snapshot every targeted sheet before mutating so shared row values and
    // shared template literals are read consistently and never double-replaced.
    let mut snapshots: Vec<BatchSnapshot> = Vec::with_capacity(target_sheets.len());
    for &k in &target_sheets {
        let record = match sheet_record(at, &rows_now, k, &mut Vec::new()) {
            Ok(r) => r,
            Err(e) => return fail(vec![k], e),
        };
        let objects = match at.objects(k) {
            Ok(o) => o,
            Err(e) => return fail(vec![k], e),
        };
        snapshots.push((k, record, objects));
    }

    let mut edited_template_places: BTreeSet<TemplatePlaceKey> = BTreeSet::new();
    let mut edited_row_values: BTreeSet<(usize, String)> = BTreeSet::new();
    let mut replacements: Vec<TextReplaced> = Vec::new();
    let mut touched_sheets: BTreeSet<usize> = BTreeSet::new();
    let mut first_record: Option<PathBuf> = None;

    for (k, record, objects_k) in snapshots {
        let sheet_name = files.name(k).to_owned();
        let t = files.template_of[k];
        let mut sheet_touched = false;

        for (obj_idx, object) in record.sheet.objects.iter().enumerate() {
            let Some(&(file, list, list_idx, ref listed)) = objects_k.get(obj_idx) else {
                continue;
            };
            let obj_id = format!("{sheet_name}{ID_SEPARATOR}{}", listed.id);
            match object {
                Object::Text(text) if scopes.contains(&TextScope::Text) => {
                    let Some((after, count)) = matcher.replace(&text.contents, replace) else {
                        continue;
                    };
                    let binding = at
                        .file(file)
                        .ok()
                        .and_then(|r| r.text_binding(list, list_idx, None));
                    let row_val = if let Some(binding) = binding {
                        match apply_batch_from_row(
                            at,
                            op,
                            k,
                            &binding,
                            &after,
                            false,
                            &obj_id,
                            &format!("the contents of `{obj_id}`"),
                            "replace",
                            &mut edited_row_values,
                            None,
                        ) {
                            Ok(rv) => {
                                first_record.get_or_insert_with(|| {
                                    files.rows_files[files.rows[k].file_index].path.clone()
                                });
                                rv
                            }
                            Err(e) => return fail(vec![k], e),
                        }
                    } else {
                        if let Some(layer) =
                            batch_object_locked_layer(at, files, k, file, list, list_idx)
                        {
                            let reach = at.reach(file, k);
                            return fail(
                                reach,
                                op_err(
                                    ErrorCode::Locked,
                                    format!(
                                        "`{obj_id}` is on layer `{layer}`, which is locked; \
                                         unlock it first with change_layer, set: {{\"locked\": false}}"
                                    ),
                                    "find",
                                ),
                            );
                        }
                        if let FileRef::Template(t_idx) = file {
                            if let Some(e) =
                                check_shared_template_scope(files, t_idx, &target_set, &obj_id, op)
                            {
                                return fail(vec![k], e);
                            }
                            if let Some(refusal) = many_paths(files, file, op, "sheets") {
                                return refusal;
                            }
                            if edited_template_places.insert((t_idx, list_idx, None)) {
                                let mut set = serde_json::Map::new();
                                set.insert("contents".to_owned(), Value::String(after.clone()));
                                if let Err(e) = at.file(file).and_then(|r| {
                                    r.change(list, list_idx, &set, files.units, op.index)
                                }) {
                                    return fail(
                                        at.reach(file, k),
                                        op_err(e.code, e.message, "replace"),
                                    );
                                }
                                first_record
                                    .get_or_insert_with(|| files.templates[t_idx].path.clone());
                            }
                        } else if let FileRef::Rows(f_idx) = file {
                            let mut set = serde_json::Map::new();
                            set.insert("contents".to_owned(), Value::String(after.clone()));
                            if let Err(e) = at
                                .file(file)
                                .and_then(|r| r.change(list, list_idx, &set, files.units, op.index))
                            {
                                return fail(vec![k], op_err(e.code, e.message, "replace"));
                            }
                            first_record
                                .get_or_insert_with(|| files.rows_files[f_idx].path.clone());
                        }
                        None
                    };
                    replacements.push(TextReplaced {
                        sheet: sheet_name.clone(),
                        id: obj_id,
                        kind: "text".to_owned(),
                        tag: None,
                        occurrence: None,
                        before: text.contents.clone(),
                        after,
                        count,
                        row_value: row_val,
                    });
                    sheet_touched = true;
                }
                Object::MText(mtext) if scopes.contains(&TextScope::MText) => {
                    let Some((after, count)) = matcher.replace(&mtext.contents, replace) else {
                        continue;
                    };
                    let binding = at
                        .file(file)
                        .ok()
                        .and_then(|r| r.text_binding(list, list_idx, None));
                    let row_val = if let Some(binding) = binding {
                        match apply_batch_from_row(
                            at,
                            op,
                            k,
                            &binding,
                            &after,
                            true,
                            &obj_id,
                            &format!("the contents of `{obj_id}`"),
                            "replace",
                            &mut edited_row_values,
                            None,
                        ) {
                            Ok(rv) => {
                                first_record.get_or_insert_with(|| {
                                    files.rows_files[files.rows[k].file_index].path.clone()
                                });
                                rv
                            }
                            Err(e) => return fail(vec![k], e),
                        }
                    } else {
                        if let Some(layer) =
                            batch_object_locked_layer(at, files, k, file, list, list_idx)
                        {
                            let reach = at.reach(file, k);
                            return fail(
                                reach,
                                op_err(
                                    ErrorCode::Locked,
                                    format!(
                                        "`{obj_id}` is on layer `{layer}`, which is locked; \
                                         unlock it first with change_layer, set: {{\"locked\": false}}"
                                    ),
                                    "find",
                                ),
                            );
                        }
                        if let FileRef::Template(t_idx) = file {
                            if let Some(e) =
                                check_shared_template_scope(files, t_idx, &target_set, &obj_id, op)
                            {
                                return fail(vec![k], e);
                            }
                            if let Some(refusal) = many_paths(files, file, op, "sheets") {
                                return refusal;
                            }
                            if edited_template_places.insert((t_idx, list_idx, None)) {
                                let mut set = serde_json::Map::new();
                                set.insert("contents".to_owned(), Value::String(after.clone()));
                                if let Err(e) = at.file(file).and_then(|r| {
                                    r.change(list, list_idx, &set, files.units, op.index)
                                }) {
                                    return fail(
                                        at.reach(file, k),
                                        op_err(e.code, e.message, "replace"),
                                    );
                                }
                                first_record
                                    .get_or_insert_with(|| files.templates[t_idx].path.clone());
                            }
                        } else if let FileRef::Rows(f_idx) = file {
                            let mut set = serde_json::Map::new();
                            set.insert("contents".to_owned(), Value::String(after.clone()));
                            if let Err(e) = at
                                .file(file)
                                .and_then(|r| r.change(list, list_idx, &set, files.units, op.index))
                            {
                                return fail(vec![k], op_err(e.code, e.message, "replace"));
                            }
                            first_record
                                .get_or_insert_with(|| files.rows_files[f_idx].path.clone());
                        }
                        None
                    };
                    replacements.push(TextReplaced {
                        sheet: sheet_name.clone(),
                        id: obj_id,
                        kind: "mtext".to_owned(),
                        tag: None,
                        occurrence: None,
                        before: mtext.contents.clone(),
                        after,
                        count,
                        row_value: row_val,
                    });
                    sheet_touched = true;
                }
                Object::Insert(insert) if scopes.contains(&TextScope::Attribute) => {
                    if let Some(block) = block_filter {
                        if !insert.block.eq_ignore_ascii_case(block) {
                            continue;
                        }
                    }
                    let mut tag_totals: BTreeMap<String, usize> = BTreeMap::new();
                    for attr in &insert.attributes {
                        *tag_totals.entry(tag_in_capitals(&attr.tag)).or_default() += 1;
                    }
                    let mut tag_seen: BTreeMap<String, usize> = BTreeMap::new();
                    for attr in &insert.attributes {
                        let cap_tag = tag_in_capitals(&attr.tag);
                        let occ_0 = {
                            let entry = tag_seen.entry(cap_tag.clone()).or_default();
                            let current = *entry;
                            *entry += 1;
                            current
                        };
                        let repeated = tag_totals.get(&cap_tag).copied().unwrap_or(0) > 1;
                        let slot = repeated.then_some(occ_0);
                        let occ_1 = repeated.then_some(occ_0 + 1);
                        if let Some(wanted_tag) = tag_filter {
                            if !cap_tag.eq_ignore_ascii_case(wanted_tag) {
                                continue;
                            }
                        }
                        let Some((after, count)) = matcher.replace(&attr.value, replace) else {
                            continue;
                        };
                        let binding = at
                            .file(file)
                            .ok()
                            .and_then(|r| r.text_binding(list, list_idx, Some((&cap_tag, slot))));
                        let row_val = if let Some(binding) = binding {
                            match apply_batch_from_row(
                                at,
                                op,
                                k,
                                &binding,
                                &after,
                                false,
                                &obj_id,
                                &format!("attribute `{cap_tag}` of `{obj_id}`"),
                                "replace",
                                &mut edited_row_values,
                                None,
                            ) {
                                Ok(rv) => {
                                    first_record.get_or_insert_with(|| {
                                        files.rows_files[files.rows[k].file_index].path.clone()
                                    });
                                    rv
                                }
                                Err(e) => return fail(vec![k], e),
                            }
                        } else {
                            if let Some(layer) =
                                batch_object_locked_layer(at, files, k, file, list, list_idx)
                            {
                                let reach = at.reach(file, k);
                                return fail(
                                    reach,
                                    op_err(
                                        ErrorCode::Locked,
                                        format!(
                                            "`{obj_id}` is on layer `{layer}`, which is locked; \
                                             unlock it first with change_layer, set: {{\"locked\": false}}"
                                        ),
                                        "find",
                                    ),
                                );
                            }
                            if let FileRef::Template(t_idx) = file {
                                if let Some(e) = check_shared_template_scope(
                                    files,
                                    t_idx,
                                    &target_set,
                                    &obj_id,
                                    op,
                                ) {
                                    return fail(vec![k], e);
                                }
                                if let Some(refusal) = many_paths(files, file, op, "sheets") {
                                    return refusal;
                                }
                                if edited_template_places.insert((
                                    t_idx,
                                    list_idx,
                                    Some((cap_tag.clone(), slot)),
                                )) {
                                    if let Err(e) = at.file(file).and_then(|r| {
                                        r.set_attribute(
                                            list, list_idx, &cap_tag, slot, &after, op.index,
                                        )
                                    }) {
                                        return fail(
                                            at.reach(file, k),
                                            op_err(e.code, e.message, "replace"),
                                        );
                                    }
                                    first_record
                                        .get_or_insert_with(|| files.templates[t_idx].path.clone());
                                }
                            } else if let FileRef::Rows(f_idx) = file {
                                if let Err(e) = at.file(file).and_then(|r| {
                                    r.set_attribute(
                                        list, list_idx, &cap_tag, slot, &after, op.index,
                                    )
                                }) {
                                    return fail(vec![k], op_err(e.code, e.message, "replace"));
                                }
                                first_record
                                    .get_or_insert_with(|| files.rows_files[f_idx].path.clone());
                            }
                            None
                        };
                        replacements.push(TextReplaced {
                            sheet: sheet_name.clone(),
                            id: obj_id.clone(),
                            kind: "attribute".to_owned(),
                            tag: Some(cap_tag),
                            occurrence: occ_1,
                            before: attr.value.clone(),
                            after,
                            count,
                            row_value: row_val,
                        });
                        sheet_touched = true;
                    }
                }
                _ => {}
            }
        }
        let _ = t;
        if sheet_touched {
            touched_sheets.insert(k);
        }
    }

    let sheets_out: Vec<usize> = touched_sheets.into_iter().collect();
    let total_matches: usize = replacements.iter().map(|r| r.count).sum();
    let message = if replacements.is_empty() {
        format!("found no {} matches for `{find}`", matching.name())
    } else {
        format!(
            "replaces {total_matches} match{} of `{find}` with `{replace}` in {} place{} across {} sheet{}",
            if total_matches == 1 { "" } else { "es" },
            replacements.len(),
            if replacements.len() == 1 { "" } else { "s" },
            sheets_out.len(),
            if sheets_out.len() == 1 { "" } else { "s" },
        )
    };
    Applied {
        outcome: Ok(Done {
            record: first_record.unwrap_or_default(),
            id: None,
            before: None,
            after: None,
            row_value: None,
            row_values: Vec::new(),
            ids_written: Vec::new(),
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed: Vec::new(),
            created_row_values: Vec::new(),
            replacements,
            attributes_set: Vec::new(),
            split_id: None,
            deleted: Vec::new(),
            created: Vec::new(),
            snapped: Vec::new(),
            message,
        }),
        sheets: sheets_out,
        refuse: false,
    }
}

/// Sets an attribute on every insert matching `selection` across the targeted
/// sheets (`set_attributes` / AutoCAD `GATTE`).
#[allow(clippy::too_many_arguments)]
fn apply_set_attributes(
    at: &mut Attempt,
    op: &Operation,
    selection: &crate::query::Selection,
    sheets: Option<&[String]>,
    tag: &str,
    occurrence: Option<usize>,
    value: &str,
    settings: &Settings,
) -> Applied {
    let files = at.files;
    let op_err =
        |code, message, field: &str| op_error(op.index, op.spec.name, code, message, field);
    let fail = |sheets: Vec<usize>, error: Error| Applied {
        sheets,
        outcome: Err(error),
        refuse: false,
    };
    let cap_tag = tag_in_capitals(tag);
    let target_sheets = batch_target_sheets(files, sheets);
    let target_set: BTreeSet<usize> = target_sheets.iter().copied().collect();
    let rows_now = match at.rows_now() {
        Ok(r) => r,
        Err(e) => return fail(target_sheets, e),
    };
    let mut snapshots: Vec<BatchSnapshot> = Vec::with_capacity(target_sheets.len());
    for &k in &target_sheets {
        let record = match sheet_record(at, &rows_now, k, &mut Vec::new()) {
            Ok(r) => r,
            Err(e) => return fail(vec![k], e),
        };
        let objects = match at.objects(k) {
            Ok(o) => o,
            Err(e) => return fail(vec![k], e),
        };
        snapshots.push((k, record, objects));
    }

    let mut edited_template_places: BTreeSet<(usize, usize, String, Option<usize>)> =
        BTreeSet::new();
    let mut edited_row_values: BTreeSet<(usize, String)> = BTreeSet::new();
    let mut attributes_set: Vec<AttributeUpdated> = Vec::new();
    let mut touched_sheets: BTreeSet<usize> = BTreeSet::new();
    let mut first_record: Option<PathBuf> = None;

    for (k, record, objects_k) in snapshots {
        let sheet_name = files.name(k).to_owned();
        let selected = match crate::query::select_on_sheet(selection, &record, settings) {
            Ok(s) => s,
            Err(e) => return fail(vec![k], op_err(e.code, e.message, "where")),
        };
        let mut sheet_touched = false;
        for obj_idx in selected {
            let Object::Insert(insert) = &record.sheet.objects[obj_idx] else {
                continue;
            };
            let Some(&(file, list, list_idx, ref listed)) = objects_k.get(obj_idx) else {
                continue;
            };
            let obj_id = format!("{sheet_name}{ID_SEPARATOR}{}", listed.id);
            let editable_values: Vec<&str> = insert
                .attributes
                .iter()
                .filter(|a| tag_in_capitals(&a.tag) == cap_tag)
                .map(|a| a.value.as_str())
                .collect();
            let has_constant = record.sheet.block_for_insert(insert).is_some_and(|b| {
                b.objects.iter().any(|o| match o {
                    Object::AttributeDefinition(d) => {
                        d.constant && tag_in_capitals(&d.tag) == cap_tag
                    }
                    _ => false,
                })
            });
            let (slot, occ_1, before) = match (editable_values.len(), occurrence) {
                (0, _) => {
                    if has_constant {
                        return fail(
                            vec![k],
                            op_err(
                                ErrorCode::BadValue,
                                format!(
                                    "attribute `{cap_tag}` on `{obj_id}` (block `{}`) is constant \
                                     and cannot be set",
                                    insert.block
                                ),
                                "tag",
                            ),
                        );
                    }
                    let available: Vec<String> = insert
                        .attributes
                        .iter()
                        .map(|a| tag_in_capitals(&a.tag))
                        .collect();
                    let msg = if available.is_empty() {
                        format!(
                            "`{obj_id}` (block `{}`) has no editable attributes; cannot set `{cap_tag}`",
                            insert.block
                        )
                    } else {
                        format!(
                            "`{obj_id}` (block `{}`) has no attribute `{cap_tag}`; its attributes: {}",
                            insert.block,
                            available.join(", ")
                        )
                    };
                    return fail(vec![k], op_err(ErrorCode::UnknownName, msg, "tag"));
                }
                (1, None | Some(1)) => (None, None, editable_values[0].to_owned()),
                (1, Some(o)) => {
                    return fail(
                        vec![k],
                        op_err(
                            ErrorCode::BadValue,
                            format!(
                                "block `{}` defines `{cap_tag}` once, so occurrence is 1 or left \
                                 out, not {o}",
                                insert.block
                            ),
                            "occurrence",
                        ),
                    );
                }
                (n, None) => {
                    return fail(
                        vec![k],
                        op_err(
                            ErrorCode::MissingKey,
                            format!(
                                "block `{}` defines `{cap_tag}` {n} times on `{obj_id}`; give \
                                 `occurrence` (1 to {n})",
                                insert.block
                            ),
                            "occurrence",
                        ),
                    );
                }
                (n, Some(o)) if o > n => {
                    return fail(
                        vec![k],
                        op_err(
                            ErrorCode::BadValue,
                            format!(
                                "block `{}` defines `{cap_tag}` {n} times on `{obj_id}`, so \
                                 occurrence is 1 to {n}, not {o}",
                                insert.block
                            ),
                            "occurrence",
                        ),
                    );
                }
                (_, Some(o)) => (Some(o - 1), Some(o), editable_values[o - 1].to_owned()),
            };

            let binding = at
                .file(file)
                .ok()
                .and_then(|r| r.text_binding(list, list_idx, Some((&cap_tag, slot))));
            if let (Some(asked), None) = (&op.row_value, &binding) {
                return fail(
                    vec![k],
                    op_err(
                        ErrorCode::BadValue,
                        format!(
                            "row_value names `{asked}`, but `{obj_id}` does not take `{cap_tag}` \
                             from the row"
                        ),
                        ROW_VALUE_KEY,
                    ),
                );
            }
            let (after_shown, row_val) = if let Some(binding) = binding {
                let rv = match apply_batch_from_row(
                    at,
                    op,
                    k,
                    &binding,
                    value,
                    false,
                    &obj_id,
                    &format!("attribute `{cap_tag}` of `{obj_id}`"),
                    "value",
                    &mut edited_row_values,
                    op.row_value.as_deref(),
                ) {
                    Ok(rv) => rv,
                    Err(e) => return fail(vec![k], e),
                };
                first_record
                    .get_or_insert_with(|| files.rows_files[files.rows[k].file_index].path.clone());
                let after = match (&binding, op.row_value.as_deref()) {
                    (FromRow::Parts(held), Some(_)) => {
                        let row = files.rows[k].index;
                        let (rows, _) = at.row_record(k);
                        held.iter()
                            .map(|p| match p {
                                parts::Part::Text(t) => t.clone(),
                                parts::Part::Row(n) => rows.row_value(row, n).unwrap_or_default(),
                            })
                            .collect()
                    }
                    _ => value.to_owned(),
                };
                (after, rv)
            } else {
                if let Some(layer) = batch_object_locked_layer(at, files, k, file, list, list_idx) {
                    let reach = at.reach(file, k);
                    return fail(
                        reach,
                        op_err(
                            ErrorCode::Locked,
                            format!(
                                "`{obj_id}` is on layer `{layer}`, which is locked; unlock it \
                                 first with change_layer, set: {{\"locked\": false}}"
                            ),
                            "where",
                        ),
                    );
                }
                if let FileRef::Template(t_idx) = file {
                    if let Some(e) =
                        check_shared_template_scope(files, t_idx, &target_set, &obj_id, op)
                    {
                        return fail(vec![k], e);
                    }
                    if let Some(refusal) = many_paths(files, file, op, "sheets") {
                        return refusal;
                    }
                    if edited_template_places.insert((t_idx, list_idx, cap_tag.clone(), slot)) {
                        if let Err(e) = at.file(file).and_then(|r| {
                            r.set_attribute(list, list_idx, &cap_tag, slot, value, op.index)
                        }) {
                            return fail(at.reach(file, k), op_err(e.code, e.message, "value"));
                        }
                        first_record.get_or_insert_with(|| files.templates[t_idx].path.clone());
                    }
                } else if let FileRef::Rows(f_idx) = file {
                    if let Err(e) = at.file(file).and_then(|r| {
                        r.set_attribute(list, list_idx, &cap_tag, slot, value, op.index)
                    }) {
                        return fail(vec![k], op_err(e.code, e.message, "value"));
                    }
                    first_record.get_or_insert_with(|| files.rows_files[f_idx].path.clone());
                }
                (value.to_owned(), None)
            };

            attributes_set.push(AttributeUpdated {
                sheet: sheet_name.clone(),
                id: obj_id,
                block: insert.block.clone(),
                tag: cap_tag.clone(),
                occurrence: occ_1,
                before,
                after: after_shown,
                row_value: row_val,
            });
            sheet_touched = true;
        }
        if sheet_touched {
            touched_sheets.insert(k);
        }
    }

    let sheets_out: Vec<usize> = touched_sheets.into_iter().collect();
    let message = if attributes_set.is_empty() {
        format!(
            "no insert matches {} on the targeted sheets",
            crate::query::selection_words(selection)
        )
    } else {
        format!(
            "sets `{cap_tag}` to `{value}` on {} insert{} across {} sheet{}",
            attributes_set.len(),
            if attributes_set.len() == 1 { "" } else { "s" },
            sheets_out.len(),
            if sheets_out.len() == 1 { "" } else { "s" },
        )
    };
    Applied {
        outcome: Ok(Done {
            record: first_record.unwrap_or_default(),
            id: None,
            before: None,
            after: None,
            row_value: None,
            row_values: Vec::new(),
            ids_written: Vec::new(),
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed: Vec::new(),
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set,
            split_id: None,
            deleted: Vec::new(),
            created: Vec::new(),
            snapped: Vec::new(),
            message,
        }),
        sheets: sheets_out,
        refuse: false,
    }
}

/// Draws a parallel offset of a line, polyline, arc, circle, ellipse or spline
/// (`offset` / AutoCAD `OFFSET`, `O`).
#[allow(clippy::too_many_arguments)]
fn apply_offset(
    at: &mut Attempt,
    op: &Operation,
    target: &Target,
    by: &OpOffsetBy,
    id: Option<&str>,
    name: Option<&str>,
    layer: Option<&str>,
    erase_source: bool,
    settings: &Settings,
) -> Applied {
    let files = at.files;
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    let fail = |sheets: Vec<usize>, e: Error| Applied {
        sheets,
        outcome: Err(e),
        refuse: false,
    };
    let found = match resolve(at, target, op) {
        Ok(found) => found,
        Err(applied) => return *applied,
    };
    let reach = at.reach(found.file, found.row);
    if let Some(refusal) = many_paths(files, found.file, op, "target") {
        return refusal;
    }
    if erase_source {
        if let Some(locked_layer) =
            batch_object_locked_layer(at, files, found.row, found.file, found.list, found.index)
        {
            return fail(
                reach,
                op_err(
                    ErrorCode::Locked,
                    format!(
                        "`{}` is on layer `{locked_layer}`, which is locked; cannot erase source \
                         without unlocking it first",
                        found.id
                    ),
                    "target",
                ),
            );
        }
    }
    let rows = match at.rows_now() {
        Ok(rows) => rows,
        Err(e) => return fail(reach, e),
    };
    let record = match sheet_record(at, &rows, found.row, &mut Vec::new()) {
        Ok(record) => record,
        Err(e) => return fail(reach, e),
    };
    let local = found
        .id
        .split_once(ID_SEPARATOR)
        .map_or(found.id.as_str(), |(_, local)| local);
    let in_row = matches!(found.file, FileRef::Rows(_));
    let Some(obj_idx) = record
        .objects
        .iter()
        .position(|o| o.in_row == in_row && o.id.eq_ignore_ascii_case(local))
    else {
        return fail(
            reach,
            op_err(
                ErrorCode::UnknownName,
                format!("could not read `{}` to offset it", found.id),
                "target",
            ),
        );
    };
    let base = &record.sheet.objects[obj_idx];
    let space = crate::query::space_of(&record.sheet, obj_idx);

    let mut snapped: Vec<SnapResolved> = Vec::new();
    let resolve_pt = |p: &OpPoint, snapped: &mut Vec<SnapResolved>| -> Result<[f64; 3], Error> {
        match p {
            OpPoint::At(xyz) => Ok(*xyz),
            OpPoint::Snap(snap) => {
                snapped_points(at, op, &reach, &[snap], settings).map(|mut pts| {
                    let (xyz, res) = pts.remove(0);
                    snapped.push(res);
                    xyz
                })
            }
        }
    };
    let resolved_by = match by {
        OpOffsetBy::Distance { distance, side } => {
            let side = match side {
                OpOffsetSide::Left => OffsetSide::Left,
                OpOffsetSide::Right => OffsetSide::Right,
                OpOffsetSide::Inside => OffsetSide::Inside,
                OpOffsetSide::Outside => OffsetSide::Outside,
                OpOffsetSide::Point(pt) => match resolve_pt(pt, &mut snapped) {
                    Ok(xyz) => OffsetSide::Point(xyz),
                    Err(e) => return fail(reach, e),
                },
            };
            OffsetBy::Distance {
                distance: *distance,
                side,
            }
        }
        OpOffsetBy::Through(pt) => match resolve_pt(pt, &mut snapped) {
            Ok(xyz) => OffsetBy::Through(xyz),
            Err(e) => return fail(reach, e),
        },
    };
    let (new_object, eff_distance) = match compute_offset(base, &resolved_by, layer) {
        Ok(res) => res,
        Err(e) => {
            let field = if e.message.contains("side") {
                "side"
            } else if e.message.contains("through") {
                "through"
            } else if e.message.contains("works on") {
                "target"
            } else {
                "distance"
            };
            return fail(reach, op_err(e.code, e.message, field));
        }
    };
    let new_local_id = match id {
        Some(explicit) => explicit.to_owned(),
        None => {
            let mut taken: BTreeSet<String> = BTreeSet::new();
            for &k in &reach {
                match at.objects(k) {
                    Ok(objects) => {
                        taken.extend(objects.into_iter().map(|o| o.3.id.to_lowercase()));
                    }
                    Err(e) => return fail(reach.clone(), e),
                }
            }
            let prefix = new_object.type_name();
            (1..)
                .map(|n| format!("{prefix}-{n}"))
                .find(|candidate| !taken.contains(&candidate.to_lowercase()))
                .expect("a free id")
        }
    };
    let (kind, keys) = object_to_record_keys(&new_object, &new_local_id, name, space.as_deref());
    let path = match found.file {
        FileRef::Rows(f) => files.rows_files[f].path.clone(),
        FileRef::Template(t) => files.templates[t].path.clone(),
    };
    let rec = match at.file(found.file) {
        Ok(r) => r,
        Err(e) => return fail(reach, e),
    };
    let mut ids_written = Vec::new();
    let added = match rec.add(found.list, kind, &keys, op.index) {
        Ok(idx) => idx,
        Err(e) => return fail(reach, op_err(e.code, e.message, "")),
    };
    let after = rec.object_text(found.list, added);
    if erase_source {
        ids_written = rec.delete(found.list, found.index, op.index);
    }
    let full_id = format!("{}{ID_SEPARATOR}{new_local_id}", files.name(found.row));
    let template_note = match found.file {
        FileRef::Rows(_) => String::new(),
        FileRef::Template(t) => format!(
            " in template `{}` ({})",
            files.templates[t].written,
            every_sheet(&files.names(&reach))
        ),
    };
    let erase_note = if erase_source {
        format!(" and erases `{}`", found.id)
    } else {
        String::new()
    };
    let dist_str = crate::description::number_text(eff_distance);
    Applied {
        outcome: Ok(Done {
            record: path,
            id: Some(full_id.clone()),
            before: None,
            after: Some(after),
            row_value: None,
            row_values: Vec::new(),
            ids_written,
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed: Vec::new(),
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set: Vec::new(),
            split_id: None,
            deleted: Vec::new(),
            created: Vec::new(),
            snapped,
            message: format!(
                "offsets `{}` by {dist_str} ({}) to {kind} `{full_id}`{erase_note}{template_note}",
                found.id, files.units
            ),
        }),
        sheets: reach,
        refuse: false,
    }
}

/// Collects all geometry curves of `boundaries` on sheet `target_row` in the
/// same space as `target_idx`.
#[allow(clippy::too_many_arguments)]
fn collect_boundary_curves(
    at: &mut Attempt,
    op: &Operation,
    record: &SheetRecord,
    target_row: usize,
    target_idx: usize,
    target_file: FileRef,
    boundaries: &[(Target, String)],
    settings: &Settings,
) -> Result<Vec<Curve>, Error> {
    let files = at.files;
    let fonts = SheetFonts::find(&record.sheet, settings);
    let reader = Reader {
        sheet: &record.sheet,
        fonts: &fonts,
    };
    let mut curves = Vec::new();
    for (b_target, b_field) in boundaries {
        let b_found = match resolve(at, b_target, op) {
            Ok(f) => f,
            Err(applied) => {
                return Err(applied.outcome.err().unwrap_or_else(|| {
                    op_error(
                        op.index,
                        op.spec.name,
                        ErrorCode::UnknownName,
                        "boundary object not found",
                        b_field,
                    )
                }));
            }
        };
        if b_found.row != target_row {
            return Err(op_error(
                op.index,
                op.spec.name,
                ErrorCode::BadValue,
                format!(
                    "boundary `{}` is on sheet `{}`, not on sheet `{}`",
                    b_found.id,
                    files.name(b_found.row),
                    files.name(target_row)
                ),
                b_field,
            ));
        }
        if matches!(target_file, FileRef::Template(t) if files.templates[t].sheets.len() > 1)
            && matches!(b_found.file, FileRef::Rows(_))
        {
            return Err(op_error(
                op.index,
                op.spec.name,
                ErrorCode::BadValue,
                format!(
                    "boundary `{}` is a row-only object on `{}`, so it cannot cut a shared \
                     template object",
                    b_found.id,
                    files.name(target_row)
                ),
                b_field,
            ));
        }
        let b_local = b_found
            .id
            .split_once(ID_SEPARATOR)
            .map_or(b_found.id.as_str(), |(_, l)| l);
        let b_in_row = matches!(b_found.file, FileRef::Rows(_));
        let Some(b_idx) = record
            .objects
            .iter()
            .position(|o| o.in_row == b_in_row && o.id.eq_ignore_ascii_case(b_local))
        else {
            return Err(op_error(
                op.index,
                op.spec.name,
                ErrorCode::UnknownName,
                format!("could not read boundary `{}`", b_found.id),
                b_field,
            ));
        };
        if !record.sheet.same_space(target_idx, b_idx) {
            return Err(op_error(
                op.index,
                op.spec.name,
                ErrorCode::BadValue,
                format!(
                    "boundary `{}` is in a different space from `target`",
                    b_found.id
                ),
                b_field,
            ));
        }
        for part in reader.parts(&record.sheet.objects[b_idx]) {
            curves.push(part.curve);
        }
    }
    Ok(curves)
}

/// Cuts `target` back to `boundaries` (`trim` / AutoCAD `TRIM`, `TR`).
fn apply_trim(
    at: &mut Attempt,
    op: &Operation,
    target: &Target,
    boundaries: &[(Target, String)],
    keep: TrimKeep,
    new_id: Option<&str>,
    settings: &Settings,
) -> Applied {
    let files = at.files;
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    let fail = |sheets: Vec<usize>, e: Error| Applied {
        sheets,
        outcome: Err(e),
        refuse: false,
    };
    let found = match resolve(at, target, op) {
        Ok(found) => found,
        Err(applied) => return *applied,
    };
    let reach = at.reach(found.file, found.row);
    if let Some(refusal) = many_paths(files, found.file, op, "target") {
        return refusal;
    }
    if let Some(layer) =
        batch_object_locked_layer(at, files, found.row, found.file, found.list, found.index)
    {
        return fail(
            reach,
            op_err(
                ErrorCode::Locked,
                format!(
                    "`{}` is on layer `{layer}`, which is locked; unlock it first with \
                     change_layer, set: {{\"locked\": false}}",
                    found.id
                ),
                "target",
            ),
        );
    }
    let rows = match at.rows_now() {
        Ok(r) => r,
        Err(e) => return fail(reach, e),
    };
    let record = match sheet_record(at, &rows, found.row, &mut Vec::new()) {
        Ok(r) => r,
        Err(e) => return fail(reach, e),
    };
    let local = found
        .id
        .split_once(ID_SEPARATOR)
        .map_or(found.id.as_str(), |(_, l)| l)
        .to_owned();
    let in_row = matches!(found.file, FileRef::Rows(_));
    let Some(obj_idx) = record
        .objects
        .iter()
        .position(|o| o.in_row == in_row && o.id.eq_ignore_ascii_case(&local))
    else {
        return fail(
            reach,
            op_err(
                ErrorCode::UnknownName,
                format!("could not read `{}` to trim it", found.id),
                "target",
            ),
        );
    };
    let boundary_curves = match collect_boundary_curves(
        at, op, &record, found.row, obj_idx, found.file, boundaries, settings,
    ) {
        Ok(c) => c,
        Err(e) => return fail(reach, e),
    };
    let base = &record.sheet.objects[obj_idx];
    let space = crate::query::space_of(&record.sheet, obj_idx);
    let obj_name = record.objects[obj_idx].name.clone();
    let trimmed = match compute_trim(base, &boundary_curves, keep) {
        Ok(t) => t,
        Err(e) => return fail(reach, op_err(e.code, e.message, "boundary")),
    };
    let split_local_id = if let Some(ref split_obj) = trimmed.split {
        Some(match new_id {
            Some(explicit) => explicit.to_owned(),
            None => {
                let mut taken: BTreeSet<String> = BTreeSet::new();
                for &k in &reach {
                    match at.objects(k) {
                        Ok(objects) => {
                            taken.extend(objects.into_iter().map(|o| o.3.id.to_lowercase()));
                        }
                        Err(e) => return fail(reach.clone(), e),
                    }
                }
                let prefix = split_obj.type_name();
                (1..)
                    .map(|n| format!("{prefix}-{n}"))
                    .find(|c| !taken.contains(&c.to_lowercase()))
                    .expect("a free id")
            }
        })
    } else {
        None
    };

    let path = match found.file {
        FileRef::Rows(f) => files.rows_files[f].path.clone(),
        FileRef::Template(t) => files.templates[t].path.clone(),
    };
    let (kept_kind, kept_keys) =
        object_to_record_keys(&trimmed.kept, &local, obj_name.as_deref(), space.as_deref());
    let rec = match at.file(found.file) {
        Ok(r) => r,
        Err(e) => return fail(reach, e),
    };
    let before = rec.object_text(found.list, found.index);
    if let Err(e) = rec.replace_object(found.list, found.index, kept_kind, &kept_keys, op.index) {
        return fail(reach, op_err(e.code, e.message, "target"));
    }
    let after = rec.object_text(found.list, found.index);
    let mut split_full_id = None;
    if let (Some(split_obj), Some(split_local)) = (&trimmed.split, &split_local_id) {
        let (sk, skeys) = object_to_record_keys(split_obj, split_local, None, space.as_deref());
        if let Err(e) = rec.add(found.list, sk, &skeys, op.index) {
            return fail(reach, op_err(e.code, e.message, "new_id"));
        }
        split_full_id = Some(format!(
            "{}{ID_SEPARATOR}{split_local}",
            files.name(found.row)
        ));
    }
    let message = match &split_full_id {
        Some(sid) => format!(
            "trims `{}` (keep `{}`) and splits off `{sid}`",
            found.id,
            keep.name()
        ),
        None => format!("trims `{}` (keep `{}`)", found.id, keep.name()),
    };
    Applied {
        outcome: Ok(Done {
            record: path,
            id: Some(found.id),
            before: Some(before),
            after: Some(after),
            row_value: None,
            row_values: Vec::new(),
            ids_written: Vec::new(),
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed: Vec::new(),
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set: Vec::new(),
            split_id: split_full_id,
            deleted: Vec::new(),
            created: Vec::new(),
            snapped: Vec::new(),
            message,
        }),
        sheets: reach,
        refuse: false,
    }
}

/// Carries an open `target` on to `boundaries` (`extend` / AutoCAD `EXTEND`, `EX`).
fn apply_extend(
    at: &mut Attempt,
    op: &Operation,
    target: &Target,
    boundaries: &[(Target, String)],
    end: ExtendEnd,
    settings: &Settings,
) -> Applied {
    let files = at.files;
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    let fail = |sheets: Vec<usize>, e: Error| Applied {
        sheets,
        outcome: Err(e),
        refuse: false,
    };
    let found = match resolve(at, target, op) {
        Ok(found) => found,
        Err(applied) => return *applied,
    };
    let reach = at.reach(found.file, found.row);
    if let Some(refusal) = many_paths(files, found.file, op, "target") {
        return refusal;
    }
    if let Some(layer) =
        batch_object_locked_layer(at, files, found.row, found.file, found.list, found.index)
    {
        return fail(
            reach,
            op_err(
                ErrorCode::Locked,
                format!(
                    "`{}` is on layer `{layer}`, which is locked; unlock it first with \
                     change_layer, set: {{\"locked\": false}}",
                    found.id
                ),
                "target",
            ),
        );
    }
    let rows = match at.rows_now() {
        Ok(r) => r,
        Err(e) => return fail(reach, e),
    };
    let record = match sheet_record(at, &rows, found.row, &mut Vec::new()) {
        Ok(r) => r,
        Err(e) => return fail(reach, e),
    };
    let local = found
        .id
        .split_once(ID_SEPARATOR)
        .map_or(found.id.as_str(), |(_, l)| l)
        .to_owned();
    let in_row = matches!(found.file, FileRef::Rows(_));
    let Some(obj_idx) = record
        .objects
        .iter()
        .position(|o| o.in_row == in_row && o.id.eq_ignore_ascii_case(&local))
    else {
        return fail(
            reach,
            op_err(
                ErrorCode::UnknownName,
                format!("could not read `{}` to extend it", found.id),
                "target",
            ),
        );
    };
    let boundary_curves = match collect_boundary_curves(
        at, op, &record, found.row, obj_idx, found.file, boundaries, settings,
    ) {
        Ok(c) => c,
        Err(e) => return fail(reach, e),
    };
    let base = &record.sheet.objects[obj_idx];
    let space = crate::query::space_of(&record.sheet, obj_idx);
    let obj_name = record.objects[obj_idx].name.clone();
    let extended = match compute_extend(base, &boundary_curves, end) {
        Ok(obj) => obj,
        Err(e) => return fail(reach, op_err(e.code, e.message, "boundary")),
    };
    let path = match found.file {
        FileRef::Rows(f) => files.rows_files[f].path.clone(),
        FileRef::Template(t) => files.templates[t].path.clone(),
    };
    let (ext_kind, ext_keys) =
        object_to_record_keys(&extended, &local, obj_name.as_deref(), space.as_deref());
    let rec = match at.file(found.file) {
        Ok(r) => r,
        Err(e) => return fail(reach, e),
    };
    let before = rec.object_text(found.list, found.index);
    if let Err(e) = rec.replace_object(found.list, found.index, ext_kind, &ext_keys, op.index) {
        return fail(reach, op_err(e.code, e.message, "target"));
    }
    let after = rec.object_text(found.list, found.index);
    Applied {
        outcome: Ok(Done {
            record: path,
            id: Some(found.id.clone()),
            before: Some(before),
            after: Some(after),
            row_value: None,
            row_values: Vec::new(),
            ids_written: Vec::new(),
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed: Vec::new(),
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set: Vec::new(),
            split_id: None,
            deleted: Vec::new(),
            created: Vec::new(),
            snapped: Vec::new(),
            message: format!("extends `{}` ({} end)", found.id, end.name()),
        }),
        sheets: reach,
        refuse: false,
    }
}

#[derive(Clone, Copy)]
enum CornerOp {
    Fillet(f64),
    Chamfer(ChamferSpec),
}

impl CornerOp {
    fn verb(self) -> &'static str {
        match self {
            Self::Fillet(_) => "fillets",
            Self::Chamfer(_) => "chamfers",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Fillet(_) => "fillet",
            Self::Chamfer(_) => "chamfer",
        }
    }

    fn size_field(self) -> &'static str {
        match self {
            Self::Fillet(_) => "radius",
            Self::Chamfer(_) => "distance",
        }
    }
}

/// Rounds or bevels corners on a polyline (`target`) or between two objects
/// (`first` and `second`): AutoCAD's `FILLET` (`F`) and `CHAMFER` (`CHA`).
fn apply_corner(at: &mut Attempt, op: &Operation, mode: &CornerMode, kind: CornerOp) -> Applied {
    let files = at.files;
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    let fail = |sheets: Vec<usize>, e: Error| Applied {
        sheets,
        outcome: Err(e),
        refuse: false,
    };

    match mode {
        CornerMode::Polyline { target } => {
            let found = match resolve(at, target, op) {
                Ok(found) => found,
                Err(applied) => return *applied,
            };
            let reach = at.reach(found.file, found.row);
            if let Some(refusal) = many_paths(files, found.file, op, "target") {
                return refusal;
            }
            if let Some(layer) =
                batch_object_locked_layer(at, files, found.row, found.file, found.list, found.index)
            {
                return fail(
                    reach,
                    op_err(
                        ErrorCode::Locked,
                        format!(
                            "`{}` is on layer `{layer}`, which is locked; unlock it first with \
                             change_layer, set: {{\"locked\": false}}",
                            found.id
                        ),
                        "target",
                    ),
                );
            }
            let rows = match at.rows_now() {
                Ok(r) => r,
                Err(e) => return fail(reach, e),
            };
            let record = match sheet_record(at, &rows, found.row, &mut Vec::new()) {
                Ok(r) => r,
                Err(e) => return fail(reach, e),
            };
            let local = found
                .id
                .split_once(ID_SEPARATOR)
                .map_or(found.id.as_str(), |(_, l)| l)
                .to_owned();
            let in_row = matches!(found.file, FileRef::Rows(_));
            let Some(obj_idx) = record
                .objects
                .iter()
                .position(|o| o.in_row == in_row && o.id.eq_ignore_ascii_case(&local))
            else {
                return fail(
                    reach,
                    op_err(
                        ErrorCode::UnknownName,
                        format!("could not read `{}` to {} it", found.id, kind.noun()),
                        "target",
                    ),
                );
            };
            let base = &record.sheet.objects[obj_idx];
            let space = crate::query::space_of(&record.sheet, obj_idx);
            let obj_name = record.objects[obj_idx].name.clone();
            let res = match kind {
                CornerOp::Fillet(r) => compute_fillet_polyline(base, r),
                CornerOp::Chamfer(spec) => compute_chamfer_polyline(base, spec),
            };
            let res = match res {
                Ok(r) => r,
                Err(e) => return fail(reach, op_err(e.code, e.message, kind.size_field())),
            };
            let path = match found.file {
                FileRef::Rows(f) => files.rows_files[f].path.clone(),
                FileRef::Template(t) => files.templates[t].path.clone(),
            };
            let (new_kind, new_keys) =
                object_to_record_keys(&res.polyline, &local, obj_name.as_deref(), space.as_deref());
            let rec = match at.file(found.file) {
                Ok(r) => r,
                Err(e) => return fail(reach, e),
            };
            let before = rec.object_text(found.list, found.index);
            if let Err(e) =
                rec.replace_object(found.list, found.index, new_kind, &new_keys, op.index)
            {
                return fail(reach, op_err(e.code, e.message, "target"));
            }
            let after = rec.object_text(found.list, found.index);
            let skipped_note = if res.corners_skipped > 0 {
                format!(" ({} skipped)", res.corners_skipped)
            } else {
                String::new()
            };
            Applied {
                outcome: Ok(Done {
                    record: path,
                    id: Some(found.id.clone()),
                    before: Some(before),
                    after: Some(after),
                    row_value: None,
                    row_values: Vec::new(),
                    ids_written: Vec::new(),
                    inserts: Vec::new(),
                    attributes_removed: Vec::new(),
                    imported: None,
                    copies: Vec::new(),
                    copy_sheets: Vec::new(),
                    transformed: Vec::new(),
                    created_row_values: Vec::new(),
                    replacements: Vec::new(),
                    attributes_set: Vec::new(),
                    split_id: None,
                    deleted: Vec::new(),
                    created: Vec::new(),
                    snapped: Vec::new(),
                    message: format!(
                        "{} {} corner{} of `{}`{skipped_note}",
                        kind.verb(),
                        res.corners_modified,
                        if res.corners_modified == 1 { "" } else { "s" },
                        found.id,
                    ),
                }),
                sheets: reach,
                refuse: false,
            }
        }
        CornerMode::TwoObjects {
            first,
            second,
            trim,
            first_end,
            second_end,
            new_id,
            layer,
        } => {
            let f1 = match resolve(at, first, op) {
                Ok(f) => f,
                Err(applied) => return keyed(*applied, op, "first"),
            };
            let f2 = match resolve(at, second, op) {
                Ok(f) => f,
                Err(applied) => return keyed(*applied, op, "second"),
            };
            let mut reach_set: BTreeSet<usize> = BTreeSet::new();
            reach_set.extend(at.reach(f1.file, f1.row));
            reach_set.extend(at.reach(f2.file, f2.row));
            let reach: Vec<usize> = reach_set.into_iter().collect();

            if let Some(refusal) = many_paths(files, f1.file, op, "first") {
                return refusal;
            }
            if let Some(refusal) = many_paths(files, f2.file, op, "second") {
                return refusal;
            }
            if f1.row != f2.row {
                return fail(
                    reach,
                    op_err(
                        ErrorCode::BadValue,
                        format!(
                            "`second` (`{}`) is on sheet `{}`, not on sheet `{}`",
                            f2.id,
                            files.name(f2.row),
                            files.name(f1.row)
                        ),
                        "second",
                    ),
                );
            }
            if (f1.file, f1.list, f1.index) == (f2.file, f2.list, f2.index) {
                return fail(
                    reach,
                    op_err(
                        ErrorCode::BadValue,
                        format!(
                            "`first` and `second` both name `{}`; to {} a single polyline's \
                             corners, pass it under `target`",
                            f1.id,
                            kind.noun()
                        ),
                        "second",
                    ),
                );
            }
            if f1.file != f2.file {
                let shared = |file: FileRef| matches!(file, FileRef::Template(t) if files.templates[t].sheets.len() > 1);
                if shared(f1.file) || shared(f2.file) {
                    return fail(
                        reach,
                        op_err(
                            ErrorCode::BadValue,
                            format!(
                                "cannot {} a shared template object with a row-only object on `{}`",
                                kind.noun(),
                                files.name(f1.row)
                            ),
                            "second",
                        ),
                    );
                }
            }
            if *trim {
                for (f, key) in [(&f1, "first"), (&f2, "second")] {
                    if let Some(locked) =
                        batch_object_locked_layer(at, files, f.row, f.file, f.list, f.index)
                    {
                        return fail(
                            reach,
                            op_err(
                                ErrorCode::Locked,
                                format!(
                                    "`{}` is on layer `{locked}`, which is locked; unlock it \
                                     first with change_layer, set: {{\"locked\": false}}",
                                    f.id
                                ),
                                key,
                            ),
                        );
                    }
                }
            }
            let rows = match at.rows_now() {
                Ok(r) => r,
                Err(e) => return fail(reach, e),
            };
            let record = match sheet_record(at, &rows, f1.row, &mut Vec::new()) {
                Ok(r) => r,
                Err(e) => return fail(reach, e),
            };
            let local1 = f1
                .id
                .split_once(ID_SEPARATOR)
                .map_or(f1.id.as_str(), |(_, l)| l)
                .to_owned();
            let local2 = f2
                .id
                .split_once(ID_SEPARATOR)
                .map_or(f2.id.as_str(), |(_, l)| l)
                .to_owned();
            let in_row1 = matches!(f1.file, FileRef::Rows(_));
            let in_row2 = matches!(f2.file, FileRef::Rows(_));
            let Some(idx1) = record
                .objects
                .iter()
                .position(|o| o.in_row == in_row1 && o.id.eq_ignore_ascii_case(&local1))
            else {
                return fail(
                    reach,
                    op_err(
                        ErrorCode::UnknownName,
                        format!("could not read `{}`", f1.id),
                        "first",
                    ),
                );
            };
            let Some(idx2) = record
                .objects
                .iter()
                .position(|o| o.in_row == in_row2 && o.id.eq_ignore_ascii_case(&local2))
            else {
                return fail(
                    reach,
                    op_err(
                        ErrorCode::UnknownName,
                        format!("could not read `{}`", f2.id),
                        "second",
                    ),
                );
            };
            if !record.sheet.same_space(idx1, idx2) {
                return fail(
                    reach,
                    op_err(
                        ErrorCode::BadValue,
                        format!("`{}` is in a different space from `{}`", f2.id, f1.id),
                        "second",
                    ),
                );
            }
            let obj1 = &record.sheet.objects[idx1];
            let obj2 = &record.sheet.objects[idx2];
            let space = crate::query::space_of(&record.sheet, idx1);
            let name1 = record.objects[idx1].name.clone();
            let name2 = record.objects[idx2].name.clone();

            let res = match kind {
                CornerOp::Fillet(r) => compute_fillet_two(
                    obj1,
                    obj2,
                    r,
                    *trim,
                    *first_end,
                    *second_end,
                    layer.as_deref(),
                ),
                CornerOp::Chamfer(spec) => compute_chamfer_two(
                    obj1,
                    obj2,
                    spec,
                    *trim,
                    *first_end,
                    *second_end,
                    layer.as_deref(),
                ),
            };
            let res = match res {
                Ok(r) => r,
                Err(e) => return fail(reach, op_err(e.code, e.message, kind.size_field())),
            };

            let added_local_id = if let Some(ref added_obj) = res.added {
                Some(match new_id {
                    Some(explicit) => explicit.clone(),
                    None => {
                        let mut taken: BTreeSet<String> = BTreeSet::new();
                        for &k in &reach {
                            match at.objects(k) {
                                Ok(objects) => {
                                    taken
                                        .extend(objects.into_iter().map(|o| o.3.id.to_lowercase()));
                                }
                                Err(e) => return fail(reach.clone(), e),
                            }
                        }
                        let prefix = added_obj.type_name();
                        (1..)
                            .map(|n| format!("{prefix}-{n}"))
                            .find(|c| !taken.contains(&c.to_lowercase()))
                            .expect("a free id")
                    }
                })
            } else {
                None
            };

            let mut transformed: Vec<(String, Vec<usize>)> = Vec::new();
            let mut before_text = None;
            let mut after_text = None;

            if let Some(ref new_obj1) = res.first {
                let (k1, keys1) =
                    object_to_record_keys(new_obj1, &local1, name1.as_deref(), space.as_deref());
                let rec1 = match at.file(f1.file) {
                    Ok(r) => r,
                    Err(e) => return fail(reach, e),
                };
                before_text = Some(rec1.object_text(f1.list, f1.index));
                if let Err(e) = rec1.replace_object(f1.list, f1.index, k1, &keys1, op.index) {
                    return fail(reach, op_err(e.code, e.message, "first"));
                }
                after_text = Some(rec1.object_text(f1.list, f1.index));
                transformed.push((f1.id.clone(), at.reach(f1.file, f1.row)));
            }
            if let Some(ref new_obj2) = res.second {
                let (k2, keys2) =
                    object_to_record_keys(new_obj2, &local2, name2.as_deref(), space.as_deref());
                let rec2 = match at.file(f2.file) {
                    Ok(r) => r,
                    Err(e) => return fail(reach, e),
                };
                if let Err(e) = rec2.replace_object(f2.list, f2.index, k2, &keys2, op.index) {
                    return fail(reach, op_err(e.code, e.message, "second"));
                }
                transformed.push((f2.id.clone(), at.reach(f2.file, f2.row)));
            }

            // Place the added arc/bevel in the row if either object is in a row;
            // otherwise in the shared template.
            let (dest_file, dest_list) = if matches!(f1.file, FileRef::Rows(_)) {
                (f1.file, f1.list)
            } else if matches!(f2.file, FileRef::Rows(_)) {
                (f2.file, f2.list)
            } else {
                (f1.file, f1.list)
            };
            let path = match dest_file {
                FileRef::Rows(f) => files.rows_files[f].path.clone(),
                FileRef::Template(t) => files.templates[t].path.clone(),
            };

            let mut created_full_id = None;
            if let (Some(added_obj), Some(added_local)) = (&res.added, &added_local_id) {
                let (ak, akeys) =
                    object_to_record_keys(added_obj, added_local, None, space.as_deref());
                let rec = match at.file(dest_file) {
                    Ok(r) => r,
                    Err(e) => return fail(reach, e),
                };
                let added_idx = match rec.add(dest_list, ak, &akeys, op.index) {
                    Ok(i) => i,
                    Err(e) => return fail(reach, op_err(e.code, e.message, "new_id")),
                };
                after_text = Some(rec.object_text(dest_list, added_idx));
                created_full_id =
                    Some(format!("{}{ID_SEPARATOR}{added_local}", files.name(f1.row)));
            }

            let message = match &created_full_id {
                Some(cid) => format!("{} `{}` and `{}` with `{cid}`", kind.verb(), f1.id, f2.id),
                None => format!(
                    "{} `{}` and `{}` to a sharp corner",
                    kind.verb(),
                    f1.id,
                    f2.id
                ),
            };

            Applied {
                outcome: Ok(Done {
                    record: path,
                    id: created_full_id,
                    before: before_text,
                    after: after_text,
                    row_value: None,
                    row_values: Vec::new(),
                    ids_written: Vec::new(),
                    inserts: Vec::new(),
                    attributes_removed: Vec::new(),
                    imported: None,
                    copies: Vec::new(),
                    copy_sheets: Vec::new(),
                    transformed,
                    created_row_values: Vec::new(),
                    replacements: Vec::new(),
                    attributes_set: Vec::new(),
                    split_id: None,
                    deleted: Vec::new(),
                    created: Vec::new(),
                    snapped: Vec::new(),
                    message,
                }),
                sheets: reach,
                refuse: false,
            }
        }
    }
}

/// Splits `target` at a point (with optional centered `gap`) or cuts the span
/// between `from` and `to`: AutoCAD's `BREAK` (`BR`) and `BREAKATPOINT`.
fn apply_break(
    at: &mut Attempt,
    op: &Operation,
    target: &Target,
    by: &OpBreakBy,
    new_id: Option<&str>,
    settings: &Settings,
) -> Applied {
    let files = at.files;
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    let fail = |sheets: Vec<usize>, e: Error| Applied {
        sheets,
        outcome: Err(e),
        refuse: false,
    };
    let found = match resolve(at, target, op) {
        Ok(found) => found,
        Err(applied) => return *applied,
    };
    let reach = at.reach(found.file, found.row);
    if let Some(refusal) = many_paths(files, found.file, op, "target") {
        return refusal;
    }
    if let Some(layer) =
        batch_object_locked_layer(at, files, found.row, found.file, found.list, found.index)
    {
        return fail(
            reach,
            op_err(
                ErrorCode::Locked,
                format!(
                    "`{}` is on layer `{layer}`, which is locked; unlock it first with \
                     change_layer, set: {{\"locked\": false}}",
                    found.id
                ),
                "target",
            ),
        );
    }

    let mut snapped: Vec<SnapResolved> = Vec::new();
    let resolve_pt = |p: &OpPoint, snapped: &mut Vec<SnapResolved>| -> Result<[f64; 3], Error> {
        match p {
            OpPoint::At(xyz) => Ok(*xyz),
            OpPoint::Snap(snap) => {
                snapped_points(at, op, &reach, &[snap], settings).map(|mut pts| {
                    let (xyz, res) = pts.remove(0);
                    snapped.push(res);
                    xyz
                })
            }
        }
    };
    let spec = match by {
        OpBreakBy::AtPoint { at: pt, gap } => match resolve_pt(pt, &mut snapped) {
            Ok(xyz) => BreakSpec::AtPoint { at: xyz, gap: *gap },
            Err(e) => return fail(reach, e),
        },
        OpBreakBy::Between { from, to } => {
            let p1 = match resolve_pt(from, &mut snapped) {
                Ok(xyz) => xyz,
                Err(e) => return fail(reach, e),
            };
            let p2 = match resolve_pt(to, &mut snapped) {
                Ok(xyz) => xyz,
                Err(e) => return fail(reach, e),
            };
            BreakSpec::Between { from: p1, to: p2 }
        }
    };

    let rows = match at.rows_now() {
        Ok(r) => r,
        Err(e) => return fail(reach, e),
    };
    let record = match sheet_record(at, &rows, found.row, &mut Vec::new()) {
        Ok(r) => r,
        Err(e) => return fail(reach, e),
    };
    let local = found
        .id
        .split_once(ID_SEPARATOR)
        .map_or(found.id.as_str(), |(_, l)| l)
        .to_owned();
    let in_row = matches!(found.file, FileRef::Rows(_));
    let Some(obj_idx) = record
        .objects
        .iter()
        .position(|o| o.in_row == in_row && o.id.eq_ignore_ascii_case(&local))
    else {
        return fail(
            reach,
            op_err(
                ErrorCode::UnknownName,
                format!("could not read `{}` to break it", found.id),
                "target",
            ),
        );
    };
    let base = &record.sheet.objects[obj_idx];
    let space = crate::query::space_of(&record.sheet, obj_idx);
    let obj_name = record.objects[obj_idx].name.clone();
    let broken = match compute_break(base, spec) {
        Ok(b) => b,
        Err(e) => {
            let field = match by {
                OpBreakBy::AtPoint { .. } => "at",
                OpBreakBy::Between { .. } => "from",
            };
            return fail(reach, op_err(e.code, e.message, field));
        }
    };

    let split_local_id = if let Some(ref split_obj) = broken.split {
        Some(match new_id {
            Some(explicit) => explicit.to_owned(),
            None => {
                let mut taken: BTreeSet<String> = BTreeSet::new();
                for &k in &reach {
                    match at.objects(k) {
                        Ok(objects) => {
                            taken.extend(objects.into_iter().map(|o| o.3.id.to_lowercase()));
                        }
                        Err(e) => return fail(reach.clone(), e),
                    }
                }
                let prefix = split_obj.type_name();
                (1..)
                    .map(|n| format!("{prefix}-{n}"))
                    .find(|c| !taken.contains(&c.to_lowercase()))
                    .expect("a free id")
            }
        })
    } else {
        None
    };

    let path = match found.file {
        FileRef::Rows(f) => files.rows_files[f].path.clone(),
        FileRef::Template(t) => files.templates[t].path.clone(),
    };
    let (kept_kind, kept_keys) =
        object_to_record_keys(&broken.kept, &local, obj_name.as_deref(), space.as_deref());
    let rec = match at.file(found.file) {
        Ok(r) => r,
        Err(e) => return fail(reach, e),
    };
    let before = rec.object_text(found.list, found.index);
    if let Err(e) = rec.replace_object(found.list, found.index, kept_kind, &kept_keys, op.index) {
        return fail(reach, op_err(e.code, e.message, "target"));
    }
    let after = rec.object_text(found.list, found.index);
    let mut split_full_id = None;
    if let (Some(split_obj), Some(split_local)) = (&broken.split, &split_local_id) {
        let (sk, skeys) = object_to_record_keys(split_obj, split_local, None, space.as_deref());
        if let Err(e) = rec.add(found.list, sk, &skeys, op.index) {
            return fail(reach, op_err(e.code, e.message, "new_id"));
        }
        split_full_id = Some(format!(
            "{}{ID_SEPARATOR}{split_local}",
            files.name(found.row)
        ));
    }
    let message = match &split_full_id {
        Some(sid) => format!(
            "breaks `{}`; `{}` carries on and `{sid}` is new{}",
            found.id,
            found.id,
            snapped_words(&snapped)
        ),
        None => format!(
            "breaks `{}` (carries on as {}){}",
            found.id,
            broken.kept.type_name(),
            snapped_words(&snapped)
        ),
    };
    Applied {
        outcome: Ok(Done {
            record: path,
            id: Some(found.id),
            before: Some(before),
            after: Some(after),
            row_value: None,
            row_values: Vec::new(),
            ids_written: Vec::new(),
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed: Vec::new(),
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set: Vec::new(),
            split_id: split_full_id,
            deleted: Vec::new(),
            created: Vec::new(),
            snapped,
            message,
        }),
        sheets: reach,
        refuse: false,
    }
}

/// Joins two or more lines, arcs, and open polylines into one object: AutoCAD's
/// `JOIN` (`J`). `targets[0]`'s ID carries on and `targets[1..]` are deleted.
fn apply_join(
    at: &mut Attempt,
    op: &Operation,
    targets: &[(Target, String)],
    tolerance: f64,
) -> Applied {
    let files = at.files;
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    let fail = |sheets: Vec<usize>, e: Error| Applied {
        sheets,
        outcome: Err(e),
        refuse: false,
    };

    let mut found: Vec<(Found, &str)> = Vec::with_capacity(targets.len());
    let mut reach_set: BTreeSet<usize> = BTreeSet::new();
    for (t, key) in targets {
        match resolve(at, t, op) {
            Ok(f) => {
                reach_set.extend(at.reach(f.file, f.row));
                found.push((f, key.as_str()));
            }
            Err(applied) => return keyed(*applied, op, key),
        }
    }
    let reach: Vec<usize> = reach_set.into_iter().collect();
    let row0 = found[0].0.row;

    for (i, (f, key)) in found.iter().enumerate() {
        if let Some(refusal) = many_paths(files, f.file, op, key) {
            return refusal;
        }
        if f.row != row0 {
            return fail(
                reach,
                op_err(
                    ErrorCode::BadValue,
                    format!(
                        "`{}` is on sheet `{}`, not on sheet `{}`",
                        f.id,
                        files.name(f.row),
                        files.name(row0)
                    ),
                    key,
                ),
            );
        }
        if let Some((_, first_key)) = found[..i]
            .iter()
            .find(|(g, _)| (g.file, g.list, g.index) == (f.file, f.list, f.index))
        {
            return fail(
                reach,
                op_err(
                    ErrorCode::BadValue,
                    format!("`{}` is named twice, at {first_key} and {key}", f.id),
                    key,
                ),
            );
        }
        if let Some(layer) = batch_object_locked_layer(at, files, f.row, f.file, f.list, f.index) {
            return fail(
                reach,
                op_err(
                    ErrorCode::Locked,
                    format!(
                        "`{}` is on layer `{layer}`, which is locked; unlock it first with \
                         change_layer, set: {{\"locked\": false}}",
                        f.id
                    ),
                    key,
                ),
            );
        }
        if i > 0 && f.file != found[0].0.file {
            let shared = |file: FileRef| matches!(file, FileRef::Template(t) if files.templates[t].sheets.len() > 1);
            if shared(found[0].0.file) || shared(f.file) {
                return fail(
                    reach,
                    op_err(
                        ErrorCode::BadValue,
                        format!(
                            "cannot join a shared template object with a row-only object on `{}`",
                            files.name(row0)
                        ),
                        key,
                    ),
                );
            }
        }
    }

    let rows = match at.rows_now() {
        Ok(r) => r,
        Err(e) => return fail(reach, e),
    };
    let record = match sheet_record(at, &rows, row0, &mut Vec::new()) {
        Ok(r) => r,
        Err(e) => return fail(reach, e),
    };

    let mut indices = Vec::with_capacity(found.len());
    for (f, key) in &found {
        let local =
            f.id.split_once(ID_SEPARATOR)
                .map_or(f.id.as_str(), |(_, l)| l);
        let in_row = matches!(f.file, FileRef::Rows(_));
        let Some(idx) = record
            .objects
            .iter()
            .position(|o| o.in_row == in_row && o.id.eq_ignore_ascii_case(local))
        else {
            return fail(
                reach,
                op_err(
                    ErrorCode::UnknownName,
                    format!("could not read `{}` to join it", f.id),
                    key,
                ),
            );
        };
        if let Some(&first_idx) = indices.first() {
            if !record.sheet.same_space(first_idx, idx) {
                return fail(
                    reach,
                    op_err(
                        ErrorCode::BadValue,
                        format!(
                            "`{}` is in a different space from `{}`",
                            f.id, found[0].0.id
                        ),
                        key,
                    ),
                );
            }
        }
        indices.push(idx);
    }

    let objs: Vec<&Object> = indices.iter().map(|&i| &record.sheet.objects[i]).collect();
    let joined = match compute_join(&objs, tolerance) {
        Ok(o) => o,
        Err(e) => return fail(reach, op_err(e.code, e.message, "targets")),
    };

    let f0 = &found[0].0;
    let local0 = f0
        .id
        .split_once(ID_SEPARATOR)
        .map_or(f0.id.as_str(), |(_, l)| l)
        .to_owned();
    let name0 = record.objects[indices[0]].name.clone();
    let space = crate::query::space_of(&record.sheet, indices[0]);
    let (jkind, jkeys) =
        object_to_record_keys(&joined, &local0, name0.as_deref(), space.as_deref());

    let path = match f0.file {
        FileRef::Rows(f) => files.rows_files[f].path.clone(),
        FileRef::Template(t) => files.templates[t].path.clone(),
    };
    let (before, after) = {
        let rec0 = match at.file(f0.file) {
            Ok(r) => r,
            Err(e) => return fail(reach, e),
        };
        let b = rec0.object_text(f0.list, f0.index);
        if let Err(e) = rec0.replace_object(f0.list, f0.index, jkind, &jkeys, op.index) {
            return fail(reach, op_err(e.code, e.message, "targets[0]"));
        }
        let a = rec0.object_text(f0.list, f0.index);
        (b, a)
    };

    // Delete absorbed objects (`found[1..]`) in descending index order per list
    // so earlier indices never shift before deletion.
    let mut to_delete: Vec<(FileRef, ObjectList, usize, String, Vec<usize>)> = found[1..]
        .iter()
        .map(|(f, _)| {
            (
                f.file,
                f.list,
                f.index,
                f.id.clone(),
                at.reach(f.file, f.row),
            )
        })
        .collect();
    let deleted_in_order: Vec<(String, Vec<usize>)> = to_delete
        .iter()
        .map(|(_, _, _, id, on)| (id.clone(), on.clone()))
        .collect();
    to_delete.sort_by_key(|a| std::cmp::Reverse(a.2));

    let mut ids_written: Vec<String> = Vec::new();
    for (file, list, idx, _, on) in to_delete {
        let rec = match at.file(file) {
            Ok(r) => r,
            Err(e) => return fail(reach, e),
        };
        let kept = rec.delete(list, idx, op.index);
        for place in kept {
            for &s in &on {
                ids_written.push(format!("{}/{place}", files.name(s)));
            }
        }
    }

    let removed_list: Vec<String> = deleted_in_order
        .iter()
        .map(|(id, _)| format!("`{id}`"))
        .collect();
    let message = format!(
        "joins {} objects into {} `{}` (carries on; removed {})",
        found.len(),
        joined.type_name(),
        f0.id,
        removed_list.join(", ")
    );
    Applied {
        outcome: Ok(Done {
            record: path,
            id: Some(f0.id.clone()),
            before: Some(before),
            after: Some(after),
            row_value: None,
            row_values: Vec::new(),
            ids_written,
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed: Vec::new(),
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set: Vec::new(),
            split_id: None,
            deleted: deleted_in_order,
            created: Vec::new(),
            snapped: Vec::new(),
            message,
        }),
        sheets: reach,
        refuse: false,
    }
}

/// Moves endpoints, vertices and rigid objects lying inside `corners` by
/// `motion`, leaving vertices outside the box fixed: AutoCAD's `STRETCH` (`S`).
#[allow(clippy::too_many_arguments)]
fn apply_stretch(
    at: &mut Attempt,
    op: &Operation,
    to: &AddTo,
    space: Option<&str>,
    targets: Option<&[(Target, String)]>,
    corners: &[Box<OpPoint>; 2],
    motion: &Motion,
    settings: &Settings,
) -> Applied {
    let files = at.files;
    let units = files.units;
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    let fail = |sheets: Vec<usize>, e: Error| Applied {
        sheets,
        outcome: Err(e),
        refuse: false,
    };

    let (primary_row, scope_reach, template_only) = match to {
        AddTo::Sheet(name) => {
            let k = files.sheet(name).expect("checked before");
            (k, vec![k], None)
        }
        AddTo::Template(written) => {
            let t = files.template(written).expect("checked before");
            let sheets = files.templates[t].sheets.clone();
            if let Some(refusal) = many_paths(files, FileRef::Template(t), op, "template") {
                return refusal;
            }
            (sheets[0], sheets, Some(t))
        }
    };

    let mut snapped: Vec<SnapResolved> = Vec::new();
    let resolve_pt =
        |at: &Attempt, p: &OpPoint, snapped: &mut Vec<SnapResolved>| -> Result<[f64; 3], Error> {
            match p {
                OpPoint::At(xyz) => Ok(*xyz),
                OpPoint::Snap(snap) => {
                    snapped_points(at, op, &scope_reach, &[snap], settings).map(|mut pts| {
                        let (xyz, res) = pts.remove(0);
                        snapped.push(res);
                        xyz
                    })
                }
            }
        };
    let c0 = match resolve_pt(at, &corners[0], &mut snapped) {
        Ok(p) => p,
        Err(e) => return fail(scope_reach, e),
    };
    let c1 = match resolve_pt(at, &corners[1], &mut snapped) {
        Ok(p) => p,
        Err(e) => return fail(scope_reach, e),
    };
    let stretch_box = match StretchBox::from_corners(c0, c1) {
        Ok(b) => b,
        Err(e) => return fail(scope_reach, op_err(e.code, e.message, "box")),
    };
    let delta = match motion {
        Motion::By(d) => *d,
        Motion::Between { from, to } => {
            let a = match resolve_pt(at, from, &mut snapped) {
                Ok(p) => p,
                Err(e) => return fail(scope_reach, e),
            };
            let b = match resolve_pt(at, to, &mut snapped) {
                Ok(p) => p,
                Err(e) => return fail(scope_reach, e),
            };
            [b[0] - a[0], b[1] - a[1], b[2] - a[2]]
        }
    };

    let rows = match at.rows_now() {
        Ok(r) => r,
        Err(e) => return fail(scope_reach, e),
    };
    let record = match sheet_record(at, &rows, primary_row, &mut Vec::new()) {
        Ok(r) => r,
        Err(e) => return fail(scope_reach, e),
    };
    let wanted_space = space.filter(|s| !s.eq_ignore_ascii_case("model"));
    if let Some(layout) = wanted_space {
        let has_layout = record
            .sheet
            .layouts()
            .any(|l| l.name.eq_ignore_ascii_case(layout));
        if !has_layout {
            return fail(
                scope_reach,
                op_err(
                    ErrorCode::UnknownName,
                    format!(
                        "sheet `{}` has no layout `{layout}`",
                        files.name(primary_row)
                    ),
                    "space",
                ),
            );
        }
    }

    // Build optional target filter if `targets` was explicitly given.
    let explicit_filter: Option<Vec<(FileRef, ObjectList, usize)>> = if let Some(ts) = targets {
        let mut set = Vec::with_capacity(ts.len());
        for (t, key) in ts {
            let f = match resolve(at, t, op) {
                Ok(f) => f,
                Err(applied) => return keyed(*applied, op, key),
            };
            if f.row != primary_row {
                return fail(
                    scope_reach,
                    op_err(
                        ErrorCode::BadValue,
                        format!(
                            "`{}` is on sheet `{}`, not `{}`",
                            f.id,
                            files.name(f.row),
                            files.name(primary_row)
                        ),
                        key,
                    ),
                );
            }
            if let Some(layer) =
                batch_object_locked_layer(at, files, f.row, f.file, f.list, f.index)
            {
                return fail(
                    scope_reach,
                    op_err(
                        ErrorCode::Locked,
                        format!(
                            "`{}` is on layer `{layer}`, which is locked; unlock it first with \
                             change_layer, set: {{\"locked\": false}}",
                            f.id
                        ),
                        key,
                    ),
                );
            }
            set.push((f.file, f.list, f.index));
        }
        Some(set)
    } else {
        None
    };

    let objects_k = match at.objects(primary_row) {
        Ok(o) => o,
        Err(e) => return fail(scope_reach, e),
    };

    struct PendingStretch {
        file: FileRef,
        list: ObjectList,
        list_idx: usize,
        full_id: String,
        local_id: String,
        name: Option<String>,
        space: Option<String>,
        reach: Vec<usize>,
        action: StretchAction,
    }

    let mut pending: Vec<PendingStretch> = Vec::new();
    let mut locked_skipped = 0usize;
    let sheet_name = files.name(primary_row).to_owned();

    for (obj_idx, &(file, list, list_idx, ref listed)) in objects_k.iter().enumerate() {
        if let Some(t_only) = template_only {
            if file != FileRef::Template(t_only) {
                continue;
            }
        }
        if let Some(ref filter) = explicit_filter {
            if !filter.contains(&(file, list, list_idx)) {
                continue;
            }
        }
        let obj_space = crate::query::space_of(&record.sheet, obj_idx);
        let in_space = match (wanted_space, obj_space.as_deref()) {
            (None, None) => true,
            (Some(w), Some(o)) => w.eq_ignore_ascii_case(o),
            _ => false,
        };
        if !in_space {
            continue;
        }
        let obj = &record.sheet.objects[obj_idx];
        let action = match compute_stretch(obj, stretch_box, delta) {
            Ok(a) => a,
            Err(e) => return fail(scope_reach, op_err(e.code, e.message, "by")),
        };
        if matches!(action, StretchAction::Untouched) {
            continue;
        }
        if explicit_filter.is_none()
            && batch_object_locked_layer(at, files, primary_row, file, list, list_idx).is_some()
        {
            locked_skipped += 1;
            continue;
        }
        let full_id = format!("{sheet_name}{ID_SEPARATOR}{}", listed.id);
        if template_only.is_none() {
            if let FileRef::Template(t_idx) = file {
                if files.templates[t_idx].sheets.len() > 1 {
                    return fail(
                        scope_reach,
                        op_err(
                            ErrorCode::BadValue,
                            format!(
                                "`{full_id}` lies inside the stretch box but belongs to shared \
                                 template `{}`; stretch with `template: \"{}\"` to move it on \
                                 every sheet, lock its layer, or pass `targets`",
                                files.templates[t_idx].written, files.templates[t_idx].written
                            ),
                            "box",
                        ),
                    );
                }
            }
        }
        if let Some(refusal) = many_paths(files, file, op, "box") {
            return refusal;
        }
        let reach = at.reach(file, primary_row);
        pending.push(PendingStretch {
            file,
            list,
            list_idx,
            full_id,
            local_id: listed.id.clone(),
            name: record.objects[obj_idx].name.clone(),
            space: obj_space,
            reach,
            action,
        });
    }

    let mut transformed: Vec<(String, Vec<usize>)> = Vec::with_capacity(pending.len());
    let mut all_reach: BTreeSet<usize> = scope_reach.iter().copied().collect();
    let mut stretched_count = 0usize;
    let mut moved_count = 0usize;
    let mut first_record: Option<PathBuf> = None;

    for item in pending {
        let rec_path = match item.file {
            FileRef::Rows(f) => files.rows_files[f].path.clone(),
            FileRef::Template(t) => files.templates[t].path.clone(),
        };
        first_record.get_or_insert(rec_path);
        let rec = match at.file(item.file) {
            Ok(r) => r,
            Err(e) => return fail(scope_reach, e),
        };
        match item.action {
            StretchAction::MoveWhole => {
                if let Err(e) = rec.move_by(item.list, item.list_idx, delta, units, op.index) {
                    return fail(scope_reach, op_err(e.code, e.message, "by"));
                }
                moved_count += 1;
            }
            StretchAction::Replace(new_obj) => {
                let (kind, keys) = object_to_record_keys(
                    &new_obj,
                    &item.local_id,
                    item.name.as_deref(),
                    item.space.as_deref(),
                );
                if let Err(e) = rec.replace_object(item.list, item.list_idx, kind, &keys, op.index)
                {
                    return fail(scope_reach, op_err(e.code, e.message, "by"));
                }
                stretched_count += 1;
            }
            StretchAction::Untouched => unreachable!(),
        }
        all_reach.extend(item.reach.iter().copied());
        transformed.push((item.full_id, item.reach));
    }

    let total = transformed.len();
    let locked_note = if locked_skipped > 0 {
        format!(" ({locked_skipped} on locked layers skipped)")
    } else {
        String::new()
    };
    let message = if total == 0 {
        format!(
            "no unlocked object lies inside the stretch box{locked_note}{}",
            snapped_words(&snapped)
        )
    } else {
        format!(
            "stretches {stretched_count} and moves {moved_count} object{} ({total} touched){locked_note}{}",
            if total == 1 { "" } else { "s" },
            snapped_words(&snapped)
        )
    };
    let record_path = first_record.unwrap_or_else(|| match to {
        AddTo::Sheet(_) => files.rows_files[files.rows[primary_row].file_index]
            .path
            .clone(),
        AddTo::Template(w) => files.templates[files.template(w).unwrap()].path.clone(),
    });
    Applied {
        outcome: Ok(Done {
            record: record_path,
            id: None,
            before: None,
            after: None,
            row_value: None,
            row_values: Vec::new(),
            ids_written: Vec::new(),
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed,
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set: Vec::new(),
            split_id: None,
            deleted: Vec::new(),
            created: Vec::new(),
            snapped,
            message,
        }),
        sheets: all_reach.into_iter().collect(),
        refuse: false,
    }
}

/// Ensures any layer, linetype, or text style brought into `sheet` from the
/// block library and referenced by `pieces` is defined in the template so
/// exploding the last insert of a block never leaves an undefined table entry.
fn ensure_template_tables_for_exploded(
    at: &mut Attempt,
    template_idx: usize,
    sheet: &crate::model::Sheet,
    pieces: &[Object],
) -> Result<(), Error> {
    let mut needed_layers: Vec<String> = Vec::new();
    let mut needed_linetypes: Vec<String> = Vec::new();
    let mut needed_styles: Vec<String> = Vec::new();

    for obj in pieces {
        let layer = obj.layer();
        if layer != crate::model::LAYER_ZERO && !needed_layers.iter().any(|l| l == layer) {
            needed_layers.push(layer.to_owned());
        }
        if let crate::model::ObjectLinetype::Named(lt) = &obj.properties().linetype {
            if lt != crate::model::CONTINUOUS && !needed_linetypes.iter().any(|t| t == lt) {
                needed_linetypes.push(lt.clone());
            }
        }
        match obj {
            Object::Text(t)
                if t.style != crate::text::STANDARD
                    && !needed_styles.iter().any(|s| s == &t.style) =>
            {
                needed_styles.push(t.style.clone());
            }
            Object::MText(t)
                if t.style != crate::text::STANDARD
                    && !needed_styles.iter().any(|s| s == &t.style) =>
            {
                needed_styles.push(t.style.clone());
            }
            _ => {}
        }
    }
    for lname in &needed_layers {
        if let Some(layer) = sheet.layers.iter().find(|l| l.name == *lname) {
            if layer.linetype != crate::model::CONTINUOUS
                && !needed_linetypes.iter().any(|t| t == &layer.linetype)
            {
                needed_linetypes.push(layer.linetype.clone());
            }
        }
    }

    let tmpl = at.template(template_idx)?;
    let existing_lt: BTreeSet<String> = tmpl
        .table_names(Table::Linetype.key())
        .into_iter()
        .map(|s| s.to_lowercase())
        .collect();
    for lt_name in &needed_linetypes {
        if existing_lt.contains(&lt_name.to_lowercase()) {
            continue;
        }
        if let Some(lt) = sheet.linetypes.iter().find(|t| t.name == *lt_name) {
            let mut keys = serde_json::Map::new();
            if !lt.description.is_empty() {
                keys.insert(
                    "description".to_owned(),
                    Value::String(lt.description.clone()),
                );
            }
            keys.insert("pattern".to_owned(), serde_json::json!(lt.pattern));
            tmpl.table_add(
                Table::Linetype.key(),
                Table::Linetype.entry_keys(),
                &lt.name,
                &keys,
            )?;
        }
    }
    let existing_layers: BTreeSet<String> = tmpl
        .table_names(Table::Layer.key())
        .into_iter()
        .map(|s| s.to_lowercase())
        .collect();
    for l_name in &needed_layers {
        if existing_layers.contains(&l_name.to_lowercase()) {
            continue;
        }
        if let Some(layer) = sheet.layers.iter().find(|l| l.name == *l_name) {
            let mut keys = serde_json::Map::new();
            keys.insert("color".to_owned(), serde_json::json!(layer.color));
            if layer.linetype != crate::model::CONTINUOUS {
                keys.insert("linetype".to_owned(), Value::String(layer.linetype.clone()));
            }
            if let Some(lw) = layer.lineweight {
                keys.insert("lineweight".to_owned(), serde_json::json!(lw));
            }
            if !layer.on {
                keys.insert("on".to_owned(), Value::Bool(false));
            }
            if layer.frozen {
                keys.insert("frozen".to_owned(), Value::Bool(true));
            }
            if layer.locked {
                keys.insert("locked".to_owned(), Value::Bool(true));
            }
            if !layer.plot {
                keys.insert("plot".to_owned(), Value::Bool(false));
            }
            if !layer.description.is_empty() {
                keys.insert(
                    "description".to_owned(),
                    Value::String(layer.description.clone()),
                );
            }
            tmpl.table_add(
                Table::Layer.key(),
                Table::Layer.entry_keys(),
                &layer.name,
                &keys,
            )?;
        }
    }
    let existing_styles: BTreeSet<String> = tmpl
        .table_names(Table::TextStyle.key())
        .into_iter()
        .map(|s| s.to_lowercase())
        .collect();
    for s_name in &needed_styles {
        if existing_styles.contains(&s_name.to_lowercase()) {
            continue;
        }
        if let Some(style) = sheet.text_styles.iter().find(|s| s.name == *s_name) {
            let mut keys = serde_json::Map::new();
            keys.insert("font".to_owned(), Value::String(style.font.clone()));
            if (style.width_factor - 1.0).abs() > 1e-12 {
                keys.insert(
                    "width_factor".to_owned(),
                    serde_json::json!(style.width_factor),
                );
            }
            tmpl.table_add(
                Table::TextStyle.key(),
                Table::TextStyle.entry_keys(),
                &style.name,
                &keys,
            )?;
        }
    }
    Ok(())
}

/// Explodes one or more compound objects into the simpler objects that draw
/// them: AutoCAD's `EXPLODE` (`X`).
fn apply_explode(
    at: &mut Attempt,
    op: &Operation,
    targets: &[(Target, String)],
    id_prefix: Option<&str>,
    settings: &Settings,
) -> Applied {
    let files = at.files;
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
    let fail = |sheets: Vec<usize>, e: Error| Applied {
        sheets,
        outcome: Err(e),
        refuse: false,
    };

    let mut found: Vec<(Found, &str)> = Vec::with_capacity(targets.len());
    let mut all_reach: BTreeSet<usize> = BTreeSet::new();
    for (t, key) in targets {
        let f = match resolve(at, t, op) {
            Ok(f) => f,
            Err(applied) => return keyed(*applied, op, key),
        };
        let reach_f = at.reach(f.file, f.row);
        all_reach.extend(reach_f.iter().copied());
        if let Some(refusal) = many_paths(files, f.file, op, key) {
            return refusal;
        }
        if let Some(first) = found.first() {
            if f.row != first.0.row {
                return fail(
                    all_reach.into_iter().collect(),
                    op_err(
                        ErrorCode::BadValue,
                        format!(
                            "`{}` is on sheet `{}`, not `{}` — explode objects on one sheet at a \
                             time",
                            f.id,
                            files.name(f.row),
                            files.name(first.0.row)
                        ),
                        key,
                    ),
                );
            }
        }
        if found
            .iter()
            .any(|(prev, _)| prev.file == f.file && prev.list == f.list && prev.index == f.index)
        {
            return fail(
                all_reach.into_iter().collect(),
                op_err(
                    ErrorCode::BadValue,
                    format!("`{}` is listed more than once to explode", f.id),
                    key,
                ),
            );
        }
        if let Some(layer) = batch_object_locked_layer(at, files, f.row, f.file, f.list, f.index) {
            return fail(
                all_reach.into_iter().collect(),
                op_err(
                    ErrorCode::Locked,
                    format!(
                        "`{}` is on layer `{layer}`, which is locked; unlock it first with \
                         change_layer, set: {{\"locked\": false}}",
                        f.id
                    ),
                    key,
                ),
            );
        }
        found.push((f, key.as_str()));
    }

    let reach: Vec<usize> = all_reach.into_iter().collect();
    let row_k = found[0].0.row;
    let sheet_name = files.name(row_k).to_owned();
    let rows = match at.rows_now() {
        Ok(r) => r,
        Err(e) => return fail(reach, e),
    };
    let record = match sheet_record(at, &rows, row_k, &mut Vec::new()) {
        Ok(r) => r,
        Err(e) => return fail(reach, e),
    };
    let fonts = crate::text::placement::SheetFonts::find(&record.sheet, settings);

    struct ExplodedTarget {
        file: FileRef,
        list: ObjectList,
        list_idx: usize,
        full_id: String,
        local_id: String,
        space: Option<String>,
        reach: Vec<usize>,
        pieces: Vec<Object>,
    }

    let mut plan: Vec<ExplodedTarget> = Vec::with_capacity(found.len());
    for (f, key) in &found {
        let local =
            f.id.split_once(ID_SEPARATOR)
                .map_or(f.id.as_str(), |(_, l)| l)
                .to_owned();
        let in_row = matches!(f.file, FileRef::Rows(_));
        let Some(obj_idx) = record
            .objects
            .iter()
            .position(|o| o.in_row == in_row && o.id.eq_ignore_ascii_case(&local))
        else {
            return fail(
                reach,
                op_err(
                    ErrorCode::UnknownName,
                    format!("could not read `{}` to explode it", f.id),
                    key,
                ),
            );
        };
        let obj = &record.sheet.objects[obj_idx];
        let pieces = match compute_explode(obj, &record.sheet, &fonts) {
            Ok(p) => p,
            Err(e) => return fail(reach, op_err(e.code, e.message, key)),
        };
        let space = crate::query::space_of(&record.sheet, obj_idx);
        let target_reach = at.reach(f.file, f.row);
        plan.push(ExplodedTarget {
            file: f.file,
            list: f.list,
            list_idx: f.index,
            full_id: f.id.clone(),
            local_id: local,
            space,
            reach: target_reach,
            pieces,
        });
    }

    // Collect every existing local ID on any reached sheet so newly created IDs never collide.
    let mut taken: BTreeSet<String> = BTreeSet::new();
    for &k in &reach {
        match at.objects(k) {
            Ok(objects) => {
                taken.extend(objects.into_iter().map(|o| o.3.id.to_lowercase()));
            }
            Err(e) => return fail(reach, e),
        }
    }

    // Ensure any block-library layers, linetypes, or text styles referenced by exploded
    // objects exist in the template before we remove the source insert.
    let template_idx = files.template_of[row_k];
    let all_pieces: Vec<Object> = plan.iter().flat_map(|p| p.pieces.iter().cloned()).collect();
    if let Err(e) =
        ensure_template_tables_for_exploded(at, template_idx, &record.sheet, &all_pieces)
    {
        return fail(reach, op_err(e.code, e.message, "target"));
    }

    let first_record = match plan[0].file {
        FileRef::Rows(f) => files.rows_files[f].path.clone(),
        FileRef::Template(t) => files.templates[t].path.clone(),
    };

    let mut created: Vec<(String, Vec<usize>)> = Vec::new();
    let mut global_seq = 1usize;

    // Add all exploded objects first so indices of targets to delete remain stable.
    for item in &plan {
        let rec = match at.file(item.file) {
            Ok(r) => r,
            Err(e) => return fail(reach, e),
        };
        let mut local_seq = 1usize;
        for piece in &item.pieces {
            let new_local = match id_prefix {
                Some(prefix) => loop {
                    let cand = format!("{prefix}-{global_seq}");
                    global_seq += 1;
                    if taken.insert(cand.to_lowercase()) {
                        break cand;
                    }
                },
                None => loop {
                    let cand = format!("{}-{local_seq}", item.local_id);
                    local_seq += 1;
                    if taken.insert(cand.to_lowercase()) {
                        break cand;
                    }
                },
            };
            let (kind, keys) =
                object_to_record_keys(piece, &new_local, None, item.space.as_deref());
            if let Err(e) = rec.add(item.list, kind, &keys, op.index) {
                return fail(reach, op_err(e.code, e.message, "target"));
            }
            let full_new = format!("{sheet_name}{ID_SEPARATOR}{new_local}");
            created.push((full_new, item.reach.clone()));
        }
    }

    // Delete the exploded source objects in descending list-index order.
    let deleted_in_order: Vec<(String, Vec<usize>)> = plan
        .iter()
        .map(|p| (p.full_id.clone(), p.reach.clone()))
        .collect();
    let mut to_delete: Vec<(FileRef, ObjectList, usize, Vec<usize>)> = plan
        .iter()
        .map(|p| (p.file, p.list, p.list_idx, p.reach.clone()))
        .collect();
    to_delete.sort_by_key(|a| std::cmp::Reverse(a.2));

    let mut ids_written: Vec<String> = Vec::new();
    for (file, list, idx, on) in to_delete {
        let rec = match at.file(file) {
            Ok(r) => r,
            Err(e) => return fail(reach, e),
        };
        let kept = rec.delete(list, idx, op.index);
        for place in kept {
            for &s in &on {
                ids_written.push(format!("{}/{place}", files.name(s)));
            }
        }
    }

    let first_deleted_id = (plan.len() == 1).then(|| plan[0].full_id.clone());
    let src_list: Vec<&str> = plan.iter().map(|p| p.full_id.as_str()).collect();
    let message = format!(
        "explodes {} into {} object{}",
        quoted_list(&src_list),
        created.len(),
        if created.len() == 1 { "" } else { "s" }
    );

    Applied {
        outcome: Ok(Done {
            record: first_record,
            id: first_deleted_id,
            before: None,
            after: None,
            row_value: None,
            row_values: Vec::new(),
            ids_written,
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed: Vec::new(),
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set: Vec::new(),
            split_id: None,
            deleted: deleted_in_order,
            created,
            snapped: Vec::new(),
            message,
        }),
        sheets: reach,
        refuse: false,
    }
}

/// when `margin` is omitted), and written as a rectangular `revision_cloud`
/// object in the sheet's row or template.
#[allow(clippy::too_many_arguments)]
fn apply_revision_cloud_around(
    at: &mut Attempt,
    op: &Operation,
    to: Option<&AddTo>,
    around: &[(Target, String)],
    margin_value: Option<&serde_json::Value>,
    keys: &serde_json::Map<String, serde_json::Value>,
    draw_order: Option<&DrawOrderPlace>,
    settings: &Settings,
) -> Applied {
    let files = at.files;
    let fail = |sheets: Vec<usize>, e: Error| Applied {
        sheets,
        outcome: Err(e),
        refuse: false,
    };
    let op_err = |code: ErrorCode, message: String, field: &str| {
        op_error(op.index, op.spec.name, code, message, field)
    };
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

    let (found, _reach, k) = match found_objects(at, op, around) {
        Ok(found) => found,
        Err(applied) => return *applied,
    };

    let (file, list, sheets, first) = match to {
        Some(AddTo::Sheet(name)) => {
            let target_k = files.sheet(name).expect("checked before");
            if target_k != k {
                return fail(
                    vec![target_k],
                    op_err(
                        ErrorCode::BadValue,
                        format!(
                            "`around` objects are on sheet `{}`, not `{name}`; omit `sheet` or \
                             name the same sheet",
                            files.name(k)
                        ),
                        "sheet",
                    ),
                );
            }
            let (file, list) = files.row_list(target_k);
            (file, list, vec![target_k], target_k)
        }
        Some(AddTo::Template(written)) => {
            let t = files.template(written).expect("checked before");
            let sheets = files.templates[t].sheets.clone();
            if !sheets.contains(&k) {
                return fail(
                    sheets,
                    op_err(
                        ErrorCode::BadValue,
                        format!(
                            "`around` objects are on sheet `{}`, which is not built from template \
                             `{written}`",
                            files.name(k)
                        ),
                        "template",
                    ),
                );
            }
            (FileRef::Template(t), ObjectList::Template, sheets, k)
        }
        None => {
            let first_template = match found.first().map(|(f, _)| f.file) {
                Some(FileRef::Template(t))
                    if found.iter().all(|(f, _)| f.file == FileRef::Template(t)) =>
                {
                    Some(t)
                }
                _ => None,
            };
            if let Some(t) = first_template {
                let sheets = files.templates[t].sheets.clone();
                (FileRef::Template(t), ObjectList::Template, sheets, k)
            } else {
                let (file, list) = files.row_list(k);
                (file, list, vec![k], k)
            }
        }
    };

    if let Some(refusal) = many_paths(files, file, op, "template") {
        return refusal;
    }

    let (record, fonts) = match sheet_geometry(at, op, k, settings) {
        Ok(pair) => pair,
        Err(e) => return fail(sheets, e),
    };
    let units = record.sheet.units;
    let drawn = crate::query::measuring_objects(&record.sheet, &fonts);
    let mut points: Vec<[f64; 2]> = Vec::new();
    for (f, key) in &found {
        let Some(place) = model_index(&record, f) else {
            continue;
        };
        let before = points.len();
        points.extend(
            drawn
                .items
                .iter()
                .filter(|item| item.object == place)
                .flat_map(|item| item.points()),
        );
        points.extend(
            drawn
                .boundaries
                .iter()
                .filter(|(object, _)| *object == place)
                .flat_map(|(_, b)| [b.min, b.max]),
        );
        if points.len() == before {
            return fail(
                sheets,
                op_err(
                    ErrorCode::BadValue,
                    format!(
                        "`{}` draws nothing, so it has no extents to frame with `around`",
                        f.id
                    ),
                    key,
                ),
            );
        }
    }
    if points.is_empty() {
        return fail(
            sheets,
            op_err(
                ErrorCode::BadValue,
                "the `around` objects have no drawn extents to frame".to_owned(),
                "around",
            ),
        );
    }
    let (min, max) = points.iter().fold(
        ([f64::MAX, f64::MAX], [f64::MIN, f64::MIN]),
        |(min, max), p| {
            (
                [min[0].min(p[0]), min[1].min(p[1])],
                [max[0].max(p[0]), max[1].max(p[1])],
            )
        },
    );
    let raw_w = (max[0] - min[0]).max(0.0);
    let raw_h = (max[1] - min[1]).max(0.0);

    let (arc_length, write_arc_length) = match keys.get("arc_length").filter(|v| !v.is_null()) {
        Some(value) => match crate::description::parse_length_json(value, units, "arc_length") {
            Ok(len) if len.is_finite() && len > 0.0 => (len, false),
            Ok(_) => {
                return fail(
                    sheets,
                    op_err(
                        ErrorCode::BadValue,
                        "a revision cloud's arc_length is more than 0".to_owned(),
                        "arc_length",
                    ),
                );
            }
            Err(e) => return fail(sheets, op_err(e.code, e.message, "arc_length")),
        },
        None => (
            crate::model::RevisionCloud::default_arc_length(raw_w, raw_h),
            true,
        ),
    };

    let margin = match margin_value {
        Some(value) => match crate::description::parse_length_json(value, units, "margin") {
            Ok(m) if m.is_finite() && m >= 0.0 => m,
            Ok(_) => {
                return fail(
                    sheets,
                    op_err(
                        ErrorCode::BadValue,
                        "a revision cloud's margin cannot be negative".to_owned(),
                        "margin",
                    ),
                );
            }
            Err(e) => return fail(sheets, op_err(e.code, e.message, "margin")),
        },
        None => crate::model::RevisionCloud::default_margin(arc_length),
    };

    let x0 = min[0] - margin;
    let y0 = min[1] - margin;
    let x1 = max[0] + margin;
    let y1 = max[1] + margin;
    if !(x1 > x0 && y1 > y0) {
        return fail(
            sheets,
            op_err(
                ErrorCode::BadValue,
                "the framed extents have zero width or height; give a margin more than 0"
                    .to_owned(),
                "margin",
            ),
        );
    }

    let mut resolved_keys = keys.clone();
    if resolved_keys
        .get("id")
        .is_none_or(serde_json::Value::is_null)
    {
        let mut taken: BTreeSet<String> = BTreeSet::new();
        for &s in &sheets {
            match at.objects(s) {
                Ok(objects) => {
                    taken.extend(objects.into_iter().map(|o| o.3.id.to_lowercase()));
                }
                Err(e) => return fail(sheets.clone(), e),
            }
        }
        let id = (1..)
            .map(|n| format!("revision_cloud-{n}"))
            .find(|id| !taken.contains(id))
            .expect("a free number");
        resolved_keys.insert("id".into(), serde_json::Value::String(id));
    }
    resolved_keys.insert(
        "rectangle".into(),
        serde_json::json!({
            "from": [x0, y0, 0.0],
            "to": [x1, y1, 0.0],
        }),
    );
    if write_arc_length {
        resolved_keys.insert("arc_length".into(), serde_json::json!(arc_length));
    }

    let insert_at = match resolve_add_insert_at(at, op, file, list, &sheets, draw_order) {
        Ok(pos) => pos,
        Err(applied) => return *applied,
    };
    let record = match at.file(file) {
        Ok(record) => record,
        Err(e) => return fail(sheets, e),
    };
    let (index, kept) =
        match record.add_at(list, "revision_cloud", &resolved_keys, insert_at, op.index) {
            Ok(res) => res,
            Err(e) => return fail(sheets, from_record(e)),
        };
    let listed = &record.objects(list)[index];
    let id = format!("{}/{}", files.name(first), listed.id);
    let after = record.object_text(list, index);
    let path = record.path.clone();
    let ids_written: Vec<String> = kept
        .iter()
        .flat_map(|place| {
            sheets
                .iter()
                .map(move |&s| format!("{}/{place}", files.name(s)))
        })
        .collect();
    let place = match file {
        FileRef::Rows(_) => format!("to sheet `{}`, in its row", files.name(first)),
        FileRef::Template(t) => format!(
            "to the template `{}`, so it is on {}",
            files.templates[t].written,
            every_sheet(&files.names(&sheets))
        ),
    };
    let around_ids: Vec<&str> = found.iter().map(|(f, _)| f.id.as_str()).collect();
    Applied {
        outcome: Ok(Done {
            record: path,
            id: Some(id.clone()),
            before: None,
            after: Some(after),
            row_value: None,
            row_values: Vec::new(),
            ids_written,
            inserts: Vec::new(),
            attributes_removed: Vec::new(),
            imported: None,
            copies: Vec::new(),
            copy_sheets: Vec::new(),
            transformed: Vec::new(),
            created_row_values: Vec::new(),
            replacements: Vec::new(),
            attributes_set: Vec::new(),
            split_id: None,
            deleted: Vec::new(),
            created: Vec::new(),
            message: format!(
                "adds a revision_cloud, `{id}`, around {}, {place}",
                quoted_list(&around_ids)
            ),
            snapped: Vec::new(),
        }),
        sheets,
        refuse: false,
    }
}

/// The centre of the box round the objects `found` names, on sheet `k`:
/// the base point of a polar array's copies that are not turned, when the
/// operation gives none.
fn extents_center(
    at: &Attempt,
    op: &Operation,
    k: usize,
    found: &[(Found, &str)],
    settings: &Settings,
) -> Result<[f64; 3], Error> {
    let (record, fonts) = sheet_geometry(at, op, k, settings)?;
    // The sheet drawn as `query extents` draws it, each object in its own
    // space: every kind of object has bounds there, an insert's its
    // block's as placed and a text's its letters'.
    let drawn = crate::query::measuring_objects(&record.sheet, &fonts);
    let mut points: Vec<[f64; 2]> = Vec::new();
    for (f, key) in found {
        let Some(place) = model_index(&record, f) else {
            continue;
        };
        let before = points.len();
        points.extend(
            drawn
                .items
                .iter()
                .filter(|item| item.object == place)
                .flat_map(|item| item.points()),
        );
        // A patterned hatch reaches its boundary, as `query extents` takes
        // it, though its lines may stop short of it or draw none.
        points.extend(
            drawn
                .boundaries
                .iter()
                .filter(|(object, _)| *object == place)
                .flat_map(|(_, b)| [b.min, b.max]),
        );
        if points.len() == before {
            return Err(op_error(
                op.index,
                op.spec.name,
                ErrorCode::BadValue,
                format!(
                    "`{}` draws nothing, so it has no bounds to take a base point from; give \
                     base_point",
                    f.id
                ),
                key,
            ));
        }
    }
    let (min, max) = points.iter().fold(
        ([f64::MAX, f64::MAX], [f64::MIN, f64::MIN]),
        |(min, max), p| {
            (
                [min[0].min(p[0]), min[1].min(p[1])],
                [max[0].max(p[0]), max[1].max(p[1])],
            )
        },
    );
    if points.is_empty() {
        return Err(op_error(
            op.index,
            op.spec.name,
            ErrorCode::BadValue,
            "the objects have no bounds to take a base point from; give base_point",
            "base_point",
        ));
    }
    Ok([(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0, 0.0])
}

/// The ID and the curves, end to end, of the path a path array runs along,
/// which `path` names on sheet `k`, the objects' sheet: a line, a
/// polyline, an arc, a circle, an ellipse or a spline. A path of a row's
/// own is on its sheet alone, so it does not carry a template's objects.
fn path_curves(
    at: &mut Attempt,
    op: &Operation,
    path: &Target,
    k: usize,
    found: &[(Found, &str)],
    settings: &Settings,
) -> Result<(String, Vec<Curve>), Box<Applied>> {
    let files = at.files;
    let err = |sheets: Vec<usize>, code: ErrorCode, message: String| {
        Box::new(Applied {
            sheets,
            outcome: Err(op_error(op.index, op.spec.name, code, message, "path")),
            refuse: false,
        })
    };
    let mut reach: BTreeSet<usize> = BTreeSet::new();
    for (f, _) in found {
        reach.extend(at.reach(f.file, f.row));
    }
    let on = match resolve(at, path, op) {
        Ok(on) => on,
        Err(applied) => {
            let mut failed = keyed(*applied, op, "path");
            if !failed.refuse {
                reach.extend(failed.sheets.iter().copied());
                failed.sheets = reach.into_iter().collect();
            }
            return Err(Box::new(failed));
        }
    };
    let reach: Vec<usize> = reach.into_iter().collect();
    if on.row != k {
        return Err(err(
            reach,
            ErrorCode::BadValue,
            format!(
                "the path `{}` is on sheet `{}`, and the objects on sheet `{}`; a path array \
                 runs along a path of the objects' sheet",
                on.id,
                files.name(on.row),
                files.name(k)
            ),
        ));
    }
    let carried = found
        .iter()
        .find(|(f, _)| matches!(f.file, FileRef::Template(_)));
    if let (FileRef::Rows(_), Some((f, _)), true) = (on.file, carried, reach.len() > 1) {
        return Err(err(
            reach,
            ErrorCode::BadValue,
            format!(
                "the path `{}` is sheet `{}`'s own, on that sheet alone, and `{}` is its \
                 template's, so its copies go on every sheet built from the template; run them \
                 along a path of the template",
                on.id,
                files.name(k),
                f.id
            ),
        ));
    }
    let (record, fonts) = sheet_geometry(at, op, k, settings).map_err(|e| {
        Box::new(Applied {
            sheets: reach.clone(),
            outcome: Err(e),
            refuse: false,
        })
    })?;
    let reader = Reader {
        sheet: &record.sheet,
        fonts: &fonts,
    };
    let Some(object) = model_index(&record, &on).and_then(|i| record.sheet.objects.get(i)) else {
        return Err(err(
            reach,
            ErrorCode::UnknownName,
            format!("the path `{}` is not on sheet `{}`", on.id, files.name(k)),
        ));
    };
    match object {
        Object::Line(_)
        | Object::Polyline(_)
        | Object::Arc(_)
        | Object::Circle(_)
        | Object::Ellipse(_)
        | Object::Spline(_) => {}
        other => {
            let what = other.type_name();
            return Err(err(
                reach,
                ErrorCode::BadValue,
                format!(
                    "the path `{}` is {} {what}; a path array runs along a line, a polyline, \
                     an arc, a circle, an ellipse or a spline",
                    on.id,
                    article(what)
                ),
            ));
        }
    }
    let curves = reader.curves(object).into_iter().map(|(_, c)| c).collect();
    Ok((on.id, curves))
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
    let record = sheet_record(at, rows, k, notices)?;
    let dwg = files.dwg(k);
    writable(&dwg)?;
    prepare_sheet(
        &record,
        &files.templates[t].path,
        &dwg,
        &files.mark(k),
        true,
        settings,
    )
}

/// Reads sheet `k` from the records in memory, adding what reading noticed
/// to `notices`.
/// The key a viewport lists the layers frozen in it under.
const FROZEN_LAYERS: &str = "frozen_layers";

/// The keys a clip_viewport sets on the viewport `id`, on the first of
/// `sheets` it is on, as it stands, to clip it to `clip`, or with `clip`
/// null to take its boundary away, as AutoCAD's VPCLIP does: clipped, its
/// window becomes the box round the boundary and its view centre the model
/// point there; unclipped, it keeps the box as its window.
fn clipped_keys(
    at: &Attempt,
    op: &Operation,
    sheets: &[usize],
    id: &str,
    clip: &Value,
    units: Unit,
) -> Result<serde_json::Map<String, Value>, Error> {
    let fail =
        |code: ErrorCode, message: String| op_error(op.index, op.spec.name, code, message, "clip");
    let files = at.files;
    let rows = at.rows_now()?;
    // The sheet the target is named on: a template's viewport is on every
    // sheet built from it, and its ID names one of them.
    let on = id.split_once(ID_SEPARATOR).map(|(sheet, _)| sheet);
    let &k = sheets
        .iter()
        .find(|&&k| on.is_some_and(|on| files.name(k).eq_ignore_ascii_case(on)))
        .or_else(|| sheets.first())
        .ok_or_else(|| fail(ErrorCode::BadValue, format!("`{id}` is on no sheet")))?;
    let record = sheet_record(at, &rows, k, &mut Vec::new()).map_err(|e| {
        fail(
            e.code,
            format!(
                "sheet `{}` cannot be read to clip the viewport: {}",
                files.name(k),
                e.message
            ),
        )
    })?;
    let viewport = (0..record.sheet.objects.len())
        .find(|&i| record.id(i).eq_ignore_ascii_case(id))
        .and_then(|i| match &record.sheet.objects[i] {
            crate::model::Object::Viewport(v) => Some(v.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            fail(
                ErrorCode::BadValue,
                format!("clip_viewport clips a viewport, and `{id}` is not one"),
            )
        })?;
    let length = |v: f64| coordinates([v, 0.0, 0.0])[0].clone();
    let mut set = serde_json::Map::new();
    if clip.is_null() {
        if viewport.clip.is_none() {
            return Err(fail(
                ErrorCode::BadValue,
                format!(
                    "viewport `{id}` is not clipped, so it has no boundary to take away, as \
                     AutoCAD's VPCLIP has none to delete"
                ),
            ));
        }
        let c = viewport.center;
        set.insert("clip".into(), Value::Null);
        set.insert("center".into(), coordinates([c.x, c.y, 0.0]));
        set.insert("width".into(), length(viewport.width));
        set.insert("height".into(), length(viewport.height));
        return Ok(set);
    }
    let boundary = crate::description::clip_from(clip, units)
        .map_err(|e| op_error(op.index, op.spec.name, e.code, e.message, "clip"))?;
    let clipped = viewport.clipped_to(boundary);
    let v = clipped.view_center;
    set.insert("clip".into(), clip.clone());
    for key in ["center", "width", "height"] {
        set.insert(key.into(), Value::Null);
    }
    set.insert("view_center".into(), coordinates([v.x, v.y, 0.0]));
    Ok(set)
}

fn sheet_record(
    at: &Attempt,
    rows: &[Row],
    k: usize,
    notices: &mut Vec<Notice>,
) -> Result<SheetRecord, Error> {
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
        files.units,
        notices,
    )?;
    Ok(record)
}

/// The point each of `snaps`, the snapped points operation `op` takes,
/// names on `sheets`, the sheets the operation reaches, as the change set
/// has left them so far, with what it was worked out to, for the reply.
/// Each must name one point, and the same on every sheet: an object of a
/// template is drawn through the same point on every sheet built from it.
/// An ID in a snap names its object on the sheet it gives, which must be
/// one the operation reaches; the object of the same place is looked for
/// on each other sheet.
fn snapped_points(
    at: &Attempt,
    op: &Operation,
    sheets: &[usize],
    snaps: &[&PointSnap],
    settings: &Settings,
) -> Result<Vec<([f64; 3], SnapResolved)>, Error> {
    if snaps.is_empty() {
        return Ok(Vec::new());
    }
    let files = at.files;
    let rows = at.rows_now()?;
    let mut records: Vec<SheetRecord> = Vec::with_capacity(sheets.len());
    for &k in sheets {
        records.push(
            sheet_record(at, &rows, k, &mut Vec::new()).map_err(|e| Error {
                message: format!(
                    "operation {} (`{}`): sheet `{}` cannot be read to work out its snapped \
                     points: {}",
                    op.index,
                    op.spec.name,
                    files.name(k),
                    e.message
                ),
                ..e
            })?,
        );
    }
    let names: Vec<&str> = sheets.iter().map(|&k| files.name(k)).collect();
    let mut out = Vec::with_capacity(snaps.len());
    for snap in snaps {
        let field = format!("{}.snap", snap.field);
        let mut first: Option<(usize, [f64; 3], SnapResolved)> = None;
        for (s, record) in records.iter().enumerate() {
            let on = localized(&snap.snap, names[s], &names, &field).map_err(|(message, at)| {
                op_error(op.index, op.spec.name, ErrorCode::BadValue, message, &at)
            })?;
            let (object, point) = crate::query::snap::one_point(&[record], &on, settings, &field)
                .map_err(|e| {
                let at = e.field.clone().unwrap_or_default();
                op_error(op.index, op.spec.name, e.code, e.message, &at)
            })?;
            let xyz = [point.at.x, point.at.y, point.at.z];
            match &first {
                None => {
                    first = Some((
                        s,
                        xyz,
                        SnapResolved {
                            field: snap.field.clone(),
                            id: object.id,
                            snap: point.snap,
                            which: on.which.clone().or(point.which),
                            at: point.at,
                        },
                    ));
                }
                Some((f, seen, _)) if *seen != xyz => {
                    return Err(op_error(
                        op.index,
                        op.spec.name,
                        ErrorCode::BadValue,
                        format!(
                            "the snapped point is [{}, {}, {}] on sheet `{}` and [{}, {}, {}] on \
                             sheet `{}`; the operation reaches every sheet built from its \
                             template, where an object is drawn through the same point on \
                             each, so give its coordinates, or snap to an object whose place \
                             the rows do not set",
                            seen[0], seen[1], seen[2], names[*f], xyz[0], xyz[1], xyz[2], names[s]
                        ),
                        &snap.field,
                    ));
                }
                Some(_) => {}
            }
        }
        let (_, xyz, mut resolved) = first.expect("an operation reaches a sheet");
        // The object as the snap names it: on the sheet its ID gives.
        if let Some((sheet, _)) = snap
            .snap
            .id
            .as_deref()
            .and_then(|id| id.split_once(ID_SEPARATOR))
        {
            if let Some((_, local)) = resolved.id.split_once(ID_SEPARATOR) {
                if let Some(named) = names.iter().find(|n| n.eq_ignore_ascii_case(sheet)) {
                    resolved.id = format!("{named}{ID_SEPARATOR}{local}");
                }
            }
        }
        out.push((xyz, resolved));
    }
    Ok(out)
}

/// `snap` as it names its objects on sheet `sheet`: an ID that gives one
/// of `reached`, the sheets the operation reaches, names the object of
/// the same ID on `sheet`; or the message and field of an ID on a sheet the
/// operation does not reach. `field` is the snap's place in the operation.
fn localized(
    snap: &SnapRef,
    sheet: &str,
    reached: &[&str],
    field: &str,
) -> Result<SnapRef, (String, String)> {
    let local = |id: &Option<String>, at: &str| -> Result<Option<String>, (String, String)> {
        let Some(id) = id else {
            return Ok(None);
        };
        let Some((on, rest)) = id.split_once(ID_SEPARATOR) else {
            return Ok(Some(id.clone()));
        };
        if !reached.iter().any(|r| r.eq_ignore_ascii_case(on)) {
            return Err((
                format!(
                    "the snapped point's object `{id}` is on sheet `{on}`, which the operation \
                     does not reach; a snapped point is on the sheet it is drawn on: {}",
                    reached
                        .iter()
                        .map(|r| format!("`{r}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                format!("{at}.id"),
            ));
        }
        Ok(Some(format!("{sheet}{ID_SEPARATOR}{rest}")))
    };
    let mut out = snap.clone();
    out.id = local(&snap.id, field)?;
    if let Some(with) = &snap.with {
        let mut with = with.clone();
        with.id = local(&with.id, &format!("{field}.with"))?;
        out.with = Some(with);
    }
    if let Some(from) = &snap.from {
        out.from = Some(Box::new(localized(
            from,
            sheet,
            reached,
            &format!("{field}.from"),
        )?));
    }
    Ok(out)
}

/// A point as JSON, as a record writes it: a whole number as one.
fn coordinates(point: [f64; 3]) -> serde_json::Value {
    serde_json::Value::Array(
        point
            .iter()
            .map(|&c| {
                let c = c + 0.0;
                if c.fract() == 0.0 && c.abs() < 1e15 {
                    serde_json::Value::from(c as i64)
                } else {
                    serde_json::Number::from_f64(c)
                        .map_or(serde_json::Value::Null, serde_json::Value::Number)
                }
            })
            .collect(),
    )
}

/// The snapped points an operation took, in words, for its message:
/// "; `from` is the endpoint `objects[0].to` of `S/strip`, [50, 52, 0]".
fn snapped_words(snapped: &[SnapResolved]) -> String {
    snapped
        .iter()
        .map(|s| {
            format!(
                "; `{}` is the {}{} of `{}`, [{}, {}, {}]",
                s.field,
                s.snap.word(),
                s.which
                    .as_ref()
                    .map(|w| format!(" `{w}`"))
                    .unwrap_or_default(),
                s.id,
                s.at.x,
                s.at.y,
                s.at.z
            )
        })
        .collect()
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
    let in_rows = error.file.as_deref().map(Path::new) == Some(files.rows_path(k));
    let in_template =
        error.file.as_deref().map(Path::new) == Some(files.templates[t].path.as_path());
    let field = error.field.clone().unwrap_or_default();
    // The operation, the place within its object, and the place the
    // operation set that covers it, when one does.
    let mut cause: Option<(usize, String, Option<Matched>)> = None;
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
                .and_then(|(_, name, i)| within(name).map(|within| (*i, within, None)));
        }
    } else if in_rows {
        let row = &files.rows[k];
        if let Some((list @ ObjectList::Row(_), index, rest)) = split_place(&field) {
            if let Some((i, matched)) = at.rows[row.file_index].blame_at(list, index, rest) {
                cause = Some((i, rest.to_owned(), matched));
            }
        } else if let Some(value) = field.strip_prefix(&format!("rows[{}].values.", row.index)) {
            let blamed = blame_row_values(at, k, value, &error);
            match blamed.as_slice() {
                [] => {}
                [i] => cause = Some((*i, String::new(), None)),
                [first, ..] => {
                    // More than one operation set the values the text
                    // joins, and none alone made it fail: each is named.
                    let each: Vec<String> = blamed
                        .iter()
                        .map(|&i| format!("{i} (`{}`)", ops[i].spec.name))
                        .collect();
                    return Error {
                        message: format!(
                            "operations {} on sheet `{}`, each setting a row value the text \
                             joins: {}",
                            join_and(&each),
                            files.name(k),
                            error.message
                        ),
                        field: Some(format!("operations[{first}]")),
                        line: None,
                        column: None,
                        file: None,
                        ..error
                    };
                }
            }
        }
    } else if in_template {
        if let Some((ObjectList::Template, index, rest)) = split_place(&field) {
            if let Some(Some(Ok(template))) = at.templates.get(t) {
                if let Some((i, matched)) = template.blame_at(ObjectList::Template, index, rest) {
                    cause = Some((i, rest.to_owned(), matched));
                }
            }
        }
    }
    if let Some((i, rest, matched)) = cause {
        let op = &ops[i];
        let suffix = op_field(op, &rest, error.code, matched.as_ref());
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

/// The operations to blame for `error`, about sheet `k`'s row value
/// `value`, in order. A value a template joins in a list of parts with
/// others may fail for another's text, so every operation that set one of
/// those values is tried: the sheet is read with that operation's values
/// alone, the others as they were, and an operation whose values alone
/// fail so is to blame. When exactly one is, it is named; otherwise each
/// that set one is. A value no list joins is blamed on the operation that
/// set it.
fn blame_row_values(at: &Attempt, k: usize, value: &str, error: &Error) -> Vec<usize> {
    let files = at.files;
    let t = files.template_of[k];
    let set_by = |name: &str| {
        at.values
            .iter()
            .find(|(r, n, _)| *r == k && n == name)
            .map(|(_, _, i)| *i)
    };
    // The template as the change set has it, or as it was when the change
    // set has not read it.
    let read;
    let template = match &at.templates[t] {
        Some(Ok(template)) => template,
        _ => match files.templates[t]
            .text
            .clone()
            .and_then(|text| RecordFile::from_text(&files.templates[t].path, text))
        {
            Ok(found) => {
                read = found;
                &read
            }
            Err(_) => return set_by(value).into_iter().collect(),
        },
    };
    // How many times each block the template's inserts place carries a tag,
    // as the library stands.
    let library = files.library_of[t].and_then(|l| match &at.libraries[l] {
        Some(Ok(library)) => Some(library),
        _ => None,
    });
    let carried: Vec<(String, Vec<String>)> = template
        .inserts(ObjectList::Template)
        .into_iter()
        .filter_map(|insert| {
            let tags = library?.block_carried_tags(&insert.block)?;
            Some((insert.block, tags))
        })
        .collect();
    let carries = |block: &str, tag: &str| {
        carried
            .iter()
            .find(|(b, _)| b == block)
            .map(|(_, tags)| tags.iter().filter(|t| *t == tag).count())
    };
    let beside = template.parts_beside(value, &carries);
    let mut contributed: Vec<usize> = beside.iter().filter_map(|name| set_by(name)).collect();
    contributed.sort_unstable();
    contributed.dedup();
    if contributed.len() < 2 {
        return set_by(value)
            .or(contributed.first().copied())
            .into_iter()
            .collect();
    }
    let rows = match at.rows_now() {
        Ok(rows) => rows,
        Err(_) => return contributed,
    };
    let alone: Vec<usize> = contributed
        .iter()
        .copied()
        .filter(|&i| {
            let names: Vec<String> = at
                .values
                .iter()
                .filter(|(r, _, by)| *r == k && *by == i)
                .map(|(_, n, _)| n.clone())
                .collect();
            let mut tried = files.rows.clone();
            tried[k] = files.rows[k].with_values_of(&rows[k], &names);
            sheet_record(at, &tried, k, &mut Vec::new()).is_err_and(|e| e.code == error.code)
        })
        .collect();
    if alone.len() == 1 {
        alone
    } else {
        contributed
    }
}

/// Whether operation `op` reaches sheet `k`, as far as can be told without
/// applying it: an add names its sheet or template, and an edit its
/// target's sheet.
fn at_reaches(op: &Operation, k: usize, at: &Attempt) -> bool {
    let files = at.files;
    let reaches = |target: &Target| match target {
        Target::Id { sheet, .. }
        | Target::Name {
            sheet: Some(sheet), ..
        } => files
            .sheet(sheet)
            .is_some_and(|s| s == k || files.template_of[s] == files.template_of[k]),
        Target::Name { sheet: None, .. } => true,
    };
    match &op.body {
        Body::Add {
            to: AddTo::Sheet(sheet),
            ..
        }
        | Body::TextToFront {
            to: AddTo::Sheet(sheet),
            ..
        }
        | Body::Stretch {
            to: AddTo::Sheet(sheet),
            ..
        }
        | Body::AddRevisionCloud {
            to: Some(AddTo::Sheet(sheet)),
            ..
        } => files.sheet(sheet) == Some(k),
        Body::Add {
            to: AddTo::Template(written),
            ..
        }
        | Body::TextToFront {
            to: AddTo::Template(written),
            ..
        }
        | Body::Stretch {
            to: AddTo::Template(written),
            ..
        }
        | Body::AddRevisionCloud {
            to: Some(AddTo::Template(written)),
            ..
        }
        | Body::Table {
            to: AddTo::Template(written),
            ..
        }
        | Body::SetPageSetup {
            to: AddTo::Template(written),
            ..
        } => files.template(written) == Some(files.template_of[k]),
        Body::AddRevisionCloud {
            to: None, around, ..
        } => around.iter().any(|(t, _)| reaches(t)),
        Body::Table {
            to: AddTo::Sheet(sheet),
            ..
        }
        | Body::SetPageSetup {
            to: AddTo::Sheet(sheet),
            ..
        } => files
            .sheet(sheet)
            .is_some_and(|s| files.template_of[s] == files.template_of[k]),
        Body::Change { target, .. }
        | Body::Move { target, .. }
        | Body::Delete { target }
        | Body::SetAttribute { target, .. }
        | Body::SetBlockState { target, .. }
        | Body::SetField { target, .. }
        | Body::Offset { target, .. }
        | Body::Trim { target, .. }
        | Body::Extend { target, .. }
        | Body::Break { target, .. }
        | Body::Fillet {
            mode: CornerMode::Polyline { target },
            ..
        }
        | Body::Chamfer {
            mode: CornerMode::Polyline { target },
            ..
        }
        | Body::TableEdit { target, .. }
        | Body::LeaderLine { target, .. }
        | Body::DimensionFrom { target, .. } => reaches(target),
        Body::Fillet {
            mode: CornerMode::TwoObjects { first, second, .. },
            ..
        }
        | Body::Chamfer {
            mode: CornerMode::TwoObjects { first, second, .. },
            ..
        } => reaches(first) || reaches(second),
        Body::Copy { targets, .. }
        | Body::Transform { targets, .. }
        | Body::DrawOrder { targets, .. }
        | Body::Join { targets, .. }
        | Body::Explode { targets, .. } => targets.iter().any(|(t, _)| reaches(t)),
        Body::Block { library, .. } => {
            files.library(library.as_deref()).ok() == files.library_of[files.template_of[k]]
        }
        Body::ReplaceText { sheets, .. } | Body::SetAttributes { sheets, .. } => match sheets {
            None => true,
            Some(names) => names.iter().any(|s| files.sheet(s) == Some(k)),
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
        files.units,
        &mut Vec::new(),
    )
    .and_then(|(record, _)| {
        let dwg = files.dwg(k);
        writable(&dwg)?;
        prepare_sheet(
            &record,
            &template.path,
            &dwg,
            &files.mark(k),
            true,
            settings,
        )
        .map(|_| ())
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
/// it edits, such as `line.to[1]`, is about. `matched` is the place within
/// the object the operation set that covers it, when one does.
fn op_field(op: &Operation, rest: &str, code: ErrorCode, matched: Option<&Matched>) -> String {
    let within = rest.split_once('.').map_or("", |(_, within)| within);
    match &op.body {
        Body::Add { .. } | Body::AddRevisionCloud { .. } => within.to_owned(),
        Body::Table {
            op: TableOp::Change,
            keys,
            ..
        }
        | Body::SetPageSetup { set: keys, .. } => {
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
                // An error in a row's value is in the text the change gave it.
                "set.contents".to_owned()
            } else if rest.is_empty() && set.contains_key("content") {
                "set.content.text".to_owned()
            } else if rest.is_empty() && set.contains_key("annotation") {
                "set.annotation.text".to_owned()
            } else {
                "set".to_owned()
            }
        }
        Body::Move {
            motion: Motion::By(_),
            ..
        } => "by".to_owned(),
        Body::Move { .. } => "to".to_owned(),
        Body::Delete { .. } => String::new(),
        Body::TableEdit { edit, .. } => grid_field(edit, within, matched),
        Body::LeaderLine { edit, .. } => match edit {
            LeaderLineEdit::Add { .. } => "points".to_owned(),
            LeaderLineEdit::Delete { .. } => "leader".to_owned(),
        },
        // What is wrong with the dimension it adds comes of its point.
        Body::DimensionFrom { .. } => "point".to_owned(),
        // What is wrong with a copy comes of the operation as a whole.
        Body::Copy { .. } | Body::Transform { .. } => String::new(),
        Body::DrawOrder { .. } => "order".to_owned(),
        Body::TextToFront { .. } => "which".to_owned(),
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
        Body::SetBlockState { .. } => {
            if rest.starts_with("state.") {
                rest.to_owned()
            } else {
                "state".to_owned()
            }
        }
        Body::SetField { .. } => {
            if rest.starts_with("field") || rest.starts_with("attribute_fields") {
                "field".to_owned()
            } else if code == ErrorCode::UnknownName {
                "tag".to_owned()
            } else {
                "field".to_owned()
            }
        }
        Body::ReplaceText { .. } => "replace".to_owned(),
        Body::SetAttributes { .. } => {
            if code == ErrorCode::UnknownName {
                "tag".to_owned()
            } else {
                "value".to_owned()
            }
        }
        Body::Offset { .. } => "distance".to_owned(),
        Body::Trim { .. } => "boundary".to_owned(),
        Body::Extend { .. } => "boundary".to_owned(),
        Body::Fillet { .. } => "radius".to_owned(),
        Body::Chamfer { .. } => "distance".to_owned(),
        Body::Break { .. } => "at".to_owned(),
        Body::Join { .. } => "targets".to_owned(),
        Body::Stretch { .. } => "box".to_owned(),
        Body::Explode { .. } => "target".to_owned(),
    }
}

/// The field within a table operation that `within`, a place within its
/// table such as `cells[2][1].merge[0]` or `rows[3]`, is about. `matched` is
/// the place the operation set that covers it, which follows its cell when
/// a later operation moved the cell; an empty `within` is a value of the
/// row the operation set.
fn grid_field(edit: &TableEdit, within: &str, matched: Option<&Matched>) -> String {
    /// `cells[R][C]` and what follows, or `None`.
    fn cell_of(within: &str) -> Option<(usize, usize, &str)> {
        let rest = within.strip_prefix("cells[")?;
        let (row, rest) = rest.split_once(']')?;
        let rest = rest.strip_prefix('[')?;
        let (column, rest) = rest.split_once(']')?;
        Some((row.parse().ok()?, column.parse().ok()?, rest))
    }
    match edit {
        TableEdit::SetCell { cell, set } => {
            // Where the cell is now.
            let now = matched
                .and_then(|m| cell_of(&m.path))
                .map_or(*cell, |(r, c, _)| [r, c]);
            match cell_of(within) {
                Some((r, c, rest)) if [r, c] == now => {
                    let key = rest
                        .strip_prefix('.')
                        .and_then(|k| k.split(['.', '[']).next())
                        .unwrap_or("");
                    if set.contains_key(key) {
                        format!("set{rest}")
                    } else if key.is_empty() && set.contains_key("text") {
                        // A cell written as its text alone, or as a value
                        // from the row, is its text.
                        "set.text".to_owned()
                    } else {
                        "set".to_owned()
                    }
                }
                None if within.is_empty() && set.contains_key("text") => "set.text".to_owned(),
                _ if set.contains_key("merge") => "set.merge".to_owned(),
                _ => "set".to_owned(),
            }
        }
        TableEdit::Add { axis, size, .. } => {
            if within.starts_with(axis.sizes_key()) {
                return if size.is_some() {
                    axis.size_key().to_owned()
                } else {
                    String::new()
                };
            }
            // A cell the operation gave, by its key in the operation, which
            // stays put however later edits move the cell.
            match (cell_of(within), matched.and_then(|m| m.given.as_deref())) {
                (Some((_, _, rest)), Some(given)) => format!("{given}{rest}"),
                _ => String::new(),
            }
        }
        TableEdit::Delete { axis, .. } => axis.word().to_owned(),
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
        | Body::SetAttribute { target, .. }
        | Body::SetBlockState { target, .. }
        | Body::SetField { target, .. }
        | Body::Offset { target, .. }
        | Body::Trim { target, .. }
        | Body::Extend { target, .. }
        | Body::Break { target, .. }
        | Body::Fillet {
            mode: CornerMode::Polyline { target },
            ..
        }
        | Body::Chamfer {
            mode: CornerMode::Polyline { target },
            ..
        }
        | Body::TableEdit { target, .. }
        | Body::LeaderLine { target, .. }
        | Body::DimensionFrom { target, .. } => target.words(),
        Body::Fillet {
            mode: CornerMode::TwoObjects { first, second, .. },
            ..
        }
        | Body::Chamfer {
            mode: CornerMode::TwoObjects { first, second, .. },
            ..
        } => format!("{} and {}", first.words(), second.words()),
        Body::Copy { targets, .. }
        | Body::Transform { targets, .. }
        | Body::DrawOrder { targets, .. }
        | Body::Join { targets, .. }
        | Body::Explode { targets, .. } => match targets.as_slice() {
            [(one, _)] => one.words(),
            [(first, _), rest @ ..] => format!("{} and {} more", first.words(), rest.len()),
            [] => String::new(),
        },
        Body::Add {
            to: AddTo::Sheet(s),
            ..
        }
        | Body::TextToFront {
            to: AddTo::Sheet(s),
            ..
        }
        | Body::Stretch {
            to: AddTo::Sheet(s),
            ..
        }
        | Body::AddRevisionCloud {
            to: Some(AddTo::Sheet(s)),
            ..
        } => format!("to sheet `{s}`"),
        Body::ReplaceText { find, .. } => format!("`{find}`"),
        Body::SetAttributes { tag, .. } => format!("`{tag}` on matching inserts"),
        Body::Add {
            to: AddTo::Template(t),
            ..
        }
        | Body::TextToFront {
            to: AddTo::Template(t),
            ..
        }
        | Body::Stretch {
            to: AddTo::Template(t),
            ..
        }
        | Body::AddRevisionCloud {
            to: Some(AddTo::Template(t)),
            ..
        } => format!("to the template `{t}`"),
        Body::AddRevisionCloud {
            to: None, around, ..
        } => match around.as_slice() {
            [(one, _)] => format!("around {}", one.words()),
            [(first, _), rest @ ..] => {
                format!("around {} and {} more", first.words(), rest.len())
            }
            [] => String::new(),
        },
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
        Body::SetPageSetup {
            layout: Some(layout),
            to: AddTo::Template(t),
            ..
        } => format!("layout `{layout}`'s page setup in template `{t}`"),
        Body::SetPageSetup {
            layout: Some(layout),
            to: AddTo::Sheet(s),
            ..
        } => format!("layout `{layout}`'s page setup in sheet `{s}`'s template"),
        Body::SetPageSetup {
            layout: None,
            to: AddTo::Template(t),
            ..
        } => format!("the page setup of template `{t}`"),
        Body::SetPageSetup {
            layout: None,
            to: AddTo::Sheet(s),
            ..
        } => format!("the page setup of sheet `{s}`'s template"),
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

/// One step of a guide, as the walk takes it (#157): its points and its
/// operation as the guide gives them, `made_by` and all, or the layer a
/// setting step makes current.
pub(super) struct WalkStep<'a> {
    pub points: &'a [super::guides::GuidePoint],
    pub operation: Option<&'a Value>,
    pub layer: Option<&'a str>,
}

/// What the walk found of one step.
#[derive(Clone, Debug, Default)]
pub(super) struct StepWalked {
    /// Each point, in order.
    pub points: Vec<PointWalked>,
    /// The step's operation with each `made_by` named by its object's ID,
    /// once every step it names has made its object.
    pub operation: Option<Value>,
    /// The ID of the object its operation adds.
    pub made: Option<String>,
    /// Its operation's name, as the core names it.
    pub op_name: Option<&'static str>,
    /// Its operation as it applied, each snapped point written as the
    /// point it was worked out to.
    pub resolved: Option<Value>,
    /// The objects its operation moves, copies or deletes, by ID on the
    /// guide's sheet.
    pub targets: Vec<String>,
    /// The layer a setting step makes current, as the sheet names it.
    pub layer: Option<String>,
}

/// What the walk found of one point of a step.
#[derive(Clone, Debug, Default)]
pub(super) struct PointWalked {
    /// Where it is, in the set's declared unit.
    pub at: Option<[f64; 3]>,
    /// The snap it names, worked out.
    pub snapped: Option<SnapResolved>,
    /// The object it picks, by ID.
    pub object: Option<String>,
    /// The step whose object it waits on, while that step has made none.
    pub waits_on: Option<u64>,
    /// Why it names no point now, such as an object since deleted.
    pub error: Option<String>,
}

/// Walks a guide's steps on sheet `sheet` of the drawing set in `set`.
///
/// With `made` none, the guide is checked as it is given: each step's
/// points are worked out on the sheet as the steps before it leave it, and
/// its operation is applied in memory, as a dry run applies one, so a later
/// step may name what an earlier one adds (`made_by`). Any point that names
/// nothing, and any operation the set would refuse, is an error at its key
/// under `guide.steps`, and nothing is written.
///
/// With `made` given, the object each step's change set made on the set as
/// it stands, the steps are worked out on the set as it is now, with no
/// operation applied: a point that waits on a step that has made nothing
/// says so, and one that names nothing now says why.
pub(super) fn walk_guide(
    set: &Path,
    sheet: &str,
    steps: &[WalkStep],
    made: Option<&BTreeMap<u64, String>>,
    settings: &Settings,
) -> Result<(String, Vec<StepWalked>), Error> {
    let files = SetFiles::load(set)?;
    let Some(k) = files.sheet(sheet) else {
        return Err(Error::new(
            ErrorCode::UnknownName,
            format!(
                "the set holds no sheet `{sheet}`; its sheets: {}",
                files.sheet_names()
            ),
        )
        .at_field("guide.sheet"));
    };
    let name = files.name(k).to_owned();
    let checking = made.is_none();
    let mut made_now: BTreeMap<u64, String> = made.cloned().unwrap_or_default();
    let mut at = Attempt::new(&files)?;
    let mut walked = Vec::with_capacity(steps.len());
    for (s, step) in steps.iter().enumerate() {
        let here = format!("guide.steps[{s}]");
        let rows = at.rows_now()?;
        let record = sheet_record(&at, &rows, k, &mut Vec::new()).map_err(|e| Error {
            message: format!(
                "step {}: sheet `{name}` cannot be read to work out its points: {}",
                s + 1,
                e.message
            ),
            ..e
        })?;
        let mut out = StepWalked::default();
        if let Some(layer) = step.layer {
            out.layer = if layer == "0" {
                Some("0".to_owned())
            } else {
                record
                    .sheet
                    .layers
                    .iter()
                    .find(|l| l.name.eq_ignore_ascii_case(layer))
                    .map(|l| l.name.clone())
            };
            if out.layer.is_none() {
                let names: Vec<&str> = std::iter::once("0")
                    .chain(record.sheet.layers.iter().map(|l| l.name.as_str()))
                    .collect();
                return Err(Error::new(
                    ErrorCode::UnknownName,
                    format!(
                        "step {}: sheet `{name}` has no layer `{layer}`; its layers: {}",
                        s + 1,
                        names.join(", ")
                    ),
                )
                .at_field(format!("{here}.setting.current_layer")));
            }
        }
        for (p, point) in step.points.iter().enumerate() {
            let mut found = PointWalked::default();
            let (given, key) = match (&point.at, &point.object) {
                (Some(given), _) => (given, "at"),
                (None, Some(given)) => (given, "object"),
                (None, None) => unreachable!("a point is read with `at` or `object`"),
            };
            let given = match super::guides::substituted(given, &made_now) {
                Ok(given) => given,
                Err(step) => {
                    found.waits_on = Some(step);
                    out.points.push(found);
                    continue;
                }
            };
            let field = format!("{here}.points[{p}].{key}");
            let worked = if key == "at" {
                guide_point(&record, &given, settings, &field).map(|(xyz, snapped)| {
                    found.at = Some(xyz);
                    found.snapped = snapped;
                })
            } else {
                guide_object(&record, &given, &field).map(|id| found.object = Some(id))
            };
            if let Err(e) = worked {
                if checking {
                    return Err(Error {
                        message: format!("step {}, point {}: {}", s + 1, p + 1, e.message),
                        ..e
                    });
                }
                found.error = Some(e.message);
            }
            out.points.push(found);
        }
        if let Some(op) = step.operation {
            if let Ok(op) = super::guides::substituted(op, &made_now) {
                out.operation = Some(op);
            }
        }
        if let (true, Some(op)) = (checking, &out.operation) {
            let applied = guide_op(&mut at, &files, k, op, s, settings)?;
            out.op_name = Some(applied.name);
            out.made = applied.made;
            out.resolved = Some(applied.resolved);
            out.targets = applied.targets;
            if let Some(id) = &out.made {
                made_now.insert(s as u64 + 1, id.clone());
            }
        }
        walked.push(out);
    }
    Ok((name, walked))
}

/// A guide's point `given`, `[x, y]`, `[x, y, z]` or a snapped point, on
/// `record`: where it is, and the snap it names, worked out.
fn guide_point(
    record: &SheetRecord,
    given: &Value,
    settings: &Settings,
    field: &str,
) -> Result<([f64; 3], Option<SnapResolved>), Error> {
    match given {
        Value::Array(items) if (2..=3).contains(&items.len()) => {
            let mut xyz = [0.0; 3];
            for (i, item) in items.iter().enumerate() {
                xyz[i] = item.as_f64().ok_or_else(|| {
                    Error::new(ErrorCode::WrongType, "expected a number")
                        .at_field(format!("{field}[{i}]"))
                })?;
            }
            Ok((xyz, None))
        }
        Value::Object(map) => {
            let snap = super::operations::read_snap(map, field)
                .map_err(|(code, message, at)| Error::new(code, message).at_field(at))?;
            let (object, point) = crate::query::snap::one_point(
                &[record],
                &snap,
                settings,
                &format!("{field}.snap"),
            )?;
            Ok((
                [point.at.x, point.at.y, point.at.z],
                Some(SnapResolved {
                    field: field.to_owned(),
                    id: object.id,
                    snap: point.snap,
                    which: snap.which.clone().or(point.which),
                    at: point.at,
                }),
            ))
        }
        _ => Err(Error::new(
            ErrorCode::WrongType,
            "expected a point, [x, y] or [x, y, z], or a snapped point, {\"snap\": {...}}",
        )
        .at_field(field)),
    }
}

/// The object a guide's step picks, `given` as an operation's target
/// names one, on `record`, by its ID.
fn guide_object(record: &SheetRecord, given: &Value, field: &str) -> Result<String, Error> {
    let wrong = || {
        Error::new(
            ErrorCode::WrongType,
            "expected an object, {\"id\": ...}, {\"name\": ...} or {\"made_by\": <step>}",
        )
        .at_field(field)
    };
    let Value::Object(map) = given else {
        return Err(wrong());
    };
    let found = match (map.get("id"), map.get("name")) {
        (Some(Value::String(id)), None) => {
            (0..record.objects.len()).find(|&i| record.id(i).eq_ignore_ascii_case(id))
        }
        (None, Some(Value::String(name))) => (0..record.objects.len()).find(|&i| {
            record.objects[i]
                .name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        }),
        _ => return Err(wrong()),
    };
    found.map(|i| record.id(i)).ok_or_else(|| {
        Error::new(
            ErrorCode::UnknownName,
            format!(
                "sheet `{}` has no such object; its IDs: {}",
                record.name,
                (0..record.objects.len().min(LISTED_IDS))
                    .map(|i| record.id(i))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
        .at_field(field)
    })
}

/// The most IDs a guide's error lists.
const LISTED_IDS: usize = 40;

/// What a guide step's operation did, applied in memory.
struct GuideOpDone {
    name: &'static str,
    /// The object it adds, or the one copy it makes, by its ID on the
    /// guide's sheet.
    made: Option<String>,
    /// The operation, each snapped point written as its point.
    resolved: Value,
    /// The objects it moves, copies or deletes, by ID on the guide's sheet.
    targets: Vec<String>,
}

/// Applies a guide step's operation `op` in memory, as a dry run does, on
/// `at`, and says what it did. It must reach sheet `k`.
fn guide_op(
    at: &mut Attempt,
    files: &SetFiles,
    k: usize,
    op: &Value,
    s: usize,
    settings: &Settings,
) -> Result<GuideOpDone, Error> {
    let here = format!("guide.steps[{s}].completes");
    let step = s + 1;
    let moved = |e: Error| -> Error {
        let field = e
            .field
            .as_deref()
            .map(|f| match f.strip_prefix("operations[0]") {
                Some(rest) => format!("{here}{rest}"),
                None => format!("{here}.{f}"),
            });
        let message = match e.message.strip_prefix("operation 0 (") {
            Some(rest) => format!("step {step}'s operation ({rest}"),
            None => format!("step {step}'s operation: {}", e.message),
        };
        Error {
            message,
            field: field.or(Some(here.clone())),
            ..e
        }
    };
    let set = serde_json::json!({ "operations": [op] });
    let change_set = read_change_set(&set, files.units).map_err(moved)?;
    precheck(files, &change_set.operations).map_err(moved)?;
    let operation = &change_set.operations[0];
    let applied = apply_op(at, operation, settings);
    let done = applied.outcome.map_err(moved)?;
    if !applied.sheets.contains(&k) {
        return Err(Error::new(
            ErrorCode::BadValue,
            format!(
                "step {step}'s operation does not reach sheet `{}`, the guide's sheet",
                files.name(k)
            ),
        )
        .at_field(here));
    }
    // The sheet is read as it now stands, as a dry run builds it, so an
    // operation that leaves it unreadable, such as one naming a layer it
    // does not have, is the step's.
    let rows = at.rows_now()?;
    sheet_record(at, &rows, k, &mut Vec::new()).map_err(|e| {
        let key = e
            .field
            .as_deref()
            .and_then(|f| f.rsplit('.').next())
            .filter(|key| op.get(*key).is_some())
            .map(|key| format!("{here}.{key}"));
        Error {
            message: format!("step {step}'s operation: {}", e.message),
            field: key.or(Some(here.clone())),
            ..e
        }
    })?;
    let name = operation.spec.name;
    // An object of a template's, added or copied through another sheet
    // built from it, is on the guide's sheet with the same ID after its
    // name.
    let here = |id: &str| {
        let local = id.split_once(ID_SEPARATOR).map_or(id, |(_, l)| l);
        format!("{}{ID_SEPARATOR}{local}", files.name(k))
    };
    let made = if name.starts_with("add_") {
        done.id.as_deref().map(here)
    } else if let [one] = done.copies.as_slice() {
        Some(here(&one.id))
    } else {
        None
    };
    let mut targets: Vec<String> = if name == "copy" {
        done.copies.iter().map(|c| here(&c.source)).collect()
    } else if name.starts_with("add_") {
        Vec::new()
    } else {
        done.id.as_deref().map(here).into_iter().collect()
    };
    targets.sort_by_key(|t| t.to_lowercase());
    targets.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    Ok(GuideOpDone {
        name,
        made,
        resolved: super::guides::resolved(op, &done.snapped),
        targets,
    })
}
