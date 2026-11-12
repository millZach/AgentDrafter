//! Stamps: what an output records about the files it came from, and the
//! check that tells when one of them has moved on and the output is stale.
//!
//! Every output carries a stamp: the PNG in a text chunk keyed
//! `AgentDrafter`, the PDF in its document information under the key
//! `AgentDrafter`. A stamp lists every file read to produce the output,
//! the record first (the sheet description, the DWG, or a drawing set's
//! template), then the files it draws on, such as the block library and a
//! set's rows. It names each by its kind, where it was and a SHA-256
//! fingerprint of its bytes. An output is stale when any listed file's
//! bytes no longer have that fingerprint, or the file is gone.
//!
//! A stamp also says whether the revision the output was made from was
//! issued, and which accept issued it, as Zach decided on 2026-11-11: the
//! output is from an issued revision when every file it lists is, byte for
//! byte, a file an accept of its drawing set issued. The drawing itself is
//! the same either way: the stamp alone says it.
//!
//! A stamp is JSON with a format number; a reader that meets a number it
//! does not know says so rather than guessing.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::change::{stamp_revision, StampRevision};
use crate::error::{Error, ErrorCode};
use crate::{pdf, raster};

/// The format of the stamps this version writes and reads.
pub const STAMP_FORMAT: u32 = 1;

/// What an output records about where it came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    /// The stamp's format: 1 so far.
    pub stamp: u32,
    /// What produced the output, such as `agentdrafter 0.0.0`.
    pub produced_by: String,
    /// Every file read to produce the output: the record first, then the
    /// files it draws on.
    pub sources: Vec<SourceStamp>,
    /// Whether the revision those files were read from was issued, and by
    /// which accept. A stamp written before revisions were issued does not
    /// say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<StampRevision>,
}

/// A file an output was produced from, as its stamp names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceStamp {
    /// What the file is.
    pub kind: SourceKind,
    /// Where the file was: from the output's folder when the two share a
    /// folder anywhere above them, with `/` between folders, and otherwise
    /// in full.
    pub path: String,
    /// The SHA-256 of the file's bytes, in lower-case hexadecimal.
    pub sha256: String,
}

/// What kind of file an output was produced from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A sheet description: a new sheet's record.
    SheetDescription,
    /// A DWG.
    Dwg,
    /// The block library a sheet description or a template draws its
    /// blocks from.
    BlockLibrary,
    /// A drawing set's template, which a sheet of the set is drawn from
    /// with its row.
    Template,
    /// A drawing set's rows file, which holds the row a sheet of the set is
    /// drawn with.
    Rows,
}

impl SourceKind {
    /// What a record is, by its name: a DWG when it ends `.dwg` in any
    /// case, and otherwise a sheet description.
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
            SourceKind::BlockLibrary => "block library",
            SourceKind::Template => "template",
            SourceKind::Rows => "rows file",
        }
    }
}

impl Stamp {
    /// The stamp for an output at `output` produced from `files`: each
    /// one's kind, where it was read from and the bytes read, the record
    /// first.
    pub(crate) fn new(files: &[(SourceKind, &Path, &[u8])], output: &Path) -> Stamp {
        let read: Vec<(&Path, &[u8])> = files
            .iter()
            .map(|&(_, path, bytes)| (path, bytes))
            .collect();
        Stamp {
            stamp: STAMP_FORMAT,
            produced_by: format!("agentdrafter {}", env!("CARGO_PKG_VERSION")),
            sources: files
                .iter()
                .map(|&(kind, path, bytes)| SourceStamp {
                    kind,
                    path: path_from(output, path),
                    sha256: fingerprint(bytes),
                })
                .collect(),
            revision: Some(stamp_revision(&read)),
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
    /// The output to check: a PNG AgentDrafter rendered or a PDF it
    /// plotted.
    pub output: PathBuf,
    /// Where the record is now, the first file the stamp lists. Without
    /// it, where the stamp names it, found from the output's folder. The
    /// other files are always found as the stamp names them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
}

/// Whether an output is stale, and why.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StaleReport {
    /// The output that was checked.
    pub output: String,
    /// Whether any file the output was produced from has moved on since,
    /// or is gone.
    pub stale: bool,
    /// Why, in words: which files have moved on, or that none has.
    pub why: String,
    /// Each file the stamp lists, as it was checked.
    pub sources: Vec<SourceCheck>,
    /// Whether the output was made from an issued revision, as its stamp
    /// says: false too when the stamp does not say.
    pub issued: bool,
    /// What the stamp says of the revision the output was made from, in
    /// words: issued, and by which accept, or the working revision.
    pub revision: String,
    /// The output's stamp.
    pub stamp: Stamp,
}

/// One file an output was produced from, checked.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SourceCheck {
    /// What the file is.
    pub kind: SourceKind,
    /// Where it was looked for.
    pub path: String,
    /// Whether it has changed since the output was produced, or is gone.
    pub stale: bool,
    /// Why, in words.
    pub why: String,
}

/// Checks whether an output is stale: whether any file its stamp lists has
/// changed since, or is gone.
pub fn check_stale(request: &StaleRequest) -> Result<StaleReport, Error> {
    let output = request.output.as_path();
    let stamp = read_stamp(output)?;
    let sources: Vec<SourceCheck> = stamp
        .sources
        .iter()
        .enumerate()
        .map(|(i, named)| {
            let path = match (&request.source, i) {
                (Some(source), 0) => source.clone(),
                _ => resolve(output, &named.path),
            };
            check(named, &path)
        })
        .collect();
    let moved_on: Vec<&str> = sources
        .iter()
        .filter(|s| s.stale)
        .map(|s| s.why.as_str())
        .collect();
    let why = if moved_on.is_empty() {
        "no file the output was produced from has changed since".to_owned()
    } else {
        moved_on.join("; ")
    };
    let issued = stamp.revision.as_ref().is_some_and(|r| r.issued);
    let revision = match &stamp.revision {
        Some(StampRevision {
            issued: true,
            accept,
            accepted_at,
            via,
        }) => format!(
            "the output was made from an issued revision: the one the accept in entry {} of the \
             set's journal issued{}{}",
            accept.map_or_else(|| "?".to_owned(), |a| a.to_string()),
            via.map_or_else(String::new, |via| format!(", through {}", via.words())),
            accepted_at
                .as_ref()
                .map_or_else(String::new, |at| format!(", at {at}"))
        ),
        Some(_) => "the output was made from the working revision, which was not issued: no \
                    accept had issued the files it was made from"
            .to_owned(),
        None => "the output's stamp does not say whether its revision was issued: it was made \
                 before outputs said so"
            .to_owned(),
    };
    Ok(StaleReport {
        output: output.display().to_string(),
        stale: !moved_on.is_empty(),
        why,
        sources,
        issued,
        revision,
        stamp,
    })
}

/// Checks one file a stamp lists, found at `path`.
fn check(named: &SourceStamp, path: &Path) -> SourceCheck {
    let kind = named.kind.words();
    let shown = path.display();
    let (stale, why) = match fs::read(path) {
        Ok(bytes) if fingerprint(&bytes) == named.sha256 => (
            false,
            format!("the {kind} `{shown}` has not changed since the output was produced"),
        ),
        Ok(_) => (
            true,
            format!("the {kind} `{shown}` has changed since the output was produced"),
        ),
        Err(e) => (
            true,
            format!("the {kind} `{shown}` the output was produced from cannot be read ({e})"),
        ),
    };
    SourceCheck {
        kind: named.kind,
        path: shown.to_string(),
        stale,
        why,
    }
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

/// The stamp in an output's bytes, a PNG's or a PDF's, or why there is
/// none.
pub(crate) fn stamp_of(bytes: &[u8]) -> Result<Stamp, String> {
    let text = if bytes.starts_with(b"%PDF-") {
        pdf::stamp_in(bytes)
            .ok_or("the file is not a PDF AgentDrafter plotted: it carries no AgentDrafter stamp")?
    } else {
        raster::stamp_in(bytes).ok_or(
            "the file is not a PNG AgentDrafter rendered or a PDF it plotted: it carries no \
             AgentDrafter stamp",
        )?
    };
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
            sources: vec![SourceStamp {
                kind: SourceKind::Dwg,
                path: "a.dwg".into(),
                sha256: fingerprint(b""),
            }],
            revision: Some(StampRevision {
                issued: true,
                accept: Some(4),
                accepted_at: Some("2026-11-11T18:57:46Z".into()),
                via: Some(crate::Interface::Viewer),
            }),
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
