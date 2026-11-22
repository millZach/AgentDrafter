//! A sheet as its record states it, with every object's ID: a sheet
//! description on its own, or a template filled in from a row of a drawing
//! set (`template.rs` reads that one).
//!
//! A sheet's name joins its objects' IDs. In a drawing set it is the row's
//! `sheet`, which also names the sheet's DWG; a sheet description built on
//! its own is named by its file, without the extension, as its DWG is
//! unless a build is told otherwise. Neither holds the ID separator, `/`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::blocks::Noticed;
use super::ids::{object_id, ObjectRecord};
use super::written::{sheet_sources, SheetSources};
use super::{
    library_beside, on_its_own_with_records, read_file, read_file_in, read_set_rows_in,
    read_set_settings_in, read_template_in, template_in, template_path, Disk, ReadFiles, Row,
};
use crate::error::Error;
use crate::model::Sheet;
use crate::set::{Notice, SET_SETTINGS_FILE};
use crate::units::Unit;

/// Where a sheet read from a DWG came from: its objects' resolved IDs,
/// whether each ID was carried in AgentDrafter's extended data, and the
/// handles of its sheet and block objects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DwgOrigin {
    /// The resolved unique ID of each of `sheet.objects`, in the same order:
    /// the carried AgentDrafter ID when unique, else `{sheet}/handle:{handle}`.
    pub ids: Vec<String>,
    /// Whether each of `sheet.objects` carried its ID in AgentDrafter's
    /// extended data (rather than taking its handle ID).
    pub carried: Vec<bool>,
    /// The handle of each of `sheet.objects`, in hexadecimal as AutoCAD
    /// lists it.
    pub handles: Vec<String>,
    /// For each block in `sheet.blocks`, the handle of each of its objects,
    /// in hexadecimal.
    pub block_handles: Vec<Vec<String>>,
}

/// One sheet as its record states it, with the ID of each of its objects.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SheetRecord {
    /// The sheet's name: its row's `sheet` in a drawing set, or the sheet
    /// description's or DWG's file name without its extension.
    pub name: String,
    /// The file the sheet's objects are written in: the sheet description,
    /// the sheet's template, or the DWG.
    pub file: PathBuf,
    pub sheet: Sheet,
    /// Each of the sheet's objects as its file writes it, in the order of
    /// `sheet.objects`.
    pub objects: Vec<ObjectRecord>,
    /// The rows file the sheet's row is in, which holds the row's own
    /// objects, if it is a sheet of a set.
    pub rows_file: Option<PathBuf>,
    /// Present when this sheet was read from a DWG rather than a sheet
    /// description or drawing set.
    pub dwg: Option<DwgOrigin>,
}

impl SheetRecord {
    /// Whether this record was read from a DWG.
    pub(crate) fn is_dwg(&self) -> bool {
        self.dwg.is_some()
    }

    /// The ID of the sheet's object at `index` in its objects.
    pub(crate) fn id(&self, index: usize) -> String {
        if let Some(dwg) = &self.dwg {
            return dwg.ids[index].clone();
        }
        object_id(&self.name, &self.objects[index])
    }

    /// Whether the record writes an explicit AgentDrafter ID for the object
    /// at `index`.
    pub(crate) fn id_written(&self, index: usize) -> bool {
        if let Some(dwg) = &self.dwg {
            return dwg.carried.get(index).copied().unwrap_or(false);
        }
        self.objects[index].written()
    }

    /// The DWG handle of the sheet's object at `index`, when read from a DWG.
    pub(crate) fn handle_of(&self, index: usize) -> Option<&str> {
        self.dwg
            .as_ref()
            .and_then(|dwg| dwg.handles.get(index).map(String::as_str))
    }

    /// The DWG handle of object `object_index` in block `block_index`, when
    /// read from a DWG.
    pub(crate) fn block_handle_of(&self, block_index: usize, object_index: usize) -> Option<&str> {
        self.dwg.as_ref().and_then(|dwg| {
            dwg.block_handles
                .get(block_index)
                .and_then(|hs| hs.get(object_index))
                .map(String::as_str)
        })
    }

    /// The file the sheet's object at `index` is written in: the rows file
    /// for one of its row's own objects, or else the sheet description,
    /// template or DWG.
    pub(crate) fn file_of(&self, index: usize) -> &Path {
        match &self.rows_file {
            Some(rows) if self.objects[index].in_row => rows,
            _ => &self.file,
        }
    }
}

/// The notices of what reading sheet `sheet` noticed, whose objects are
/// written in `file`, and whose template is `template` as its row writes
/// it, or for a sheet description built on its own, its path. Each is
/// placed in the file it names, or `file` when it names none.
pub(crate) fn notices_of(
    noticed: Vec<Noticed>,
    sheet: &str,
    template: &str,
    file: &Path,
) -> impl Iterator<Item = Notice> {
    let sheet = sheet.to_owned();
    let template = template.to_owned();
    let file = file.to_owned();
    noticed.into_iter().map(move |n| Notice {
        code: n.code,
        message: n.message,
        sheet: sheet.clone(),
        template: template.clone(),
        field: n.field,
        line: n.at.line,
        column: n.at.column,
        file: n.file.as_deref().unwrap_or(&file).display().to_string(),
    })
}

/// A drawing set's settings and rows as they are saved, ready to read its
/// sheets from their records: what a query and the checker read.
pub(crate) struct SavedSet {
    folder: PathBuf,
    /// The set's declared unit.
    pub units: Unit,
    /// The set's rows, in its order, each with its rows file.
    pub rows: Vec<Row>,
    /// Where its files are read from.
    files: Arc<dyn ReadFiles + Send + Sync>,
}

impl SavedSet {
    /// Reads the settings and the rows of the drawing set whose folder is
    /// `folder`.
    pub(crate) fn read(folder: &Path) -> Result<SavedSet, Error> {
        Self::read_from(folder, Arc::new(Disk))
    }

    /// Reads the settings and the rows of the drawing set whose folder is
    /// `folder` from `files`, and its sheets from them too: the set as it
    /// was at an entry of its journal, say.
    pub(crate) fn read_from(
        folder: &Path,
        files: Arc<dyn ReadFiles + Send + Sync>,
    ) -> Result<SavedSet, Error> {
        let settings = read_set_settings_in(files.as_ref(), &folder.join(SET_SETTINGS_FILE))?;
        let rows = read_set_rows_in(files.as_ref(), folder, &settings)?;
        Ok(SavedSet {
            folder: folder.to_owned(),
            units: settings.units,
            rows,
            files,
        })
    }

    /// The set's sheets, by name, in its order.
    pub(crate) fn names(&self) -> Vec<&str> {
        self.rows.iter().map(|row| row.sheet.as_str()).collect()
    }

    /// Reads the sheet `row` names from its template, filled in from the
    /// row, with the block library the template names.
    pub(crate) fn sheet(&self, row: &Row) -> Result<SheetRecord, Error> {
        let template = template_path(&self.folder, row);
        read_template_in(
            self.files.as_ref(),
            &template,
            row,
            self.units,
            &mut Vec::new(),
        )
    }

    /// Reads the sheet `row` names as [`SavedSet::sheet`] does, and where
    /// each of its values is written: by each object itself, by the row,
    /// or nowhere. Both come from one read of the template, so a change
    /// that lands between two reads cannot pair one sheet's objects with
    /// another's sources.
    pub(crate) fn sheet_with_sources(
        &self,
        row: &Row,
    ) -> Result<(SheetRecord, SheetSources), Error> {
        let template = template_path(&self.folder, row);
        let text = read_file_in(self.files.as_ref(), &template, "the template")?;
        let (record, _) = template_in(
            self.files.as_ref(),
            &text,
            &template,
            row,
            self.units,
            &mut Vec::new(),
        )?;
        let sources = sheet_sources(&text, row, &record.objects, &record.sheet.blocks);
        Ok((record, sources))
    }
}

/// Reads the sheet description in the file at `path`, built on its own,
/// with the block library it names, as `read_sheet_description` does, and
/// the record of each of its objects. The sheet is named by the file.
pub(crate) fn read_sheet_record(path: &Path) -> Result<SheetRecord, Error> {
    read_sheet_record_noting(path, &mut Vec::new())
}

/// Reads the sheet description in the file at `path` as
/// [`read_sheet_record`] does, adding what reading it and its block library
/// noticed to `notices`, such as an attribute tag written in lowercase.
pub(crate) fn read_sheet_record_noting(
    path: &Path,
    notices: &mut Vec<Notice>,
) -> Result<SheetRecord, Error> {
    let text = read_file(path, "the sheet description")?;
    let mut library = None;
    let mut noticed = Vec::new();
    let (sheet, objects) =
        on_its_own_with_records(&text, &mut library_beside(path, &mut library), &mut noticed)
            .map_err(|e| e.or_in_file(path))?;
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let written = path.display().to_string();
    let from_library = library.map(|l| l.noticed).unwrap_or_default();
    notices.extend(notices_of(from_library, &name, &written, path));
    notices.extend(notices_of(noticed, &name, &written, path));
    Ok(SheetRecord {
        name,
        file: path.to_owned(),
        sheet,
        objects,
        rows_file: None,
        dwg: None,
    })
}
