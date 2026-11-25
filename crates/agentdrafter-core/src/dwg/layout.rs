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
//! A DWG is read with every layout it holds, whatever tab it was saved
//! on: AutoCAD's TILEMODE says only whether it opens on the Model tab or
//! on a layout, which. A layout's viewport is read when the core draws
//! it: on, seen from above with no twist, perspective or front or back
//! clipping, rectangular or clipped to a boundary the core reads, and
//! freezing layers of the drawing's own. Anything else is counted not
//! read.

use acadrust::entities::{
    Circle, EntityType, GridFlags, LwPolyline, Viewport as DwgViewport, ViewportRenderMode,
    ViewportStatusFlags,
};
use acadrust::objects::{Layout as DwgLayout, ObjectType, PlotFlags};
use acadrust::types::{Handle, Vector2, Vector3};
use acadrust::CadDocument;

use crate::description::paper::STANDARD_SIZES;
use crate::error::{Error, ErrorCode};
use crate::model::layout::{CheckPaper, DwgExtraLayout, DwgLayoutSetup, DwgPageSetup};
use crate::model::{Clip, Paper, Point3, Sheet, Viewport};
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

/// A clipped viewport's status as AutoCAD 2026's VPCLIP and MVIEW's Object
/// and Polygonal options made it: on, clipped to its boundary, its UCS
/// icon at the origin but not shown, and front clipping not at the eye.
const CLIPPED_STATUS: i32 = 0x18050;

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

/// Makes AutoCAD's `Layout1` the sheet's first layout, when it has one:
/// named after it, with its page setup and its overall viewport, the
/// drawing opening on it. Each of its other layouts is added after it, in
/// order, with its own page setup and overall viewport, as AutoCAD 2026
/// numbered made-up layouts' tabs in order. A sheet without a layout
/// leaves the drawing as it is.
pub(super) fn prepare(drawing: &mut CadDocument, sheet: &Sheet) -> Result<(), Error> {
    let Some(layout) = &sheet.layout else {
        if let Some(ps) = &sheet.page_setup {
            if let Some(model_layout) = layout_mut(drawing, "Model") {
                super::page_setup::apply_page_setup(
                    model_layout,
                    ps,
                    sheet.paper.as_ref(),
                    sheet.units,
                    true,
                );
            }
        }
        return Ok(());
    };
    let failed = |why: String| Error::new(ErrorCode::DwgWrite, why);
    let paper = sheet
        .paper
        .as_ref()
        .ok_or_else(|| failed("a sheet drawn in a layout declares its paper".to_owned()))?;
    let dictionary = drawing.header.acad_layout_dict_handle;
    let dwg_layout = layout_mut(drawing, FIRST_LAYOUT)
        .ok_or_else(|| failed(format!("the drawing has no `{FIRST_LAYOUT}` to draw in")))?;
    let handle = dwg_layout.handle;
    layout.name.clone_into(&mut dwg_layout.name);
    let has_transparency = sheet.layers.iter().any(|l| l.transparency > 0)
        || sheet
            .objects
            .iter()
            .chain(sheet.blocks.iter().flat_map(|b| b.objects.iter()))
            .any(|o| {
                matches!(
                    o.properties().transparency,
                    crate::model::ObjectTransparency::Percent(p) if p > 0
                )
            });
    if let Some(ps) = layout.page_setup.as_ref().or(sheet.page_setup.as_ref()) {
        super::page_setup::apply_page_setup(dwg_layout, ps, Some(paper), sheet.units, false);
    } else {
        set_page_setup(dwg_layout, paper, sheet.units)?;
        if has_transparency {
            dwg_layout.plot_flags.unknown_bits |= super::page_setup::PLOT_TRANSPARENCY_BIT;
        }
    }
    if let Some(ObjectType::Dictionary(entries)) = drawing.objects.get_mut(&dictionary) {
        entries.remove_entry(FIRST_LAYOUT);
        entries.add_entry(&layout.name, handle);
    }
    // The drawing opens on its layout, as AutoCAD saves one drawn there.
    drawing.header.show_model_space = false;
    let mut overall = DwgViewport::new();
    overall.id = 1;
    set_overall(&mut overall, paper);
    drawing
        .add_entity_to_layout(EntityType::Viewport(Box::new(overall)), &layout.name)
        .map_err(|e| failed(format!("could not add the layout's own viewport: {e}")))?;
    for other in &sheet.other_layouts {
        let paper = other.paper.as_ref().ok_or_else(|| {
            failed(format!(
                "layout `{}` declares its paper, a standard paper by name",
                other.name
            ))
        })?;
        drawing
            .add_layout(&other.name)
            .map_err(|e| failed(format!("could not add layout `{}`: {e}", other.name)))?;
        let dwg_layout = layout_mut(drawing, &other.name)
            .ok_or_else(|| failed(format!("layout `{}` was not added", other.name)))?;
        if let Some(ps) = &other.page_setup {
            super::page_setup::apply_page_setup(dwg_layout, ps, Some(paper), sheet.units, false);
        } else {
            set_page_setup(dwg_layout, paper, sheet.units)?;
            if has_transparency {
                dwg_layout.plot_flags.unknown_bits |= super::page_setup::PLOT_TRANSPARENCY_BIT;
            }
        }
        // acadrust adds the layout's overall viewport; it is set as the
        // first layout's is.
        let overall = dwg_layout.viewport;
        if let Some(EntityType::Viewport(v)) = drawing.get_entity_mut(overall) {
            set_overall(v, paper);
        }
    }
    Ok(())
}

/// Sets `dwg_layout`'s page setup to plot `paper`, a standard paper by
/// name, whole at full size, on a drawing in `units`.
fn set_page_setup(dwg_layout: &mut DwgLayout, paper: &Paper, units: Unit) -> Result<(), Error> {
    let setup = paper
        .name
        .as_deref()
        .and_then(PageSetup::of)
        .ok_or_else(|| {
            Error::new(
                ErrorCode::DwgWrite,
                "a sheet drawn in a layout declares a standard paper by name",
            )
        })?;
    let (numerator, denominator) = setup.plot_scale(units);
    // AutoCAD 2026 kept its standard 1:1 as the scale type of an inch
    // drawing on millimetre paper, at 25.4:1, but not of a millimetre
    // drawing on inch paper, at 1:25.4.
    let standard = !(units == Unit::Mm && setup.unit == Unit::In);
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
    Ok(())
}

/// Sets a layout's own overall viewport, which shows paper space itself,
/// as AutoCAD 2026 made it for a layout on `paper`: centred on the paper,
/// zoomed out round it.
fn set_overall(overall: &mut DwgViewport, paper: &Paper) {
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
/// drawing in `units`: clipped to the boundary whose handle is `boundary`,
/// when it is clipped, and freezing the layers whose handles are
/// `frozen`, as VPLAYER freezes them.
pub(super) fn dwg_viewport(
    viewport: &Viewport,
    units: Unit,
    boundary: Option<Handle>,
    frozen: Vec<Handle>,
) -> EntityType {
    let mut dwg = DwgViewport::new();
    dwg.center = Vector3::new(viewport.center.x, viewport.center.y, viewport.center.z);
    dwg.width = viewport.width;
    dwg.height = viewport.height;
    if let Some(vd) = viewport.view_direction {
        dwg.view_direction = Vector3::new(vd.x, vd.y, vd.z);
        dwg.view_target = Vector3::new(
            viewport.view_center.x,
            viewport.view_center.y,
            viewport.view_center.z,
        );
        dwg.view_center = Vector3::ZERO;
    } else {
        dwg.view_center = Vector3::new(viewport.view_center.x, viewport.view_center.y, 0.0);
    }
    dwg.view_height = viewport.view_height();
    dwg.custom_scale = viewport.scale;
    dwg.twist_angle = viewport.rotation.to_radians();
    let mut status = ViewportStatusFlags::from_bits(if boundary.is_some() {
        CLIPPED_STATUS
    } else {
        VIEWPORT_STATUS
    });
    status.locked = viewport.locked;
    dwg.status = status;
    if let Some(boundary) = boundary {
        dwg.clip_boundary_handle = boundary;
    }
    dwg.frozen_layers = frozen;
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

/// The paper-space object a viewport's boundary is in the DWG, as
/// AutoCAD's VPCLIP keeps one: a closed lightweight polyline, its bulges
/// among it, or a circle.
pub(super) fn dwg_boundary(clip: &Clip) -> EntityType {
    match clip {
        Clip::Polyline { points, .. } => {
            let mut polyline = LwPolyline::new();
            for (i, p) in points.iter().enumerate() {
                polyline.add_point_with_bulge(Vector2::new(p.x, p.y), clip.bulge(i));
            }
            polyline.is_closed = true;
            EntityType::LwPolyline(polyline)
        }
        Clip::Circle { center, radius } => EntityType::Circle(Circle::from_center_radius(
            Vector3::new(center.x, center.y, 0.0),
            *radius,
        )),
    }
}

/// The boundary a DWG object draws, if it is one a viewport can be clipped
/// to as a build writes it: a closed, flat lightweight polyline with no
/// width, seen from above, or a circle on the paper, seen from above.
pub(super) fn read_boundary(object: &EntityType) -> Option<Clip> {
    let up = |n: &Vector3| n.x == 0.0 && n.y == 0.0 && n.z == 1.0;
    match object {
        EntityType::LwPolyline(p)
            if p.is_closed
                && p.vertices.len() >= 3
                && p.constant_width == 0.0
                && p.elevation == 0.0
                && p.thickness == 0.0
                && up(&p.normal)
                && p.vertices
                    .iter()
                    .all(|v| v.start_width == 0.0 && v.end_width == 0.0) =>
        {
            let points = p
                .vertices
                .iter()
                .map(|v| Point3::new(v.location.x, v.location.y, 0.0))
                .collect();
            let mut bulges: Vec<f64> = p.vertices.iter().map(|v| v.bulge).collect();
            if bulges.iter().all(|&b| b == 0.0) {
                bulges.clear();
            }
            Some(Clip::Polyline { points, bulges })
        }
        EntityType::Circle(c) if c.center.z == 0.0 && c.thickness == 0.0 && up(&c.normal) => {
            Some(Clip::Circle {
                center: Point3::new(c.center.x, c.center.y, 0.0),
                radius: c.radius,
            })
        }
        _ => None,
    }
}

/// The layout a drawing opens on, when it opens on one: its name and the
/// handle of its block of paper-space objects.
pub(super) fn opened_on(drawing: &CadDocument) -> Option<(String, Handle)> {
    if drawing.header.show_model_space {
        return None;
    }
    let block = drawing.header.paper_space_block_handle;
    drawing
        .objects
        .values()
        .filter_map(|o| match o {
            ObjectType::Layout(layout) if layout.block_record == block && !block.is_null() => {
                Some(layout)
            }
            _ => None,
        })
        .min_by_key(|l| l.handle.value())
        .map(|layout| (layout.name.clone(), block))
}

/// A layout of a drawing the core reads: its name, the handle of its block
/// of paper-space objects, the full-bleed standard paper its page setup plots
/// (when the core can render/plot it directly), and its stored page setup and
/// checkable paper bounds for `query layout` and `check`.
pub(super) struct ReadLayout {
    pub(super) name: String,
    pub(super) block: Handle,
    pub(super) paper: Option<Paper>,
    pub(super) page_setup: Option<crate::model::PageSetup>,
    pub(super) setup: DwgLayoutSetup,
}

/// Reads the `"Model"` layout's page setup, if configured on `drawing`.
pub(super) fn read_model_page_setup(
    drawing: &CadDocument,
    units: Option<Unit>,
) -> Option<crate::model::PageSetup> {
    let model = drawing.header.model_space_block_handle;
    let layout = drawing.objects.values().find_map(|o| match o {
        ObjectType::Layout(l)
            if (l.block_record == model && !model.is_null())
                || l.name.eq_ignore_ascii_case("Model") =>
        {
            Some(l)
        }
        _ => None,
    })?;
    super::page_setup::read_page_setup(layout, units, true)
}

/// The layouts of `drawing`, in `units` (when declared), the core reads, in
/// the order of their tabs, and how many of them it reads no full-bleed page
/// setup of, whatever tab the drawing was saved on: the one it opens on, if
/// it opens on a layout, one whose full-bleed page setup the core reads, and
/// one that holds anything but its own overall viewport, whose page setup it
/// does not read and counts. AutoCAD's own empty layout, on its own page
/// setup, is neither read nor counted, so a drawing of model space alone
/// reads as one.
///
/// Additional inactive empty layouts with a configured page setup (when the
/// drawing uses layouts) are returned separately in `extra_layout_setups` for
/// `query layout` alone, so `render`, `plot`, `display-list` and `check`
/// never receive unplottable blank pages in `Sheet.other_layouts`.
pub(super) fn read_layouts(
    drawing: &CadDocument,
    units: Option<Unit>,
) -> (Vec<ReadLayout>, usize, Vec<DwgExtraLayout>) {
    let model = drawing.header.model_space_block_handle;
    let mut all: Vec<&DwgLayout> = drawing
        .objects
        .values()
        .filter_map(|o| match o {
            ObjectType::Layout(layout)
                if layout.block_record != model && !layout.block_record.is_null() =>
            {
                Some(layout)
            }
            _ => None,
        })
        .collect();
    all.sort_by_key(|l| (l.tab_order, l.handle.value()));
    let holds_anything = |block: Handle| {
        drawing
            .block_records
            .iter()
            .find(|r| r.handle == block)
            .is_some_and(|r| drawing.entities_in_block(&r.name).any(|e| !is_overall(e)))
    };
    // The layout the drawing opens on, if it opens on one: read even when
    // it holds nothing. A drawing that opens on the Model tab has none.
    let opened = opened_on(drawing).map(|(_, block)| block);
    let mut read = Vec::new();
    let mut not_read = 0;
    let mut extra = Vec::new();
    let render_units = units.unwrap_or(Unit::Mm);
    for (tab_position, layout) in all.into_iter().enumerate() {
        let block = layout.block_record;
        let paper = read_paper(drawing, block, render_units);
        let setup = read_layout_setup(layout, units, tab_position);
        if paper.is_none() {
            if Some(block) != opened && !holds_anything(block) {
                if has_configured_setup(layout) {
                    extra.push(DwgExtraLayout {
                        name: layout.name.clone(),
                        setup,
                    });
                }
                continue;
            }
            not_read += 1;
        }
        let page_setup = super::page_setup::read_page_setup(layout, units, false);
        read.push(ReadLayout {
            name: layout.name.clone(),
            block,
            paper,
            page_setup,
            setup,
        });
    }
    // Keep a drawing of model space alone as model space alone: AutoCAD
    // templates often leave a default printer/paper on untouched empty
    // `Layout1`/`Layout2` tabs even when the drawing only uses Model space.
    if read.is_empty() {
        extra.clear();
    }
    (read, not_read, extra)
}

/// Whether `layout` has an explicit page setup configured (a plotter other than
/// AutoCAD's blank `None`/`none_device`, a plot style sheet, or a paper size
/// when the plotter is not `None`/`none_device`), as opposed to AutoCAD's or
/// acadrust's untouched default blank layout tab.
fn has_configured_setup(layout: &DwgLayout) -> bool {
    let is_none_plotter = layout.plot_printer_name.eq_ignore_ascii_case("none_device")
        || layout.plot_printer_name.eq_ignore_ascii_case("None");
    let has_plotter = !layout.plot_printer_name.is_empty() && !is_none_plotter;
    let has_style = !layout.plot_style_sheet.is_empty();
    let has_media = !is_none_plotter && !layout.paper_size.is_empty();
    has_plotter || has_style || has_media
}

/// AutoCAD's plot-type code as a lowercase word, matching `query layout`.
fn plot_area_name(plot_type: i16) -> String {
    match plot_type {
        0 => "display".to_owned(),
        1 => "extents".to_owned(),
        2 => "limits".to_owned(),
        3 => "view".to_owned(),
        4 => "window".to_owned(),
        5 => "layout".to_owned(),
        other => format!("unknown ({other})"),
    }
}

/// The stored plot scale of `layout`, such as `1:1` or `1:25.4`.
fn plot_scale_text(layout: &DwgLayout) -> String {
    if layout.plot_scale_numerator.is_finite()
        && layout.plot_scale_denominator.is_finite()
        && layout.plot_scale_numerator > 0.0
        && layout.plot_scale_denominator > 0.0
    {
        format!(
            "{}:{}",
            layout.plot_scale_numerator, layout.plot_scale_denominator
        )
    } else {
        String::new()
    }
}

/// Reads the stored plot settings and checkable paper bounds from `layout`.
fn read_layout_setup(
    layout: &DwgLayout,
    units: Option<Unit>,
    tab_position: usize,
) -> DwgLayoutSetup {
    let has_setup = !layout.plot_printer_name.is_empty()
        || !layout.paper_size.is_empty()
        || !layout.plot_style_sheet.is_empty()
        || (layout.paper_width > 0.0 && layout.paper_height > 0.0);
    let page_setup = has_setup.then(|| DwgPageSetup {
        plotter: layout.plot_printer_name.clone(),
        paper: layout.paper_size.clone(),
        area: plot_area_name(layout.plot_type),
        scale: plot_scale_text(layout),
        plot_style: layout.plot_style_sheet.clone(),
    });
    let paper_unit = match layout.plot_paper_units {
        0 => Some(Unit::In),
        1 => Some(Unit::Mm),
        _ => None,
    };
    let stored_paper = read_stored_paper(layout, paper_unit);
    let (check_paper, skip_reason) = read_check_paper(layout, units);
    DwgLayoutSetup {
        page_setup,
        check_paper,
        stored_paper,
        paper_unit,
        skip_reason,
        tab_position,
    }
}

/// The physical paper dimensions `layout` stores in its own `paper_unit`,
/// rotated by `plot_rotation`, regardless of plot scale or plot area, for
/// `query layout` reporting.
fn read_stored_paper(layout: &DwgLayout, paper_unit: Option<Unit>) -> Option<Paper> {
    let unit = paper_unit?;
    if !layout.paper_width.is_finite()
        || !layout.paper_height.is_finite()
        || layout.paper_width <= 0.0
        || layout.paper_height <= 0.0
    {
        return None;
    }
    let (w_mm, h_mm) = match layout.plot_rotation {
        0 | 2 => (layout.paper_width, layout.paper_height),
        1 | 3 => (layout.paper_height, layout.paper_width),
        _ => return None,
    };
    Some(Paper {
        name: standard_paper_name(w_mm, h_mm),
        width: Unit::Mm.convert(w_mm, unit),
        height: Unit::Mm.convert(h_mm, unit),
    })
}

/// The paper bounds `layout` declares in paper-space drawing units for
/// `check` (`paper_size` and `off_sheet`), or the reason those checks must be
/// skipped on this layout.
fn read_check_paper(
    layout: &DwgLayout,
    units: Option<Unit>,
) -> (Option<CheckPaper>, Option<String>) {
    if layout.plot_type != PLOT_LAYOUT {
        return (
            None,
            Some(format!(
                "layout `{}` plots {} rather than layout",
                layout.name,
                plot_area_name(layout.plot_type)
            )),
        );
    }
    let paper_unit = match layout.plot_paper_units {
        0 => Unit::In,
        1 => Unit::Mm,
        _ => {
            return (
                None,
                Some(format!(
                    "layout `{}` uses unsupported paper units ({})",
                    layout.name, layout.plot_paper_units
                )),
            );
        }
    };
    if !layout.paper_width.is_finite()
        || !layout.paper_height.is_finite()
        || layout.paper_width <= 0.0
        || layout.paper_height <= 0.0
    {
        return (
            None,
            Some(format!("layout `{}` declares no paper size", layout.name)),
        );
    }
    let ((w_mm, h_mm), (left_mm, bottom_mm, right_mm, top_mm)) = match layout.plot_rotation {
        0 => (
            (layout.paper_width, layout.paper_height),
            (
                layout.plot_margin_left,
                layout.plot_margin_bottom,
                layout.plot_margin_right,
                layout.plot_margin_top,
            ),
        ),
        1 => (
            (layout.paper_height, layout.paper_width),
            (
                layout.plot_margin_bottom,
                layout.plot_margin_right,
                layout.plot_margin_top,
                layout.plot_margin_left,
            ),
        ),
        2 => (
            (layout.paper_width, layout.paper_height),
            (
                layout.plot_margin_right,
                layout.plot_margin_top,
                layout.plot_margin_left,
                layout.plot_margin_bottom,
            ),
        ),
        3 => (
            (layout.paper_height, layout.paper_width),
            (
                layout.plot_margin_top,
                layout.plot_margin_left,
                layout.plot_margin_bottom,
                layout.plot_margin_right,
            ),
        ),
        other => {
            return (
                None,
                Some(format!(
                    "layout `{}` has an unsupported plot rotation ({other})",
                    layout.name
                )),
            );
        }
    };
    let margins_ok = [
        left_mm,
        bottom_mm,
        right_mm,
        top_mm,
        layout.plot_origin_x,
        layout.plot_origin_y,
    ]
    .iter()
    .all(|v| v.is_finite())
        && left_mm >= 0.0
        && bottom_mm >= 0.0
        && right_mm >= 0.0
        && top_mm >= 0.0
        && left_mm + right_mm < w_mm
        && bottom_mm + top_mm < h_mm;
    if !margins_ok {
        return (
            None,
            Some(format!(
                "layout `{}` has invalid printer margins or plot origin",
                layout.name
            )),
        );
    }
    if !layout.plot_scale_numerator.is_finite()
        || !layout.plot_scale_denominator.is_finite()
        || layout.plot_scale_numerator <= 0.0
        || layout.plot_scale_denominator <= 0.0
    {
        return (
            None,
            Some(format!(
                "layout `{}` has no valid 1:1 plot scale",
                layout.name
            )),
        );
    }
    let actual = layout.plot_scale_numerator / layout.plot_scale_denominator;
    // In AutoCAD, a layout's paper-space units follow its own `plot_paper_units`
    // when plotted at 1:1 (`numerator == denominator`), regardless of model-space
    // INSUNITS. Cross-unit 1:1 layouts (such as a mm sheet built on inch paper
    // at 1:25.4, or an inch sheet on mm paper at 25.4:1) use the drawing's unit.
    let layout_unit = if (actual - 1.0).abs() <= 1e-6 {
        paper_unit
    } else if let Some(drawing_unit) = units.filter(|&u| u != paper_unit) {
        let expected = drawing_unit.convert(1.0, paper_unit);
        if ((actual - expected).abs() / expected) <= 1e-6 {
            drawing_unit
        } else {
            return (
                None,
                Some(format!(
                    "layout `{}` uses plot scale {}:{}, not 1:1",
                    layout.name, layout.plot_scale_numerator, layout.plot_scale_denominator
                )),
            );
        }
    } else {
        return (
            None,
            Some(format!(
                "layout `{}` uses plot scale {}:{}, not 1:1",
                layout.name, layout.plot_scale_numerator, layout.plot_scale_denominator
            )),
        );
    };
    let paper = Paper {
        name: standard_paper_name(w_mm, h_mm),
        width: Unit::Mm.convert(w_mm, layout_unit),
        height: Unit::Mm.convert(h_mm, layout_unit),
    };
    let clean_zero = |v: f64| if v == 0.0 { 0.0 } else { v };
    let origin = [
        clean_zero(Unit::Mm.convert(-(left_mm + layout.plot_origin_x), layout_unit)),
        clean_zero(Unit::Mm.convert(-(bottom_mm + layout.plot_origin_y), layout_unit)),
    ];
    (
        Some(CheckPaper {
            paper,
            origin,
            unit: layout_unit,
        }),
        None,
    )
}

/// The standard paper name matching physical dimensions `(w_mm, h_mm)` within
/// AutoCAD's single-precision float storage tolerance (`1e-3` mm), without
/// ever altering the layout's stored dimensions.
fn standard_paper_name(w_mm: f64, h_mm: f64) -> Option<String> {
    crate::description::paper::paper_names()
        .into_iter()
        .find_map(|(name, width, height, own)| {
            let sw = own.convert(width, Unit::Mm);
            let sh = own.convert(height, Unit::Mm);
            ((sw - w_mm).abs() <= 1e-3 && (sh - h_mm).abs() <= 1e-3).then_some(name)
        })
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
    let layout = drawing
        .objects
        .values()
        .filter_map(|o| match o {
            ObjectType::Layout(layout) if layout.block_record == block => Some(layout),
            _ => None,
        })
        .min_by_key(|l| l.handle.value())?;
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

/// One of our viewports, if the DWG viewport is one the core draws: on,
/// seen from above with no twist, perspective or front or back clipping,
/// rectangular or clipped to `clip`, the boundary its clip handle names,
/// and freezing the layers `frozen` names.
pub(super) fn read_viewport(
    dwg: &DwgViewport,
    layer: String,
    clip: Option<Clip>,
    frozen_layers: Vec<String>,
) -> Option<Viewport> {
    let status = &dwg.status;
    let vd = dwg.view_direction;
    let vd_len = (vd.x * vd.x + vd.y * vd.y + vd.z * vd.z).sqrt();
    let valid_dir = vd_len.is_finite() && vd_len > TOLERANCE;
    let top = (vd.x.abs() <= TOLERANCE) && (vd.y.abs() <= TOLERANCE) && vd.z > 0.0;
    let at_origin = [dwg.view_target.x, dwg.view_target.y, dwg.view_target.z]
        .iter()
        .all(|v| v.abs() <= TOLERANCE);
    let drawn = dwg.is_on()
        && !status.perspective
        && !status.front_clipping
        && !status.back_clipping
        && status.non_rectangular_clipping == clip.is_some()
        && dwg.clip_boundary_handle.is_null() == clip.is_none()
        && valid_dir
        && (at_origin || !top)
        && dwg.twist_angle.is_finite()
        && dwg.render_mode == ViewportRenderMode::Wireframe2D
        && dwg.width > 0.0
        && dwg.height > 0.0
        && dwg.view_height > 0.0
        && dwg.center.z == 0.0;
    if !drawn {
        return None;
    }
    let scale = Viewport::scale_of(dwg.height, dwg.view_height);
    let view_direction = (!top).then(|| Point3::new(vd.x, vd.y, vd.z));
    let view_center = match view_direction {
        Some(dir) => {
            let ov = crate::geometry::solid3d::OrthoView::from_direction(dir);
            Point3::new(
                dwg.view_target.x + ov.right[0] * dwg.view_center.x + ov.up[0] * dwg.view_center.y,
                dwg.view_target.y + ov.right[1] * dwg.view_center.x + ov.up[1] * dwg.view_center.y,
                dwg.view_target.z + ov.right[2] * dwg.view_center.x + ov.up[2] * dwg.view_center.y,
            )
        }
        None => Point3::new(dwg.view_center.x, dwg.view_center.y, 0.0),
    };
    let rotation = if dwg.twist_angle == 0.0 {
        0.0
    } else {
        let deg = crate::description::normalize_degrees(dwg.twist_angle.to_degrees());
        let snapped = deg.round();
        if (deg - snapped).abs() < 1e-9 && snapped % 90.0 == 0.0 {
            crate::description::normalize_degrees(snapped)
        } else {
            deg
        }
    };
    let viewport = Viewport {
        center: Point3::new(dwg.center.x, dwg.center.y, 0.0),
        width: dwg.width,
        height: dwg.height,
        view_center,
        scale,
        rotation,
        view_direction,
        annotation_scale: super::ids::read_viewport_annoscale(&dwg.common),
        locked: status.locked,
        clip,
        frozen_layers,
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
