//! Convert a drawing (`.dwg` or `.dxf`), a folder of drawings, or a drawing
//! set's built drawings to another DWG version (`SAVEAS` to a version).
//!
//! Converting is always an explicit step and never a side effect of a revision
//! (ADR-0006). An original drawing is never overwritten (ADR-0004): the output
//! must resolve to a distinct path, and every converted drawing reports any
//! features downgraded, dropped, or changed by the target version.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::build::{is_same_file, write_whole, DrawingFormat};
use crate::description::read_set_settings;
use crate::dwg;
use crate::error::{Error, ErrorCode};
use crate::model::DecodeReport;
use crate::original::is_drawing_file;
use crate::reply::Reply;
use crate::set::SET_SETTINGS_FILE;

/// A writable AutoCAD DWG / DXF version, ordered chronologically from R12 to
/// 2018.
///
/// Serializes as the canonical 6-character DWG header code (`AC1009`..`AC1032`)
/// and accepts either that code or its AutoCAD release name (`R12`, `R13`,
/// `R14`, `2000`, `2004`, `2007`, `2010`, `2013`, `2018`, as string or year
/// integer) when deserialized.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DwgVersion {
    /// AutoCAD R12 (`AC1009`).
    R12,
    /// AutoCAD R13 (`AC1012`).
    R13,
    /// AutoCAD R14 (`AC1014`).
    R14,
    /// AutoCAD 2000 / 2000i / 2002 (`AC1015`).
    R2000,
    /// AutoCAD 2004 / 2005 / 2006 (`AC1018`).
    R2004,
    /// AutoCAD 2007 / 2008 / 2009 (`AC1021`).
    R2007,
    /// AutoCAD 2010 / 2011 / 2012 (`AC1024`).
    R2010,
    /// AutoCAD 2013 / 2014 / 2015 / 2016 / 2017 (`AC1027`).
    R2013,
    /// AutoCAD 2018+ (`AC1032`) — the default for new sheets (ADR-0006).
    #[default]
    R2018,
}

impl DwgVersion {
    /// Every DWG version AgentDrafter can write, from oldest to newest.
    pub const ALL: [DwgVersion; 9] = [
        DwgVersion::R12,
        DwgVersion::R13,
        DwgVersion::R14,
        DwgVersion::R2000,
        DwgVersion::R2004,
        DwgVersion::R2007,
        DwgVersion::R2010,
        DwgVersion::R2013,
        DwgVersion::R2018,
    ];

    /// Canonical string values accepted in schemas (`AC10xx` codes and AutoCAD
    /// release names).
    pub const ACCEPTED_STRINGS: [&'static str; 18] = [
        "AC1009", "R12", "AC1012", "R13", "AC1014", "R14", "AC1015", "2000", "AC1018", "2004",
        "AC1021", "2007", "AC1024", "2010", "AC1027", "2013", "AC1032", "2018",
    ];

    /// Release year integers accepted for `dwg_version` in YAML/JSON.
    pub const ACCEPTED_YEARS: [u16; 6] = [2000, 2004, 2007, 2010, 2013, 2018];

    /// The 6-byte canonical DWG header code (`AC1009`..`AC1032`).
    pub const fn code(self) -> &'static str {
        match self {
            DwgVersion::R12 => "AC1009",
            DwgVersion::R13 => "AC1012",
            DwgVersion::R14 => "AC1014",
            DwgVersion::R2000 => "AC1015",
            DwgVersion::R2004 => "AC1018",
            DwgVersion::R2007 => "AC1021",
            DwgVersion::R2010 => "AC1024",
            DwgVersion::R2013 => "AC1027",
            DwgVersion::R2018 => "AC1032",
        }
    }

    /// The AutoCAD release name (`R12`, `R13`, `R14`, `2000`..`2018`).
    pub const fn release(self) -> &'static str {
        match self {
            DwgVersion::R12 => "R12",
            DwgVersion::R13 => "R13",
            DwgVersion::R14 => "R14",
            DwgVersion::R2000 => "2000",
            DwgVersion::R2004 => "2004",
            DwgVersion::R2007 => "2007",
            DwgVersion::R2010 => "2010",
            DwgVersion::R2013 => "2013",
            DwgVersion::R2018 => "2018",
        }
    }

    /// Lowercase tag used when deriving a default converted file or folder name
    /// (such as `panel.2010.dwg` or `converted-r14`).
    pub const fn file_tag(self) -> &'static str {
        match self {
            DwgVersion::R12 => "r12",
            DwgVersion::R13 => "r13",
            DwgVersion::R14 => "r14",
            DwgVersion::R2000 => "2000",
            DwgVersion::R2004 => "2004",
            DwgVersion::R2007 => "2007",
            DwgVersion::R2010 => "2010",
            DwgVersion::R2013 => "2013",
            DwgVersion::R2018 => "2018",
        }
    }

    /// Whether this is the default 2018 format (`AC1032`).
    pub const fn is_default(&self) -> bool {
        matches!(self, DwgVersion::R2018)
    }

    /// Parses a version code or release name, case-insensitively (`AC1024`,
    /// `2010`, `R2010`, `R14`, `r12`, etc.).
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_uppercase().as_str() {
            "AC1009" | "R12" | "12" => Some(DwgVersion::R12),
            "AC1012" | "R13" | "13" => Some(DwgVersion::R13),
            "AC1014" | "R14" | "14" => Some(DwgVersion::R14),
            "AC1015" | "R2000" | "2000" => Some(DwgVersion::R2000),
            "AC1018" | "R2004" | "2004" => Some(DwgVersion::R2004),
            "AC1021" | "R2007" | "2007" => Some(DwgVersion::R2007),
            "AC1024" | "R2010" | "2010" => Some(DwgVersion::R2010),
            "AC1027" | "R2013" | "2013" => Some(DwgVersion::R2013),
            "AC1032" | "R2018" | "2018" => Some(DwgVersion::R2018),
            _ => None,
        }
    }

    /// Strict parser matching [`Self::ACCEPTED_STRINGS`] (used by `set.yaml`
    /// so the YAML reader and JSON Schema agree on every value).
    pub fn parse_strict(text: &str) -> Option<Self> {
        match text {
            "AC1009" | "R12" => Some(DwgVersion::R12),
            "AC1012" | "R13" => Some(DwgVersion::R13),
            "AC1014" | "R14" => Some(DwgVersion::R14),
            "AC1015" | "2000" => Some(DwgVersion::R2000),
            "AC1018" | "2004" => Some(DwgVersion::R2004),
            "AC1021" | "2007" => Some(DwgVersion::R2007),
            "AC1024" | "2010" => Some(DwgVersion::R2010),
            "AC1027" | "2013" => Some(DwgVersion::R2013),
            "AC1032" | "2018" => Some(DwgVersion::R2018),
            _ => None,
        }
    }

    /// Parses one of the release year integers (`2000`, `2004`, `2007`,
    /// `2010`, `2013`, `2018`).
    pub const fn from_year(year: u64) -> Option<Self> {
        match year {
            2000 => Some(DwgVersion::R2000),
            2004 => Some(DwgVersion::R2004),
            2007 => Some(DwgVersion::R2007),
            2010 => Some(DwgVersion::R2010),
            2013 => Some(DwgVersion::R2013),
            2018 => Some(DwgVersion::R2018),
            _ => None,
        }
    }

    /// Human-readable summary of all accepted version codes and release names.
    pub fn list_choices() -> String {
        Self::ALL
            .iter()
            .map(|v| format!("{} ({})", v.code(), v.release()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl fmt::Display for DwgVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl Serialize for DwgVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.code())
    }
}

impl<'de> Deserialize<'de> for DwgVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = DwgVersion;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    "a DWG version code or release name ({})",
                    DwgVersion::list_choices()
                )
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                DwgVersion::parse(v).ok_or_else(|| {
                    E::custom(format!(
                        "unknown DWG version `{v}`; choose one of {}",
                        DwgVersion::list_choices()
                    ))
                })
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                DwgVersion::from_year(v).ok_or_else(|| {
                    E::custom(format!(
                        "unknown DWG version `{v}`; choose one of {}",
                        DwgVersion::list_choices()
                    ))
                })
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                u64::try_from(v)
                    .ok()
                    .and_then(DwgVersion::from_year)
                    .ok_or_else(|| {
                        E::custom(format!(
                            "unknown DWG version `{v}`; choose one of {}",
                            DwgVersion::list_choices()
                        ))
                    })
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

/// What happened to a drawing object, table entry, or feature during a version
/// conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionLossKind {
    /// A feature or property not supported by the target version is downgraded
    /// or approximated (for example, true color to ACI, transparency, or
    /// annotative scale metadata).
    Downgraded,
    /// An object, table entry, or non-graphical record was absent when the
    /// converted drawing was re-read.
    Dropped,
    /// An object or table entry differed structurally when the converted
    /// drawing was re-read.
    Changed,
    /// The converted drawing had a decode regression when re-read.
    Decode,
}

/// One loss or fidelity change reported for a converted drawing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConversionLoss {
    /// The DWG object, table, or feature kind (such as `PDFUNDERLAY`,
    /// `MULTILEADER`, `ACAD_TABLE`, `LAYER`, `TRUECOLOR`, or `DECODE`).
    pub kind: String,
    /// The record's hexadecimal handle, table name, or `"0"` for a drawing-wide
    /// setting.
    pub handle: String,
    /// Whether the feature was downgraded, dropped, changed, or failed to
    /// decode cleanly.
    pub change: ConversionLossKind,
    /// Short human-readable explanation of what was lost or changed.
    pub detail: String,
}

/// A request to convert a drawing (`.dwg` or `.dxf`), a folder of drawings, or
/// a drawing set's built drawings to another DWG version.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConvertRequest {
    /// The `.dwg` / `.dxf` file, folder of drawings, or drawing set folder
    /// (with `set.yaml`) to convert. Originals are never overwritten.
    pub source: PathBuf,
    /// Target DWG version (`AC1009`..`AC1032` or `R12`, `R13`, `R14`, `2000`,
    /// `2004`, `2007`, `2010`, `2013`, `2018`).
    pub version: DwgVersion,
    /// Output file path (for one drawing) or output folder (for a folder or
    /// drawing set). Without it, a single drawing writes beside the source as
    /// `<stem>.<release>.<ext>` (such as `panel.2010.dwg`), and a folder or
    /// drawing set writes into `<folder>/converted-<release>/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<PathBuf>,
    /// Output file format (`dwg`, `dxf`, or `dxf_binary`). Without it, inferred
    /// from `output`'s extension (`.dxf` -> `dxf`, `.dwg` -> `dwg`), or keeps
    /// the source drawing's format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<DrawingFormat>,
    /// Replace an existing file at `output`. Still refused if `output` resolves
    /// to `source` itself — an original is never overwritten (ADR-0004).
    #[serde(default)]
    pub overwrite: bool,
    /// Proceed even when the DWG library could not decode every record of the
    /// source drawing (ADR-0004). Without it, an incomplete decode is refused.
    #[serde(default)]
    pub allow_loss: bool,
    /// Run the full conversion and post-write re-read loss check in memory
    /// without writing any file.
    #[serde(default)]
    pub dry_run: bool,
}

/// Result of converting one drawing to a target DWG version.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConvertDrawingReport {
    /// The source drawing path.
    pub source: String,
    /// Where the converted drawing was written (or would be written on a dry
    /// run).
    pub output: String,
    /// Format of the source drawing (`dwg`, `dxf`, or `dxf_binary`).
    pub source_format: DrawingFormat,
    /// Format of the converted drawing (`dwg`, `dxf`, or `dxf_binary`).
    pub format: DrawingFormat,
    /// DWG version the source arrived in (such as `AC1032`).
    pub source_version: String,
    /// Target DWG version written (such as `AC1024`).
    pub target_version: String,
    /// True when no file was written (`dry_run: true`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub dry_run: bool,
    /// Size of the converted drawing in bytes.
    pub bytes: usize,
    /// Number of graphical objects (including owned attributes) in the source.
    pub total_objects: usize,
    /// Number of graphical objects preserved with matching geometry and
    /// properties when the converted output was re-read.
    pub unchanged_objects: usize,
    /// True when the source decoded cleanly, the output decoded cleanly, and no
    /// conversion losses were detected.
    pub lossless: bool,
    /// Features, objects, or table entries downgraded, dropped, or changed by
    /// the target version.
    pub losses: Vec<ConversionLoss>,
    /// Decode report of the source drawing.
    pub decode: DecodeReport,
    /// Decode report of the re-read converted drawing.
    pub output_decode: DecodeReport,
}

/// Outcome of one drawing inside a batch conversion.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConvertDrawingOutcome {
    /// Drawing file name (such as `DEMO-001.dwg`).
    pub name: String,
    /// Full path of the source drawing.
    pub source: String,
    /// Whether this drawing converted without error.
    pub ok: bool,
    /// The drawing's conversion report when `ok` is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ConvertDrawingReport>,
    /// Why this drawing failed when `ok` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
}

/// Result of converting a folder of drawings or a drawing set's built drawings.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConvertBatchReport {
    /// The source folder or drawing set folder.
    pub source: String,
    /// The output folder the converted drawings were written to (or would be
    /// written to on a dry run).
    pub output: String,
    /// Target DWG version written (such as `AC1024`).
    pub target_version: String,
    /// True when no files were written (`dry_run: true`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub dry_run: bool,
    /// Count of drawings converted.
    pub converted: usize,
    /// Count of drawings that failed to convert.
    pub failed: usize,
    /// Sum of reported losses across all converted drawings.
    pub total_losses: usize,
    /// Per-drawing conversion outcomes, in sorted file-name order.
    pub drawings: Vec<ConvertDrawingOutcome>,
}

/// Result of `convert`: one drawing or a batch of drawings.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ConvertReport {
    Drawing(Box<ConvertDrawingReport>),
    Batch(ConvertBatchReport),
}

/// Converts a single drawing, a folder of drawings, or a drawing set's built
/// drawings to `request.version`.
///
/// Originals are never overwritten. For a batch where some drawings succeed and
/// others fail, use [`convert_reply`] to preserve partial results alongside the
/// `sheets_failed` error.
pub fn convert(request: &ConvertRequest) -> Result<ConvertReport, Error> {
    if !request.source.exists() {
        return Err(Error::new(
            ErrorCode::ReadFailed,
            format!("no drawing or folder at `{}`", request.source.display()),
        )
        .in_file(&request.source));
    }
    if request.source.is_dir() {
        let batch = convert_folder(request)?;
        Ok(ConvertReport::Batch(batch))
    } else {
        let report = convert_single(request)?;
        Ok(ConvertReport::Drawing(Box::new(report)))
    }
}

/// Runs [`convert`] and wraps partial batch failures into a [`Reply`] carrying
/// every drawing's outcome alongside a `sheets_failed` error (matching
/// `build_set` and `plot_set`).
pub fn convert_reply(request: &ConvertRequest) -> Reply<ConvertReport> {
    match convert(request) {
        Ok(ConvertReport::Batch(batch)) if batch.failed > 0 => {
            let error = Error::new(
                ErrorCode::SheetsFailed,
                format!(
                    "{} of {} drawings failed to convert, and nothing was written for them; {} \
                     converted. Each drawing's outcome is under `drawings`",
                    batch.failed,
                    batch.drawings.len(),
                    batch.converted
                ),
            );
            Reply::partly(ConvertReport::Batch(batch), error)
        }
        other => Reply::from(other),
    }
}

fn convert_single(request: &ConvertRequest) -> Result<ConvertDrawingReport, Error> {
    let source = request.source.as_path();
    let bytes = fs::read(source).map_err(|e| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("could not read the drawing: {e}"),
        )
        .in_file(source)
    })?;
    let source_format = dwg::detect_drawing_format(&bytes, source);
    let target_format =
        resolve_convert_format(request.format, request.output.as_deref(), source_format)?;
    let output = match &request.output {
        Some(path) => path.clone(),
        None => default_output_file(source, request.version, target_format),
    };
    validate_distinct_output(source, &output, request.overwrite, request.dry_run)?;

    let converted = dwg::convert_drawing_bytes(
        &bytes,
        source,
        request.version,
        target_format,
        request.allow_loss,
    )
    .map_err(|e| e.or_in_file(source))?;

    if !request.dry_run {
        if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|e| {
                Error::new(
                    ErrorCode::WriteFailed,
                    format!("could not make the output folder: {e}"),
                )
                .in_file(parent)
            })?;
        }
        write_whole(&output, &converted.bytes)?;
    }

    let lossless =
        converted.losses.is_empty() && converted.decode.decoded && converted.output_decode.decoded;
    Ok(ConvertDrawingReport {
        source: source.display().to_string(),
        output: output.display().to_string(),
        source_format: converted.source_format,
        format: target_format,
        source_version: converted.source_version,
        target_version: request.version.code().to_owned(),
        dry_run: request.dry_run,
        bytes: converted.bytes.len(),
        total_objects: converted.total_objects,
        unchanged_objects: converted.unchanged_objects,
        lossless,
        losses: converted.losses,
        decode: converted.decode,
        output_decode: converted.output_decode,
    })
}

fn convert_folder(request: &ConvertRequest) -> Result<ConvertBatchReport, Error> {
    let source_root = request.source.as_path();
    let settings_file = source_root.join(SET_SETTINGS_FILE);
    let scan_dir = if settings_file.is_file() {
        let settings = read_set_settings(&settings_file)?;
        source_root.join(&settings.output)
    } else {
        source_root.to_path_buf()
    };
    if !scan_dir.is_dir() {
        return Err(Error::new(
            ErrorCode::ReadFailed,
            format!(
                "no built drawings folder at `{}`; build the drawing set before converting it",
                scan_dir.display()
            ),
        )
        .in_file(&scan_dir));
    }

    let out_dir = match &request.output {
        Some(dir) => dir.clone(),
        None => scan_dir.join(format!("converted-{}", request.version.file_tag())),
    };
    if fs::symlink_metadata(&out_dir).is_ok() {
        if !out_dir.is_dir() {
            return Err(Error::usage(format!(
                "the output path `{}` is an existing file, not a folder",
                out_dir.display()
            ))
            .in_file(&out_dir));
        }
        if is_same_file(&scan_dir, &out_dir) || is_same_file(source_root, &out_dir) {
            return Err(Error::usage(
                "the output folder is the source folder itself; originals are never overwritten, \
                 so choose another output folder",
            )
            .in_file(&out_dir));
        }
    }

    let mut entries: Vec<PathBuf> = fs::read_dir(&scan_dir)
        .map_err(|e| {
            Error::new(
                ErrorCode::ReadFailed,
                format!("could not read the folder: {e}"),
            )
            .in_file(&scan_dir)
        })?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_file() && is_drawing_file(path))
        .collect();
    entries.sort();

    if entries.is_empty() {
        return Err(Error::new(
            ErrorCode::ReadFailed,
            format!(
                "no `.dwg` or `.dxf` drawings found in `{}`",
                scan_dir.display()
            ),
        )
        .in_file(&scan_dir));
    }

    if !request.dry_run {
        fs::create_dir_all(&out_dir).map_err(|e| {
            Error::new(
                ErrorCode::WriteFailed,
                format!("could not make the output folder: {e}"),
            )
            .in_file(&out_dir)
        })?;
    }

    let mut drawings = Vec::with_capacity(entries.len());
    let mut converted_count = 0usize;
    let mut total_losses = 0usize;
    for path in entries {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("drawing")
            .to_owned();
        let target_ext = match request.format {
            Some(fmt) => fmt.extension(),
            None => path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| {
                    if e.eq_ignore_ascii_case("dxf") {
                        "dxf"
                    } else {
                        "dwg"
                    }
                })
                .unwrap_or("dwg"),
        };
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("drawing");
        let out_file = out_dir.join(format!("{stem}.{target_ext}"));
        let single_req = ConvertRequest {
            source: path.clone(),
            version: request.version,
            output: Some(out_file),
            format: request.format,
            overwrite: request.overwrite,
            allow_loss: request.allow_loss,
            dry_run: request.dry_run,
        };
        match convert_single(&single_req) {
            Ok(report) => {
                converted_count += 1;
                total_losses += report.losses.len();
                drawings.push(ConvertDrawingOutcome {
                    name,
                    source: path.display().to_string(),
                    ok: true,
                    result: Some(report),
                    error: None,
                });
            }
            Err(error) => {
                drawings.push(ConvertDrawingOutcome {
                    name,
                    source: path.display().to_string(),
                    ok: false,
                    result: None,
                    error: Some(error),
                });
            }
        }
    }
    let failed = drawings.len() - converted_count;
    Ok(ConvertBatchReport {
        source: source_root.display().to_string(),
        output: out_dir.display().to_string(),
        target_version: request.version.code().to_owned(),
        dry_run: request.dry_run,
        converted: converted_count,
        failed,
        total_losses,
        drawings,
    })
}

fn resolve_convert_format(
    explicit: Option<DrawingFormat>,
    output: Option<&Path>,
    source_format: DrawingFormat,
) -> Result<DrawingFormat, Error> {
    let ext = output
        .and_then(|p| p.extension())
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match (explicit, ext.as_deref()) {
        (Some(DrawingFormat::Dwg), Some("dxf")) => Err(Error::usage(
            "the output path ends `.dxf` while `format` is `dwg`; choose a `.dwg` path or ask \
             for `dxf` / `dxf_binary`",
        )
        .at_field("format")),
        (Some(f @ (DrawingFormat::Dxf | DrawingFormat::DxfBinary)), Some("dwg")) => {
            Err(Error::usage(format!(
                "the output path ends `.dwg` while `format` is `{}`; choose a `.dxf` path or ask \
                 for `dwg`",
                f.name()
            ))
            .at_field("format"))
        }
        (Some(f), _) => Ok(f),
        (None, Some("dxf")) => Ok(if source_format.is_binary_dxf() {
            DrawingFormat::DxfBinary
        } else {
            DrawingFormat::Dxf
        }),
        (None, Some("dwg")) => Ok(DrawingFormat::Dwg),
        (None, _) => Ok(source_format),
    }
}

fn default_output_file(source: &Path, version: DwgVersion, format: DrawingFormat) -> PathBuf {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("drawing");
    let file_name = format!("{stem}.{}.{}", version.file_tag(), format.extension());
    match source.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(file_name),
        _ => PathBuf::from(file_name),
    }
}

fn validate_distinct_output(
    source: &Path,
    output: &Path,
    overwrite: bool,
    dry_run: bool,
) -> Result<(), Error> {
    if source == output {
        return Err(Error::usage(
            "the output path is the source drawing itself; an original is never overwritten, so \
             choose another output path",
        )
        .in_file(output));
    }
    if fs::symlink_metadata(output).is_ok() {
        if is_same_file(source, output) {
            return Err(Error::usage(
                "the output path resolves to the source drawing itself; an original is never \
                 overwritten, so choose another output path",
            )
            .in_file(output));
        }
        if !overwrite && !dry_run {
            return Err(Error::new(
                ErrorCode::OutputExists,
                "a file is already at the output path; pass `overwrite` (`--overwrite`) to replace \
                 it or choose another output path",
            )
            .in_file(output));
        }
    }
    Ok(())
}
