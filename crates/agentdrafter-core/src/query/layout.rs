//! `query layout`: each sheet's paper-space layout, its page setup and its
//! viewports, with the part of model space each shows.

use serde::Serialize;

use super::{counted, measuring_objects, written_at, Answer, Extents, LookedAt, WrittenAt};
use crate::description::SheetRecord;
use crate::draw::Bounds;
use crate::dwg::{PageSetup, PLOTTER, PLOT_STYLE};
use crate::model::{Paper, Point3};
use crate::settings::Settings;
use crate::text::placement::SheetFonts;
use crate::units::Unit;

/// One sheet's layout.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SheetLayout {
    pub sheet: String,
    /// The layout's name; none for a sheet drawn in model space alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    /// The paper the sheet is drawn on, which the layout's page setup
    /// plots.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paper: Option<Paper>,
    /// How the layout plots, as its DWG's page setup holds it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_setup: Option<PageSetupUsed>,
    /// How many of the sheet's objects are in paper space, its viewports
    /// among them.
    pub paper_space: usize,
    /// How many of the sheet's objects are in model space: every one of
    /// them on a sheet with no layout.
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
    pub layer: String,
    /// The part of model space it shows: its lower left and upper right
    /// corners in model space.
    pub view: [[f64; 2]; 2],
}

/// Each sheet's layout and viewports.
pub(super) fn layout(
    sheets: &[&SheetRecord],
    settings: &Settings,
    units: Unit,
    looked: &LookedAt,
) -> (String, Answer) {
    let on = looked.on();
    let listed: Vec<SheetLayout> = sheets
        .iter()
        .map(|record| sheet_layout(record, settings))
        .collect();
    let count: usize = listed.iter().map(|s| s.viewports.len()).sum();
    let message = match listed.as_slice() {
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
            let drawn = all.iter().filter(|s| s.layout.is_some()).count();
            format!(
                "{drawn} of the {} sheets {on} {} drawn in a layout, with {} in all; \
                 lengths in {units}",
                all.len(),
                if drawn == 1 { "is" } else { "are" },
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

/// One sheet's layout and viewports.
fn sheet_layout(record: &SheetRecord, settings: &Settings) -> SheetLayout {
    let sheet = &record.sheet;
    let paper_space = (0..sheet.objects.len())
        .filter(|&i| sheet.in_paper_space(i))
        .count();
    let fonts = SheetFonts::find(sheet, settings);
    let own = measuring_objects(sheet, &fonts);
    let model_extents = Bounds::of(
        own.items
            .iter()
            .filter(|item| !sheet.in_paper_space(item.object))
            .flat_map(|item| item.points()),
    )
    .map(|b| Extents::of(&b));
    let page_setup = sheet.layout.as_ref().and_then(|_| {
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
    });
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
            layer: v.layer.clone(),
            view: v.view(),
        })
        .collect();
    SheetLayout {
        sheet: record.name.clone(),
        layout: sheet.layout.as_ref().map(|l| l.name.clone()),
        paper: sheet.paper.clone(),
        page_setup,
        paper_space,
        model_space: sheet.objects.len() - paper_space,
        model_extents,
        viewports,
    }
}
