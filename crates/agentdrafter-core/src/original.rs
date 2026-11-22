//! Reads an original whole: a DWG someone else drew, such as a plant's
//! drawing, which AgentDrafter never writes to (ADR-0004).
//!
//! A read says, for the drawing, whether the DWG library decoded every
//! record of the file, and names by kind, count and handle each it did not
//! ([`DecodeReport`]); it names in the same way each object of the sheet
//! the core has no model for, which no output draws. Every object the core
//! reads gets an ID an agent can name it by: the ID AgentDrafter's own
//! extended data on it gives, on a drawing AgentDrafter built, and
//! otherwise the drawing's name joined to its handle, such as
//! `P-101/handle:2F`. A handle is the DWG's own name for a record, so the
//! ID is the same each time the same file is read.
//!
//! The read writes the drawing's text mirror beside it (ADR-0003): a
//! readable text rendering of what the core read, for reading and
//! comparing, which is never the record. Editing it changes nothing. It is
//! written whole or not at all, and replaces only a mirror AgentDrafter
//! wrote, as its first line shows; any other file may be someone's own,
//! and the original itself is never written.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::description::{number_text, ID_SEPARATOR};
use crate::dwg;
use crate::error::{Error, ErrorCode};
use crate::model::{DrawingContents, Layout, Object, RecordsOfKind};
use crate::stamp::fingerprint;
use crate::units::Unit;

/// How a text mirror's first line starts. A mirror is replaced only when
/// its first line starts so.
pub const MIRROR_MARK: &str = "# AgentDrafter's text mirror of ";

/// What a text mirror's name ends with, after the original's name without
/// its `.dwg`: `P-101.dwg`'s mirror is `P-101.mirror.yaml`.
pub const MIRROR_EXTENSION: &str = "mirror.yaml";

/// What joins an original's name to an object's handle in the object's
/// ID, after [`ID_SEPARATOR`]: `P-101/handle:2F`.
pub const HANDLE_ID: &str = "handle:";

/// What `agentdrafter read` is asked to do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadRequest {
    /// The original to read: a DWG.
    pub source: PathBuf,
    /// Where to write the text mirror. Without it, beside the original,
    /// named after it: `P-101.mirror.yaml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<PathBuf>,
    /// Read only, and write no mirror.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_mirror: bool,
}

/// What a read found.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReadReport {
    /// The original read.
    pub source: String,
    /// The name its objects' IDs start with: the original's name without
    /// its `.dwg`.
    pub sheet: String,
    /// What was read, in words.
    pub message: String,
    /// The DWG version the original is saved in, such as `AC1032` for
    /// 2018, which a revision of it keeps (ADR-0006).
    pub dwg_version: String,
    /// The drawing's unit; none when it states none the core knows.
    pub units: Option<Unit>,
    /// True when the DWG library decoded every record of the file.
    pub decoded: bool,
    /// The records the DWG library read but could not decode, by kind,
    /// with the handle of each.
    pub not_decoded: Vec<RecordsOfKind>,
    /// How many records the DWG library could not read at all.
    pub skipped: usize,
    /// How many errors the DWG library recovered from while reading.
    pub recovered_errors: usize,
    /// How many objects of each type the core read, such as `{"line": 12}`.
    pub objects: BTreeMap<String, usize>,
    /// The sheet's objects the core has no model for, by kind, with the
    /// handle of each: what no output draws.
    pub not_read: Vec<RecordsOfKind>,
    /// How many entries of each of the drawing's tables, and how many of
    /// its block definitions and layouts, the core has no model for, such
    /// as `{"layers": 2}`. Only those with any are named.
    pub tables_not_read: BTreeMap<String, usize>,
    /// The text mirror written; none when none was asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mirror: Option<String>,
}

/// Reads the original `request.source` and, unless asked not to, writes
/// its text mirror.
pub fn read_original(request: &ReadRequest) -> Result<ReadReport, Error> {
    let source = &request.source;
    if !is_dwg(source) {
        return Err(Error::usage(
            "an original to read is a DWG, whose name ends .dwg; a sheet description is read by \
             query, render and plot",
        )
        .in_file(source));
    }
    let bytes = fs::read(source).map_err(|e| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("could not read the DWG: {e}"),
        )
        .in_file(source)
    })?;
    let drawing = dwg::read(source)?;
    let sheet = sheet_name(source);
    let mirror = if request.no_mirror {
        if request.mirror.is_some() {
            return Err(Error::usage(
                "`mirror` names where to write the text mirror, and `no_mirror` writes none; give \
                 one or the other",
            )
            .at_field("mirror"));
        }
        None
    } else {
        let path = request
            .mirror
            .clone()
            .unwrap_or_else(|| source.with_extension(MIRROR_EXTENSION));
        refuse_to_replace(source, &path)?;
        let text = mirror_text(source, &bytes, &sheet, &drawing);
        write_whole(&path, text.as_bytes())?;
        Some(path)
    };
    let mut objects = BTreeMap::new();
    for object in &drawing.objects {
        *objects.entry(object.type_name().to_owned()).or_insert(0) += 1;
    }
    let tables_not_read = tables_not_read(&drawing);
    let message = message(source, &drawing, mirror.as_deref());
    Ok(ReadReport {
        source: source.display().to_string(),
        sheet,
        message,
        dwg_version: drawing.dwg_version,
        units: drawing.units,
        decoded: drawing.decode.decoded,
        not_decoded: drawing.decode.not_decoded,
        skipped: drawing.decode.skipped,
        recovered_errors: drawing.decode.recovered_errors,
        objects,
        not_read: drawing.not_read,
        tables_not_read,
        mirror: mirror.map(|p| p.display().to_string()),
    })
}

/// The ID of each object the core read from the DWG at `source`, in the
/// order of `drawing.objects`, with its name: the ID AgentDrafter's
/// extended data on it gives, or the drawing's name joined to its handle.
pub(crate) fn ids_of(source: &Path, drawing: &DrawingContents) -> Vec<(String, Option<String>)> {
    let sheet = sheet_name(source);
    drawing
        .handles
        .iter()
        .enumerate()
        .map(|(i, handle)| {
            let carried = drawing.ids_and_names.get(i);
            let id = carried
                .and_then(|c| c.id.clone())
                .unwrap_or_else(|| handle_id(&sheet, handle));
            (id, carried.and_then(|c| c.name.clone()))
        })
        .collect()
}

/// The ID of the object of handle `handle` on the original named `sheet`,
/// such as `P-101/handle:2F`.
pub(crate) fn handle_id(sheet: &str, handle: &str) -> String {
    format!("{sheet}{ID_SEPARATOR}{HANDLE_ID}{handle}")
}

/// The name an original's objects' IDs start with: its file's name
/// without its `.dwg`.
pub(crate) fn sheet_name(source: &Path) -> String {
    source
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Whether `source` is read as a DWG: its name ends `.dwg`, in any case.
fn is_dwg(source: &Path) -> bool {
    source
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dwg"))
}

/// Refuses to write the mirror at `mirror` when it is the original itself,
/// or a file other than a mirror AgentDrafter wrote is there.
fn refuse_to_replace(source: &Path, mirror: &Path) -> Result<(), Error> {
    if mirror.file_name().is_none() {
        return Err(Error::usage("the mirror's path does not name a file").at_field("mirror"));
    }
    if fs::symlink_metadata(mirror).is_err() {
        return Ok(());
    }
    if let (Ok(a), Ok(b)) = (fs::canonicalize(source), fs::canonicalize(mirror)) {
        if a == b {
            return Err(Error::usage(
                "the mirror's path is the original itself, which is never written; choose \
                 another path for the mirror",
            )
            .in_file(mirror));
        }
    }
    let is_mirror = fs::read(mirror)
        .map(|bytes| bytes.starts_with(MIRROR_MARK.as_bytes()))
        .unwrap_or(false);
    if is_mirror {
        Ok(())
    } else {
        Err(Error::new(
            ErrorCode::NotAnOutput,
            "a file that is not a text mirror AgentDrafter wrote is at the mirror's path; it may \
             be someone's own file, which is never overwritten, so choose another path for the \
             mirror",
        )
        .in_file(mirror))
    }
}

/// Writes `bytes` to a temporary file beside `path`, then moves it onto
/// `path`, so a reader never sees a half-written mirror.
fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let temporary = path.with_file_name(format!(".agentdrafter-{}.mirror.tmp", std::process::id()));
    let written = fs::write(&temporary, bytes).and_then(|()| fs::rename(&temporary, path));
    written.map_err(|e| {
        let _ = fs::remove_file(&temporary);
        Error::new(
            ErrorCode::WriteFailed,
            format!("could not write the text mirror: {e}"),
        )
        .in_file(path)
    })
}

/// The entries of each table, and the blocks and layouts, the core has no
/// model for, by the name the reply gives each.
fn tables_not_read(drawing: &DrawingContents) -> BTreeMap<String, usize> {
    [
        ("linetypes", drawing.linetypes_not_read),
        ("layers", drawing.layers_not_read),
        ("text_styles", drawing.text_styles_not_read),
        ("table_styles", drawing.table_styles_not_read),
        ("mleader_styles", drawing.mleader_styles_not_read),
        ("dimension_styles", drawing.dimension_styles_not_read),
        ("blocks", drawing.blocks_not_read),
        ("layouts", drawing.layouts_not_read),
    ]
    .into_iter()
    .filter(|(_, n)| *n > 0)
    .map(|(name, n)| (name.to_owned(), n))
    .collect()
}

/// What a read found, in words.
fn message(source: &Path, drawing: &DrawingContents, mirror: Option<&Path>) -> String {
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let decode = &drawing.decode;
    let decoded = if decode.decoded {
        "the DWG library decoded every record".to_owned()
    } else {
        let mut parts: Vec<String> = Vec::new();
        if !decode.not_decoded.is_empty() {
            parts.push(format!(
                "could not decode {}",
                kinds_in_words(&decode.not_decoded)
            ));
        }
        if decode.skipped > 0 {
            parts.push(format!("skipped {} it could not read", decode.skipped));
        }
        if decode.recovered_errors > 0 {
            parts.push(format!(
                "recovered from {} error{}",
                decode.recovered_errors,
                plural(decode.recovered_errors)
            ));
        }
        format!("the DWG library {}", parts.join(", "))
    };
    let read = drawing.objects.len();
    let not_read = if drawing.not_read.is_empty() {
        String::new()
    } else {
        format!(
            "; {} the core has no model for, which no output draws: {}",
            drawing.objects_not_read,
            kinds_in_words(&drawing.not_read)
        )
    };
    let written = match mirror {
        Some(path) => format!("; wrote its text mirror to {}", path.display()),
        None => String::new(),
    };
    format!(
        "read {name} ({}): {decoded}; {read} object{} read{not_read}{written}",
        drawing.dwg_version,
        plural(read)
    )
}

/// `3 ACAD_PROXY_ENTITY, 1 MLINE`.
fn kinds_in_words(kinds: &[RecordsOfKind]) -> String {
    kinds
        .iter()
        .map(|k| format!("{} {}", k.count, k.kind))
        .collect::<Vec<_>>()
        .join(", ")
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// The text mirror of the original at `source`, whose bytes are `bytes`,
/// read as `drawing`: YAML, a key to a line, each entry of a table, each
/// object of a block and each object of the sheet on a line of its own,
/// in the order the file holds them, so that two mirrors compare line by
/// line. The same file always gives the same mirror.
fn mirror_text(source: &Path, bytes: &[u8], sheet: &str, drawing: &DrawingContents) -> String {
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut out = String::new();
    out.push_str(&format!(
        "{MIRROR_MARK}{name}, for reading and comparing. The DWG is the record: this file is \
         never read back, and editing it changes nothing.\n"
    ));
    let key = |out: &mut String, key: &str, value: Value| {
        out.push_str(&pair(key, &value));
        out.push('\n');
    };
    key(&mut out, "mirror_of", json!(name));
    key(&mut out, "sha256", json!(fingerprint(bytes)));
    key(&mut out, "dwg_version", json!(drawing.dwg_version));
    key(&mut out, "units", json!(drawing.units.map(Unit::symbol)));
    key(&mut out, "sheet", json!(sheet));
    let decode = &drawing.decode;
    key(&mut out, "decoded", json!(decode.decoded));
    out.push_str(&format!("not_decoded: {}\n", records(&decode.not_decoded)));
    key(&mut out, "skipped", json!(decode.skipped));
    key(&mut out, "recovered_errors", json!(decode.recovered_errors));
    out.push_str(&format!("not_read: {}\n", records(&drawing.not_read)));
    key(
        &mut out,
        "tables_not_read",
        to_value(&tables_not_read(drawing)),
    );
    key(&mut out, "ltscale", json!(drawing.ltscale));
    if let Some(opens_on) = &drawing.opens_on {
        key(&mut out, "opens_on", json!(opens_on));
    }
    list(&mut out, "linetypes", &drawing.linetypes);
    list(&mut out, "layers", &drawing.layers);
    list(&mut out, "text_styles", &drawing.text_styles);
    list(&mut out, "table_styles", &drawing.table_styles);
    list(&mut out, "mleader_styles", &drawing.mleader_styles);
    list(&mut out, "dimension_styles", &drawing.dimension_styles);
    let layouts: Vec<&Layout> = drawing
        .layout
        .iter()
        .chain(&drawing.other_layouts)
        .collect();
    let layout_entries: Vec<Value> = layouts
        .iter()
        .map(|l| {
            let mut entry = Map::new();
            entry.insert("name".into(), json!(l.name));
            if let Some(paper) = &l.paper {
                entry.insert("paper".into(), to_value(paper));
            }
            Value::Object(entry)
        })
        .collect();
    list(&mut out, "layouts", &layout_entries);
    if drawing.blocks.is_empty() {
        out.push_str("blocks: []\n");
    } else {
        out.push_str("blocks:\n");
        for block in &drawing.blocks {
            out.push_str(&format!("  - name: {}\n", flow(&json!(block.name))));
            out.push_str(&format!(
                "    base_point: {}\n",
                flow(&to_value(&block.base_point))
            ));
            if block.objects.is_empty() {
                out.push_str("    objects: []\n");
            } else {
                out.push_str("    objects:\n");
                for object in &block.objects {
                    out.push_str(&format!("      - {}\n", object_entry(object, [])));
                }
            }
        }
    }
    let ids = ids_of(source, drawing);
    if drawing.objects.is_empty() {
        out.push_str("objects: []\n");
    } else {
        out.push_str("objects:\n");
        for (i, object) in drawing.objects.iter().enumerate() {
            let (id, name) = &ids[i];
            let mut first = vec![("id", json!(id))];
            if let Some(name) = name {
                first.push(("name", json!(name)));
            }
            first.push(("handle", json!(drawing.handles[i])));
            if !layouts.is_empty() {
                let space = layouts
                    .iter()
                    .find(|l| l.paper_space.get(i).copied().unwrap_or(false))
                    .map_or("model", |l| l.name.as_str());
                first.push(("space", json!(space)));
            }
            out.push_str(&format!("  - {}\n", object_entry(object, first)));
        }
    }
    out
}

/// `key: []`, or `key:` and each of `entries` on a line of its own.
fn list<T: Serialize>(out: &mut String, key: &str, entries: &[T]) {
    if entries.is_empty() {
        out.push_str(&format!("{key}: []\n"));
        return;
    }
    out.push_str(&format!("{key}:\n"));
    for entry in entries {
        out.push_str(&format!("  - {}\n", flow(&to_value(entry))));
    }
}

/// Records by kind, as the mirror writes them.
fn records(kinds: &[RecordsOfKind]) -> String {
    let inner: Vec<String> = kinds
        .iter()
        .map(|k| {
            format!(
                "{{{}, {}, {}}}",
                pair("kind", &json!(k.kind)),
                pair("count", &json!(k.count)),
                pair("handles", &json!(k.handles))
            )
        })
        .collect();
    format!("[{}]", inner.join(", "))
}

fn to_value<T: Serialize + ?Sized>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// An object as the mirror writes it, in braces on one line: the keys of
/// `first`, in order, then its `type`, then its own keys.
fn object_entry<'a>(object: &Object, first: impl IntoIterator<Item = (&'a str, Value)>) -> String {
    let mut keys: Vec<(String, Value)> =
        first.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
    let mut own = match to_value(object) {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    if let Some(kind) = own.remove("type") {
        keys.push(("type".to_owned(), kind));
    }
    keys.extend(own);
    let inner: Vec<String> = keys.iter().map(|(k, v)| pair(k, v)).collect();
    format!("{{{}}}", inner.join(", "))
}

/// `key: value`, a handle or a list of handles, and a fingerprint, always in
/// double quotes, so that one such as `1E3` is never read back as a
/// number.
fn pair(key: &str, value: &Value) -> String {
    let value = match (key, value) {
        ("handle" | "sha256", Value::String(h)) => quoted(h),
        ("handles", Value::Array(hs)) => {
            let inner: Vec<String> = hs
                .iter()
                .map(|h| h.as_str().map_or_else(|| flow(h), quoted))
                .collect();
            format!("[{}]", inner.join(", "))
        }
        _ => flow(value),
    };
    format!("{}: {value}", scalar(key))
}

/// `text` in double quotes, escaped as JSON escapes it, which YAML reads
/// the same.
fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

/// `value` in YAML's flow style on one line: mappings in braces, lists in
/// brackets, and a string plain where YAML reads it back as that string,
/// otherwise in double quotes.
fn flow(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() => number_text(f),
            _ => n.to_string(),
        },
        Value::String(s) => scalar(s),
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(flow).collect();
            format!("[{}]", inner.join(", "))
        }
        Value::Object(map) => {
            // A named entry's name first, then its other keys in order.
            let named = map.get("name").map(|name| ("name", name));
            let inner: Vec<String> = named
                .into_iter()
                .chain(
                    map.iter()
                        .filter(|(k, _)| *k != "name")
                        .map(|(k, v)| (k.as_str(), v)),
                )
                .map(|(k, v)| pair(k, v))
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
    }
}

/// A string as the mirror writes it: plain when it is a word YAML reads
/// back as that string, such as `E-WIRE` or `P-101/handle:2F`, and
/// otherwise in double quotes, escaped as JSON escapes it, which YAML reads
/// the same.
fn scalar(text: &str) -> String {
    let word = text
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-./:".contains(c))
        && !text.ends_with(':')
        && !text.contains(": ")
        && !matches!(
            text.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n"
        );
    if word {
        text.to_owned()
    } else {
        quoted(text)
    }
}

#[cfg(test)]
#[path = "original_tests.rs"]
mod tests;
