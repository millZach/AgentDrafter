//! Writes nodes of the strict YAML subset back out as text, for an
//! operation that rewrites one object of a record (`edit.rs`).
//!
//! A node keeps the style it was read in: a list or mapping written in
//! brackets or braces stays so, and one written a key or item to a line
//! stays so. A scalar the edit did not touch is written exactly as the file
//! wrote it, quotes and all, wherever that is allowed. A node an edit made
//! is written in a plain style: a list of scalars or of points in brackets,
//! a mapping in braces when it fits on a line, a key to a line otherwise,
//! and a scalar plain unless YAML would read it as something else.
//!
//! Whatever is written is read back before it is saved, and must read as
//! the tree the edit made, so a mistake here refuses the edit rather than
//! changing a record unseen.

use super::strict_yaml::{reads_as_null, Kind, Node, Style};

/// The widest line a node an edit made is written on in braces or brackets,
/// in characters, before it is written a key to a line instead.
const WIDEST_FLOW: usize = 100;

/// Where a scalar is written: inside brackets or braces, where `,`, `[`,
/// `]`, `{`, `}` and `:` end it, or anywhere else.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Context {
    Flow,
    Block,
}

/// Writes nodes read from `text`, ending lines with `newline`.
pub(super) struct Emitter<'a> {
    pub text: &'a str,
    pub newline: &'a str,
}

impl Emitter<'_> {
    /// `node` as an item of a list written an item to a line, starting just
    /// after its `- ` at `column`: its first line, then any more, each
    /// indented, with no line break at the end.
    pub(super) fn item(&self, node: &Node, column: usize) -> String {
        let mut out = String::new();
        self.in_item(node, column, &mut out);
        out
    }

    /// `node` in brackets and braces, on one line.
    pub(super) fn flow(&self, node: &Node) -> String {
        let mut out = String::new();
        self.write_flow(node, &mut out);
        out
    }

    /// `node` as the value of a key at `column`, starting just after the
    /// key's `:`: a space and the value, or a line break and the value
    /// indented under the key.
    #[cfg(test)]
    pub(super) fn after_key(&self, node: &Node, column: usize) -> String {
        let mut out = String::new();
        self.write_after_key(node, column, &mut out);
        out
    }

    /// The text of a scalar node in `context`; a scalar of several lines
    /// written at `indent` in a block. Its own text when the edit did not
    /// touch it and that text can stand here.
    pub(super) fn scalar(&self, node: &Node, context: Context, indent: usize) -> String {
        let Kind::Scalar(value) = &node.kind else {
            return self.flow(node);
        };
        // A bare null an edit made is written as one: quoted, it would be
        // text.
        if node.source.is_none() && node.is_bare_null() {
            return value.clone();
        }
        if let Some(source) = node.source {
            let raw = &self.text[source.start..source.end];
            let one_line = !raw.contains(['\n', '\r']);
            match source.style {
                Style::Quoted if one_line => return raw.to_owned(),
                Style::Plain if one_line && raw == value && plain_ok(value, context) => {
                    return raw.to_owned()
                }
                // A bare null stays one: quoted, it would be text.
                Style::Plain if node.is_bare_null() && raw == value => return raw.to_owned(),
                _ => {}
            }
        }
        scalar_text(value, context, indent, self.newline)
    }

    fn in_item(&self, node: &Node, column: usize, out: &mut String) {
        match &node.kind {
            Kind::Scalar(_) => out.push_str(&self.scalar(node, Context::Block, column)),
            _ if self.is_flow(node, column) => self.write_flow(node, out),
            _ => self.block(node, column, out),
        }
    }

    fn write_after_key(&self, node: &Node, column: usize, out: &mut String) {
        match &node.kind {
            Kind::Scalar(_) => {
                // A value left empty is written as it was: nothing after
                // the key.
                let text = self.scalar(node, Context::Block, column + 2);
                if !text.is_empty() {
                    out.push(' ');
                    out.push_str(&text);
                }
            }
            _ if self.is_flow(node, column + 2) => {
                out.push(' ');
                self.write_flow(node, out);
            }
            _ => {
                out.push_str(self.newline);
                push_indent(out, column + 2);
                self.block(node, column + 2, out);
            }
        }
    }

    /// A list or mapping written an item or key to a line, starting at
    /// `column` on the current line.
    fn block(&self, node: &Node, column: usize, out: &mut String) {
        match &node.kind {
            Kind::Mapping(entries) => {
                for (i, entry) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push_str(self.newline);
                        push_indent(out, column);
                    }
                    out.push_str(&key_text(&entry.key, Context::Block));
                    out.push(':');
                    self.write_after_key(&entry.value, column, out);
                }
            }
            Kind::Sequence(items) => {
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(self.newline);
                        push_indent(out, column);
                    }
                    out.push_str("- ");
                    self.in_item(item, column + 2, out);
                }
            }
            Kind::Scalar(_) => out.push_str(&self.scalar(node, Context::Block, column)),
        }
    }

    fn write_flow(&self, node: &Node, out: &mut String) {
        match &node.kind {
            Kind::Scalar(_) => out.push_str(&self.scalar(node, Context::Flow, 0)),
            Kind::Sequence(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    self.write_flow(item, out);
                }
                out.push(']');
            }
            Kind::Mapping(entries) => {
                out.push('{');
                for (i, entry) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&key_text(&entry.key, Context::Flow));
                    out.push_str(": ");
                    self.write_flow(&entry.value, out);
                }
                out.push('}');
            }
        }
    }

    /// Whether a list or mapping is written in brackets or braces: as the
    /// file wrote it, or, for one an edit made, when it is empty, holds only
    /// scalars or points, or fits on a line from `column`.
    fn is_flow(&self, node: &Node, column: usize) -> bool {
        let empty = match &node.kind {
            Kind::Sequence(items) => items.is_empty(),
            Kind::Mapping(entries) => entries.is_empty(),
            Kind::Scalar(_) => return false,
        };
        if empty {
            return true;
        }
        match node.source.map(|s| s.style) {
            Some(Style::Flow) => return true,
            Some(Style::Block) => return false,
            _ => {}
        }
        if has_lines(node) {
            return false;
        }
        // An object, `line: {...}`, is a mapping of one key whose value is
        // a list or mapping: its key starts its line.
        if let Kind::Mapping(entries) = &node.kind {
            if let [entry] = entries.as_slice() {
                if !matches!(entry.value.kind, Kind::Scalar(_)) {
                    return false;
                }
            }
        }
        if let Kind::Sequence(items) = &node.kind {
            let leaf = |n: &Node| match &n.kind {
                Kind::Scalar(_) => true,
                Kind::Sequence(inner) => inner.iter().all(|i| matches!(i.kind, Kind::Scalar(_))),
                Kind::Mapping(_) => false,
            };
            if items.iter().all(leaf) {
                return true;
            }
        }
        column + self.flow(node).chars().count() <= WIDEST_FLOW
    }
}

/// Whether any scalar in `node` holds a line break.
fn has_lines(node: &Node) -> bool {
    match &node.kind {
        Kind::Scalar(text) => text.contains('\n'),
        Kind::Sequence(items) => items.iter().any(has_lines),
        Kind::Mapping(entries) => entries.iter().any(|e| has_lines(&e.value)),
    }
}

fn push_indent(out: &mut String, column: usize) {
    out.extend(std::iter::repeat_n(' ', column));
}

/// A mapping's key as written: plain when it can be, quoted otherwise.
fn key_text(key: &str, context: Context) -> String {
    if plain_ok(key, context) {
        key.to_owned()
    } else {
        quoted(key)
    }
}

/// `value` written as a scalar in `context`: plain when YAML reads it back
/// as the same text, as a literal block at `indent` when it holds lines and
/// stands in a block, and quoted otherwise.
pub(super) fn scalar_text(value: &str, context: Context, indent: usize, newline: &str) -> String {
    if plain_ok(value, context) {
        return value.to_owned();
    }
    if context == Context::Block {
        if let Some(block) = literal(value, indent, newline) {
            return block;
        }
    }
    quoted(value)
}

/// Whether `value` reads back as itself written plain in `context`. Kept
/// narrow on purpose: anything doubtful is quoted.
pub(super) fn plain_ok(value: &str, context: Context) -> bool {
    // Written plain, `null` or `~` would read back as a bare null.
    if reads_as_null(value) {
        return false;
    }
    let Some(first) = value.chars().next() else {
        return false;
    };
    if value.starts_with([' ', '\t']) || value.ends_with([' ', '\t', ':']) {
        return false;
    }
    if value.chars().any(|c| c.is_control() || needs_escape(c)) {
        return false;
    }
    if value.contains(": ") || value.contains(" #") || value.contains('\t') {
        return false;
    }
    if value.starts_with("---") || value.starts_with("...") {
        return false;
    }
    let second = value.chars().nth(1);
    let leading = match first {
        '-' => second.is_some_and(|c| c.is_ascii_alphanumeric() || c == '.'),
        '?' | ':' | ',' | '[' | ']' | '{' | '}' | '#' | '&' | '*' | '!' | '|' | '>' | '\''
        | '"' | '%' | '@' | '`' => false,
        _ => true,
    };
    if !leading {
        return false;
    }
    context == Context::Block || !value.contains([',', '[', ']', '{', '}', ':'])
}

/// Characters a scalar never holds unescaped: YAML reads some as line
/// breaks or drops them.
fn needs_escape(c: char) -> bool {
    matches!(c, '\u{85}' | '\u{2028}' | '\u{2029}' | '\u{FEFF}')
}

/// `value` in quotes: single when it holds no control characters, double
/// with escapes otherwise.
fn quoted(value: &str) -> String {
    if !value.chars().any(|c| c.is_control() || needs_escape(c)) {
        return format!("'{}'", value.replace('\'', "''"));
    }
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() || needs_escape(c) => {
                out.push_str(&format!("\\u{:04X}", u32::from(c)));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `value`, which holds lines, as a literal block whose lines are indented
/// to `indent`, when it can be written so and read back the same: no
/// control characters but tabs, no line of spaces alone, its first line of
/// text not starting with a space, and at most one line break at its end.
fn literal(value: &str, indent: usize, newline: &str) -> Option<String> {
    if !value.contains('\n') {
        return None;
    }
    if value
        .chars()
        .any(|c| (c.is_control() && c != '\n' && c != '\t') || needs_escape(c))
    {
        return None;
    }
    let body = value.strip_suffix('\n').unwrap_or(value);
    if body.ends_with('\n') || body.is_empty() {
        return None;
    }
    let lines: Vec<&str> = body.split('\n').collect();
    if lines
        .iter()
        .any(|line| !line.is_empty() && line.trim_matches([' ', '\t']).is_empty())
    {
        return None;
    }
    let first_text = lines.iter().find(|line| !line.is_empty())?;
    if first_text.starts_with([' ', '\t']) {
        return None;
    }
    let mut out = String::from(if value.ends_with('\n') { "|" } else { "|-" });
    for line in lines {
        out.push_str(newline);
        if !line.is_empty() {
            push_indent(&mut out, indent);
            out.push_str(line);
        }
    }
    Some(out)
}

/// A number as a record writes it: as few digits as hold it, without the
/// noise of adding binary fractions, such as `10.6` rather than
/// `10.600000000000001`; never with a sign on zero.
pub(crate) fn number_text(value: f64) -> String {
    let value = value + 0.0;
    let shortest = format!("{value}");
    let scale = value.abs().max(1.0);
    for decimals in 0..=12 {
        let rounded = format!("{value:.decimals$}");
        let back: f64 = rounded.parse().unwrap_or(f64::NAN);
        if (back - value).abs() <= scale * 1e-12 {
            let rounded = if rounded.starts_with('-') && back == 0.0 {
                rounded[1..].to_owned()
            } else {
                rounded
            };
            if rounded.len() <= shortest.len() {
                return rounded;
            }
            break;
        }
    }
    shortest
}

#[cfg(test)]
mod tests {
    use super::super::strict_yaml;
    use super::*;

    fn read(text: &str) -> Node {
        strict_yaml::read(text).unwrap().unwrap()
    }

    #[test]
    fn numbers_drop_the_noise_of_binary_fractions() {
        assert_eq!(number_text(0.1 + 0.2), "0.3");
        assert_eq!(number_text(10.5 + 0.1), "10.6");
        assert_eq!(number_text(17.0), "17");
        assert_eq!(number_text(-0.0), "0");
        assert_eq!(number_text(-1e-14), "0");
        assert_eq!(number_text(0.123456789), "0.123456789");
        assert_eq!(number_text(-2.5), "-2.5");
        assert_eq!(number_text(1e-7), "0.0000001");
    }

    #[test]
    fn a_scalar_that_would_read_as_something_else_is_quoted() {
        for (value, written) in [
            ("plain", "plain"),
            ("4 in", "4 in"),
            ("-0.5", "-0.5"),
            ("", "''"),
            ("null", "'null'"),
            ("NULL", "'NULL'"),
            ("~", "'~'"),
            ("nullable", "nullable"),
            ("- item", "'- item'"),
            ("a: b", "'a: b'"),
            ("it's", "it's"),
            ("'quoted'", "'''quoted'''"),
            ("# not a comment", "'# not a comment'"),
            (" spaced", "' spaced'"),
            ("tab\there", "\"tab\\there\""),
        ] {
            assert_eq!(scalar_text(value, Context::Block, 0, "\n"), written);
        }
        assert_eq!(
            scalar_text("objects[2]", Context::Block, 0, "\n"),
            "objects[2]"
        );
        assert_eq!(
            scalar_text("objects[2]", Context::Flow, 0, "\n"),
            "'objects[2]'"
        );
        assert_eq!(scalar_text("a, b", Context::Flow, 0, "\n"), "'a, b'");
    }

    #[test]
    fn lines_are_a_literal_block_in_a_block_and_escaped_in_braces() {
        assert_eq!(
            scalar_text("NOTES:\n1. ONE.\n", Context::Block, 4, "\n"),
            "|\n    NOTES:\n    1. ONE."
        );
        assert_eq!(
            scalar_text("A\n\nB", Context::Block, 2, "\n"),
            "|-\n  A\n\n  B"
        );
        assert_eq!(
            scalar_text("A\nB\n", Context::Flow, 0, "\n"),
            "\"A\\nB\\n\""
        );
        assert_eq!(scalar_text(" A\nB", Context::Block, 2, "\n"), "\" A\\nB\"");
    }

    #[test]
    fn what_is_written_reads_back_as_the_same_tree() {
        let text = "objects:\n  - line: {from: [0, 0, 0], to: ['4 in', 1, 0], id: a}\n  - mtext:\n      contents: |\n        NOTES:\n        1. X.\n      position: [1, 2, 0]\n      name: \"it's: odd\"\n";
        let root = read(text);
        let Kind::Mapping(entries) = &root.kind else {
            panic!()
        };
        let emitter = Emitter {
            text,
            newline: "\n",
        };
        let written = format!("objects:{}\n", emitter.after_key(&entries[0].value, 0));
        assert!(read(&written).same_as(&root), "{written}");
        assert_eq!(written, text);
    }

    #[test]
    fn a_bare_null_is_written_back_bare_and_the_text_null_quoted() {
        let text = "objects:\n  - circle: {center: [0, 0, 0], radius: 1, name: ~, layer: 'null', id: }\n  - text:\n      contents: A\n      style: null\n      id:\n";
        let root = read(text);
        let Kind::Mapping(entries) = &root.kind else {
            panic!()
        };
        let emitter = Emitter {
            text,
            newline: "\n",
        };
        let written = format!("objects:{}\n", emitter.after_key(&entries[0].value, 0));
        assert!(read(&written).same_as(&root), "{written}");
        assert_eq!(written, text);
        // Text an edit made is quoted where it would read as a bare null.
        assert!(!read("name: null\n").same_as(&read("name: 'null'\n")));
        assert_eq!(scalar_text("null", Context::Flow, 0, "\n"), "'null'");
        // A bare null an edit made is written bare, and reads back as one.
        let mut made = read("circle: {radius: 1, layer: notes}\n");
        forget(&mut made);
        let Kind::Mapping(entries) = &mut made.kind else {
            panic!()
        };
        let Kind::Mapping(keys) = &mut entries[0].value.kind else {
            panic!()
        };
        keys[1].value = Node::bare_null();
        let emitter = Emitter {
            text: "",
            newline: "\n",
        };
        let written = emitter.item(&made, 2);
        assert_eq!(written, "circle: {radius: 1, layer: null}");
        assert!(read(&written).same_as(&made));
        assert!(!read("layer: 'null'\n").same_as(&read(&format!(
            "layer: {}\n",
            emitter.flow(&Node::bare_null())
        ))));
    }

    #[test]
    fn a_node_an_edit_made_is_written_in_braces_when_it_fits() {
        let mut root = read("objects:\n  - line: {from: [0, 0, 0], to: [1, 1, 0]}\n");
        let Kind::Mapping(entries) = &mut root.kind else {
            panic!()
        };
        let Kind::Sequence(items) = &mut entries[0].value.kind else {
            panic!()
        };
        let mut fresh = items[0].clone();
        forget(&mut fresh);
        let emitter = Emitter {
            text: "",
            newline: "\n",
        };
        assert_eq!(
            emitter.item(&fresh, 4),
            "line: {from: [0, 0, 0], to: [1, 1, 0]}"
        );
        let long = read(&format!(
            "insert: {{block: B, position: [0, 0, 0], attributes: {{TAG: {}}}}}",
            "X".repeat(90)
        ));
        let mut long = long;
        forget(&mut long);
        let written = emitter.item(&long, 4);
        assert!(
            written.starts_with("insert:\n      block: B\n"),
            "{written}"
        );
        assert!(read(&written).same_as(&long));
    }

    /// Makes `node` and everything in it a node an edit made.
    fn forget(node: &mut Node) {
        node.source = None;
        match &mut node.kind {
            Kind::Scalar(_) => {}
            Kind::Sequence(items) => items.iter_mut().for_each(forget),
            Kind::Mapping(entries) => entries.iter_mut().for_each(|e| forget(&mut e.value)),
        }
    }
}
