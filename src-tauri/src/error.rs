use serde::Serialize;

/// Everything that can go wrong, in a shape the UI can render.
///
/// The UI never sees a panic or a raw driver string: `code` drives behaviour
/// (e.g. showing the connect modal on `NotConnected`) and `message` is the
/// human-readable line shown in the grid footer or terminal.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not connected to a database")]
    NotConnected,

    #[error("connection failed: {0}")]
    Connect(String),

    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("TLS error: {0}")]
    Tls(String),

    /// An error reported by the server while running a statement. `detail`
    /// carries psql-style extras (LINE/HINT) when the server supplies them.
    #[error("{message}")]
    Query {
        message: String,
        detail: Option<String>,
        /// SQLSTATE, e.g. "42601".
        sqlstate: Option<String>,
    },

    /// A name is already taken. Distinct from `Invalid` so the frontend can
    /// recognise a collision and offer to overwrite, rather than string-matching
    /// the message.
    #[error("{0}")]
    Exists(String),

    #[error("query cancelled")]
    Cancelled,

    #[error("query timed out")]
    Timeout,

    #[error("no such session: {0}")]
    NoSession(String),

    #[error("invalid request: {0}")]
    Invalid(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("keychain error: {0}")]
    Keychain(String),
}

impl AppError {
    /// The stable string the UI branches on. Distinct from `message`, which is
    /// prose and may change freely: these are part of the wire contract, so
    /// renaming one silently breaks whatever frontend check depends on it.
    ///
    /// # Arguments
    /// * `&self` — `&AppError`: the error whose variant is being named.
    ///
    /// # Returns
    /// `&'static str` — the wire code for this variant; total, so there is no
    /// fallback string for the UI to handle.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotConnected => "not_connected",
            Self::Connect(_) => "connect",
            Self::Auth(_) => "auth",
            Self::Tls(_) => "tls",
            Self::Query { .. } => "query",
            Self::Exists(_) => "exists",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
            Self::NoSession(_) => "no_session",
            Self::Invalid(_) => "invalid",
            Self::Storage(_) => "storage",
            Self::Keychain(_) => "keychain",
        }
    }

    /// The psql-style DETAIL/HINT block, when the server sent one.
    ///
    /// Only ever present on a `Query`: no other variant has a server-side origin
    /// to have supplied extras. `None` is the common case even there, and means
    /// the terminal prints the message alone.
    ///
    /// # Arguments
    /// * `&self` — `&AppError`: the error to read extras from.
    ///
    /// # Returns
    /// `Option<String>` — the cloned DETAIL/HINT block, or `None` on every
    /// non-`Query` variant and on a `Query` the server sent no extras for.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::Query { detail, .. } => detail.clone(),
            _ => None,
        }
    }

    /// The five-character SQLSTATE, when one survived classification.
    ///
    /// `None` for errors that never had one, and also for `Cancelled` and `Auth`
    /// — the `From` impl below folds 57014 and 28xxx into dedicated variants and
    /// drops the code, since `code()` already tells the UI everything it acts on.
    ///
    /// # Arguments
    /// * `&self` — `&AppError`: the error to read the SQLSTATE from.
    ///
    /// # Returns
    /// `Option<String>` — the cloned five-character code, or `None` on every
    /// variant that does not carry one.
    pub fn sqlstate(&self) -> Option<String> {
        match self {
            Self::Query { sqlstate, .. } => sqlstate.clone(),
            _ => None,
        }
    }
}

/// Classify a driver error. Connection-time failures get specific codes so the
/// connect modal can point at the right field; everything else is a query error
/// carrying the server's own message.
impl From<tokio_postgres::Error> for AppError {
    /// A driver error either carries a server response or it does not, and that
    /// split is the whole shape of this function.
    ///
    /// With one, the SQLSTATE decides: cancellation and authentication get their
    /// own variants (losing the code, and with it the DETAIL/HINT block, since
    /// neither is rendered for those), and everything else becomes a `Query`
    /// preserving the server's message, extras and SQLSTATE verbatim — that is
    /// what makes the terminal read like psql.
    ///
    /// Without one, nothing failed server-side, so the error is from connecting.
    /// Sniffing the message for TLS is crude, but the driver flattens handshake
    /// failures into an opaque string and the connect modal needs to know whether
    /// to point at the SSL mode field or the host.
    ///
    /// # Arguments
    /// * `e` — `tokio_postgres::Error`: the driver error, consumed so its server
    ///   response can be read and its message moved into the new variant.
    ///
    /// # Returns
    /// `Self` — `Cancelled`, `Auth` or `Query` when the server responded;
    /// `Tls` or `Connect` when it did not. Never fails, so classification cannot
    /// itself become an error the UI has to render.
    fn from(e: tokio_postgres::Error) -> Self {
        if let Some(db) = e.as_db_error() {
            let sqlstate = db.code().code().to_string();

            // 57014 = query_canceled, from an explicit cancel or statement_timeout.
            if sqlstate == "57014" {
                return Self::Cancelled;
            }

            let detail = {
                let mut parts = Vec::new();
                if let Some(d) = db.detail() {
                    parts.push(format!("DETAIL:  {d}"));
                }
                if let Some(h) = db.hint() {
                    parts.push(format!("HINT:  {h}"));
                }
                if parts.is_empty() {
                    None
                } else {
                    Some(parts.join("\n"))
                }
            };

            // 28xxx = invalid authorization specification / invalid password.
            if sqlstate.starts_with("28") {
                return Self::Auth(db.message().to_string());
            }

            return Self::Query {
                message: db.message().to_string(),
                detail,
                sqlstate: Some(sqlstate),
            };
        }

        let msg = with_causes(&e);
        if msg.contains("tls") || msg.contains("TLS") || msg.contains("certificate") {
            Self::Tls(msg)
        } else {
            Self::Connect(msg)
        }
    }
}

/// An error's message with its whole `source()` chain appended.
///
/// The driver's `Display` is often a bare category — a config failure renders as
/// just "invalid configuration", and the reason it is invalid ("password
/// missing", "both host and hostaddr are missing") lives only in the source.
/// Taking `to_string()` alone hands the user an error with the diagnosis
/// removed.
///
/// # Arguments
/// * `e` — `&dyn Error`: the error to flatten; its chain is walked, not consumed.
///
/// # Returns
/// `String` — the messages joined with `": "`, skipping any link whose text is
/// already present so a driver that repeats itself does not stutter.
fn with_causes(e: &dyn std::error::Error) -> String {
    let mut msg = e.to_string();
    let mut source = e.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !msg.contains(&text) {
            msg.push_str(": ");
            msg.push_str(&text);
        }
        source = cause.source();
    }
    msg
}

impl From<deadpool_postgres::PoolError> for AppError {
    /// Unwrap the pool: a backend error is a real database error and is
    /// classified as one above, keeping its SQLSTATE and detail rather than being
    /// flattened into a string. Pool-specific failures — timeout waiting for a
    /// slot, closed pool — are not the server's fault and surface as `Connect`.
    ///
    /// # Arguments
    /// * `e` — `deadpool_postgres::PoolError`: the pool error, consumed so a
    ///   `Backend` variant's inner driver error can be moved out and reclassified.
    ///
    /// # Returns
    /// `Self` — whatever the driver conversion above produces for a backend
    /// error, otherwise `Connect` carrying the pool's own message.
    fn from(e: deadpool_postgres::PoolError) -> Self {
        match e {
            deadpool_postgres::PoolError::Backend(e) => e.into(),
            other => Self::Connect(other.to_string()),
        }
    }
}

impl From<std::io::Error> for AppError {
    /// Every I/O error in the app comes from `crate::store` touching a file, so
    /// they all land in `Storage`. The `ErrorKind` is discarded — the message
    /// already names the file and what went wrong, and the UI has no per-kind
    /// behaviour to drive.
    ///
    /// # Arguments
    /// * `e` — `std::io::Error`: the filesystem error; only its `Display` output
    ///   is kept.
    ///
    /// # Returns
    /// `Self` — always `Storage`, carrying that message.
    fn from(e: std::io::Error) -> Self {
        Self::Storage(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    /// Also `Storage`: the only JSON this crate parses is its own on-disk state,
    /// so a serde failure means a corrupt file, not a bad request. Serde's
    /// message keeps the line and column, which is what makes a hand-edited
    /// `profiles.json` fixable.
    ///
    /// # Arguments
    /// * `e` — `serde_json::Error`: the (de)serialisation failure, whose message
    ///   already names the line and column.
    ///
    /// # Returns
    /// `Self` — always `Storage`, carrying that message.
    fn from(e: serde_json::Error) -> Self {
        Self::Storage(e.to_string())
    }
}

/// Wire format. The UI reads `{ code, message, detail, sqlstate }`.
impl Serialize for AppError {
    /// Hand-written rather than derived so the four fields the UI reads are
    /// always present regardless of which variant this is — `detail` and
    /// `sqlstate` come out as null on the variants that have neither, instead of
    /// the frontend having to cope with a differently-shaped object per variant.
    ///
    /// `message` is the `Display` output, which is why the `#[error(...)]`
    /// attributes above are user-facing copy and not developer notes.
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("AppError", 4)?;
        st.serialize_field("code", self.code())?;
        st.serialize_field("message", &self.to_string())?;
        st.serialize_field("detail", &self.detail())?;
        st.serialize_field("sqlstate", &self.sqlstate())?;
        st.end()
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod cause_tests {
    use super::*;

    #[derive(Debug)]
    struct Inner(&'static str);
    impl std::fmt::Display for Inner {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }
    impl std::error::Error for Inner {}

    #[derive(Debug)]
    struct Outer(&'static str, Option<Inner>);
    impl std::fmt::Display for Outer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }
    impl std::error::Error for Outer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.1
                .as_ref()
                .map(|i| i as &(dyn std::error::Error + 'static))
        }
    }

    #[test]
    fn a_bare_category_gains_its_reason() {
        // The real case: the driver renders a config failure as just "invalid
        // configuration" and hides *why* in the source, which is the difference
        // between a useless error and an actionable one.
        let e = Outer("invalid configuration", Some(Inner("password missing")));
        assert_eq!(with_causes(&e), "invalid configuration: password missing");
    }

    #[test]
    fn an_error_with_no_source_is_unchanged() {
        assert_eq!(
            with_causes(&Outer("connection refused", None)),
            "connection refused"
        );
    }

    #[test]
    fn a_cause_already_in_the_message_is_not_repeated() {
        // Some drivers already interpolate the cause; appending it again would
        // read as a stutter.
        let e = Outer("failed: password missing", Some(Inner("password missing")));
        assert_eq!(with_causes(&e), "failed: password missing");
    }
}
