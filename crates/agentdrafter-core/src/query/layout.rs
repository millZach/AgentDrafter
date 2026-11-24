//! `query layout`: each sheet's paper-space layouts, each with its page
//! setup and its viewports, with the part of model space each shows, in
//! the order of their tabs.

use serde::Serialize;

use super::{
    counted, measuring_drawing, units_words, written_at, Answer, Extents, LookedAt, WrittenAt,
};
use crate::description::SheetRecord;
use crate::dwg::{PageSetup, PLOTTER, PLOT_STYLE};
use crate::model::{Clip, Paper, Point3, Sheet};
use crate::settings::Settings;
use crate::text::placement::SheetFonts;
use crate::units::Unit;

/// One layout of a sheet, or a sheet drawn in model space alone.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SheetLayout {
    pub sheet: String,
    /// The layout's name; none for a sheet drawn in model space alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    /// The paper the layout is drawn on, which its page setup plots.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paper: Option<Paper>,
    /// The unit `paper`'s dimensions are in when the reply itself has no
    /// declared `units` (a unitless DWG). Omitted whenever the reply declares
    /// `units` (in which case `paper` is converted into the reply's `units`)
    /// or when `paper` is absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paper_unit: Option<Unit>,
    /// How the layout plots, as its DWG's page setup holds it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_setup: Option<PageSetupUsed>,
    /// How many of the sheet's objects are in the layout's paper space, its
    /// viewports among them.
    pub paper_space: usize,
    /// How many of the sheet's objects are in model space, which its
    /// layouts share: every one of them on a sheet with no layout.
    pub model_space: usize,
    /// The box round what model space draws, at full size; none when it
    /// draws nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_extents: Option<Extents>,
    /// The layout's viewports, in the order the sheet lists them.
    pub viewports: Vec<ViewportFound>,
}

/// How a layout plots, as its page setup holds it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PageSetupUsed {
    /// The plotter it plots on: AutoCAD's own PDF plotter.
    pub plotter: String,
    /// AutoCAD's name of the paper it plots on: its full-bleed paper of
    /// the sheet's size, laid the sheet's way.
    pub paper: String,
    /// What it plots: the layout, its whole paper.
    pub area: String,
    /// The plot scale, paper to drawing units, such as `1:1`: the paper
    /// space at full size.
    pub scale: String,
    /// The plot style it plots through.
    pub plot_style: String,
}

/// One viewport of a layout.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ViewportFound {
    /// Its ID, such as `PANEL-001/view-a`.
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(flatten)]
    pub written_at: WrittenAt,
    /// The middle of its window, on the paper.
    pub center: Point3,
    pub width: f64,
    pub height: f64,
    /// Its window's lower left and upper right corners, on the paper.
    pub corners: [[f64; 2]; 2],
    /// The point of model space at its middle.
    pub view_center: Point3,
    /// How long a length of model space draws on the paper, per unit.
    pub scale: f64,
    /// Its scale as a ratio, paper to model, such as `1:4`.
    pub ratio: String,
    /// Whether its view is locked.
    pub locked: bool,
    /// The boundary it is clipped to, when it is not rectangular: its
    /// window is the box round it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip: Option<Clip>,
    /// The layers frozen in it alone.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub frozen_layers: Vec<String>,
    pub layer: String,
    /// The part of model space it shows: its lower left and upper right
    /// corners in model space.
    pub view: [[f64; 2]; 2],
}

/// Each sheet's layout and viewports.
pub(super) fn layout(
    sheets: &[&SheetRecord],
    settings: &Settings,
    units: Option<Unit>,
    looked: &LookedAt,
) -> (String, Answer) {
    let on = looked.on();
    let listed: Vec<SheetLayout> = sheets
        .iter()
        .flat_map(|record| sheet_layouts(record, settings, units))
        .collect();
    let count: usize = listed.iter().map(|s| s.viewports.len()).sum();
    let one_sheet = sheets.len() == 1;
    let message = match listed.as_slice() {
        several if one_sheet && several.len() > 1 => format!(
            "sheet `{}` is drawn in {} layouts, which share its model space: {}",
            several[0].sheet,
            several.len(),
            several
                .iter()
                .map(|l| format!(
                    "`{}`{}, with {}",
                    l.layout.as_deref().unwrap_or_default(),
                    l.paper
                        .as_ref()
                        .and_then(|p| p.name.as_deref())
                        .map(|p| format!(" on {p}"))
                        .unwrap_or_default(),
                    counted(l.viewports.len(), "viewport"),
                ))
                .collect::<Vec<_>>()
                .join("; ")
        ),
        [one] => match &one.layout {
            Some(name) => format!(
                "sheet `{}` is drawn in layout `{name}`{}, with {}{}",
                one.sheet,
                one.paper
                    .as_ref()
                    .and_then(|p| p.name.as_deref())
                    .map(|p| format!(" on {p}"))
                    .unwrap_or_default(),
                counted(one.viewports.len(), "viewport"),
                if one.viewports.is_empty() {
                    String::new()
                } else {
                    format!(
                        ": {}",
                        one.viewports
                            .iter()
                            .map(|v| format!("`{}` at {}", v.id, v.ratio))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
            ),
            None => format!(
                "sheet `{}` is drawn in model space alone, with no layout",
                one.sheet
            ),
        },
        all => {
            let drawn = sheets.iter().filter(|s| s.sheet.layout.is_some()).count();
            let layouts = all.iter().filter(|s| s.layout.is_some()).count();
            let u = units_words(units);
            format!(
                "{drawn} of the {} sheets {on} {} drawn in a layout, with {} and {} in all; \
                 lengths in {u}",
                sheets.len(),
                if drawn == 1 { "is" } else { "are" },
                counted(layouts, "layout"),
                counted(count, "viewport"),
            )
        }
    };
    (
        message,
        Answer::Layout {
            count,
            layouts: listed,
        },
    )
}

/// All layouts of `record`, in tab order, including any inactive empty
/// configured layout tabs read from a DWG.
fn sheet_layouts(
    record: &SheetRecord,
    settings: &Settings,
    report_units: Option<Unit>,
) -> Vec<SheetLayout> {
    let mut entries: Vec<(usize, SheetLayout)> = record
        .sheet
        .pages()
        .iter()
        .enumerate()
        .map(|(layout_index, page)| {
            let tab_position = record
                .layout_setup(layout_index)
                .map_or(layout_index, |s| s.tab_position);
            (
                tab_position,
                sheet_layout(record, page, layout_index, settings, report_units),
            )
        })
        .collect();
    let extras = record.extra_layout_setups();
    if record.sheet.layout.is_some() && !extras.is_empty() {
        let (model_space, model_extents) = entries
            .first()
            .map(|(_, first)| (first.model_space, first.model_extents))
            .unwrap_or((0, None));
        for extra in extras {
            let (paper, paper_unit) = dwg_layout_paper(None, Some(&extra.setup), report_units);
            let page_setup = extra.setup.page_setup.as_ref().map(|ps| PageSetupUsed {
                plotter: ps.plotter.clone(),
                paper: ps.paper.clone(),
                area: ps.area.clone(),
                scale: ps.scale.clone(),
                plot_style: ps.plot_style.clone(),
            });
            entries.push((
                extra.setup.tab_position,
                SheetLayout {
                    sheet: record.name.clone(),
                    layout: Some(extra.name.clone()),
                    paper,
                    paper_unit,
                    page_setup,
                    paper_space: 0,
                    model_space,
                    model_extents,
                    viewports: Vec::new(),
                },
            ));
        }
        entries.sort_by_key(|(pos, _)| *pos);
    }
    entries.into_iter().map(|(_, layout)| layout).collect()
}

fn paper_in_units(paper: &Paper, from: Unit, to: Unit) -> Paper {
    if from == to {
        return paper.clone();
    }
    Paper {
        name: paper.name.clone(),
        width: from.convert(paper.width, to),
        height: from.convert(paper.height, to),
    }
}

/// Resolves a DWG layout's `paper` (converted into `report_units` whenever
/// the DWG declares a unit so every length in the reply matches `units`) and
/// its optional `paper_unit` tag (present only on unitless DWGs).
fn dwg_layout_paper(
    sheet_paper: Option<&Paper>,
    setup: Option<&crate::model::layout::DwgLayoutSetup>,
    report_units: Option<Unit>,
) -> (Option<Paper>, Option<Unit>) {
    if let Some(paper) = sheet_paper {
        let tag = report_units.is_none().then_some(Unit::Mm);
        return (Some(paper.clone()), tag);
    }
    let Some(setup) = setup else {
        return (None, None);
    };
    let (raw, raw_unit) = if let Some(cp) = &setup.check_paper {
        (&cp.paper, cp.unit)
    } else if let (Some(stored), Some(unit)) = (&setup.stored_paper, setup.paper_unit) {
        (stored, unit)
    } else {
        return (None, None);
    };
    match report_units {
        Some(target) => (Some(paper_in_units(raw, raw_unit, target)), None),
        None => (Some(raw.clone()), Some(raw_unit)),
    }
}

/// One layout of a sheet and its viewports: the layout `sheet` draws, the
/// sheet `record` holds as one of its layouts prints it.
fn sheet_layout(
    record: &SheetRecord,
    sheet: &Sheet,
    layout_index: usize,
    settings: &Settings,
    report_units: Option<Unit>,
) -> SheetLayout {
    let paper_space = (0..sheet.objects.len())
        .filter(|&i| sheet.in_paper_space(i))
        .count();
    // Model space's extents as `query extents` gives a sheet's: its objects
    // drawn on their own, a hatch's boundary and an insert's block box
    // among what they reach.
    let fonts = SheetFonts::find(sheet, settings);
    let mut model = sheet.clone();
    model.layout = None;
    model.other_layouts.clear();
    model.objects = sheet
        .objects
        .iter()
        .enumerate()
        .filter(|&(i, _)| sheet.in_model_space(i))
        .map(|(_, o)| o.clone())
        .collect();
    let model_space = model.objects.len();
    let model_extents = measuring_drawing(&model, &fonts)
        .extents()
        .map(|b| Extents::of(&b));
    let (paper, paper_unit, page_setup) = if record.is_dwg() && sheet.layout.is_some() {
        let setup = record.layout_setup(layout_index);
        let (paper, paper_unit) = dwg_layout_paper(sheet.paper.as_ref(), setup, report_units);
        let page_setup = setup
            .and_then(|s| s.page_setup.as_ref())
            .map(|ps| PageSetupUsed {
                plotter: ps.plotter.clone(),
                paper: ps.paper.clone(),
                area: ps.area.clone(),
                scale: ps.scale.clone(),
                plot_style: ps.plot_style.clone(),
            });
        (paper, paper_unit, page_setup)
    } else {
        let effective = sheet.effective_page_setup();
        let page_setup = if let Some(ps) = effective {
            let effective_paper = ps.paper.as_ref().or(sheet.paper.as_ref());
            let dwg_setup = effective_paper
                .and_then(|p| p.name.as_deref())
                .and_then(PageSetup::of);
            let media = dwg_setup
                .as_ref()
                .map(|s| s.media.clone())
                .or_else(|| {
                    effective_paper.map(|p| {
                        p.name
                            .clone()
                            .unwrap_or_else(|| format!("{} x {}", p.width, p.height))
                    })
                })
                .unwrap_or_default();
            let area = match ps.area {
                Some(crate::model::PlotArea::Extents) => "extents",
                Some(crate::model::PlotArea::Window { .. }) => "window",
                Some(crate::model::PlotArea::Layout) => "layout",
                None if sheet.layout.is_some() => "layout",
                None => "extents",
            }
            .to_owned();
            let scale = match ps.scale {
                crate::model::PlotScale::Fit => "fit".to_owned(),
                crate::model::PlotScale::Ratio {
                    numerator,
                    denominator,
                } => {
                    if sheet.layout.is_some()
                        && (numerator - 1.0).abs() < 1e-9
                        && (denominator - 1.0).abs() < 1e-9
                    {
                        if let Some(s) = dwg_setup.as_ref() {
                            let (n, d) = s.plot_scale(sheet.units);
                            format!("{n}:{d}")
                        } else {
                            "1:1".to_owned()
                        }
                    } else {
                        format!("{numerator}:{denominator}")
                    }
                }
            };
            Some(PageSetupUsed {
                plotter: ps.plotter.clone().unwrap_or_else(|| PLOTTER.to_owned()),
                paper: media,
                area,
                scale,
                plot_style: ps
                    .plot_style
                    .clone()
                    .unwrap_or_else(|| PLOT_STYLE.to_owned()),
            })
        } else {
            sheet.layout.as_ref().and_then(|_| {
                let paper = sheet.paper.as_ref()?;
                let setup = PageSetup::of(paper.name.as_deref()?)?;
                let (numerator, denominator) = setup.plot_scale(sheet.units);
                Some(PageSetupUsed {
                    plotter: PLOTTER.to_owned(),
                    paper: setup.media,
                    area: "layout".to_owned(),
                    scale: format!("{numerator}:{denominator}"),
                    plot_style: PLOT_STYLE.to_owned(),
                })
            })
        };
        let paper = effective
            .and_then(|ps| ps.paper.clone())
            .or_else(|| sheet.paper.clone());
        (paper, None, page_setup)
    };
    let viewports = sheet
        .viewports()
        .map(|(i, v)| ViewportFound {
            id: record.id(i),
            name: record.objects[i].name.clone(),
            written_at: written_at(record, i),
            center: v.center,
            width: v.width,
            height: v.height,
            corners: v.corners(),
            view_center: v.view_center,
            scale: v.scale,
            ratio: v.ratio(),
            locked: v.locked,
            clip: v.clip.clone(),
            frozen_layers: v.frozen_layers.clone(),
            layer: v.layer.clone(),
            view: v.view(),
        })
        .collect();
    SheetLayout {
        sheet: record.name.clone(),
        layout: sheet.layout.as_ref().map(|l| l.name.clone()),
        paper,
        paper_unit,
        page_setup,
        paper_space,
        model_space,
        model_extents,
        viewports,
    }
}
