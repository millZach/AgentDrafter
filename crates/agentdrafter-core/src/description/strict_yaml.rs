//! Reads the strict YAML subset into a tree of text, lists and mappings.
//!
//! The subset is written down in `docs/sheet-description.md`. This reader
//! refuses anything outside it: anchors, aliases, tags, directives, more
//! than one document, keys that are not plain text, and a key given twice
//! in the same mapping. It gives scalars no type: every scalar is text, and
//! the schema in the parent module decides what the text means.

use saphyr_parser::{Event, Parser, Span};

use crate::error::{Error, ErrorCode};

/// A place in the sheet description, counted from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pos {
    pub line: usize,
    pub column: usize,
}

impl Pos {
    fn of(span: &Span) -> Self {
        Self {
            line: span.start.line(),
            column: span.start.col() + 1,
        }
    }
}

#[derive(Debug)]
pub(super) struct Node {
    pub kind: Kind,
    pub at: Pos,
}

#[derive(Debug)]
pub(super) enum Kind {
    Scalar(String),
    Sequence(Vec<Node>),
    Mapping(Vec<Entry>),
}

#[derive(Debug)]
pub(super) struct Entry {
    pub key: String,
    pub key_at: Pos,
    pub value: Node,
}

/// The path to a child of the field at `parent`: `objects[0]` and `line` give `objects[0].line`.
pub(super) fn key_path(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        key.to_owned()
    } else {
        format!("{parent}.{key}")
    }
}

/// The path to item `index` of the list at `parent`: `objects` and 0 give `objects[0]`.
pub(super) fn index_path(parent: &str, index: usize) -> String {
    format!("{parent}[{index}]")
}

/// A collection still being read.
enum Frame {
    Sequence {
        at: Pos,
        items: Vec<Node>,
    },
    Mapping {
        at: Pos,
        entries: Vec<Entry>,
        key: Option<(String, Pos)>,
    },
}

/// Reads `text` as the strict YAML subset. Returns `None` for a document
/// that holds nothing.
///
/// A leading UTF-8 byte-order mark is dropped first, so every rule of the
/// subset applies as if it were not there.
pub(super) fn read(text: &str) -> Result<Option<Node>, Error> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    refuse_directives(text)?;
    let mut stack: Vec<Frame> = Vec::new();
    let mut root: Option<Node> = None;
    let mut documents = 0;
    for item in Parser::new_from_str(text) {
        let (event, span) = item.map_err(|e| {
            let error = Error::new(ErrorCode::YamlSyntax, e.info().to_owned())
                .at_position(e.marker().line(), e.marker().col() + 1);
            with_path(error, &stack)
        })?;
        let at = Pos::of(&span);
        let refuse = |message: &str| {
            with_path(
                Error::new(ErrorCode::NotInSubset, message).at_position(at.line, at.column),
                &stack,
            )
        };
        match event {
            Event::StreamStart | Event::StreamEnd | Event::DocumentEnd | Event::Nothing => {}
            Event::DocumentStart(_) => {
                documents += 1;
                if documents > 1 {
                    return Err(refuse(
                        "a sheet description is one YAML document; remove the second `---`",
                    ));
                }
            }
            Event::Alias(_) => {
                return Err(refuse(
                    "aliases (`*name`) are not allowed in a sheet description; write the value out",
                ))
            }
            Event::Scalar(value, _style, anchor, tag) => {
                plain_node(anchor, tag.is_some()).map_err(refuse)?;
                let node = Node {
                    kind: Kind::Scalar(value.into_owned()),
                    at,
                };
                place(node, &mut stack, &mut root)?;
            }
            Event::SequenceStart(anchor, tag) => {
                plain_node(anchor, tag.is_some()).map_err(refuse)?;
                open_collection(&stack).map_err(refuse)?;
                stack.push(Frame::Sequence {
                    at,
                    items: Vec::new(),
                });
            }
            Event::MappingStart(anchor, tag) => {
                plain_node(anchor, tag.is_some()).map_err(refuse)?;
                open_collection(&stack).map_err(refuse)?;
                stack.push(Frame::Mapping {
                    at,
                    entries: Vec::new(),
                    key: None,
                });
            }
            Event::SequenceEnd | Event::MappingEnd => {
                let node = match stack.pop() {
                    Some(Frame::Sequence { at, items }) => Node {
                        kind: Kind::Sequence(items),
                        at,
                    },
                    Some(Frame::Mapping { at, entries, .. }) => Node {
                        kind: Kind::Mapping(entries),
                        at,
                    },
                    None => unreachable!("the YAML parser closed a collection it never opened"),
                };
                place(node, &mut stack, &mut root)?;
            }
        }
    }
    Ok(root)
}

/// Refuses a node that carries an anchor or a tag.
fn plain_node(anchor: usize, tagged: bool) -> Result<(), &'static str> {
    if anchor != 0 {
        return Err(
            "anchors (`&name`) are not allowed in a sheet description; write each value out",
        );
    }
    if tagged {
        return Err("tags (such as `!!str` or `!name`) are not allowed in a sheet description");
    }
    Ok(())
}

/// Refuses a list or mapping that opens where a mapping key belongs.
fn open_collection(stack: &[Frame]) -> Result<(), &'static str> {
    match stack.last() {
        Some(Frame::Mapping { key: None, .. }) => {
            Err("a key must be plain text, not a list or a mapping")
        }
        _ => Ok(()),
    }
}

/// Puts a finished node where it belongs: as the root, as the next item of
/// a list, or as the next key or value of a mapping.
fn place(node: Node, stack: &mut [Frame], root: &mut Option<Node>) -> Result<(), Error> {
    let path = path_of(stack);
    match stack.last_mut() {
        None => *root = Some(node),
        Some(Frame::Sequence { items, .. }) => items.push(node),
        Some(Frame::Mapping { entries, key, .. }) => match key.take() {
            Some((name, key_at)) => entries.push(Entry {
                key: name,
                key_at,
                value: node,
            }),
            None => {
                let Kind::Scalar(name) = node.kind else {
                    unreachable!("collections as keys are refused when they open")
                };
                if let Some(first) = entries.iter().find(|e| e.key == name) {
                    return Err(Error::new(
                        ErrorCode::DuplicateKey,
                        format!(
                            "duplicate key `{name}`: it is first given at line {}",
                            first.key_at.line
                        ),
                    )
                    .at_field(key_path(&path, &name))
                    .at_position(node.at.line, node.at.column));
                }
                *key = Some((name, node.at));
            }
        },
    }
    Ok(())
}

/// The path of the place the reader is at, such as `objects[0].line`.
fn path_of(stack: &[Frame]) -> String {
    let mut path = String::new();
    for frame in stack {
        match frame {
            Frame::Sequence { items, .. } => path = index_path(&path, items.len()),
            Frame::Mapping {
                key: Some((name, _)),
                ..
            } => path = key_path(&path, name),
            Frame::Mapping { key: None, .. } => {}
        }
    }
    path
}

fn with_path(error: Error, stack: &[Frame]) -> Error {
    let path = path_of(stack);
    if path.is_empty() {
        error
    } else {
        error.at_field(path)
    }
}

/// Refuses YAML directives (`%YAML`, `%TAG`). A directive is a line that
/// starts with `%`; nothing else in the subset can start a line with it.
fn refuse_directives(text: &str) -> Result<(), Error> {
    match text.lines().position(|line| line.starts_with('%')) {
        Some(index) => Err(Error::new(
            ErrorCode::NotInSubset,
            "directives (lines starting with `%`) are not allowed in a sheet description",
        )
        .at_position(index + 1, 1)),
        None => Ok(()),
    }
}
