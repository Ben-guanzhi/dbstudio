use std::fmt;

/// A typed failure for the database layer.
///
/// The [`dbstudio_db::Connection`](../dbstudio_db/struct.Connection.html)
/// facade used to surface every driver failure as an opaque
/// `anyhow::Error`, which forced the UI to string-match on error text to tell
/// "the connection is gone" apart from "this SQL is wrong". `DbError` makes
/// that distinction a value: the UI can decide to prompt a reconnect for
/// [`DbError::Lost`] without parsing, and keep displaying a plain message for
/// everything else.
///
/// Server-reported SQL failures (a well-formed `SELECT` that the database
/// rejected, e.g. a syntax error) still surface as
/// [`SqlResult::Error`](crate::result::SqlResult), not as `DbError` —
/// `DbError` is reserved for the transport/driver layer around the round trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbError {
    /// Connection establishment failed (authentication, host unreachable,
    /// refused). Arisen only on the connect path.
    Connect { message: String },
    /// The live connection was lost mid-session (network drop, server restart,
    /// pool exhaustion). The session is no longer usable and the UI should
    /// offer to reconnect.
    Lost { message: String },
    /// The operation timed out before the server responded.
    Timeout { message: String },
    /// A generic internal driver failure that does not fit the above.
    Other { message: String },
}

impl DbError {
    /// The human-readable message, regardless of variant.
    pub fn message(&self) -> &str {
        match self {
            DbError::Connect { message }
            | DbError::Lost { message }
            | DbError::Timeout { message }
            | DbError::Other { message } => message,
        }
    }

    /// Whether this failure means the session's connection is no longer
    /// usable and the UI should mark it disconnected / offer a reconnect.
    pub fn is_connection_lost(&self) -> bool {
        matches!(self, DbError::Lost { .. })
    }

    /// Wrap a connect-path failure.
    pub fn connect(message: impl Into<String>) -> Self {
        DbError::Connect {
            message: message.into(),
        }
    }

    /// Wrap a "live connection went away" failure.
    pub fn lost(message: impl Into<String>) -> Self {
        DbError::Lost {
            message: message.into(),
        }
    }

    /// Wrap a timeout failure.
    pub fn timeout(message: impl Into<String>) -> Self {
        DbError::Timeout {
            message: message.into(),
        }
    }

    /// Wrap a generic driver failure.
    pub fn other(message: impl Into<String>) -> Self {
        DbError::Other {
            message: message.into(),
        }
    }

    /// Best-effort classification of an opaque driver error into a
    /// [`DbError`] variant by scanning its source chain.
    ///
    /// This is intentionally conservative: only unambiguous markers are
    /// promoted to `Timeout` or `Lost`; anything unrecognized stays `Other`
    /// so a misclassification never turns a real SQL problem into a false
    /// "connection lost" prompt.
    pub fn from_anyhow(err: &anyhow::Error) -> Self {
        let chain: Vec<String> = err.chain().map(|e| e.to_string().to_lowercase()).collect();
        let text = chain.join(" | ");

        if text.contains("timed out")
            || text.contains("timeout")
            || text.contains("deadline exceeded")
        {
            return DbError::Timeout {
                message: chain.into_iter().last().unwrap_or_default(),
            };
        }
        // Connection-loss markers seen across the common driver/IO stack.
        if text.contains("connection reset")
            || text.contains("broken pipe")
            || text.contains("target machine")
            || text.contains("server has gone away")
            || text.contains("socket hang")
            || text.contains("connection lost")
            || text.contains("pool is empty")
            || (text.contains("connection") && text.contains("closed"))
            || (text.contains("connection") && text.contains("refused"))
        {
            return DbError::Lost {
                message: chain.into_iter().last().unwrap_or_default(),
            };
        }
        DbError::Other {
            message: chain.into_iter().last().unwrap_or_default(),
        }
    }
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for DbError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_is_explicit_and_not_a_lost() {
        let e = DbError::connect("could not reach host");
        assert!(matches!(e, DbError::Connect { .. }));
        assert!(!e.is_connection_lost());
        assert_eq!(e.message(), "could not reach host");
    }

    #[test]
    fn lost_is_reported_as_connection_lost() {
        let e = DbError::lost("connection reset by peer");
        assert!(e.is_connection_lost());
    }

    #[test]
    fn from_anyhow_promotes_timeout_markers() {
        let err = anyhow::anyhow!("request to server failed: deadline exceeded");
        assert!(matches!(DbError::from_anyhow(&err), DbError::Timeout { .. }));
    }

    #[test]
    fn from_anyhow_promotes_reset_to_lost() {
        let err = anyhow::anyhow!("tcp connect error: connection reset by peer");
        let classified = DbError::from_anyhow(&err);
        assert!(matches!(classified, DbError::Lost { .. }));
        assert!(classified.is_connection_lost());
    }

    #[test]
    fn from_anyhow_keeps_unknown_as_other() {
        let err = anyhow::anyhow!("weird driver bug");
        assert!(matches!(DbError::from_anyhow(&err), DbError::Other { .. }));
        assert!(!DbError::from_anyhow(&err).is_connection_lost());
    }

    #[test]
    fn sql_problems_are_not_misread_as_lost() {
        // A SQL error that merely contains the word "connection" in its text
        // must not be promoted to Lost (no loss marker present).
        let err = anyhow::anyhow!("relation \"connection_log\" does not exist");
        let classified = DbError::from_anyhow(&err);
        assert!(!classified.is_connection_lost());
    }

    #[test]
    fn db_error_converts_into_anyhow() {
        let e = DbError::other("boom");
        let a: anyhow::Error = e.into();
        assert_eq!(a.to_string(), "boom");
    }
}
