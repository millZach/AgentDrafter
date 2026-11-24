//! Loading and decoding external raster images (`IMAGE`) and PDF underlays
//! (`PDFUNDERLAY`) from disk, plus WASM-safe PNG pixel helpers and image
//! adjustment math shared across PNG, PDF and WebGL2 outputs (ADR-0011).

#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
use crate::model::{MediaKind, Object, ResolvedMedia};
use crate::model::{Sheet, UnresolvedMedia};
use crate::stamp::SourceKind;

/// Maximum rasterised pixel dimension (width or height) for a PDF underlay
/// page so multi-foot E-size sheets stay crisp without exhausting memory.
#[cfg(not(target_arch = "wasm32"))]
const MAX_PDF_RENDER_PX: f32 = 2048.0;

/// Target DPI when rasterising a PDF underlay page into the display list.
#[cfg(not(target_arch = "wasm32"))]
const PDF_TARGET_DPI: f32 = 150.0;

/// What [`resolve_sheet_media`] found on disk for a sheet.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResolvedMediaReport {
    /// Files read from disk with their bytes, for output stamps (`agentdrafter stale`).
    pub(crate) files: Vec<(SourceKind, PathBuf, Vec<u8>)>,
    /// Attachments that could not be found or decoded.
    pub(crate) unresolved: Vec<UnresolvedMedia>,
}

/// Resolves every raster image and PDF underlay on `sheet` relative to
/// `host_file` (and `sheet.block_library`'s folder, if any).
pub(crate) fn resolve_sheet_media_for_host(
    sheet: &mut Sheet,
    host_file: &Path,
) -> ResolvedMediaReport {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let base = host_file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    dirs.push(base.to_path_buf());
    if let Some(lib) = &sheet.block_library {
        if let Some(lib_dir) = base.join(lib).parent() {
            if lib_dir != base {
                dirs.push(lib_dir.to_path_buf());
            }
        }
    }
    let dir_refs: Vec<&Path> = dirs.iter().map(PathBuf::as_path).collect();
    resolve_sheet_media(sheet, &dir_refs)
}

/// Resolves every [`Object::RasterImage`] and [`Object::PdfUnderlay`] on
/// `sheet` (including inside `sheet.blocks` and `sheet.resolved_xrefs`) from
/// disk, searching `search_dirs` in order (typically the sheet's folder and
/// any block-library folder).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn resolve_sheet_media(sheet: &mut Sheet, search_dirs: &[&Path]) -> ResolvedMediaReport {
    let mut report = ResolvedMediaReport::default();
    let mut requests: Vec<(MediaKind, String, Option<String>)> = Vec::new();
    let mut seen: HashSet<(MediaKind, String, Option<String>)> = HashSet::new();

    let mut collect = |objects: &[Object]| {
        for obj in objects {
            match obj {
                Object::RasterImage(img) => {
                    let key = (MediaKind::Image, img.path.clone(), None);
                    if seen.insert(key.clone()) {
                        requests.push(key);
                    }
                }
                Object::PdfUnderlay(pdf) if !pdf.unloaded => {
                    let key = (
                        MediaKind::PdfUnderlay,
                        pdf.path.clone(),
                        Some(pdf.page.clone()),
                    );
                    if seen.insert(key.clone()) {
                        requests.push(key);
                    }
                }
                _ => {}
            }
        }
    };

    collect(&sheet.objects);
    for block in &sheet.blocks {
        collect(&block.objects);
    }
    for block in &sheet.resolved_xrefs {
        collect(&block.objects);
    }

    sheet.resolved_media.clear();
    let mut seen_files: HashSet<PathBuf> = HashSet::new();

    for (kind, raw_path, page) in requests {
        let Some(disk_path) = find_media_file(&raw_path, search_dirs) else {
            report.unresolved.push(UnresolvedMedia {
                kind,
                path: raw_path,
                page,
                reason: "file not found".to_owned(),
            });
            continue;
        };
        let bytes = match std::fs::read(&disk_path) {
            Ok(b) => b,
            Err(e) => {
                report.unresolved.push(UnresolvedMedia {
                    kind,
                    path: raw_path,
                    page,
                    reason: format!("could not read file: {e}"),
                });
                continue;
            }
        };
        match kind {
            MediaKind::Image => match decode_image_to_png(&bytes) {
                Ok((width_px, height_px, png_bytes)) => {
                    if seen_files.insert(disk_path.clone()) {
                        report.files.push((SourceKind::Image, disk_path, bytes));
                    }
                    sheet.resolved_media.push(ResolvedMedia {
                        kind,
                        path: raw_path,
                        page: None,
                        width_px,
                        height_px,
                        page_pt: None,
                        png_bytes,
                    });
                }
                Err(reason) => {
                    report.unresolved.push(UnresolvedMedia {
                        kind,
                        path: raw_path,
                        page: None,
                        reason,
                    });
                }
            },
            MediaKind::PdfUnderlay => {
                let page_str = page.as_deref().unwrap_or("1");
                match render_pdf_page_to_png(&bytes, page_str) {
                    Ok((width_px, height_px, page_pt, png_bytes)) => {
                        if seen_files.insert(disk_path.clone()) {
                            report
                                .files
                                .push((SourceKind::PdfUnderlay, disk_path, bytes));
                        }
                        sheet.resolved_media.push(ResolvedMedia {
                            kind,
                            path: raw_path,
                            page,
                            width_px,
                            height_px,
                            page_pt: Some(page_pt),
                            png_bytes,
                        });
                    }
                    Err(reason) => {
                        report.unresolved.push(UnresolvedMedia {
                            kind,
                            path: raw_path,
                            page,
                            reason,
                        });
                    }
                }
            }
        }
    }

    report
}

/// Stub for `wasm32`: in WebAssembly there is no file system; the server sends
/// already-resolved media inside the display list.
#[cfg(target_arch = "wasm32")]
pub(crate) fn resolve_sheet_media(
    _sheet: &mut Sheet,
    _search_dirs: &[&Path],
) -> ResolvedMediaReport {
    ResolvedMediaReport::default()
}

/// Finds an attached image or PDF on disk relative to any of `search_dirs`
/// (normalising `\` to `/`), falling back to the plain file name beside the
/// host folder if a saved path moved (matching AutoCAD and `xref.rs`).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn find_media_file(raw_path: &str, search_dirs: &[&Path]) -> Option<PathBuf> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let direct = PathBuf::from(trimmed);
    if direct.is_absolute() && direct.is_file() {
        return Some(direct);
    }
    let normalized = trimmed.replace('\\', "/");
    let rel = Path::new(&normalized);
    for dir in search_dirs {
        let candidate = dir.join(rel);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if let Some(file_name) = Path::new(&normalized).file_name() {
        for dir in search_dirs {
            let fallback = dir.join(file_name);
            if fallback.is_file() {
                return Some(fallback);
            }
        }
    }
    None
}

/// Probes pixel dimensions `(width, height)` of a raster image on disk for
/// writing accurate `IMAGEDEF` / `IMAGE` records during `build`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn probe_image_dimensions(raw_path: &str, search_dirs: &[&Path]) -> Option<(u32, u32)> {
    let path = find_media_file(raw_path, search_dirs)?;
    let bytes = std::fs::read(&path).ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
    Some((img.width().max(1), img.height().max(1)))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn probe_image_dimensions(
    _raw_path: &str,
    _search_dirs: &[&Path],
) -> Option<(u32, u32)> {
    None
}

/// Decodes any supported raster image (PNG, JPEG, BMP, GIF, TIFF) into
/// canonical straight-RGBA8 PNG bytes.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn decode_image_to_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let decoded = image::load_from_memory(bytes)
        .map_err(|e| format!("could not decode raster image: {e}"))?;
    let rgba = decoded.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return Err("raster image has zero width or height".to_owned());
    }
    let png_bytes = encode_rgba_png(w, h, rgba.as_raw())?;
    Ok((w, h, png_bytes))
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) type RenderedPdfPage = (u32, u32, (f64, f64), Vec<u8>);

/// Renders one 1-based page of a PDF document into canonical straight-RGBA8
/// PNG bytes using `hayro` (pure Rust, transparent page background).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn render_pdf_page_to_png(
    bytes: &[u8],
    page_name: &str,
) -> Result<RenderedPdfPage, String> {
    use std::sync::Arc;

    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::hayro_syntax::Pdf;
    use hayro::{PixmapSettings, RenderCache, RenderSettings};

    let page_num: usize = page_name
        .trim()
        .parse()
        .ok()
        .filter(|&n: &usize| n >= 1)
        .ok_or_else(|| {
            format!("invalid PDF page `{page_name}` (expected a 1-based page number)")
        })?;

    let pdf = Pdf::new(Arc::new(bytes.to_vec()))
        .map_err(|e| format!("could not parse PDF underlay: {e:?}"))?;
    let pages = pdf.pages();
    if pages.is_empty() {
        return Err("PDF underlay has no pages".to_owned());
    }
    let page = pages.get(page_num - 1).ok_or_else(|| {
        format!(
            "PDF underlay has {} page(s), cannot load page {page_num}",
            pages.len()
        )
    })?;

    let (w_pt, h_pt) = page.render_dimensions();
    if !w_pt.is_finite() || !h_pt.is_finite() || w_pt <= 0.0 || h_pt <= 0.0 {
        return Err("PDF page has invalid dimensions".to_owned());
    }

    let base_scale = PDF_TARGET_DPI / 72.0;
    let max_dim = w_pt.max(h_pt);
    let scale = if max_dim * base_scale > MAX_PDF_RENDER_PX {
        (MAX_PDF_RENDER_PX / max_dim).max(1.0 / max_dim)
    } else {
        base_scale
    };

    let cache = RenderCache::new();
    let pixmap = hayro::render(
        page,
        &cache,
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            ..PixmapSettings::default()
        },
    );
    let w = u32::from(pixmap.width());
    let h = u32::from(pixmap.height());
    if w == 0 || h == 0 {
        return Err("PDF page rendered to empty image".to_owned());
    }

    // Unpremultiply vello_cpu's premultiplied RGBA8 into straight RGBA8.
    let premul = pixmap.data_as_u8_slice();
    let mut straight = vec![0u8; premul.len()];
    for (src, dst) in premul
        .as_chunks::<4>()
        .0
        .iter()
        .zip(straight.as_chunks_mut::<4>().0.iter_mut())
    {
        let a = src[3];
        if a == 0 {
            *dst = [0, 0, 0, 0];
        } else if a == 255 {
            *dst = *src;
        } else {
            let af = f32::from(a) / 255.0;
            dst[0] = (f32::from(src[0]) / af).round().clamp(0.0, 255.0) as u8;
            dst[1] = (f32::from(src[1]) / af).round().clamp(0.0, 255.0) as u8;
            dst[2] = (f32::from(src[2]) / af).round().clamp(0.0, 255.0) as u8;
            dst[3] = a;
        }
    }

    let png_bytes = encode_rgba_png(w, h, &straight)?;
    Ok((w, h, (f64::from(w_pt), f64::from(h_pt)), png_bytes))
}

/// Encodes straight RGBA8 pixels into canonical, deterministic PNG bytes
/// (no metadata/timestamps, byte-identical across platforms).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("could not write PNG header: {e}"))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| format!("could not write PNG pixels: {e}"))?;
    }
    Ok(out)
}

/// Decodes PNG bytes into `(width, height, straight_rgba8)`. Works on both
/// native and `wasm32-unknown-unknown` using the `png` crate.
pub fn decode_png_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let data = &buf[..info.buffer_size()];
    let w = info.width;
    let h = info.height;
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for c in data.as_chunks::<3>().0 {
                out.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for c in data.as_chunks::<2>().0 {
                out.extend_from_slice(&[c[0], c[0], c[0], c[1]]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for &g in data {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            out
        }
        png::ColorType::Indexed => return None,
    };
    Some((w, h, rgba))
}

/// Applies AutoCAD's brightness (`0..=100`, 50 neutral), contrast (`0..=100`,
/// 50 neutral for images or 100 neutral for PDFs — pass `contrast_neutral`),
/// fade (`0..=100`, blends towards `bg`), monochrome (luminance), and
/// transparency flag (`transparent == false` forces opaque alpha composited
/// over `bg`) onto straight-RGBA8 pixel data in place.
#[allow(clippy::too_many_arguments)]
pub fn apply_image_adjustments(
    rgba: &mut [u8],
    brightness: u8,
    contrast: u8,
    contrast_neutral: u8,
    fade: u8,
    monochrome: bool,
    transparent: bool,
    opacity_scale: f32,
    bg: [u8; 3],
) {
    let b_shift = (f32::from(brightness) - 50.0) / 50.0; // -1.0 .. +1.0
    let c_factor = if contrast_neutral == 100 {
        // PDF underlay contrast: 100 = full contrast, 0 = flat mid-gray
        f32::from(contrast) / 100.0
    } else {
        // Raster image contrast: 50 = 1.0, 0 = 0.0, 100 = 2.0
        f32::from(contrast) / 50.0
    };
    let fade_t = (f32::from(fade) / 100.0).clamp(0.0, 1.0);
    let bg_f = [
        f32::from(bg[0]) / 255.0,
        f32::from(bg[1]) / 255.0,
        f32::from(bg[2]) / 255.0,
    ];

    for px in rgba.as_chunks_mut::<4>().0 {
        let mut r = f32::from(px[0]) / 255.0;
        let mut g = f32::from(px[1]) / 255.0;
        let mut b = f32::from(px[2]) / 255.0;
        let mut a = f32::from(px[3]) / 255.0;

        if !transparent {
            r = r * a + bg_f[0] * (1.0 - a);
            g = g * a + bg_f[1] * (1.0 - a);
            b = b * a + bg_f[2] * (1.0 - a);
            a = 1.0;
        }

        if monochrome {
            let lum = 0.299 * r + 0.587 * g + 0.114 * b;
            r = lum;
            g = lum;
            b = lum;
        }

        if (c_factor - 1.0).abs() > 1e-4 {
            r = ((r - 0.5) * c_factor + 0.5).clamp(0.0, 1.0);
            g = ((g - 0.5) * c_factor + 0.5).clamp(0.0, 1.0);
            b = ((b - 0.5) * c_factor + 0.5).clamp(0.0, 1.0);
        }

        if b_shift.abs() > 1e-4 {
            r = (r + b_shift).clamp(0.0, 1.0);
            g = (g + b_shift).clamp(0.0, 1.0);
            b = (b + b_shift).clamp(0.0, 1.0);
        }

        if fade_t > 1e-4 {
            r = r * (1.0 - fade_t) + bg_f[0] * fade_t;
            g = g * (1.0 - fade_t) + bg_f[1] * fade_t;
            b = b * (1.0 - fade_t) + bg_f[2] * fade_t;
        }

        a = (a * opacity_scale).clamp(0.0, 1.0);

        px[0] = (r * 255.0).round().clamp(0.0, 255.0) as u8;
        px[1] = (g * 255.0).round().clamp(0.0, 255.0) as u8;
        px[2] = (b * 255.0).round().clamp(0.0, 255.0) as u8;
        px[3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
    }
}

const B64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard RFC 4648 base64 encoding (zero-dependency, WASM-safe).
pub fn encode_base64(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = if chunk.len() > 1 {
            u32::from(chunk[1])
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            u32::from(chunk[2])
        } else {
            0
        };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_TABLE[((n >> 18) & 0x3F) as usize] as char);
        out.push(B64_TABLE[((n >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_TABLE[((n >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_TABLE[(n & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Standard RFC 4648 base64 decoding (zero-dependency, WASM-safe).
pub fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let bytes = input.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let decode_char = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a' + 26)),
            b'0'..=b'9' => Some(u32::from(c - b'0' + 52)),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity((bytes.len() / 4) * 3);
    for chunk in bytes.as_chunks::<4>().0 {
        let v0 = decode_char(chunk[0])?;
        let v1 = decode_char(chunk[1])?;
        if chunk[2] == b'=' {
            if chunk[3] != b'=' {
                return None;
            }
            let n = (v0 << 18) | (v1 << 12);
            out.push(((n >> 16) & 0xFF) as u8);
        } else if chunk[3] == b'=' {
            let v2 = decode_char(chunk[2])?;
            let n = (v0 << 18) | (v1 << 12) | (v2 << 6);
            out.push(((n >> 16) & 0xFF) as u8);
            out.push(((n >> 8) & 0xFF) as u8);
        } else {
            let v2 = decode_char(chunk[2])?;
            let v3 = decode_char(chunk[3])?;
            let n = (v0 << 18) | (v1 << 12) | (v2 << 6) | v3;
            out.push(((n >> 16) & 0xFF) as u8);
            out.push(((n >> 8) & 0xFF) as u8);
            out.push((n & 0xFF) as u8);
        }
    }
    Some(out)
}
