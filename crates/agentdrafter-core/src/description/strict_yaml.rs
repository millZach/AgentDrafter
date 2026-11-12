//! Reads the strict YAML subset into a tree of text, lists and mappings.
//!
//! The subset is written down in `docs/sheet-description.md`. This reader
//! refuses anything outside it: anchors, aliases, tags, directives, more
//! than one document, keys that are not plain text, and a key given twice
//! in the same mapping. It gives scalars no type: every scalar is text, and
//! the schema in the parent module decides what the text means.
//!
//! Each node also keeps where it is written in the text and in what style,
//! so that an operation can rewrite one object of a record and leave every
//! other byte of the file as it was (`edit.rs`).

use saphyr_parser::{Event, Parser, ScalarStyle, Span};

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

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Node {
    pub kind: Kind,
    pub at: Pos,
    /// Where the node is written in the text it was read from, and how.
    /// `None` for a node an edit made, which is written out afresh.
    pub source: Option<Source>,
    /// Whether a node an edit made is a bare null ([`Node::bare_null`]).
    /// A node read from a text is one when its source says so.
    pub made_null: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Kind {
    Scalar(String),
    Sequence(Vec<Node>),
    Mapping(Vec<Entry>),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Entry {
    pub key: String,
    pub key_at: Pos,
    /// The byte just after the key in the text, or 0 for an entry an edit
    /// made.
    pub key_end: usize,
    pub value: Node,
}

/// Where a node is written: byte offsets in the text, without a leading
/// byte-order mark, from its first character to just after its last one.
/// A block scalar starts at its first line of contents, after its `|` or
/// `>`; a block collection ends where its last item or value ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Source {
    pub start: usize,
    pub end: usize,
    pub style: Style,
}

/// How a node is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Style {
    /// A plain scalar, such as `100` or `4 in`.
    Plain,
    /// A scalar in single or double quotes.
    Quoted,
    /// A literal (`|`) or folded (`>`) block scalar.
    BlockScalar,
    /// A list or mapping in brackets or braces.
    Flow,
    /// A list or mapping written one item or key to a line.
    Block,
}

impl Node {
    /// A node an edit made, which has no place in a text yet.
    pub(super) fn new(kind: Kind) -> Self {
        Self {
            kind,
            at: Pos { line: 0, column: 0 },
            source: None,
            made_null: false,
        }
    }

    /// A scalar an edit made.
    pub(super) fn scalar(text: impl Into<String>) -> Self {
        Self::new(Kind::Scalar(text.into()))
    }

    /// A bare null an edit made, written `null`. A change set sends its
    /// values as JSON, and JSON's `null` is what a YAML reader that guesses
    /// types makes of a bare null, so a `null` it sends is written as one,
    /// and read as the file's own bare nulls are.
    pub(super) fn bare_null() -> Self {
        Self {
            made_null: true,
            ..Self::scalar("null")
        }
    }

    /// Whether the node is a bare null: a plain scalar that a YAML reader
    /// which guesses types reads as null, `null`, `Null`, `NULL` or `~`, or
    /// a value left empty, as in `name:`. Zach decided on 2026-11-11 that a
    /// bare null on an optional text field means the field is not written,
    /// and on a required one is an error; quoted, `'null'` is the word. A
    /// node an edit made is written so that it reads back as its text,
    /// unless it is a bare null an edit made ([`Node::bare_null`]).
    pub(super) fn is_bare_null(&self) -> bool {
        match (&self.kind, self.source) {
            (Kind::Scalar(text), Some(source)) => {
                source.style == Style::Plain && reads_as_null(text)
            }
            (Kind::Scalar(_), None) => self.made_null,
            _ => false,
        }
    }

    /// Whether `self` and `other` hold the same scalars, lists and
    /// mappings, keys in the same order, wherever and however each is
    /// written. A bare null is not the same as text.
    pub(super) fn same_as(&self, other: &Node) -> bool {
        match (&self.kind, &other.kind) {
            (Kind::Scalar(a), Kind::Scalar(b)) => {
                a == b && self.is_bare_null() == other.is_bare_null()
            }
            (Kind::Sequence(a), Kind::Sequence(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.same_as(b))
            }
            (Kind::Mapping(a), Kind::Mapping(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|(a, b)| a.key == b.key && a.value.same_as(&b.value))
            }
            _ => false,
        }
    }
}

/// Whether `text`, written plain, is what a YAML reader that guesses types
/// reads as null: YAML 1.2's `null`, `Null`, `NULL`, `~` and nothing.
pub(super) fn reads_as_null(text: &str) -> bool {
    matches!(text, "" | "~" | "null" | "Null" | "NULL")
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
        opened: Opened,
        items: Vec<Node>,
    },
    Mapping {
        at: Pos,
        opened: Opened,
        entries: Vec<Entry>,
        key: Option<(String, Pos, usize)>,
    },
}

/// Where a list or mapping opened, and whether in brackets or braces.
#[derive(Clone, Copy)]
struct Opened {
    start: usize,
    flow: bool,
}

/// Byte offsets of a text's characters: saphyr counts its places in
/// characters, and an edit splices bytes.
struct Offsets(Vec<usize>);

impl Offsets {
    fn of(text: &str) -> Self {
        let mut offsets: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
        offsets.push(text.len());
        Self(offsets)
    }

    fn byte(&self, character: usize) -> usize {
        self.0[character.min(self.0.len() - 1)]
    }
}

/// Reads `text` as the strict YAML subset. Returns `None` for a document
/// that holds nothing.
///
/// A leading UTF-8 byte-order mark is dropped first, so every rule of the
/// subset applies as if it were not there.
pub(super) fn read(text: &str) -> Result<Option<Node>, Error> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    refuse_directives(text)?;
    let offsets = Offsets::of(text);
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
            Event::Scalar(value, style, anchor, tag) => {
                plain_node(anchor, tag.is_some()).map_err(refuse)?;
                let start = offsets.byte(span.start.index());
                let mut end = offsets.byte(span.end.index());
                let style = match style {
                    ScalarStyle::Plain => Style::Plain,
                    ScalarStyle::SingleQuoted | ScalarStyle::DoubleQuoted => Style::Quoted,
                    ScalarStyle::Literal | ScalarStyle::Folded => {
                        // A block scalar's span runs on to the next token;
                        // it ends where its last line of contents does.
                        end = start + text[start..end].trim_end().len();
                        Style::BlockScalar
                    }
                };
                let node = Node {
                    kind: Kind::Scalar(value.into_owned()),
                    at,
                    source: Some(Source { start, end, style }),
                    made_null: false,
                };
                place(node, &mut stack, &mut root)?;
            }
            Event::SequenceStart(anchor, tag) => {
                plain_node(anchor, tag.is_some()).map_err(refuse)?;
                open_collection(&stack).map_err(refuse)?;
                stack.push(Frame::Sequence {
                    at,
                    opened: opened(&span, &offsets),
                    items: Vec::new(),
                });
            }
            Event::MappingStart(anchor, tag) => {
                plain_node(anchor, tag.is_some()).map_err(refuse)?;
                open_collection(&stack).map_err(refuse)?;
                stack.push(Frame::Mapping {
                    at,
                    opened: opened(&span, &offsets),
                    entries: Vec::new(),
                    key: None,
                });
            }
            Event::SequenceEnd | Event::MappingEnd => {
                // A flow collection ends just after its bracket or brace:
                // the parser's span runs on over a comment after it.
                let closed = offsets.byte(span.start.index()) + 1;
                let node = match stack.pop() {
                    Some(Frame::Sequence { at, opened, items }) => {
                        let last = items.last().and_then(|i| i.source).map(|s| s.end);
                        Node {
                            source: Some(closed_source(opened, closed, last)),
                            kind: Kind::Sequence(items),
                            at,
                            made_null: false,
                        }
                    }
                    Some(Frame::Mapping {
                        at,
                        opened,
                        entries,
                        ..
                    }) => {
                        let last = entries.last().and_then(|e| e.value.source).map(|s| s.end);
                        Node {
                            source: Some(closed_source(opened, closed, last)),
                            kind: Kind::Mapping(entries),
                            at,
                            made_null: false,
                        }
                    }
                    None => unreachable!("the YAML parser closed a collection it never opened"),
                };
                place(node, &mut stack, &mut root)?;
            }
        }
    }
    Ok(root)
}

/// Where a list or mapping opens: a flow one opens with its bracket or
/// brace, which its span covers; a block one's span is empty.
fn opened(span: &Span, offsets: &Offsets) -> Opened {
    Opened {
        start: offsets.byte(span.start.index()),
        flow: span.end.index() > span.start.index(),
    }
}

/// Where a list or mapping is written, once it has closed: a flow one ends
/// at `closed`, just after its closing bracket or brace, and a block one
/// where its last item or value ends, at `last`, not where the next token
/// starts.
fn closed_source(opened: Opened, closed: usize, last: Option<usize>) -> Source {
    if opened.flow {
        Source {
            start: opened.start,
            end: closed,
            style: Style::Flow,
        }
    } else {
        Source {
            start: opened.start,
            end: last.unwrap_or(opened.start),
            style: Style::Block,
        }
    }
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
            Some((name, key_at, key_end)) => entries.push(Entry {
                key: name,
                key_at,
                key_end,
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
                let key_end = node.source.map_or(0, |s| s.end);
                *key = Some((name, node.at, key_end));
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
                key: Some((name, _, _)),
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
