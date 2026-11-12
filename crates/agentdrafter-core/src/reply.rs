//! The shape every interface sends back for one request.

use serde::Serialize;

use crate::change::RolledBack;
use crate::error::Error;

/// One request's outcome, ready to send. In JSON it is
/// `{"ok": true, "result": {...}}` or `{"ok": false, "error": {...}}`, or,
/// for a request carried out in part, such as a drawing set some of whose
/// sheets failed, `{"ok": false, "result": {...}, "error": {...}}`.
///
/// A command that first rolled back an entry of its drawing set's journal,
/// left pending by a commit that never finished, says so under
/// `rolled_back`, after `ok`, whatever came of the command itself.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Reply<T> {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rolled_back: Option<RolledBack>,
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
            rolled_back: None,
            result: Some(result),
            error: Some(error),
        }
    }

    /// The reply of a request that first rolled back `rolled_back`, when it
    /// rolled anything back.
    pub(crate) fn after(mut self, rolled_back: Option<RolledBack>) -> Self {
        if rolled_back.is_some() {
            self.rolled_back = rolled_back;
        }
        self
    }
}

impl<T> From<Result<T, Error>> for Reply<T> {
    fn from(outcome: Result<T, Error>) -> Self {
        match outcome {
            Ok(result) => Self {
                ok: true,
                rolled_back: None,
                result: Some(result),
                error: None,
            },
            Err(error) => Self {
                ok: false,
                rolled_back: None,
                result: None,
                error: Some(error),
            },
        }
    }
}
