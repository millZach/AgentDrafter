//! Stamps: what an output records about the record it came from, and the
//! check that tells when that record has moved on and the output is stale.
//!
//! Every output carries a stamp: so far the PNG, in a text chunk keyed
//! `AgentDrafter`. A stamp names the record (the sheet description or the
//! DWG it was produced from) by where it was and by a SHA-256 fingerprint
//! of its bytes. An output is stale when its record's bytes no longer have
//! that fingerprint, or the record is gone.
//!
//! The working revision arrives with ticket #9. A stamp is JSON with a
//! format number, so that ticket adds the revision beside `source` and
//! raises the number; a reader that meets a number it does not know says
//! so rather than guessing.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, ErrorCode};
use crate::raster;

/// The format of the stamps this version writes and reads.
pub const STAMP_FORMAT: u32 = 1;

/// What an output records about where it came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    /// The stamp's format: 1 so far. A stamp that records more raises it.
    pub stamp: u32,
    /// What produced the output, such as `agentdrafter 0.0.0`.
    pub produced_by: String,
    /// The record the output was produced from.
    pub source: SourceStamp,
}

/// The record an output was produced from, as its stamp names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceStamp {
    /// What the record is.
    pub kind: SourceKind,
    /// Where the record was: from the output's folder when the two share a
    /// folder anywhere above them, with `/` between folders, and otherwise
    /// in full.
    pub path: String,
    /// The SHA-256 of the record's bytes, in lower-case hexadecimal.
    pub sha256: String,
}

/// What kind of record an output was produced from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A sheet description: a new sheet's record.
    SheetDescription,
    /// A DWG.
    Dwg,
}

impl SourceKind {
    /// What a file is, by its name: a DWG when it ends `.dwg` in any case,
    /// and otherwise a sheet description.
    pub(crate) fn of(path: &Path) -> SourceKind {
        let dwg = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("dwg"));
        if dwg {
            SourceKind::Dwg
        } else {
            SourceKind::SheetDescription
        }
    }

    /// What the kind is called in a message.
    pub(crate) fn words(self) -> &'static str {
        match self {
            SourceKind::SheetDescription => "sheet description",
            SourceKind::Dwg => "DWG",
        }
    }
}

impl Stamp {
    /// The stamp for an output at `output` produced from the record at
    /// `source`, whose bytes are `bytes`.
    pub(crate) fn new(source: &Path, bytes: &[u8], output: &Path) -> Stamp {
        Stamp {
            stamp: STAMP_FORMAT,
            produced_by: format!("agentdrafter {}", env!("CARGO_PKG_VERSION")),
            source: SourceStamp {
                kind: SourceKind::of(source),
                path: path_from(output, source),
                sha256: fingerprint(bytes),
            },
        }
    }

    /// The stamp as it is written into an output: one line of JSON.
    pub(crate) fn to_json(&self) -> String {
        // A stamp holds only strings and numbers, which always serialize.
        serde_json::to_string(self).expect("a stamp serializes to JSON")
    }
}

/// The SHA-256 of `bytes`, in lower-case hexadecimal.
pub(crate) fn fingerprint(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A request to check whether an output is stale.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StaleRequest {
    /// The output to check: a PNG AgentDrafter rendered.
    pub output: PathBuf,
    /// The record to check it against. Without it, the record the stamp
    /// names, found from the output's folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
}

/// Whether an output is stale, and why.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StaleReport {
    /// The output that was checked.
    pub output: String,
    /// The record it was checked against.
    pub source: String,
    /// Whether the record has moved on since the output was produced, or
    /// is gone.
    pub stale: bool,
    /// Why, in words.
    pub why: String,
    /// The output's stamp.
    pub stamp: Stamp,
}

/// Checks whether an output is stale: whether the record its stamp names
/// has changed since, or is gone.
pub fn check_stale(request: &StaleRequest) -> Result<StaleReport, Error> {
    let output = request.output.as_path();
    let stamp = read_stamp(output)?;
    let source = match &request.source {
        Some(source) => source.clone(),
        None => resolve(output, &stamp.source.path),
    };
    let kind = stamp.source.kind.words();
    let (stale, why) = match fs::read(&source) {
        Ok(bytes) if fingerprint(&bytes) == stamp.source.sha256 => (
            false,
            format!("the {kind} has not changed since the output was produced"),
        ),
        Ok(_) => (
            true,
            format!("the {kind} has changed since the output was produced"),
        ),
        Err(e) => (
            true,
            format!("the {kind} the output was produced from cannot be read ({e})"),
        ),
    };
    Ok(StaleReport {
        output: output.display().to_string(),
        source: source.display().to_string(),
        stale,
        why,
        stamp,
    })
}

/// The stamp an output carries. An output that cannot be read is an error,
/// and so is one with no stamp AgentDrafter can read.
pub(crate) fn read_stamp(output: &Path) -> Result<Stamp, Error> {
    let bytes = fs::read(output).map_err(|e| {
        Error::new(
            ErrorCode::ReadFailed,
            format!("could not read the output: {e}"),
        )
        .in_file(output)
    })?;
    stamp_of(&bytes).map_err(|why| Error::new(ErrorCode::NotStamped, why).in_file(output))
}

/// The stamp in an output's bytes, or why there is none.
pub(crate) fn stamp_of(bytes: &[u8]) -> Result<Stamp, String> {
    let text = raster::stamp_in(bytes)
        .ok_or("the file is not a PNG AgentDrafter rendered: it carries no AgentDrafter stamp")?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("the file's AgentDrafter stamp is not JSON ({e})"))?;
    let format = value.get("stamp").and_then(serde_json::Value::as_u64);
    if format != Some(u64::from(STAMP_FORMAT)) {
        return Err(format!(
            "the file's stamp is in format {}, and this AgentDrafter reads format {STAMP_FORMAT}",
            format.map_or("unknown".to_owned(), |f| f.to_string())
        ));
    }
    serde_json::from_value(value).map_err(|e| format!("the file's stamp cannot be read ({e})"))
}

/// Where `source` is, as the stamp of an output at `output` names it: from
/// the output's folder when the two share a folder above them, and
/// otherwise in full.
fn path_from(output: &Path, source: &Path) -> String {
    let folder = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let (Ok(folder), Ok(source)) = (fs::canonicalize(folder), fs::canonicalize(source)) else {
        return slashed(source);
    };
    let from: Vec<Component> = folder.components().collect();
    let to: Vec<Component> = source.components().collect();
    let shared = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    // Only the root, or not even a drive, in common: name it in full.
    if shared <= 1 {
        return slashed(&source);
    }
    let mut parts: Vec<String> = vec!["..".to_owned(); from.len() - shared];
    parts.extend(
        to[shared..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// The record a stamp names, found from the output's folder.
fn resolve(output: &Path, path: &str) -> PathBuf {
    let named = PathBuf::from(path);
    if named.is_absolute() {
        return named;
    }
    output.parent().unwrap_or(Path::new("")).join(named)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fingerprint_is_sha_256() {
        assert_eq!(
            fingerprint(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_record_beside_the_output_is_named_from_its_folder() {
        let dir = std::env::temp_dir()
            .join(format!("agentdrafter-stamp-{}", std::process::id()))
            .join("beside");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sheets")).unwrap();
        fs::create_dir_all(dir.join("renders")).unwrap();
        let source = dir.join("sheets").join("a.yaml");
        fs::write(&source, "units: mm\nobjects: []\n").unwrap();
        assert_eq!(
            path_from(&dir.join("sheets").join("a.png"), &source),
            "a.yaml"
        );
        assert_eq!(
            path_from(&dir.join("renders").join("a.png"), &source),
            "../sheets/a.yaml"
        );
        assert_eq!(
            resolve(&dir.join("renders").join("a.png"), "../sheets/a.yaml"),
            dir.join("renders").join("../sheets/a.yaml")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stamp_in_an_unknown_format_is_not_guessed_at() {
        let mut stamp = Stamp {
            stamp: STAMP_FORMAT,
            produced_by: "agentdrafter 0.0.0".into(),
            source: SourceStamp {
                kind: SourceKind::Dwg,
                path: "a.dwg".into(),
                sha256: fingerprint(b""),
            },
        };
        let png = |stamp: &Stamp| {
            let pixmap = tiny_skia::Pixmap::new(2, 2).unwrap();
            raster::png(&pixmap, &stamp.to_json(), 1.0).unwrap()
        };
        assert_eq!(stamp_of(&png(&stamp)), Ok(stamp.clone()));
        stamp.stamp = 2;
        let why = stamp_of(&png(&stamp)).unwrap_err();
        assert!(why.contains("format 2"), "{why}");
        assert!(stamp_of(b"not a png")
            .unwrap_err()
            .contains("no AgentDrafter stamp"));
    }
}
