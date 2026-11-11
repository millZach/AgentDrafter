//! The seam between AgentDrafter and its DWG library.
//!
//! This is the only module that names acadrust (ADR-0009). Everything else in
//! the core works on its own model ([`crate::model`]), so moving the acadrust
//! pin, or the library itself, changes this file and `Cargo.toml` only.
//! Moving the pin means running the judge again.

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

/// The DWG version a new sheet is written in, as the DWG names it.
pub(crate) fn new_sheet_version() -> String {
    NEW_SHEET_VERSION.to_string()
}

/// Writes a new sheet as the bytes of a DWG in the 2018 format.
pub(crate) fn write_new_sheet(sheet: &Sheet) -> Result<Vec<u8>, Error> {
    let mut document = CadDocument::with_version(NEW_SHEET_VERSION);
    document.header.insertion_units = insertion_units(sheet.units);
    document.header.measurement = i16::from(sheet.units.is_metric());
    for object in &sheet.objects {
        let entity = match object {
            Object::Line(line) => {
                EntityType::Line(DwgLine::from_points(vector(line.from), vector(line.to)))
            }
        };
        document.add_entity(entity).map_err(|e| {
            Error::new(
                ErrorCode::DwgWrite,
                format!("could not add a {} to the drawing: {e}", object.type_name()),
            )
        })?;
    }
    match catch_unwind(AssertUnwindSafe(|| DwgWriter::write_to_vec(&document))) {
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
    let document = match catch_unwind(AssertUnwindSafe(|| {
        DwgReader::from_file(path).and_then(|mut reader| reader.read())
    })) {
        Ok(Ok(document)) => document,
        Ok(Err(e)) => {
            return Err(Error::new(
                ErrorCode::DwgRead,
                format!("the DWG library could not read the drawing: {e}"),
            )
            .in_file(path))
        }
        Err(panic) => {
            return Err(Error::new(
                ErrorCode::DwgRead,
                format!(
                    "the DWG library failed while reading the drawing: {}",
                    panic_message(panic)
                ),
            )
            .in_file(path))
        }
    };
    let mut objects = Vec::new();
    let mut objects_not_read = 0;
    for entity in document.model_space_entities() {
        match entity {
            EntityType::Line(line) => objects.push(Object::Line(Line {
                from: point(line.start),
                to: point(line.end),
            })),
            _ => objects_not_read += 1,
        }
    }
    Ok(DrawingContents {
        dwg_version: document.version.to_string(),
        units: unit(document.header.insertion_units),
        objects,
        objects_not_read,
    })
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
