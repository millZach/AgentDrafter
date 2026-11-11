//! A sheet as its record states it, with every object's ID: a sheet
//! description on its own, or a template filled in from a row of a drawing
//! set (`template.rs` reads that one).
//!
//! A sheet's name joins its objects' IDs. In a drawing set it is the row's
//! `sheet`, which also names the sheet's DWG; a sheet description built on
//! its own is named by its file, without the extension, as its DWG is
//! unless a build is told otherwise. Neither holds the ID separator, `/`.

use std::path::{Path, PathBuf};

use super::ids::{object_id, ObjectRecord};
use super::{library_beside, on_its_own_with_records, read_file};
use crate::error::Error;
use crate::model::Sheet;

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
}

impl SheetRecord {
    /// The ID of the sheet's object at `index` in its objects.
    pub(crate) fn id(&self, index: usize) -> String {
        object_id(&self.name, &self.objects[index])
    }

    /// The ID of each of the sheet's objects, in order.
    pub(crate) fn ids(&self) -> Vec<String> {
        (0..self.objects.len()).map(|i| self.id(i)).collect()
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
    })
}
