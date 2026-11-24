//! Groups (`groups:`) in a sheet description, template, or rows file (#92).

use serde_json::{json, Value};

use super::ids::{object_id, ObjectRecord};
use super::layers::named_entries;
use super::strict_yaml::{index_path, key_path, Node, Pos};
use super::{at, bad_value, boolean, scalar, sequence, Fields};
use crate::error::{Error, ErrorCode};
use crate::model::Group;

/// The top-level key a sheet description, template, or row declares its groups
/// under.
pub(crate) const GROUPS_KEY: &str = "groups";

/// The keys each group entry takes.
pub(crate) const GROUP_KEYS: [&str; 3] = ["objects", "description", "selectable"];

/// A raw group declaration parsed from YAML before its member references are
/// resolved against the sheet's objects.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RawGroup {
    pub(super) name: String,
    pub(super) name_at: Pos,
    pub(super) path: String,
    pub(super) description: String,
    pub(super) selectable: bool,
    pub(super) members: Vec<(String, String, Pos)>,
}

/// Reads a `groups:` mapping into [`RawGroup`] entries, validating keys,
/// unique names (case-insensitive) and member list shapes.
pub(super) fn read_raw_groups(node: &Node, base_path: &str) -> Result<Vec<RawGroup>, Error> {
    let entries = named_entries(node, base_path, "group")?;
    let mut read = Vec::with_capacity(entries.len());
    for entry in entries {
        let name = &entry.key;
        let path = key_path(base_path, name);
        let fields = Fields::of(&entry.value, &path, &GROUP_KEYS, "a group")?;
        let objects_node = fields.require("objects")?;
        let objects_path = fields.path_of("objects");
        let items = sequence(
            objects_node,
            &objects_path,
            "a list of object IDs or names belonging to the group",
        )?;
        if items.is_empty() {
            return Err(bad_value(
                objects_node,
                &objects_path,
                "a group must list at least one object",
            ));
        }
        let mut members = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let item_path = index_path(&objects_path, i);
            let text = scalar(item, &item_path, "an object ID or name")?.trim();
            if text.is_empty() {
                return Err(bad_value(
                    item,
                    &item_path,
                    "a group member cannot be empty",
                ));
            }
            members.push((text.to_owned(), item_path, item.at));
        }
        let description = match fields.get("description") {
            Some(node) => {
                scalar(node, &fields.path_of("description"), "a group description")?.to_owned()
            }
            None => String::new(),
        };
        let selectable = match fields.get("selectable") {
            Some(node) => boolean(node, &fields.path_of("selectable"))?,
            None => true,
        };
        read.push(RawGroup {
            name: name.clone(),
            name_at: entry.key_at,
            path,
            description,
            selectable,
            members,
        });
    }
    Ok(read)
}

/// Resolves template/sheet groups and optional row groups against `records`
/// (the sheet's objects in order), checking for duplicate group names across
/// template and row and resolving each member string by local `id`, full
/// `SHEET/id`, or unique `name`.
pub(super) fn resolve_groups(
    sheet_groups: Vec<RawGroup>,
    row_groups: Option<(Vec<RawGroup>, &std::path::Path)>,
    records: &[ObjectRecord],
    sheet_name: Option<&str>,
) -> Result<Vec<Group>, Error> {
    let mut out: Vec<Group> = Vec::new();
    for raw in sheet_groups {
        out.push(resolve_one(&raw, records, sheet_name)?);
    }
    if let Some((rgroups, row_file)) = row_groups {
        for raw in rgroups {
            if let Some(existing) = out.iter().find(|g| g.name.eq_ignore_ascii_case(&raw.name)) {
                return Err(at(
                    Error::new(
                        ErrorCode::DuplicateName,
                        format!(
                            "group `{}` is already defined as `{}` on the template; each group \
                             name is unique on a sheet",
                            raw.name, existing.name
                        ),
                    ),
                    &raw.path,
                    raw.name_at,
                )
                .in_file(row_file));
            }
            let group = resolve_one(&raw, records, sheet_name).map_err(|e| e.in_file(row_file))?;
            out.push(group);
        }
    }
    Ok(out)
}

fn resolve_one(
    raw: &RawGroup,
    records: &[ObjectRecord],
    sheet_name: Option<&str>,
) -> Result<Group, Error> {
    let mut indices = Vec::with_capacity(raw.members.len());
    for (member, item_path, item_at) in &raw.members {
        let index = find_member(member, records, sheet_name, item_path, *item_at)?;
        if !indices.contains(&index) {
            indices.push(index);
        }
    }
    Ok(Group {
        name: raw.name.clone(),
        description: raw.description.clone(),
        selectable: raw.selectable,
        unnamed: false,
        objects: indices,
    })
}

fn find_member(
    member: &str,
    records: &[ObjectRecord],
    sheet_name: Option<&str>,
    path: &str,
    pos: Pos,
) -> Result<usize, Error> {
    // 1. Exact local ID (written `id` or positional id), or full `SHEET/id`.
    for (i, rec) in records.iter().enumerate() {
        if rec.id.eq_ignore_ascii_case(member) {
            return Ok(i);
        }
        if let Some(sheet) = sheet_name {
            if object_id(sheet, rec).eq_ignore_ascii_case(member) {
                return Ok(i);
            }
        }
    }
    // 2. Object `name` (must be unique on the sheet).
    let by_name: Vec<usize> = records
        .iter()
        .enumerate()
        .filter(|(_, rec)| {
            rec.name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(member))
        })
        .map(|(i, _)| i)
        .collect();
    match by_name.as_slice() {
        [only] => Ok(*only),
        [] => {
            let known: Vec<&str> = records.iter().map(|r| r.id.as_str()).collect();
            let hint = if known.is_empty() {
                "the sheet has no objects".to_owned()
            } else {
                format!("the sheet's objects: {}", known.join(", "))
            };
            Err(at(
                Error::new(
                    ErrorCode::UnknownName,
                    format!("unknown object `{member}` in group; {hint}"),
                ),
                path,
                pos,
            ))
        }
        many => Err(at(
            Error::new(
                ErrorCode::BadValue,
                format!(
                    "object name `{member}` matches {} objects on the sheet; name the object by \
                     its `id` instead",
                    many.len()
                ),
            ),
            path,
            pos,
        )),
    }
}

/// JSON Schema property for `groups` on a sheet description, template, or row.
pub(crate) fn groups_property() -> Value {
    json!({
        "description": "Named groups of objects on this sheet, from group name to its members \
                        and options, as AutoCAD's GROUP / ACAD_GROUP holds them. Operations can \
                        target a group as one by its name.",
        "type": "object",
        "propertyNames": { "$ref": "#/$defs/name" },
        "additionalProperties": { "$ref": "#/$defs/group" },
    })
}

/// JSON Schema `$defs/group` definition.
pub(crate) fn group_def() -> Value {
    let [objects_key, description_key, selectable_key] = GROUP_KEYS;
    json!({
        "description": "One named group of objects on the sheet.",
        "type": "object",
        "required": [objects_key],
        "additionalProperties": false,
        "properties": {
            objects_key: {
                "description": "The objects in the group, in order, each named by its `id` (or \
                                unique `name`) on the sheet.",
                "type": "array",
                "minItems": 1,
                "items": {
                    "type": "string",
                    "minLength": 1,
                },
            },
            description_key: {
                "description": "An optional human-readable description of the group (AutoCAD \
                                GROUP description).",
                "type": "string",
            },
            selectable_key: {
                "description": "Whether AutoCAD selects the group as one unit (default true).",
                "type": "boolean",
            },
        },
    })
}
