//! The seam between AgentDrafter and its DWG library.
//!
//! This is the only module that names acadrust (ADR-0009). Everything else in
//! the core works on its own model ([`crate::model`]), so moving the acadrust
//! pin, or the library itself, changes this file and `Cargo.toml` only.
//! Moving the pin means running the judge again.
//!
//! Our own names here follow `CONTEXT.md`: a DWG file's contents are a
//! drawing and the things in it are objects. acadrust's names, such as
//! `CadDocument` and `EntityType`, stay as acadrust spells them.

use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use acadrust::entities::{EntityType, Line as DwgLine};
use acadrust::types::{DxfVersion, Vector3};
use acadrust::{CadDocument, DwgReader, DwgWriter};

use crate::error::{Error, ErrorCode};
use crate::model::{DrawingContents, Line, Object, Point3, Sheet};
use crate::units::Unit;

/// The DWG version a new sheet is written in: the 2018 format, AC1032 (ADR-0006).
const NEW_SHEET_VERSION: DxfVersion = DxfVersion::AC1032;

/// The build mark: a custom drawing property, as AutoCAD's DWGPROPS shows
/// it, written into every DWG AgentDrafter builds. A build replaces an
/// existing DWG only when it carries this property with this value.
const BUILD_MARK: (&str, &str) = ("AgentDrafter", "build");

/// The DWG version a new sheet is written in, as the DWG names it.
pub(crate) fn new_sheet_version() -> String {
    NEW_SHEET_VERSION.to_string()
}

/// Writes a new sheet as the bytes of a DWG in the 2018 format, carrying the
/// build mark.
pub(crate) fn write_new_sheet(sheet: &Sheet) -> Result<Vec<u8>, Error> {
    let mut drawing = CadDocument::with_version(NEW_SHEET_VERSION);
    drawing.header.insertion_units = insertion_units(sheet.units);
    drawing.header.measurement = i16::from(sheet.units.is_metric());
    let (name, value) = BUILD_MARK;
    drawing
        .summary_info
        .custom_properties
        .push((name.to_owned(), value.to_owned()));
    for object in &sheet.objects {
        let dwg_object = match object {
            Object::Line(line) => {
                EntityType::Line(DwgLine::from_points(vector(line.from), vector(line.to)))
            }
        };
        drawing.add_entity(dwg_object).map_err(|e| {
            Error::new(
                ErrorCode::DwgWrite,
                format!("could not add a {} to the drawing: {e}", object.type_name()),
            )
        })?;
    }
    match catch_unwind(AssertUnwindSafe(|| DwgWriter::write_to_vec(&drawing))) {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(e)) => Err(Error::new(
            ErrorCode::DwgWrite,
            format!("the DWG library could not write the drawing: {e}"),
        )),
        Err(panic) => Err(Error::new(
            ErrorCode::DwgWrite,
            format!(
                "the DWG library failed while writing the drawing: {}",
                panic_message(panic)
            ),
        )),
    }
}

/// Reads the model space of the DWG at `path`.
pub(crate) fn read(path: &Path) -> Result<DrawingContents, Error> {
    let drawing = read_drawing(path)?;
    let mut objects = Vec::new();
    let mut objects_not_read = 0;
    for dwg_object in drawing.model_space_entities() {
        match dwg_object {
            EntityType::Line(line) => objects.push(Object::Line(Line {
                from: point(line.start),
                to: point(line.end),
            })),
            _ => objects_not_read += 1,
        }
    }
    Ok(DrawingContents {
        dwg_version: drawing.version.to_string(),
        units: unit(drawing.header.insertion_units),
        objects,
        objects_not_read,
    })
}

/// Whether the DWG at `path` carries the build mark, that is, whether
/// AgentDrafter built it. A file that cannot be read as a DWG is an error.
pub(crate) fn carries_build_mark(path: &Path) -> Result<bool, Error> {
    read_drawing(path).map(|drawing| has_build_mark(&drawing))
}

fn has_build_mark(drawing: &CadDocument) -> bool {
    let (name, value) = BUILD_MARK;
    drawing
        .summary_info
        .custom_properties
        .iter()
        .any(|(n, v)| n == name && v == value)
}

fn read_drawing(path: &Path) -> Result<CadDocument, Error> {
    match catch_unwind(AssertUnwindSafe(|| {
        DwgReader::from_file(path).and_then(|mut reader| reader.read())
    })) {
        Ok(Ok(drawing)) => Ok(drawing),
        Ok(Err(e)) => Err(Error::new(
            ErrorCode::DwgRead,
            format!("the DWG library could not read the drawing: {e}"),
        )
        .in_file(path)),
        Err(panic) => Err(Error::new(
            ErrorCode::DwgRead,
            format!(
                "the DWG library failed while reading the drawing: {}",
                panic_message(panic)
            ),
        )
        .in_file(path)),
    }
}

/// The DWG's `INSUNITS` code for a unit.
fn insertion_units(unit: Unit) -> i16 {
    match unit {
        Unit::In => 1,
        Unit::Ft => 2,
        Unit::Mm => 4,
        Unit::Cm => 5,
        Unit::M => 6,
    }
}

/// The unit for a DWG's `INSUNITS` code, if the core knows it.
fn unit(code: i16) -> Option<Unit> {
    Unit::ALL.into_iter().find(|u| insertion_units(*u) == code)
}

fn vector(p: Point3) -> Vector3 {
    Vector3::new(p.x, p.y, p.z)
}

fn point(v: Vector3) -> Point3 {
    Point3::new(v.x, v.y, v.z)
}

fn panic_message(panic: Box<dyn Any + Send>) -> String {
    if let Some(text) = panic.downcast_ref::<String>() {
        text.clone()
    } else if let Some(text) = panic.downcast_ref::<&str>() {
        (*text).to_owned()
    } else {
        "a panic with no message".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::build::{build, BuildRequest};

    /// An empty folder for one test under the system's temporary folder.
    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("agentdrafter-dwg-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_built_dwg_carries_the_build_mark() {
        let dir = scratch("marked");
        let dwg = dir.join("sheet.dwg");
        let sheet = Sheet {
            units: Unit::Mm,
            objects: Vec::new(),
        };
        fs::write(&dwg, write_new_sheet(&sheet).unwrap()).unwrap();
        assert_eq!(carries_build_mark(&dwg), Ok(true));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dwg_someone_else_drew_is_not_replaced_even_when_asked() {
        let dir = scratch("unmarked");
        let description = dir.join("sheet.yaml");
        fs::write(&description, "units: mm\nobjects: []\n").unwrap();
        let original = dir.join("sheet.dwg");
        let bytes = DwgWriter::write_to_vec(&CadDocument::with_version(NEW_SHEET_VERSION)).unwrap();
        fs::write(&original, &bytes).unwrap();
        assert_eq!(carries_build_mark(&original), Ok(false));

        let error = build(&BuildRequest {
            description,
            output: None,
            overwrite: true,
        })
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::NotABuild, "{error:?}");
        assert!(
            error.message.contains("not built by AgentDrafter"),
            "{error:?}"
        );
        assert_eq!(fs::read(&original).unwrap(), bytes);
        let _ = fs::remove_dir_all(&dir);
    }

    /// `CONTEXT.md` says drawing and object. acadrust's own names, which
    /// say document and entity, are allowed; ours are not.
    #[test]
    fn our_names_in_the_seam_say_drawing_and_object() {
        let source = include_str!("dwg.rs");
        let (code, _tests) = source.split_once("#[cfg(test)]").unwrap();
        let mut ours = code.to_owned();
        for acadrust_name in [
            "CadDocument",
            "EntityType",
            "acadrust::entities",
            "add_entity",
            "model_space_entities",
        ] {
            ours = ours.replace(acadrust_name, "");
        }
        for word in ["document", "entit"] {
            let found: Vec<&str> = ours
                .lines()
                .filter(|line| line.to_lowercase().contains(word))
                .collect();
            assert!(found.is_empty(), "`{word}` in our own names: {found:#?}");
        }
    }
}
