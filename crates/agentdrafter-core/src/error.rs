//! The one error type every core entry point returns.
//!
//! An error is plain data so that every interface reports it the same way.
//! It names what went wrong (`code`), says it in words (`message`), and,
//! where it can, points at the place: the field in the sheet description,
//! the line and column, and the file.

use std::fmt;
use std::path::Path;

use serde::Serialize;

/// What kind of error happened. Written in `snake_case` in JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The request itself is wrong, such as a missing argument.
    Usage,
    /// A file could not be read.
    ReadFailed,
    /// The sheet description is not well-formed YAML.
    YamlSyntax,
    /// The sheet description uses YAML outside the subset: an anchor, an
    /// alias, a tag, a directive, a second document or a key that is not text.
    NotInSubset,
    /// A key appears twice in the same mapping, or a block defines one
    /// attribute tag twice.
    DuplicateKey,
    /// Two objects on a sheet carry the same name, or names that differ
    /// only in case.
    DuplicateName,
    /// A key the sheet description does not know.
    UnknownKey,
    /// A key the sheet description needs is not there.
    MissingKey,
    /// The sheet declares no unit.
    MissingUnit,
    /// A unit that does not exist.
    UnknownUnit,
    /// A value of the wrong shape, such as text where a point belongs.
    WrongType,
    /// A value that should be a number is not one.
    BadNumber,
    /// A value of the right shape that is not allowed, such as a colour
    /// outside 1 to 255 or a radius that is not positive.
    BadValue,
    /// A name that refers to nothing the sheet description defines, such as
    /// a layer that is not under `layers`.
    UnknownName,
    /// The DWG to write already exists and replacing it was not asked for.
    OutputExists,
    /// Replacing was asked for, but the file at the output path is not a DWG
    /// AgentDrafter built, or cannot be read to tell. It may be an original,
    /// and an original is never overwritten.
    NotABuild,
    /// A file could not be written.
    WriteFailed,
    /// The DWG library could not write the drawing.
    DwgWrite,
    /// The DWG library could not read the drawing.
    DwgRead,
    /// Replacing was asked for, but the file at the output path is not an
    /// output AgentDrafter produced: it carries no stamp. It may be
    /// someone's own file, and is never overwritten.
    NotAnOutput,
    /// The file is not an output AgentDrafter produced, or its stamp
    /// cannot be read, so nothing says what it came from.
    NotStamped,
}

/// An error from the core, ready to report through any interface.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
    /// The field in the sheet description, such as `objects[0].line.from[2]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// The line in the file, counted from 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// The column in the line, counted from 1 in characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    /// The file the error is about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            field: None,
            line: None,
            column: None,
            file: None,
        }
    }

    /// A request that cannot be carried out as asked.
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Usage, message)
    }

    pub(crate) fn at_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    pub(crate) fn at_position(mut self, line: usize, column: usize) -> Self {
        self.line = Some(line);
        self.column = Some(column);
        self
    }

    pub(crate) fn in_file(mut self, file: &Path) -> Self {
        self.file = Some(file.display().to_string());
        self
    }

    /// Places the error in `file` unless it is about another file already,
    /// such as the block library a sheet description names.
    pub(crate) fn or_in_file(self, file: &Path) -> Self {
        if self.file.is_some() {
            self
        } else {
            self.in_file(file)
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(file) = &self.file {
            write!(f, "{file}: ")?;
        }
        if let (Some(line), Some(column)) = (self.line, self.column) {
            write!(f, "line {line}, column {column}: ")?;
        }
        if let Some(field) = &self.field {
            write!(f, "{field}: ")?;
        }
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}
