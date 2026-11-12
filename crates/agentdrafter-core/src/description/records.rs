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

use super::ids::{object_id, ObjectRecord};
use super::{
    library_beside, on_its_own_with_records, read_file, read_rows_in, read_set_settings_in,
    read_template_in, template_path, Disk, ReadFiles, Row,
};
use crate::error::Error;
use crate::model::Sheet;
use crate::set::SET_SETTINGS_FILE;
use crate::units::Unit;

/// One sheet as its record states it, with the ID of each of its objects.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SheetRecord {
    /// The sheet's name: its row's `sheet` in a drawing set, or the sheet
    /// description's file name without its extension.
    pub name: String,
    /// The file the sheet's objects are written in: the sheet description,
    /// or the sheet's template.
    pub file: PathBuf,
    pub sheet: Sheet,
    /// Each of the sheet's objects as its file writes it, in the order of
    /// `sheet.objects`.
    pub objects: Vec<ObjectRecord>,
    /// The rows file of the drawing set the sheet is in, which holds its
    /// row's own objects, if it is a sheet of a set.
    pub rows_file: Option<PathBuf>,
}

impl SheetRecord {
    /// The ID of the sheet's object at `index` in its objects.
    pub(crate) fn id(&self, index: usize) -> String {
        object_id(&self.name, &self.objects[index])
    }

    /// The file the sheet's object at `index` is written in: the rows file
    /// for one of its row's own objects, or else the sheet description or
    /// template.
    pub(crate) fn file_of(&self, index: usize) -> &Path {
        match &self.rows_file {
            Some(rows) if self.objects[index].in_row => rows,
            _ => &self.file,
        }
    }
}

/// A drawing set's settings and rows as they are saved, ready to read its
/// sheets from their records: what a query and the checker read.
pub(crate) struct SavedSet {
    folder: PathBuf,
    /// The set's declared unit.
    pub units: Unit,
    rows_file: PathBuf,
    /// The set's rows, in its order.
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
        let rows_file = folder.join(&settings.rows);
        let rows = read_rows_in(files.as_ref(), &rows_file)?;
        Ok(SavedSet {
            folder: folder.to_owned(),
            units: settings.units,
            rows_file,
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
            &self.rows_file,
            self.units,
            &mut Vec::new(),
        )
    }
}

/// Reads the sheet description in the file at `path`, built on its own,
/// with the block library it names, as `read_sheet_description` does, and
/// the record of each of its objects. The sheet is named by the file.
pub(crate) fn read_sheet_record(path: &Path) -> Result<SheetRecord, Error> {
    let text = read_file(path, "the sheet description")?;
    let mut library = None;
    let (sheet, objects) = on_its_own_with_records(&text, &mut library_beside(path, &mut library))
        .map_err(|e| e.or_in_file(path))?;
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(SheetRecord {
        name,
        file: path.to_owned(),
        sheet,
        objects,
        rows_file: None,
    })
}
