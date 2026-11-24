//! Raster images (`IMAGE` / `IMAGEATTACH`) and PDF underlays (`PDFUNDERLAY` /
//! `PDFATTACH`) in AgentDrafter's core model (#91).

use serde::{Deserialize, Serialize};

use crate::model::{layer_zero, Point3, Properties};
use crate::units::Unit;

/// Default raster image brightness (`0..=100`): AutoCAD's default is 50.
pub const DEFAULT_IMAGE_BRIGHTNESS: u8 = 50;
/// Default raster image contrast (`0..=100`): AutoCAD's default is 50.
pub const DEFAULT_IMAGE_CONTRAST: u8 = 50;
/// Default PDF underlay contrast (`0..=100`): AutoCAD's default is 100.
pub const DEFAULT_PDF_CONTRAST: u8 = 100;
/// Maximum PDF underlay fade (`0..=80`), matching AutoCAD.
pub const MAX_PDF_FADE: u8 = 80;

pub(crate) fn default_image_brightness() -> u8 {
    DEFAULT_IMAGE_BRIGHTNESS
}

pub(crate) fn is_default_image_brightness(v: &u8) -> bool {
    *v == DEFAULT_IMAGE_BRIGHTNESS
}

pub(crate) fn default_image_contrast() -> u8 {
    DEFAULT_IMAGE_CONTRAST
}

pub(crate) fn is_default_image_contrast(v: &u8) -> bool {
    *v == DEFAULT_IMAGE_CONTRAST
}

pub(crate) fn default_pdf_contrast() -> u8 {
    DEFAULT_PDF_CONTRAST
}

pub(crate) fn is_default_pdf_contrast(v: &u8) -> bool {
    *v == DEFAULT_PDF_CONTRAST
}

pub(crate) fn default_true() -> bool {
    true
}

pub(crate) fn is_true(v: &bool) -> bool {
    *v
}

pub(crate) fn is_false(v: &bool) -> bool {
    !*v
}

pub(crate) fn default_scale() -> f64 {
    1.0
}

pub(crate) fn is_one(v: &f64) -> bool {
    (*v - 1.0).abs() <= 1e-12
}

pub(crate) fn default_page() -> String {
    "1".to_owned()
}

pub(crate) fn is_default_page(p: &str) -> bool {
    p == "1"
}

/// An attached raster image (`IMAGE` / `IMAGEATTACH`): PNG, JPEG, BMP, GIF,
/// or TIFF placed at `position` (its lower-left corner), scaled to `width`
/// (and optional `height`; when omitted, preserving the image's natural pixel
/// aspect ratio) and rotated anticlockwise by `rotation` degrees.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RasterImage {
    /// Path to the raster image file (relative to the sheet/template/library
    /// folder in sheet descriptions, or as stored in a DWG/DXF `IMAGEDEF`).
    pub path: String,
    /// Lower-left corner in the sheet's declared unit.
    pub position: Point3,
    /// Displayed width in the sheet's declared unit (strictly positive).
    pub width: f64,
    /// Optional displayed height in the sheet's declared unit (strictly
    /// positive). When omitted, follows `width * height_px / width_px` once
    /// the image is loaded (or `width` if unresolved).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// Counter-clockwise rotation around `position`, in degrees `[0, 360)`.
    #[serde(default, skip_serializing_if = "crate::model::is_zero")]
    pub rotation: f64,
    /// Optional polygonal clip boundary in drawing coordinates (at least 3
    /// vertices, non-zero area).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<Vec<[f64; 2]>>,
    /// When `true` and `clip` is set, masks the inside of `clip` and shows the
    /// outside (AutoCAD's inverted clip boundary).
    #[serde(default, skip_serializing_if = "is_false")]
    pub clip_inverted: bool,
    /// Brightness (`0..=100`, default `50`).
    #[serde(
        default = "default_image_brightness",
        skip_serializing_if = "is_default_image_brightness"
    )]
    pub brightness: u8,
    /// Contrast (`0..=100`, default `50`).
    #[serde(
        default = "default_image_contrast",
        skip_serializing_if = "is_default_image_contrast"
    )]
    pub contrast: u8,
    /// Fade towards the background (`0..=100`, default `0`).
    #[serde(default, skip_serializing_if = "crate::model::page_setup::is_zero_u8")]
    pub fade: u8,
    /// Whether the image's alpha channel (transparency) is honoured
    /// (AutoCAD's `TRANSPARENCY_ON`, default `true`).
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub transparent: bool,
    /// Whether the image pixels are drawn (`SHOW_IMAGE`, default `true`).
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub show: bool,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

impl RasterImage {
    /// Displayed height in drawing units, using `natural_px` when `height` is
    /// omitted.
    pub fn effective_height(&self, natural_px: Option<(u32, u32)>) -> f64 {
        if let Some(h) = self.height {
            return h;
        }
        match natural_px {
            Some((w, h)) if w > 0 && h > 0 => self.width * (f64::from(h) / f64::from(w)),
            _ => self.width,
        }
    }

    /// Full horizontal (`u`) and vertical (`v`) span vectors in 2D drawing
    /// units from `position` along the image's bottom and left edges.
    pub fn axes(&self, natural_px: Option<(u32, u32)>) -> ([f64; 2], [f64; 2]) {
        let h = self.effective_height(natural_px);
        let rad = self.rotation.to_radians();
        let (sin, cos) = rad.sin_cos();
        ([self.width * cos, self.width * sin], [-h * sin, h * cos])
    }

    /// The four corners in counter-clockwise order: bottom-left, bottom-right,
    /// top-right, top-left.
    pub fn corners(&self, natural_px: Option<(u32, u32)>) -> [[f64; 2]; 4] {
        let o = [self.position.x, self.position.y];
        let (u, v) = self.axes(natural_px);
        [
            o,
            [o[0] + u[0], o[1] + u[1]],
            [o[0] + u[0] + v[0], o[1] + u[1] + v[1]],
            [o[0] + v[0], o[1] + v[1]],
        ]
    }

    /// The outer boundary polygon (the clip boundary when clipped outside, or
    /// the four image corners).
    pub fn effective_outline(&self, natural_px: Option<(u32, u32)>) -> Vec<[f64; 2]> {
        if !self.clip_inverted {
            if let Some(clip) = &self.clip {
                if clip.len() >= 3 {
                    return clip.clone();
                }
            }
        }
        self.corners(natural_px).to_vec()
    }
}

/// An attached PDF underlay (`PDFUNDERLAY` / `PDFATTACH`): one page of a PDF
/// document placed at `position` (its lower-left corner), at `scale` times its
/// physical page size in the sheet's units, rotated anticlockwise by
/// `rotation` degrees.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PdfUnderlay {
    /// Path to the PDF file (relative to the sheet/template/library folder in
    /// sheet descriptions, or as stored in a DWG/DXF `PDFDEFINITION`).
    pub path: String,
    /// 1-based page number (as a string, matching AutoCAD `PDFDEFINITION`
    /// `page_name`; default `"1"`).
    #[serde(default = "default_page", skip_serializing_if = "is_default_page")]
    pub page: String,
    /// Lower-left corner in the sheet's declared unit.
    pub position: Point3,
    /// Scale factor applied to the PDF page's physical size (strictly
    /// positive, default `1.0`).
    #[serde(default = "default_scale", skip_serializing_if = "is_one")]
    pub scale: f64,
    /// Counter-clockwise rotation around `position`, in degrees `[0, 360)`.
    #[serde(default, skip_serializing_if = "crate::model::is_zero")]
    pub rotation: f64,
    /// Optional polygonal clip boundary in drawing coordinates (at least 3
    /// vertices, non-zero area).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<Vec<[f64; 2]>>,
    /// When `true` and `clip` is set, masks the inside of `clip` and shows the
    /// outside.
    #[serde(default, skip_serializing_if = "is_false")]
    pub clip_inverted: bool,
    /// Contrast (`0..=100`, default `100`).
    #[serde(
        default = "default_pdf_contrast",
        skip_serializing_if = "is_default_pdf_contrast"
    )]
    pub contrast: u8,
    /// Fade towards the background (`0..=80`, default `0`).
    #[serde(default, skip_serializing_if = "crate::model::page_setup::is_zero_u8")]
    pub fade: u8,
    /// When `true`, renders the PDF page in monochrome (grayscale).
    #[serde(default, skip_serializing_if = "is_false")]
    pub monochrome: bool,
    /// Whether the underlay is visible (`UnderlayDisplayFlags::ON`, default `true`).
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub show: bool,
    /// Whether the underlay definition is unloaded (kept in DWG without
    /// loading or drawing the PDF file; default `false`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub unloaded: bool,
    #[serde(default = "layer_zero")]
    pub layer: String,
    #[serde(flatten)]
    pub properties: Properties,
}

impl PdfUnderlay {
    /// Physical width and height of the PDF page in `sheet_unit`, scaled by
    /// `self.scale`. A PDF point is 1/72 inch. When `page_pt` is `None` (the
    /// PDF could not be resolved or is unloaded), falls back to `scale x scale`.
    pub fn size_in_units(&self, sheet_unit: Unit, page_pt: Option<(f64, f64)>) -> (f64, f64) {
        let Some((w_pt, h_pt)) = page_pt else {
            return (self.scale, self.scale);
        };
        let units_per_inch = Unit::In.convert(1.0, sheet_unit);
        let factor = (units_per_inch / 72.0) * self.scale;
        (w_pt * factor, h_pt * factor)
    }

    /// Full horizontal (`u`) and vertical (`v`) span vectors in 2D drawing
    /// units from `position` along the underlay's bottom and left edges.
    pub fn axes(&self, sheet_unit: Unit, page_pt: Option<(f64, f64)>) -> ([f64; 2], [f64; 2]) {
        let (w, h) = self.size_in_units(sheet_unit, page_pt);
        let rad = self.rotation.to_radians();
        let (sin, cos) = rad.sin_cos();
        ([w * cos, w * sin], [-h * sin, h * cos])
    }

    /// The four corners in counter-clockwise order: bottom-left, bottom-right,
    /// top-right, top-left.
    pub fn corners(&self, sheet_unit: Unit, page_pt: Option<(f64, f64)>) -> [[f64; 2]; 4] {
        let o = [self.position.x, self.position.y];
        let (u, v) = self.axes(sheet_unit, page_pt);
        [
            o,
            [o[0] + u[0], o[1] + u[1]],
            [o[0] + u[0] + v[0], o[1] + u[1] + v[1]],
            [o[0] + v[0], o[1] + v[1]],
        ]
    }

    /// The outer boundary polygon (the clip boundary when clipped outside, or
    /// the four underlay corners).
    pub fn effective_outline(
        &self,
        sheet_unit: Unit,
        page_pt: Option<(f64, f64)>,
    ) -> Vec<[f64; 2]> {
        if !self.clip_inverted {
            if let Some(clip) = &self.clip {
                if clip.len() >= 3 {
                    return clip.clone();
                }
            }
        }
        self.corners(sheet_unit, page_pt).to_vec()
    }
}

/// Kind of external media attachment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    PdfUnderlay,
}

impl MediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MediaKind::Image => "image",
            MediaKind::PdfUnderlay => "pdf_underlay",
        }
    }
}

/// A resolved raster image or PDF underlay page held in memory on a [`crate::model::Sheet`]
/// for drawing, querying and checking without touching the file system inside
/// the drawing pipeline (ADR-0011).
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedMedia {
    pub kind: MediaKind,
    pub path: String,
    pub page: Option<String>,
    pub width_px: u32,
    pub height_px: u32,
    /// Page size in PDF points (`1/72` in), set for PDF underlays.
    pub page_pt: Option<(f64, f64)>,
    /// Canonical straight-RGBA8 PNG bytes.
    pub png_bytes: Vec<u8>,
}

/// A raster image or PDF underlay that could not be found or decoded from disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedMedia {
    pub kind: MediaKind,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    pub reason: String,
}
