//! The shape every interface sends back for one request.

use serde::Serialize;

use crate::error::Error;

/// One request's outcome, ready to send. In JSON it is
/// `{"ok": true, "result": {...}}` or `{"ok": false, "error": {...}}`, or,
/// for a request carried out in part, such as a drawing set some of whose
/// sheets failed, `{"ok": false, "result": {...}, "error": {...}}`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Reply<T> {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
}

impl<T> Reply<T> {
    /// A request carried out in part: `result` says what was done and
    /// `error` why the rest was not. In JSON it is
    /// `{"ok": false, "result": {...}, "error": {...}}`.
    pub fn partly(result: T, error: Error) -> Self {
        Self {
            ok: false,
            result: Some(result),
            error: Some(error),
        }
    }
}

impl<T> From<Result<T, Error>> for Reply<T> {
    fn from(outcome: Result<T, Error>) -> Self {
        match outcome {
            Ok(result) => Self {
                ok: true,
                result: Some(result),
                error: None,
            },
            Err(error) => Self {
                ok: false,
                result: None,
                error: Some(error),
            },
        }
    }
}
