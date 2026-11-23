//! Wipeout object (#78): AutoCAD's `WIPEOUT`, a closed polygon that hides
//! everything drawn underneath it, with drawing-wide frame visibility
//! (`WIPEOUTFRAME` / `WIPEOUTVARIABLES`).

use serde::{Deserialize, Serialize};

use super::{Point3, Polyline, Properties};

/// How a drawing displays and plots wipeout frames, matching AutoCAD's
/// `WIPEOUTFRAME` system variable (`WIPEOUTVARIABLES.display_frame`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WipeoutFrame {
    /// Frames are neither displayed nor plotted (`WIPEOUTFRAME = 0`).
    #[default]
    Off,
    /// Frames are both displayed and plotted (`WIPEOUTFRAME = 1`).
    On,
    /// Frames are displayed on screen (viewer) but not plotted (`WIPEOUTFRAME = 2`).
    Display,
}

impl WipeoutFrame {
    /// AutoCAD's integer `WIPEOUTFRAME` / `WIPEOUTVARIABLES.display_frame` code.
    pub fn dwg_code(self) -> i16 {
        match self {
            WipeoutFrame::Off => 0,
            WipeoutFrame::On => 1,
            WipeoutFrame::Display => 2,
        }
    }

    /// Reads AutoCAD's integer `WIPEOUTFRAME` / `WIPEOUTVARIABLES.display_frame` code.
    pub fn from_dwg_code(code: i16) -> Self {
        match code {
            0 => WipeoutFrame::Off,
            2 => WipeoutFrame::Display,
            _ => WipeoutFrame::On,
        }
    }

    /// True when the frame is `Off` (the sheet description default).
    pub fn is_off(&self) -> bool {
        matches!(self, WipeoutFrame::Off)
    }

    /// The string name written in sheet descriptions and JSON Schema.
    pub fn as_str(self) -> &'static str {
        match self {
            WipeoutFrame::Off => "off",
            WipeoutFrame::On => "on",
            WipeoutFrame::Display => "display",
        }
    }
}

/// A wipeout object: a closed polygon of at least three coplanar points that
/// masks everything drawn underneath it (AutoCAD `WIPEOUT`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Wipeout {
    /// Closed polygon boundary vertices, sharing one `z`, enclosing non-zero area.
    pub points: Vec<Point3>,
    /// The layer the wipeout sits on; used for visibility and when the frame draws.
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

impl Wipeout {
    /// The boundary as a closed polyline, used for drawing the optional frame,
    /// computing object snaps, and geometric measurements.
    pub fn boundary_polyline(&self) -> Polyline {
        Polyline {
            points: self.points.clone(),
            closed: true,
            layer: self.layer.clone(),
            properties: self.properties.clone(),
            bulges: Vec::new(),
            width: 0.0,
            widths: Vec::new(),
        }
    }

    /// Signed 2D area of the boundary polygon (`> 0` anticlockwise, `< 0` clockwise).
    pub fn signed_area(&self) -> f64 {
        crate::model::revision_cloud::signed_area(&self.points)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wipeout_frame_dwg_codes_round_trip() {
        assert_eq!(WipeoutFrame::Off.dwg_code(), 0);
        assert_eq!(WipeoutFrame::On.dwg_code(), 1);
        assert_eq!(WipeoutFrame::Display.dwg_code(), 2);
        assert_eq!(WipeoutFrame::from_dwg_code(0), WipeoutFrame::Off);
        assert_eq!(WipeoutFrame::from_dwg_code(1), WipeoutFrame::On);
        assert_eq!(WipeoutFrame::from_dwg_code(2), WipeoutFrame::Display);
    }

    #[test]
    fn boundary_polyline_is_closed_and_preserves_area() {
        let w = Wipeout {
            points: vec![
                Point3::new(0.0, 0.0, 0.0),
                Point3::new(4.0, 0.0, 0.0),
                Point3::new(4.0, 2.0, 0.0),
                Point3::new(0.0, 2.0, 0.0),
            ],
            layer: "0".to_owned(),
            properties: Properties::default(),
        };
        assert!((w.signed_area().abs() - 8.0).abs() < 1e-12);
        let p = w.boundary_polyline();
        assert!(p.closed);
        assert_eq!(p.points.len(), 4);
    }
}
