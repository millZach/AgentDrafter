//! External references (`xrefs:`) in a sheet description or template.

use serde_json::{json, Value};

use super::block_schema::relative_path_property;
use super::blocks::{relative_path, RelativePath};
use super::layers::named_entries;
use super::strict_yaml::{key_path, Node};
use super::{at, boolean, choice, Fields};
use crate::error::{Error, ErrorCode};
use crate::model::{BlockLibrary, Xref, XrefKind};

/// The top-level key a sheet description declares its external references under.
pub(crate) const XREFS_KEY: &str = "xrefs";

/// The keys each external reference entry takes.
pub(crate) const XREF_KEYS: [&str; 3] = ["path", "kind", "unloaded"];

/// The attachment kinds an external reference accepts.
pub(crate) const XREF_KINDS: [&str; 2] = ["attach", "overlay"];

/// Reads the `xrefs:` mapping of a sheet description or template.
pub(super) fn xrefs(node: &Node, library: Option<&BlockLibrary>) -> Result<Vec<Xref>, Error> {
    let entries = named_entries(node, XREFS_KEY, "external reference")?;
    let mut read = Vec::with_capacity(entries.len());
    for entry in entries {
        let name = &entry.key;
        let path = key_path(XREFS_KEY, name);
        if let Some(lib) = library {
            if lib.blocks.iter().any(|b| b.name.eq_ignore_ascii_case(name)) {
                return Err(at(
                    Error::new(
                        ErrorCode::DuplicateName,
                        format!(
                            "external reference `{name}` has the same name as a block in the \
                             block library; each block and external reference name is unique"
                        ),
                    ),
                    &path,
                    entry.key_at,
                ));
            }
        }
        let fields = Fields::of(&entry.value, &path, &XREF_KEYS, "an external reference")?;
        let file_node = fields.require_text("path")?;
        let file_path = relative_path(
            file_node,
            &fields.path_of("path"),
            &RelativePath {
                shape: "the path of a referenced DWG or DXF file, such as `shared/grid.dwg`",
                whose: "the external reference's path",
                from: "the sheet description's folder",
                names: "a file",
            },
        )?;
        let kind = match fields.get("kind") {
            Some(node) => {
                let kind_path = fields.path_of("kind");
                let text = choice(node, &kind_path, "an xref kind", "attach or overlay")?;
                match text {
                    "attach" => XrefKind::Attach,
                    "overlay" => XrefKind::Overlay,
                    other => {
                        return Err(at(
                            Error::new(
                                ErrorCode::BadValue,
                                format!(
                                    "unknown xref kind `{other}`; expected `attach` or `overlay`"
                                ),
                            ),
                            &kind_path,
                            node.at,
                        ));
                    }
                }
            }
            None => XrefKind::Attach,
        };
        let unloaded = match fields.get("unloaded") {
            Some(node) => boolean(node, &fields.path_of("unloaded"))?,
            None => false,
        };
        read.push(Xref {
            name: name.clone(),
            path: file_path.to_owned(),
            kind,
            unloaded,
        });
    }
    Ok(read)
}

/// JSON Schema property for `xrefs` on a sheet description or template.
pub(super) fn xrefs_property() -> Value {
    json!({
        "description": "The external references (xrefs) attached or overlaid onto this sheet, \
                        from block name to file path and attachment mode. An `insert` object may \
                        place a declared external reference by name without listing it in a block \
                        library.",
        "type": "object",
        "propertyNames": { "$ref": "#/$defs/name" },
        "additionalProperties": { "$ref": "#/$defs/xref" },
    })
}

/// JSON Schema `$defs/xref` entry definition.
pub(super) fn xref_def() -> Value {
    let [path_key, kind_key, unloaded_key] = XREF_KEYS;
    json!({
        "description": "One external reference (DWG or DXF) attached or overlaid onto the sheet.",
        "type": "object",
        "required": [path_key],
        "additionalProperties": false,
        "properties": {
            path_key: relative_path_property(
                "The path of the referenced DWG or DXF file, relative to the sheet description's \
                 folder, with / between folders, such as `shared/grid.dwg`."
                    .to_owned(),
            ),
            kind_key: {
                "description": "How the external reference is attached: `attach` (the default, \
                                carried along when this drawing is referenced by another) or \
                                `overlay` (drawn in this drawing only).",
                "enum": XREF_KINDS,
            },
            unloaded_key: {
                "description": "Whether the external reference is unloaded (kept as a reference \
                                without drawing its geometry). Without it, false.",
                "anyOf": [{ "type": "boolean" }, { "enum": ["true", "false"] }],
            },
        },
    })
}
