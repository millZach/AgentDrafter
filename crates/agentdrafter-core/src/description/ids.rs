//! Reads the ID and the name of each object on a sheet, and gives the
//! sheet's objects their IDs.
//!
//! Zach decided on 2026-11-11 that an object's ID lives in its record. An
//! object on a sheet description or a template may carry an `id`, which a
//! generator can write; an object without one takes its place in the file,
//! such as `objects[3]`, which holds while the file is unchanged. A sheet's
//! object's ID is the sheet's name joined to that by [`ID_SEPARATOR`], such
//! as `DEMO-001/border`: a template's objects are on every sheet built from
//! it, and each sheet's name is given once in a set, so an `id` given once
//! in its file is unique in the set. The reader refuses an `id` given twice
//! in one file, naming both places. Building never writes an `id` into a
//! file; an operation does (`crate::change`): it gives every object it adds
//! an `id`, and when it deletes an object it writes the place of each later
//! object that has none as its `id`, such as `objects[3]`, so that no
//! object's ID moves. An `id` may be written as a place for that reason,
//! and a place that is another object's written `id` is refused.
//!
//! A row of a drawing set may hold objects of its own; one without an `id`
//! takes its place in the rows file, such as `rows[1].objects[0]`.
//!
//! Every object may also carry a `name`, a label an agent selects it by,
//! given once on a sheet. It is never a tag.
//!
//! A block's objects take neither: an insert is the object on a sheet, and
//! the objects of its block are the block's, shared by every insert of it.

use super::strict_yaml::Node;
use super::{at, bad_value, scalar, Fields};
use crate::error::{Error, ErrorCode};

/// The key of an object's ID.
pub(crate) const ID_KEY: &str = "id";

/// The key of an object's name.
pub(crate) const NAME_KEY: &str = "name";

/// The keys every object on a sheet takes besides its own: its ID and its
/// name. A block's objects do not take them.
pub(crate) const SHEET_OBJECT_KEYS: [&str; 2] = [ID_KEY, NAME_KEY];

/// What joins a sheet's name to the ID its object carries in its file. No
/// sheet's name and no written ID holds it, so an ID splits back at its
/// first one.
pub(crate) const ID_SEPARATOR: char = '/';

/// The longest `id`, in characters: a DWG's extended data holds a string of
/// at most 255, and keeps the sheet's name and the `id` apart.
pub(crate) const LONGEST_ID: usize = 255;

/// The characters an `id` holds besides ASCII letters and digits.
pub(crate) const ID_PUNCTUATION: &str = "_-.:";

/// The longest name an object can carry, in characters.
pub(crate) const LONGEST_NAME: usize = 255;

/// An object as its sheet description or template writes it, with the ID
/// it carries in that file and its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ObjectRecord {
    /// The ID the object carries in its file: its `id`, or its place, such
    /// as `objects[3]`, when it has none.
    pub id: String,
    /// The name the object carries, if it has one.
    pub name: Option<String>,
    /// Where the object is written in its file; its `field` is its place,
    /// such as `objects[3]`.
    pub object: Written,
    /// Where its `id` is written, if it has one. An object without one
    /// takes its place, which moves when an object is added or taken away
    /// before it.
    pub id_key: Option<Written>,
    /// Whether the object is one of its row's own, written in the rows
    /// file, rather than in the sheet description or template.
    pub in_row: bool,
}

/// Where something is written in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Written {
    /// Its path in the file, such as `objects[3]` or `objects[3].line.id`.
    pub field: String,
    /// Its line, counted from 1.
    pub line: usize,
    /// Its column in the line, counted from 1 in characters.
    pub column: usize,
}

impl ObjectRecord {
    /// Whether the file gives the object's ID under `id`.
    pub(crate) fn written(&self) -> bool {
        self.id_key.is_some()
    }
}

/// The record of the object at `node`, whose place in the file is `field`
/// and whose keys are `fields`. `before` holds the records of the objects
/// before it on the sheet: an `id` or a name one of them carries already,
/// in any case, is an error that names where.
pub(super) fn object_record(
    fields: &Fields,
    node: &Node,
    field: &str,
    before: &[ObjectRecord],
) -> Result<ObjectRecord, Error> {
    let name = match fields.get(NAME_KEY) {
        Some(node) => Some(object_name(node, &fields.path_of(NAME_KEY), before)?),
        None => None,
    };
    let object = Written {
        field: field.to_owned(),
        line: node.at.line,
        column: node.at.column,
    };
    let Some(id_node) = fields.get(ID_KEY) else {
        // An object without an `id` takes its place, which is no other
        // object's written `id`.
        if let Some((first, first_key)) = written_before(before, field) {
            return Err(at(
                Error::new(
                    ErrorCode::DuplicateId,
                    format!(
                        "the object at `{field}` has no `id`, so its ID is its place, `{field}`, \
                         which the object at `{}` is given at line {} already; give this one an \
                         `id`",
                        first.object.field, first_key.line
                    ),
                ),
                field,
                node.at,
            ));
        }
        return Ok(ObjectRecord {
            id: field.to_owned(),
            name,
            object,
            id_key: None,
            in_row: false,
        });
    };
    let path = fields.path_of(ID_KEY);
    let id = scalar(id_node, &path, "an ID, text such as `pump-1`")?;
    if let Some(problem) = id_problem(id) {
        return Err(bad_value(id_node, &path, problem));
    }
    if let Some(first) = before.iter().find(|r| r.id.eq_ignore_ascii_case(id)) {
        let message = match &first.id_key {
            Some(first_key) => format!(
                "the ID `{id}` is given at line {} already, to the object at `{}`; an ID is \
                 unique in the drawing set, so each is given once in a file, and IDs that \
                 differ only in case are one ID",
                first_key.line, first.object.field
            ),
            None => format!(
                "the ID `{id}` is the place of the object at `{}`, line {}, which has no `id` \
                 and takes its place as its ID; an ID is unique in the drawing set",
                first.object.field, first.object.line
            ),
        };
        return Err(at(
            Error::new(ErrorCode::DuplicateId, message),
            &path,
            id_node.at,
        ));
    }
    Ok(ObjectRecord {
        id: id.to_owned(),
        name,
        object,
        id_key: Some(Written {
            field: path,
            line: id_node.at.line,
            column: id_node.at.column,
        }),
        in_row: false,
    })
}

/// The first object of `before` whose written `id` is `place`, in any
/// case, with where it is written.
fn written_before<'a>(
    before: &'a [ObjectRecord],
    place: &str,
) -> Option<(&'a ObjectRecord, &'a Written)> {
    before.iter().find_map(|r| {
        r.id_key
            .as_ref()
            .filter(|_| r.id.eq_ignore_ascii_case(place))
            .map(|key| (r, key))
    })
}

/// Whether `id` is written as a place: `objects[N]` or
/// `rows[N].objects[N]`, each number written as digits with no leading 0.
/// An operation writes an object's place as its `id` so that it keeps it.
pub(crate) fn is_place(id: &str) -> bool {
    fn index(text: &str) -> Option<&str> {
        let rest = text.strip_prefix('[')?;
        let end = rest.find(']')?;
        let digits = &rest[..end];
        let canonical = !digits.is_empty()
            && digits.bytes().all(|b| b.is_ascii_digit())
            && (digits == "0" || !digits.starts_with('0'));
        canonical.then_some(&rest[end + 1..])
    }
    let objects = |text: &str| {
        text.strip_prefix("objects")
            .and_then(index)
            .is_some_and(str::is_empty)
    };
    objects(id)
        || id
            .strip_prefix("rows")
            .and_then(index)
            .and_then(|rest| rest.strip_prefix('.'))
            .is_some_and(objects)
}

/// Why `id` cannot be an object's `id`, if it cannot: it is 1 to
/// [`LONGEST_ID`] ASCII letters, digits and [`ID_PUNCTUATION`].
pub(crate) fn id_problem(id: &str) -> Option<String> {
    if id.is_empty() {
        return Some("an ID cannot be empty".to_owned());
    }
    if is_place(id) {
        return None;
    }
    if let Some(c) = id
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || ID_PUNCTUATION.contains(*c)))
    {
        let why = match c {
            ID_SEPARATOR => format!(
                "; `{ID_SEPARATOR}` joins a sheet's name to the ID, and is not written in the ID"
            ),
            '[' | ']' => "; an object with no ID takes its place, such as `objects[3]`, and a \
                           place is written as an ID only whole, as `objects[3]` or \
                           `rows[1].objects[0]`"
                .to_owned(),
            _ => String::new(),
        };
        return Some(format!(
            "an ID holds ASCII letters, digits and `{}` only, not `{}`{why}",
            ID_PUNCTUATION
                .chars()
                .map(String::from)
                .collect::<Vec<_>>()
                .join("`, `"),
            c.escape_default()
        ));
    }
    if id.len() > LONGEST_ID {
        return Some(format!("an ID has at most {LONGEST_ID} characters"));
    }
    None
}

/// The name an object carries, which no object before it on the sheet
/// carries, ignoring case.
fn object_name(node: &Node, path: &str, before: &[ObjectRecord]) -> Result<String, Error> {
    let name = scalar(node, path, "a name, text such as `jb`")?;
    let problem = if name.is_empty() {
        Some("a name cannot be empty".to_owned())
    } else if name.chars().count() > LONGEST_NAME {
        Some(format!("a name has at most {LONGEST_NAME} characters"))
    } else if name.trim() != name {
        Some("a name cannot start or end with a space".to_owned())
    } else {
        name.chars().find(|c| c.is_control()).map(|c| {
            format!(
                "a name cannot hold the control character `{}`",
                c.escape_default()
            )
        })
    };
    if let Some(problem) = problem {
        return Err(bad_value(node, path, problem));
    }
    let folded = name.to_lowercase();
    let first = before.iter().find(|r| {
        r.name
            .as_deref()
            .is_some_and(|n| n.to_lowercase() == folded)
    });
    if let Some(first) = first {
        return Err(at(
            Error::new(
                ErrorCode::DuplicateName,
                format!(
                    "the name `{name}` is given at line {} already, to the object at `{}`; an \
                     agent selects an object by its name, so each name is given once on a \
                     sheet, and names that differ only in case are one name",
                    first.object.line, first.object.field
                ),
            ),
            path,
            node.at,
        ));
    }
    Ok(name.to_owned())
}

/// An object's ID: its sheet's name joined to the ID it carries in its file.
pub(crate) fn object_id(sheet: &str, record: &ObjectRecord) -> String {
    format!("{sheet}{ID_SEPARATOR}{}", record.id)
}
