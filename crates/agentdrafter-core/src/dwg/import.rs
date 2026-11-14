//! Reads a drawing's blocks, from a DWG or a DXF, for an import into a
//! block library (`crate::import`).
//!
//! Part of the seam ([`super`]): only this module and its parent name
//! acadrust. Each named block is read with the readers a DWG read back
//! uses ([`super::read_object`]), so its objects come back as the core's
//! own; what they cannot read exactly is counted, by the type a DXF names
//! it by, rather than carried. A block that inserts another is read once
//! that one is, so the insert can be checked against the block it places.
//! The drawing is only read.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use acadrust::entities::EntityType;
use acadrust::tables::BlockRecord;
use acadrust::{CadDocument, DxfReader};

use super::{panic_message, point, read_drawing, read_layer, read_linetype, read_object, unit};
use crate::error::{Error, ErrorCode};
use crate::import::{carries, DrawingBlocks, ReadBlock};
use crate::model::{Block, Layer, Linetype, BUILT_IN_LINETYPES, CONTINUOUS};
use crate::text::{TextStyle, STANDARD};

/// The most lengths a linetype's pattern holds in a block library, and the
/// fewest.
const PATTERN_LENGTHS: std::ops::RangeInclusive<usize> = 2..=12;

/// The longest linetype description a block library takes, in characters.
const LONGEST_DESCRIPTION: usize = 47;

/// Reads the blocks of the DWG, or with `dxf` the DXF, at `path`, with the
/// linetypes, layers and text styles the core can read.
pub(crate) fn read_blocks(path: &Path, dxf: bool) -> Result<DrawingBlocks, Error> {
    let drawing = if dxf {
        read_dxf(path)?
    } else {
        read_drawing(path)?
    };
    let linetypes: Vec<Linetype> = drawing
        .line_types
        .iter()
        .filter(|l| {
            !BUILT_IN_LINETYPES
                .iter()
                .any(|built_in| built_in.eq_ignore_ascii_case(&l.name))
        })
        .filter_map(read_linetype)
        .filter(|l| {
            PATTERN_LENGTHS.contains(&l.pattern.len()) && l.pattern[0] >= 0.0 && l.pattern[1] < 0.0
        })
        .map(|mut l| {
            if l.description.chars().count() > LONGEST_DESCRIPTION {
                l.description.clear();
            }
            l
        })
        .collect();
    // A layer is taken with its linetype, so one whose linetype is not
    // read is not taken either.
    let layers: Vec<Layer> = drawing
        .layers
        .iter()
        .filter_map(read_layer)
        .filter_map(|mut layer| {
            if layer.linetype.is_empty() || layer.linetype.eq_ignore_ascii_case(CONTINUOUS) {
                CONTINUOUS.clone_into(&mut layer.linetype);
                return Some(layer);
            }
            let known = linetypes
                .iter()
                .find(|l| l.name.eq_ignore_ascii_case(&layer.linetype))?;
            layer.linetype.clone_from(&known.name);
            Some(layer)
        })
        .collect();
    let text_styles: Vec<TextStyle> = super::text::read_text_styles(&drawing)
        .0
        .into_iter()
        .map(|mut style| {
            if style.name.eq_ignore_ascii_case(STANDARD) {
                STANDARD.clone_into(&mut style.name);
            }
            style
        })
        .collect();

    let mut named: Vec<&BlockRecord> = Vec::new();
    let mut skipped = Vec::new();
    for record in drawing.block_records.iter() {
        let why = if record.is_model_space() || record.is_paper_space() || record.is_layout() {
            Some("the drawing's model space or a layout, not a block")
        } else if record.flags.is_xref
            || record.flags.is_xref_overlay
            || !record.xref_path.is_empty()
        {
            Some("an external reference, whose objects are in another drawing")
        } else if record.is_anonymous() {
            Some(
                "an anonymous block, which AutoCAD makes for itself, such as a dimension's or a \
                 dynamic block's copy",
            )
        } else {
            None
        };
        match why {
            Some(why) => skipped.push((record.name.clone(), why)),
            None => named.push(record),
        }
    }

    // A block is read once every named block it inserts has been, so that
    // its inserts are read against them; blocks that insert one another in
    // a round are read last, their inserts of each other not carried.
    let mut read: Vec<Option<ReadBlock>> = named.iter().map(|_| None).collect();
    let mut known: Vec<Block> = Vec::new();
    let read_one = |record: &BlockRecord, known: &[Block]| {
        let mut objects = Vec::new();
        let mut not_carried: Vec<(&'static str, usize)> = Vec::new();
        for dwg_object in drawing.entities_in_block(&record.name) {
            if matches!(dwg_object, EntityType::Block(_) | EntityType::BlockEnd(_)) {
                continue;
            }
            let object = read_object(dwg_object, known)
                .filter(|o| carries(o, &linetypes, &layers, &text_styles));
            match object {
                Some(object) => objects.push(object),
                None => {
                    let kind = object_type(dwg_object);
                    match not_carried.iter_mut().find(|(k, _)| *k == kind) {
                        Some((_, count)) => *count += 1,
                        None => not_carried.push((kind, 1)),
                    }
                }
            }
        }
        ReadBlock {
            block: Block {
                name: record.name.clone(),
                base_point: point(record.base_point),
                objects,
                cells: Vec::new(),
            },
            not_carried,
        }
    };
    loop {
        let mut more = false;
        for i in 0..named.len() {
            if read[i].is_some() {
                continue;
            }
            let waits =
                drawing
                    .entities_in_block(&named[i].name)
                    .any(|dwg_object| match dwg_object {
                        EntityType::Insert(insert) => {
                            named.iter().zip(&read).any(|(other, done)| {
                                done.is_none()
                                    && other.name == insert.block_name
                                    && other.name != named[i].name
                            })
                        }
                        _ => false,
                    });
            if waits {
                continue;
            }
            let block = read_one(named[i], &known);
            if !block.block.objects.is_empty() {
                known.push(block.block.clone());
            }
            read[i] = Some(block);
            more = true;
        }
        if !more {
            break;
        }
    }
    for i in 0..named.len() {
        if read[i].is_none() {
            let block = read_one(named[i], &known);
            if !block.block.objects.is_empty() {
                known.push(block.block.clone());
            }
            read[i] = Some(block);
        }
    }
    Ok(DrawingBlocks {
        units_code: drawing.header.insertion_units,
        units: unit(drawing.header.insertion_units),
        linetypes,
        layers,
        text_styles,
        blocks: read.into_iter().flatten().collect(),
        skipped,
    })
}

/// Reads the DXF at `path`.
fn read_dxf(path: &Path) -> Result<CadDocument, Error> {
    match catch_unwind(AssertUnwindSafe(|| {
        DxfReader::from_file(path).and_then(|reader| reader.read())
    })) {
        Ok(Ok(drawing)) => Ok(drawing),
        Ok(Err(e)) => Err(Error::new(
            ErrorCode::DwgRead,
            format!("the DWG library could not read the DXF: {e}"),
        )
        .in_file(path)),
        Err(panic) => Err(Error::new(
            ErrorCode::DwgRead,
            format!(
                "the DWG library failed while reading the DXF: {}",
                panic_message(panic)
            ),
        )
        .in_file(path)),
    }
}

/// The type of a drawing's object as a DXF names it, such as `DIMENSION`.
fn object_type(dwg_object: &EntityType) -> &'static str {
    match dwg_object {
        EntityType::Point(_) => "POINT",
        EntityType::Line(_) => "LINE",
        EntityType::Circle(_) => "CIRCLE",
        EntityType::Arc(_) => "ARC",
        EntityType::Ellipse(_) => "ELLIPSE",
        EntityType::Polyline(_)
        | EntityType::Polyline2D(_)
        | EntityType::Polyline3D(_)
        | EntityType::PolyfaceMesh(_)
        | EntityType::PolygonMesh(_) => "POLYLINE",
        EntityType::LwPolyline(_) => "LWPOLYLINE",
        EntityType::Text(_) => "TEXT",
        EntityType::MText(_) => "MTEXT",
        EntityType::Spline(_) => "SPLINE",
        EntityType::Helix(_) => "HELIX",
        EntityType::Dimension(_) => "DIMENSION",
        EntityType::Hatch(_) => "HATCH",
        EntityType::Solid(_) => "SOLID",
        EntityType::Face3D(_) => "3DFACE",
        EntityType::Insert(_) => "INSERT",
        EntityType::Ray(_) => "RAY",
        EntityType::XLine(_) => "XLINE",
        EntityType::Viewport(_) => "VIEWPORT",
        EntityType::AttributeDefinition(_) => "ATTDEF",
        EntityType::AttributeEntity(_) => "ATTRIB",
        EntityType::Leader(_) => "LEADER",
        EntityType::MultiLeader(_) => "MULTILEADER",
        EntityType::MLine(_) => "MLINE",
        EntityType::Mesh(_) => "MESH",
        EntityType::RasterImage(_) => "IMAGE",
        EntityType::Solid3D(_) => "3DSOLID",
        EntityType::Region(_) => "REGION",
        EntityType::Body(_) => "BODY",
        EntityType::Surface(_) => "SURFACE",
        EntityType::Table(_) => "ACAD_TABLE",
        EntityType::Tolerance(_) => "TOLERANCE",
        EntityType::Wipeout(_) => "WIPEOUT",
        EntityType::Shape(_) => "SHAPE",
        EntityType::Underlay(_) => "UNDERLAY",
        EntityType::Ole2Frame(_) => "OLE2FRAME",
        EntityType::Light(_) => "LIGHT",
        _ => "OTHER",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use acadrust::{DwgWriter, DxfWriter};

    use super::*;
    use crate::build::{build, BuildRequest};
    use crate::error::ErrorCode;
    use crate::import::{import_blocks, ImportBlocksRequest};
    use crate::units::Unit;

    /// A made-up block library: no plant's block.
    const LIBRARY: &str = "units: mm
layers:
  symbols: {color: yellow, lineweight: 0.35}
text_styles:
  tags: {font: romans.shx, width_factor: 0.8}
blocks:
  MARK:
    objects:
      - line: {from: [-5, 0, 0], to: [5, 0, 0], layer: symbols}
      - polyline: {points: [[0, 0, 0], {point: [4, 0, 0], bulge: 1}, [4, 4, 0]], color: red}
      - attdef: {tag: TAG, prompt: Tag, default: '?', position: [0, 2, 0], height: 2.5, style: tags, justify: middle_center}
";

    /// A made-up sheet that inserts the library's block.
    const SHEET: &str = "units: mm
block_library: blocks.yaml
objects:
  - insert: {block: MARK, position: [20, 20, 0], attributes: {TAG: M-1}}
";

    /// An empty folder for one test, holding the made-up library and sheet
    /// built into `sheet.dwg`.
    fn built(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("agentdrafter-import-{}", std::process::id()))
            .join(test);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("blocks.yaml"), LIBRARY).unwrap();
        fs::write(dir.join("sheet.yaml"), SHEET).unwrap();
        build(&BuildRequest {
            description: dir.join("sheet.yaml"),
            output: None,
            overwrite: false,
        })
        .unwrap();
        dir
    }

    fn import(drawing: PathBuf, library: PathBuf, units: Option<Unit>) -> Result<String, Error> {
        import_blocks(&ImportBlocksRequest {
            drawing,
            library: library.clone(),
            blocks: Vec::new(),
            replace: false,
            units,
            dry_run: false,
        })?;
        Ok(fs::read_to_string(library).unwrap())
    }

    #[test]
    fn a_dxf_gives_the_blocks_its_dwg_gives() {
        let dir = built("dxf");
        let drawing = read_drawing(&dir.join("sheet.dwg")).unwrap();
        // The extension is matched in any case.
        let dxf = dir.join("sheet.DXF");
        fs::write(&dxf, DxfWriter::new(&drawing).write_to_vec().unwrap()).unwrap();

        let from_dwg = read_blocks(&dir.join("sheet.dwg"), false).unwrap();
        let from_dxf = read_blocks(&dxf, true).unwrap();
        assert_eq!(from_dxf.units, Some(Unit::Mm));
        let blocks = |read: &DrawingBlocks| -> Vec<Block> {
            read.blocks.iter().map(|b| b.block.clone()).collect()
        };
        assert_eq!(blocks(&from_dxf), blocks(&from_dwg));
        assert_eq!(blocks(&from_dwg).len(), 1);
        assert!(from_dxf.blocks.iter().all(|b| b.not_carried.is_empty()));

        let by_dwg = import(dir.join("sheet.dwg"), dir.join("by-dwg.yaml"), None).unwrap();
        let by_dxf = import(dxf, dir.join("by-dxf.yaml"), None).unwrap();
        assert_eq!(by_dxf, by_dwg);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_drawing_that_states_no_unit_takes_the_one_named() {
        let dir = built("no-unit");
        let mut drawing = read_drawing(&dir.join("sheet.dwg")).unwrap();
        drawing.header.insertion_units = 0;
        let unitless = dir.join("unitless.dwg");
        fs::write(&unitless, DwgWriter::write_to_vec(&drawing).unwrap()).unwrap();

        let library = dir.join("imported.yaml");
        let error = import(unitless.clone(), library.clone(), None).unwrap_err();
        assert_eq!(error.code, ErrorCode::MissingUnit, "{error:?}");
        assert!(error.message.contains("--units"), "{error:?}");
        assert!(!library.exists());

        let text = import(unitless, library, Some(Unit::In)).unwrap();
        assert!(text.starts_with("units: in\n"), "{text}");
        let _ = fs::remove_dir_all(&dir);
    }
}
