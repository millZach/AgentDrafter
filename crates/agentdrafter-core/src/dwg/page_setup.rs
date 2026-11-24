//! Named plot styles (`ACAD_PLOTSTYLENAME`, `PSTYLEMODE`), layer/entity
//! transparency, and DWG `Layout` page setup reading & writing behind the
//! `dwg` seam (ADR-0009).

use std::collections::HashMap;

use acadrust::entities::EntityCommon;
use acadrust::objects::{Layout as DwgLayout, ObjectType, PlaceHolder, PlotFlags};
use acadrust::tables::Layer as DwgLayer;
use acadrust::types::{Handle, Transparency};
use acadrust::CadDocument;

use super::layout::{PageSetup as StandardMedia, PLOTTER, PLOT_STYLE};
use crate::model::{
    Layer, ObjectPlotStyle, ObjectTransparency, PageSetup, Paper, PlotArea, PlotScale,
    MAX_TRANSPARENCY, NORMAL_PLOT_STYLE,
};
use crate::units::Unit;

/// AutoCAD's `AcCmTransparency` extended-data application name used to store
/// layer transparency in DWG.
pub(super) const LAYER_TRANSPARENCY_APP: &str = "AcCmTransparency";

/// PlotFlags bit 15 (`0x8000`): AutoCAD's `kPlotTransparency` flag.
pub(super) const PLOT_TRANSPARENCY_BIT: i32 = 0x8000;

/// Builds a map from `ACDBPLACEHOLDER` handle to plot style name from the
/// document's `ACAD_PLOTSTYLENAME` dictionary.
pub(super) fn plot_style_handle_map(drawing: &CadDocument) -> HashMap<Handle, String> {
    let mut map = HashMap::new();
    let dict_handle = drawing.header.acad_plotstylename_dict_handle;
    if dict_handle.is_null() {
        return map;
    }
    match drawing.objects.get(&dict_handle) {
        Some(ObjectType::DictionaryWithDefault(dict)) => {
            for (name, handle) in &dict.entries {
                map.insert(*handle, name.clone());
            }
        }
        Some(ObjectType::Dictionary(dict)) => {
            for (name, handle) in &dict.entries {
                map.insert(*handle, name.clone());
            }
        }
        _ => {}
    }
    map
}

/// Returns the handle of the `"Normal"` placeholder in `ACAD_PLOTSTYLENAME`,
/// or `Handle::NULL` if none exists.
pub(super) fn normal_plot_style_handle(drawing: &CadDocument) -> Handle {
    let dict_handle = drawing.header.acad_plotstylename_dict_handle;
    if dict_handle.is_null() {
        return Handle::NULL;
    }
    match drawing.objects.get(&dict_handle) {
        Some(ObjectType::DictionaryWithDefault(dict)) => dict
            .entries
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(NORMAL_PLOT_STYLE))
            .map(|(_, h)| *h)
            .unwrap_or(dict.default_handle),
        Some(ObjectType::Dictionary(dict)) => dict
            .entries
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(NORMAL_PLOT_STYLE))
            .map(|(_, h)| *h)
            .unwrap_or(Handle::NULL),
        _ => Handle::NULL,
    }
}

/// Finds or creates an `ACDBPLACEHOLDER` entry for `name` in
/// `ACAD_PLOTSTYLENAME` and returns `(handle, changed_dict_handle, added_placeholder_handle)`.
pub(super) fn ensure_plot_style_tracked(
    drawing: &mut CadDocument,
    name: &str,
) -> (Handle, Option<u64>, Option<u64>) {
    let dict_handle = drawing.header.acad_plotstylename_dict_handle;
    if dict_handle.is_null() {
        return (Handle::NULL, None, None);
    }
    if let Some(existing) = match drawing.objects.get(&dict_handle) {
        Some(ObjectType::DictionaryWithDefault(dict)) => dict
            .entries
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, h)| *h),
        Some(ObjectType::Dictionary(dict)) => dict
            .entries
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, h)| *h),
        _ => None,
    } {
        return (existing, None, None);
    }
    let handle = drawing.allocate_handle();
    let mut ph = PlaceHolder::new();
    ph.handle = handle;
    ph.owner = dict_handle;
    drawing.objects.insert(handle, ObjectType::PlaceHolder(ph));
    match drawing.objects.get_mut(&dict_handle) {
        Some(ObjectType::DictionaryWithDefault(dict)) => {
            dict.entries.push((name.to_owned(), handle));
        }
        Some(ObjectType::Dictionary(dict)) => {
            dict.entries.push((name.to_owned(), handle));
        }
        _ => {}
    }
    (handle, Some(dict_handle.value()), Some(handle.value()))
}

/// Finds or creates an `ACDBPLACEHOLDER` entry for `name` in
/// `ACAD_PLOTSTYLENAME` and returns its handle.
pub(super) fn ensure_plot_style_handle(drawing: &mut CadDocument, name: &str) -> Handle {
    ensure_plot_style_tracked(drawing, name).0
}

/// Assigns `plotstyle_handle` and `plot_style` on every layer of `drawing`
/// for an STB drawing (`PSTYLEMODE = 0`), linking each layer to its named
/// `ACDBPLACEHOLDER` (or `"Normal"` when unspecified).
pub(super) fn assign_layer_plot_styles(drawing: &mut CadDocument, layers: &[Layer]) {
    let normal = normal_plot_style_handle(drawing);
    let layer_styles: Vec<(String, Option<String>)> = drawing
        .layers
        .iter()
        .map(|dwg_layer| {
            let named = layers
                .iter()
                .find(|l| l.name.eq_ignore_ascii_case(&dwg_layer.name))
                .and_then(|l| l.plot_style.clone());
            (dwg_layer.name.clone(), named)
        })
        .collect();
    for (name, style) in layer_styles {
        let (style_name, handle) = match style {
            Some(s) if !s.eq_ignore_ascii_case(NORMAL_PLOT_STYLE) => {
                let h = ensure_plot_style_handle(drawing, &s);
                (s, h)
            }
            _ => (NORMAL_PLOT_STYLE.to_owned(), normal),
        };
        if let Some(dwg_layer) = drawing.layers.get_mut(&name) {
            dwg_layer.plot_style = style_name;
            dwg_layer.plotstyle_handle = handle;
        }
    }
}

/// Applies an object's named plot style onto `common` in an STB drawing.
pub(super) fn set_entity_plot_style(
    drawing: &mut CadDocument,
    common: &mut EntityCommon,
    style: &ObjectPlotStyle,
) {
    match style {
        ObjectPlotStyle::ByLayer => {
            common.plotstyle_flags = 0;
            common.plotstyle_handle = None;
        }
        ObjectPlotStyle::ByBlock => {
            common.plotstyle_flags = 1;
            common.plotstyle_handle = None;
        }
        ObjectPlotStyle::Named(name) => {
            let handle = ensure_plot_style_handle(drawing, name);
            common.plotstyle_flags = 3;
            common.plotstyle_handle = Some(handle);
        }
    }
}

/// Reads an entity's named plot style from `common` when the drawing is in
/// STB mode (`named_mode == true`).
pub(super) fn read_entity_plot_style(
    common: &EntityCommon,
    named_mode: bool,
    ps_map: &HashMap<Handle, String>,
) -> ObjectPlotStyle {
    if !named_mode {
        return ObjectPlotStyle::ByLayer;
    }
    match common.plotstyle_flags {
        1 => ObjectPlotStyle::ByBlock,
        2 => ObjectPlotStyle::Named(NORMAL_PLOT_STYLE.to_owned()),
        3 => common
            .plotstyle_handle
            .and_then(|h| ps_map.get(&h))
            .map(|name| ObjectPlotStyle::Named(name.clone()))
            .unwrap_or(ObjectPlotStyle::ByLayer),
        _ => ObjectPlotStyle::ByLayer,
    }
}

/// Reads a layer's named plot style when the drawing is in STB mode.
pub(super) fn read_layer_plot_style(
    dwg_layer: &DwgLayer,
    named_mode: bool,
    ps_map: &HashMap<Handle, String>,
) -> Option<String> {
    if !named_mode {
        return None;
    }
    let resolved = (!dwg_layer.plotstyle_handle.is_null())
        .then(|| ps_map.get(&dwg_layer.plotstyle_handle).cloned())
        .flatten()
        .or_else(|| (!dwg_layer.plot_style.is_empty()).then(|| dwg_layer.plot_style.clone()));
    resolved.filter(|s| !s.eq_ignore_ascii_case(NORMAL_PLOT_STYLE))
}

/// Converts a transparency percentage `0..=90` to `acadrust::types::Transparency`.
pub(super) fn dwg_transparency_from_percent(percent: u8) -> Transparency {
    if percent == 0 {
        Transparency::OPAQUE
    } else {
        Transparency::from_percent(f64::from(percent.min(MAX_TRANSPARENCY)) / 100.0)
    }
}

/// Converts `acadrust::types::Transparency` on a layer to a percentage `0..=90`.
pub(super) fn read_layer_transparency(t: Transparency) -> u8 {
    match t {
        Transparency::Explicit(alpha) if alpha > 0 => {
            ((f64::from(alpha) * 100.0 / 255.0).round() as u8).min(MAX_TRANSPARENCY)
        }
        _ => 0,
    }
}

/// Converts `acadrust::types::Transparency` on an entity to [`ObjectTransparency`].
pub(super) fn read_entity_transparency(t: Transparency) -> ObjectTransparency {
    match t {
        Transparency::ByLayer => ObjectTransparency::ByLayer,
        Transparency::ByBlock => ObjectTransparency::ByBlock,
        Transparency::Explicit(alpha) => ObjectTransparency::Percent(
            ((f64::from(alpha) * 100.0 / 255.0).round() as u8).min(MAX_TRANSPARENCY),
        ),
    }
}

/// Converts [`ObjectTransparency`] to `acadrust::types::Transparency`.
pub(super) fn dwg_entity_transparency(t: ObjectTransparency) -> Transparency {
    match t {
        ObjectTransparency::ByLayer => Transparency::ByLayer,
        ObjectTransparency::ByBlock => Transparency::ByBlock,
        ObjectTransparency::Percent(p) => dwg_transparency_from_percent(p),
    }
}

/// Writes `setup` onto a DWG `Layout` object (either a paper-space layout or
/// the `"Model"` layout).
pub(super) fn apply_page_setup(
    dwg_layout: &mut DwgLayout,
    setup: &PageSetup,
    fallback_paper: Option<&Paper>,
    units: Unit,
    is_model: bool,
) {
    let plotter = setup.plotter.as_deref().unwrap_or(PLOTTER);
    plotter.clone_into(&mut dwg_layout.plot_printer_name);

    let effective_paper = setup.paper.as_ref().or(fallback_paper);
    let paper_unit = if let Some(paper) = effective_paper {
        if let Some(std_media) = paper.name.as_deref().and_then(StandardMedia::of) {
            dwg_layout.paper_size = std_media.media;
            (dwg_layout.paper_width, dwg_layout.paper_height) = std_media.size_mm;
            dwg_layout.plot_paper_units = i16::from(std_media.unit != Unit::In);
            std_media.unit
        } else {
            let w_mm = units.convert(paper.width, Unit::Mm);
            let h_mm = units.convert(paper.height, Unit::Mm);
            let pu = if units == Unit::In {
                Unit::In
            } else {
                Unit::Mm
            };
            dwg_layout.paper_width = w_mm;
            dwg_layout.paper_height = h_mm;
            dwg_layout.plot_paper_units = i16::from(pu != Unit::In);
            dwg_layout.paper_size = if pu == Unit::In {
                format!(
                    "Custom_({:.2}_x_{:.2}_Inches)",
                    Unit::Mm.convert(w_mm, Unit::In),
                    Unit::Mm.convert(h_mm, Unit::In)
                )
            } else {
                format!("Custom_({w_mm:.2}_x_{h_mm:.2}_MM)")
            };
            pu
        }
    } else {
        let pu = if units == Unit::In {
            Unit::In
        } else {
            Unit::Mm
        };
        dwg_layout.plot_paper_units = i16::from(pu != Unit::In);
        pu
    };

    dwg_layout.plot_rotation = 0;
    dwg_layout.plot_margin_left = setup.margins_mm[0];
    dwg_layout.plot_margin_bottom = setup.margins_mm[1];
    dwg_layout.plot_margin_right = setup.margins_mm[2];
    dwg_layout.plot_margin_top = setup.margins_mm[3];

    match setup.area {
        Some(PlotArea::Extents) => {
            dwg_layout.plot_type = 1;
        }
        Some(PlotArea::Window { min, max }) => {
            dwg_layout.plot_type = 4;
            dwg_layout.plot_window_min_x = min[0];
            dwg_layout.plot_window_min_y = min[1];
            dwg_layout.plot_window_max_x = max[0];
            dwg_layout.plot_window_max_y = max[1];
        }
        Some(PlotArea::Layout) => {
            dwg_layout.plot_type = 5;
        }
        None => {
            dwg_layout.plot_type = if is_model { 1 } else { 5 };
        }
    }

    let mut flags = PlotFlags {
        plot_centered: setup.center,
        plot_plot_styles: true,
        print_lineweights: true,
        model_type: is_model,
        ..PlotFlags::default()
    };
    if setup.plot_transparency {
        flags.unknown_bits |= PLOT_TRANSPARENCY_BIT;
    }

    match setup.scale {
        PlotScale::Fit => {
            flags.use_standard_scale = true;
            dwg_layout.plot_scale_type = 0; // ScaleToFit
            dwg_layout.plot_scale_numerator = 1.0;
            dwg_layout.plot_scale_denominator = 1.0;
            dwg_layout.plot_scale_factor = 1.0;
        }
        PlotScale::Ratio {
            numerator,
            denominator,
        } => {
            let (u_num, u_den) = match (paper_unit, units) {
                (Unit::In, Unit::Mm) => (1.0, 25.4),
                (Unit::Mm, Unit::In) => (25.4, 1.0),
                _ => (units.convert(1.0, paper_unit), 1.0),
            };
            let num = numerator * u_num;
            let den = denominator * u_den;
            dwg_layout.plot_scale_numerator = num;
            dwg_layout.plot_scale_denominator = den;
            dwg_layout.plot_scale_factor = num / den;
            let one_to_one = (num - den).abs() < 1e-12;
            flags.use_standard_scale = one_to_one;
            dwg_layout.plot_scale_type = if one_to_one { 16 } else { 0 };
        }
    }

    dwg_layout.plot_flags = flags;
    dwg_layout.plot_origin_x = units.convert(setup.offset[0], Unit::Mm);
    dwg_layout.plot_origin_y = units.convert(setup.offset[1], Unit::Mm);
    dwg_layout.plot_style_sheet = setup
        .plot_style
        .clone()
        .unwrap_or_else(|| PLOT_STYLE.to_owned());

    if !is_model {
        dwg_layout.min_extents = (1e20, 1e20, 1e20);
        dwg_layout.max_extents = (-1e20, -1e20, -1e20);
    }
}

/// Reads a configured page setup from a DWG `Layout` object (model space or
/// paper space) so `plot` obeys a plant drawing's paper, plot area
/// (`extents`, `window`, `layout`), scale (`fit` or ratio), centring,
/// margins, plot origin and plot style sheet.
///
/// Returns `None` when the layout has no usable page setup configured (for
/// example, AutoCAD's untouched blank `Model` tab with `none_device` or zero
/// paper dimensions and no plot style sheet).
pub(super) fn read_page_setup(
    layout: &DwgLayout,
    units: Option<Unit>,
    is_model: bool,
) -> Option<PageSetup> {
    let is_none_plotter = layout.plot_printer_name.is_empty()
        || layout.plot_printer_name.eq_ignore_ascii_case("none_device")
        || layout.plot_printer_name.eq_ignore_ascii_case("None");
    let has_paper = layout.paper_width.is_finite()
        && layout.paper_height.is_finite()
        && layout.paper_width > 0.0
        && layout.paper_height > 0.0;
    let has_style = !layout.plot_style_sheet.is_empty();
    let has_window = layout.plot_type == 4
        && layout.plot_window_min_x.is_finite()
        && layout.plot_window_min_y.is_finite()
        && layout.plot_window_max_x.is_finite()
        && layout.plot_window_max_y.is_finite()
        && (layout.plot_window_max_x - layout.plot_window_min_x).abs() > 0.0
        && (layout.plot_window_max_y - layout.plot_window_min_y).abs() > 0.0;

    if is_model && is_none_plotter && !has_style && !has_window {
        return None;
    }
    if !has_paper && !has_style && !has_window {
        return None;
    }

    let paper_unit = match layout.plot_paper_units {
        0 => Some(Unit::In),
        1 => Some(Unit::Mm),
        _ => None,
    };
    // A unitless DWG (`INSUNITS = 0`) with a 1:1 inch layout has paper-space
    // coordinates in inches while `Sheet.units` defaults to millimetres;
    // do not turn its 17 x 11 inch paper into a 17 x 11 mm page setup.
    if units.is_none()
        && paper_unit == Some(Unit::In)
        && layout.plot_type == 5
        && layout.plot_scale_numerator.is_finite()
        && layout.plot_scale_denominator.is_finite()
        && layout.plot_scale_denominator > 0.0
        && ((layout.plot_scale_numerator / layout.plot_scale_denominator) - 1.0).abs() <= 1e-6
    {
        return None;
    }
    let draw_unit = units.unwrap_or(Unit::Mm);

    let ((w_mm, h_mm), margins_mm) = match layout.plot_rotation {
        0 => (
            (layout.paper_width, layout.paper_height),
            [
                layout.plot_margin_left,
                layout.plot_margin_bottom,
                layout.plot_margin_right,
                layout.plot_margin_top,
            ],
        ),
        1 => (
            (layout.paper_height, layout.paper_width),
            [
                layout.plot_margin_bottom,
                layout.plot_margin_right,
                layout.plot_margin_top,
                layout.plot_margin_left,
            ],
        ),
        2 => (
            (layout.paper_width, layout.paper_height),
            [
                layout.plot_margin_right,
                layout.plot_margin_top,
                layout.plot_margin_left,
                layout.plot_margin_bottom,
            ],
        ),
        3 => (
            (layout.paper_height, layout.paper_width),
            [
                layout.plot_margin_top,
                layout.plot_margin_left,
                layout.plot_margin_bottom,
                layout.plot_margin_right,
            ],
        ),
        _ => return None,
    };
    let margins_ok = margins_mm.iter().all(|v| v.is_finite() && *v >= 0.0);
    let margins_mm = if margins_ok
        && has_paper
        && margins_mm[0] + margins_mm[2] < w_mm
        && margins_mm[1] + margins_mm[3] < h_mm
    {
        margins_mm
    } else {
        [0.0; 4]
    };

    let paper = if has_paper && (!is_none_plotter || !is_model) {
        let name = StandardMedia::named(&layout.paper_size).or_else(|| {
            crate::description::paper::paper_names()
                .into_iter()
                .find_map(|(n, pw, ph, own)| {
                    let sw = own.convert(pw, Unit::Mm);
                    let sh = own.convert(ph, Unit::Mm);
                    ((sw - w_mm).abs() <= 1e-3 && (sh - h_mm).abs() <= 1e-3).then_some(n)
                })
        });
        let (width, height) = if let Some(ref std_name) = name {
            crate::description::paper::paper_names()
                .into_iter()
                .find(|(n, ..)| n == std_name)
                .map(|(_, pw, ph, own)| (own.convert(pw, draw_unit), own.convert(ph, draw_unit)))
                .unwrap_or_else(|| {
                    (
                        Unit::Mm.convert(w_mm, draw_unit),
                        Unit::Mm.convert(h_mm, draw_unit),
                    )
                })
        } else {
            (
                Unit::Mm.convert(w_mm, draw_unit),
                Unit::Mm.convert(h_mm, draw_unit),
            )
        };
        Some(Paper {
            name,
            width,
            height,
        })
    } else {
        None
    };

    let area = match layout.plot_type {
        1 => Some(PlotArea::Extents),
        4 if has_window => Some(PlotArea::Window {
            min: [
                layout.plot_window_min_x.min(layout.plot_window_max_x),
                layout.plot_window_min_y.min(layout.plot_window_max_y),
            ],
            max: [
                layout.plot_window_min_x.max(layout.plot_window_max_x),
                layout.plot_window_min_y.max(layout.plot_window_max_y),
            ],
        }),
        5 if !is_model => Some(PlotArea::Layout),
        _ => None,
    };

    // Scale: AutoCAD uses `use_standard_scale == true && plot_scale_type == 0`
    // for ScaleToFit. Otherwise `plot_scale_numerator` is in `paper_unit` and
    // `plot_scale_denominator` is in drawing units.
    let scale = if layout.plot_flags.use_standard_scale && layout.plot_scale_type == 0 {
        PlotScale::Fit
    } else if layout.plot_scale_numerator.is_finite()
        && layout.plot_scale_denominator.is_finite()
        && layout.plot_scale_numerator > 0.0
        && layout.plot_scale_denominator > 0.0
    {
        let pu = paper_unit.unwrap_or(draw_unit);
        let paper_mm = pu.convert(layout.plot_scale_numerator, Unit::Mm);
        let draw_mm = draw_unit.convert(layout.plot_scale_denominator, Unit::Mm);
        let ratio = paper_mm / draw_mm;
        if (ratio - 1.0).abs() <= 1e-6 {
            PlotScale::one_to_one()
        } else {
            PlotScale::Ratio {
                numerator: paper_mm,
                denominator: draw_mm,
            }
        }
    } else {
        PlotScale::one_to_one()
    };

    let offset = if layout.plot_origin_x.is_finite() && layout.plot_origin_y.is_finite() {
        [
            Unit::Mm.convert(layout.plot_origin_x, draw_unit),
            Unit::Mm.convert(layout.plot_origin_y, draw_unit),
        ]
    } else {
        [0.0, 0.0]
    };

    let plot_transparency = (layout.plot_flags.unknown_bits & PLOT_TRANSPARENCY_BIT) != 0;

    Some(PageSetup {
        paper,
        area,
        scale,
        center: layout.plot_flags.plot_centered,
        offset,
        margins_mm,
        plot_style: has_style.then(|| layout.plot_style_sheet.clone()),
        plot_style_from_dwg: has_style,
        plot_transparency,
        plotter: (!is_none_plotter).then(|| layout.plot_printer_name.clone()),
    })
}
