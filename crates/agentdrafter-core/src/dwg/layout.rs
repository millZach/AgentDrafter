//! A sheet's paper-space layout, its page setup and its viewports in a
//! DWG.
//!
//! Part of the seam ([`super`]): only this module and its siblings name
//! acadrust. Every drawing has AutoCAD's `Layout1`, an empty layout on the
//! block `*Paper_Space`, which a sheet without a layout leaves as it is. A
//! sheet with one names that layout after its own, sets its page setup,
//! puts its paper-space objects and viewports on `*Paper_Space`, and opens
//! on it, as AutoCAD 2026 saved made-up layouts:
//!
//! - **The page setup** plots the layout on AutoCAD's own `DWG To PDF.pc3`,
//!   on the full-bleed paper of the sheet's standard size laid the sheet's
//!   way, so that the whole paper prints with its lower left corner at the
//!   origin, at full size: 1:1 when the paper is in the sheet's unit, and
//!   1:25.4 or 25.4:1 when one is in millimetres and the other in inches,
//!   through `monochrome.ctb`, with lineweights, paper space first. Its
//!   size is in millimetres, an inch paper's rounded to single precision,
//!   as AutoCAD keeps it.
//! - **The layout's own viewport**, AutoCAD's overall viewport, which shows
//!   paper space itself, comes first, centred on the paper.
//! - **Each viewport** is AutoCAD's VIEWPORT seen from above: its centre
//!   and size on the paper, its view centre in model space, and its view
//!   height, its height over its scale, with AutoCAD's status, display
//!   lock among it.
//!
//! A DWG is read as the sheet it opens on: model space alone when it
//! opens on the Model tab, and its layout when it opens on one, as
//! AutoCAD's TILEMODE says. A layout's viewport is read when the core
//! draws it: rectangular, on, seen from above with no twist, perspective
//! or clipping, and freezing no layer. Anything else is counted not read.

use acadrust::entities::{
    EntityType, GridFlags, Viewport as DwgViewport, ViewportRenderMode, ViewportStatusFlags,
};
use acadrust::objects::{Layout as DwgLayout, ObjectType, PlotFlags};
use acadrust::types::{Handle, Vector3};
use acadrust::CadDocument;

use crate::description::paper::STANDARD_SIZES;
use crate::error::{Error, ErrorCode};
use crate::model::{Paper, Point3, Sheet, Viewport};
use crate::units::Unit;

/// The layout every new drawing has, which a sheet with a layout takes.
const FIRST_LAYOUT: &str = "Layout1";

/// The plotter a layout's page setup plots on: AutoCAD's own PDF plotter,
/// which every AutoCAD has.
pub(crate) const PLOTTER: &str = "DWG To PDF.pc3";

/// The plot style a layout's page setup plots through.
pub(crate) const PLOT_STYLE: &str = "monochrome.ctb";

/// AutoCAD's plot type that plots a layout's whole paper: Layout.
const PLOT_LAYOUT: i16 = 5;

/// AutoCAD's standard scale 1:1.
const STANDARD_ONE_TO_ONE: i16 = 16;

/// The page setup's flags with the plot at a standard scale, AutoCAD's 1:1:
/// lineweights, plot styles and the standard scale, paper space first.
const FLAGS_STANDARD: i32 = 176;

/// The flags of a page setup at a custom scale: as [`FLAGS_STANDARD`]
/// without the standard scale.
const FLAGS_CUSTOM: i32 = 160;

/// A viewport's status as AutoCAD 2026's MVIEW made it: on, its UCS icon
/// shown at the origin, and front clipping not at the eye.
const VIEWPORT_STATUS: i32 = 0x8070;

/// The overall viewport's status as AutoCAD 2026 made it: on, its UCS icon
/// shown.
const OVERALL_STATUS: i32 = 0x8020;

/// AutoCAD's circle zoom percent on the viewports it made.
const CIRCLE_ZOOM: i16 = 100;

/// How much of the paper's height the overall viewport shows, as AutoCAD
/// zooms a new layout out round its paper.
const OVERALL_ZOOM: f64 = 1.2;

/// The overall viewport's snap and grid spacing, as AutoCAD 2026 set it.
const OVERALL_GRID: f64 = 0.5;

/// How close a viewport's view direction, target and the like must be to
/// AutoCAD's own for a top view to be read as one.
const TOLERANCE: f64 = 1e-12;

/// The page setup a standard paper is plotted with: AutoCAD's full-bleed
/// paper of its size laid its way.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PageSetup {
    /// AutoCAD's canonical name of the paper, such as
    /// `ISO_full_bleed_A3_(420.00_x_297.00_MM)`.
    pub media: String,
    /// Its width and height in millimetres, as AutoCAD keeps them.
    pub size_mm: (f64, f64),
    /// The unit the paper is sized in: millimetres or inches.
    pub unit: Unit,
}

impl PageSetup {
    /// The page setup of the standard paper named `name`, such as
    /// `ANSI B landscape`; none for another name.
    pub(crate) fn of(name: &str) -> Option<PageSetup> {
        let (size, way) = name.rsplit_once(' ')?;
        let &(_, short, long, unit) = STANDARD_SIZES.iter().find(|(s, ..)| *s == size)?;
        let (w, h) = match way {
            "landscape" => (long, short),
            "portrait" => (short, long),
            _ => return None,
        };
        let (family, letter) = size.split_once(' ')?;
        let (unit_word, size_mm) = match unit {
            Unit::In => ("Inches", (single(w * 25.4), single(h * 25.4))),
            _ => ("MM", (w, h)),
        };
        Some(PageSetup {
            media: format!("{family}_full_bleed_{letter}_({w:.2}_x_{h:.2}_{unit_word})"),
            size_mm,
            unit,
        })
    }

    /// The standard paper whose page setup plots on `media`, by name.
    pub(crate) fn named(media: &str) -> Option<String> {
        STANDARD_SIZES
            .iter()
            .flat_map(|(size, ..)| ["landscape", "portrait"].map(|way| format!("{size} {way}")))
            .find(|name| PageSetup::of(name).is_some_and(|p| p.media == media))
    }

    /// How many of the paper's own units one of `units` plots as, written
    /// as AutoCAD writes the plot scale: its numerator and denominator.
    pub(crate) fn plot_scale(&self, units: Unit) -> (f64, f64) {
        match (self.unit, units) {
            (Unit::In, Unit::Mm) => (1.0, 25.4),
            (Unit::Mm, Unit::In) => (25.4, 1.0),
            _ => (1.0, 1.0),
        }
    }
}

/// A length in millimetres rounded to single precision, as AutoCAD keeps
/// an inch paper's size.
fn single(mm: f64) -> f64 {
    #[allow(clippy::cast_possible_truncation)]
    f64::from(mm as f32)
}

/// The layout every drawing has, by name.
fn layout_mut<'a>(drawing: &'a mut CadDocument, name: &str) -> Option<&'a mut DwgLayout> {
    drawing.objects.values_mut().find_map(|o| match o {
        ObjectType::Layout(layout) if layout.name == name => Some(layout),
        _ => None,
    })
}

/// Makes AutoCAD's `Layout1` the sheet's layout, when it has one: named
/// after it, with its page setup and its overall viewport, the drawing
/// opening on it. A sheet without a layout leaves the drawing as it is.
pub(super) fn prepare(drawing: &mut CadDocument, sheet: &Sheet) -> Result<(), Error> {
    let Some(layout) = &sheet.layout else {
        return Ok(());
    };
    let failed = |why: String| Error::new(ErrorCode::DwgWrite, why);
    let paper = sheet
        .paper
        .as_ref()
        .ok_or_else(|| failed("a sheet drawn in a layout declares its paper".to_owned()))?;
    let setup = paper
        .name
        .as_deref()
        .and_then(PageSetup::of)
        .ok_or_else(|| {
            failed("a sheet drawn in a layout declares a standard paper by name".to_owned())
        })?;
    let (numerator, denominator) = setup.plot_scale(sheet.units);
    // AutoCAD 2026 kept its standard 1:1 as the scale type of an inch
    // drawing on millimetre paper, at 25.4:1, but not of a millimetre
    // drawing on inch paper, at 1:25.4.
    let standard = !(sheet.units == Unit::Mm && setup.unit == Unit::In);
    let dictionary = drawing.header.acad_layout_dict_handle;
    let dwg_layout = layout_mut(drawing, FIRST_LAYOUT)
        .ok_or_else(|| failed(format!("the drawing has no `{FIRST_LAYOUT}` to draw in")))?;
    let handle = dwg_layout.handle;
    layout.name.clone_into(&mut dwg_layout.name);
    PLOTTER.clone_into(&mut dwg_layout.plot_printer_name);
    dwg_layout.paper_size = setup.media.clone();
    (dwg_layout.paper_width, dwg_layout.paper_height) = setup.size_mm;
    dwg_layout.plot_paper_units = i16::from(setup.unit != Unit::In);
    dwg_layout.plot_rotation = 0;
    dwg_layout.plot_type = PLOT_LAYOUT;
    dwg_layout.plot_scale_numerator = numerator;
    dwg_layout.plot_scale_denominator = denominator;
    dwg_layout.plot_scale_type = if standard { STANDARD_ONE_TO_ONE } else { 0 };
    dwg_layout.plot_flags = PlotFlags::from_bits(if numerator == denominator {
        FLAGS_STANDARD
    } else {
        FLAGS_CUSTOM
    });
    PLOT_STYLE.clone_into(&mut dwg_layout.plot_style_sheet);
    // AutoCAD keeps an empty box as a layout's extents until it draws one.
    dwg_layout.min_extents = (1e20, 1e20, 1e20);
    dwg_layout.max_extents = (-1e20, -1e20, -1e20);
    if let Some(ObjectType::Dictionary(entries)) = drawing.objects.get_mut(&dictionary) {
        entries.remove_entry(FIRST_LAYOUT);
        entries.add_entry(&layout.name, handle);
    }
    // The drawing opens on its layout, as AutoCAD saves one drawn there.
    drawing.header.show_model_space = false;
    let mut overall = DwgViewport::new();
    overall.id = 1;
    overall.center = Vector3::new(paper.width / 2.0, paper.height / 2.0, 0.0);
    overall.height = paper.height * OVERALL_ZOOM;
    overall.width = paper.width * OVERALL_ZOOM;
    overall.view_center = overall.center;
    overall.view_height = overall.height;
    overall.status = ViewportStatusFlags::from_bits(OVERALL_STATUS);
    overall.grid_flags = GridFlags {
        beyond_limits: true,
        adaptive: true,
        ..GridFlags::default()
    };
    overall.circle_sides = CIRCLE_ZOOM;
    overall.snap_spacing = Vector3::new(OVERALL_GRID, OVERALL_GRID, 0.0);
    overall.grid_spacing = Vector3::new(OVERALL_GRID, OVERALL_GRID, 0.0);
    drawing
        .add_entity_to_layout(EntityType::Viewport(Box::new(overall)), &layout.name)
        .map_err(|e| failed(format!("could not add the layout's own viewport: {e}")))?;
    Ok(())
}

/// The snap and grid spacing AutoCAD's MVIEW gives a viewport in a drawing
/// in `units`.
fn grid_of(units: Unit) -> (f64, f64) {
    if units.is_metric() {
        (0.5, 10.0)
    } else {
        (0.5, 0.5)
    }
}

/// The DWG viewport for one of ours, as AutoCAD's MVIEW makes one, in a
/// drawing in `units`.
pub(super) fn dwg_viewport(viewport: &Viewport, units: Unit) -> EntityType {
    let mut dwg = DwgViewport::new();
    dwg.center = Vector3::new(viewport.center.x, viewport.center.y, viewport.center.z);
    dwg.width = viewport.width;
    dwg.height = viewport.height;
    dwg.view_center = Vector3::new(viewport.view_center.x, viewport.view_center.y, 0.0);
    dwg.view_height = viewport.view_height();
    dwg.custom_scale = viewport.scale;
    let mut status = ViewportStatusFlags::from_bits(VIEWPORT_STATUS);
    status.locked = viewport.locked;
    dwg.status = status;
    dwg.grid_flags = GridFlags {
        adaptive: true,
        ..GridFlags::default()
    };
    dwg.circle_sides = CIRCLE_ZOOM;
    let (snap, grid) = grid_of(units);
    dwg.snap_spacing = Vector3::new(snap, snap, 0.0);
    dwg.grid_spacing = Vector3::new(grid, grid, 0.0);
    EntityType::Viewport(Box::new(dwg))
}

/// The layout a drawing opens on, when it opens on one: its name and the
/// handle of its block of paper-space objects.
pub(super) fn opened_on(drawing: &CadDocument) -> Option<(String, Handle)> {
    if drawing.header.show_model_space {
        return None;
    }
    let block = drawing.header.paper_space_block_handle;
    drawing.objects.values().find_map(|o| match o {
        ObjectType::Layout(layout) if layout.block_record == block && !block.is_null() => {
            Some((layout.name.clone(), block))
        }
        _ => None,
    })
}

/// The drawing's layouts other than model space's and the one it opens
/// on, `opened`, that hold anything but their own overall viewport: what
/// the core does not read of them.
pub(super) fn other_layouts(drawing: &CadDocument, opened: Option<Handle>) -> usize {
    let model = drawing.header.model_space_block_handle;
    drawing
        .objects
        .values()
        .filter_map(|o| match o {
            ObjectType::Layout(layout) => Some(layout.block_record),
            _ => None,
        })
        .filter(|&block| block != model && Some(block) != opened && !block.is_null())
        .filter(|&block| {
            drawing
                .block_records
                .iter()
                .find(|r| r.handle == block)
                .is_some_and(|r| drawing.entities_in_block(&r.name).any(|e| !is_overall(e)))
        })
        .count()
}

/// Whether `object` is a layout's own overall viewport, which shows paper
/// space itself.
pub(super) fn is_overall(object: &EntityType) -> bool {
    matches!(object, EntityType::Viewport(v) if v.id == 1)
}

/// The paper the layout of `drawing` whose block is `block` plots on, in
/// `units`: a standard paper, whole, at full size, as a build writes it.
/// None for any other page setup.
pub(super) fn read_paper(drawing: &CadDocument, block: Handle, units: Unit) -> Option<Paper> {
    let layout = drawing.objects.values().find_map(|o| match o {
        ObjectType::Layout(layout) if layout.block_record == block => Some(layout),
        _ => None,
    })?;
    let name = PageSetup::named(&layout.paper_size)?;
    let setup = PageSetup::of(&name)?;
    let (numerator, denominator) = setup.plot_scale(units);
    let whole = layout.plot_type == PLOT_LAYOUT
        && layout.plot_rotation == 0
        && layout.plot_paper_units == i16::from(setup.unit != Unit::In)
        && layout.plot_scale_numerator == numerator
        && layout.plot_scale_denominator == denominator
        && [
            layout.plot_margin_left,
            layout.plot_margin_bottom,
            layout.plot_margin_right,
            layout.plot_margin_top,
            layout.plot_origin_x,
            layout.plot_origin_y,
        ]
        .iter()
        .all(|&v| v == 0.0)
        && (layout.paper_width, layout.paper_height) == setup.size_mm;
    if !whole {
        return None;
    }
    let paper = crate::description::paper::paper_names()
        .into_iter()
        .find(|(n, ..)| *n == name)?;
    let (_, width, height, own) = paper;
    Some(Paper {
        name: Some(name),
        width: own.convert(width, units),
        height: own.convert(height, units),
    })
}

/// One of our viewports, if the DWG viewport is one the core draws:
/// rectangular, on, seen from above with no twist, perspective or
/// clipping, and freezing no layer.
pub(super) fn read_viewport(dwg: &DwgViewport, layer: String) -> Option<Viewport> {
    let status = &dwg.status;
    let top = (dwg.view_direction.x.abs() <= TOLERANCE)
        && (dwg.view_direction.y.abs() <= TOLERANCE)
        && dwg.view_direction.z > 0.0;
    let at_origin = [dwg.view_target.x, dwg.view_target.y, dwg.view_target.z]
        .iter()
        .all(|v| v.abs() <= TOLERANCE);
    let drawn = dwg.is_on()
        && !status.perspective
        && !status.front_clipping
        && !status.back_clipping
        && !status.non_rectangular_clipping
        && dwg.clip_boundary_handle.is_null()
        && dwg.frozen_layers.is_empty()
        && top
        && at_origin
        && dwg.twist_angle == 0.0
        && dwg.render_mode == ViewportRenderMode::Wireframe2D
        && dwg.width > 0.0
        && dwg.height > 0.0
        && dwg.view_height > 0.0
        && dwg.center.z == 0.0;
    if !drawn {
        return None;
    }
    let scale = Viewport::scale_of(dwg.height, dwg.view_height);
    let viewport = Viewport {
        center: Point3::new(dwg.center.x, dwg.center.y, 0.0),
        width: dwg.width,
        height: dwg.height,
        view_center: Point3::new(dwg.view_center.x, dwg.view_center.y, 0.0),
        scale,
        locked: status.locked,
        layer,
        properties: Default::default(),
    };
    // A scale the core cannot draw is not read.
    viewport.scale_problem().is_none().then_some(viewport)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_standard_paper_plots_on_autocads_full_bleed_paper() {
        let a3 = PageSetup::of("ISO A3 landscape").unwrap();
        assert_eq!(a3.media, "ISO_full_bleed_A3_(420.00_x_297.00_MM)");
        assert_eq!(a3.size_mm, (420.0, 297.0));
        let b = PageSetup::of("ANSI B portrait").unwrap();
        assert_eq!(b.media, "ANSI_full_bleed_B_(11.00_x_17.00_Inches)");
        // AutoCAD 2026 keeps an inch paper's size in single precision.
        assert_eq!(b.size_mm, (279.399_993_896_484_4, 431.799_987_792_968_75));
        let a = PageSetup::of("ANSI A landscape").unwrap();
        assert_eq!(a.media, "ANSI_full_bleed_A_(11.00_x_8.50_Inches)");
        assert_eq!(
            PageSetup::named(&a.media).as_deref(),
            Some("ANSI A landscape")
        );
        assert_eq!(PageSetup::of("ISO A3"), None);
    }
}
