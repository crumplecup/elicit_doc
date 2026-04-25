//! Error types for elicit_doc.

/// Crate-level result alias.
pub type ElicitDocResult<T> = Result<T, ElicitDocError>;

/// Error kind for elicit_doc operations.
#[derive(Debug, Clone, PartialEq, Eq, derive_more::Display)]
pub enum ElicitDocErrorKind {
    /// Failed to invoke or locate `cargo`.
    #[display("cargo invocation failed: {}", _0)]
    CargoInvocation(String),

    /// `cargo rustdoc` produced no JSON output file.
    #[display("rustdoc JSON not found at {}", _0)]
    RustdocOutputMissing(String),

    /// Failed to parse the rustdoc JSON.
    #[display("rustdoc JSON parse error: {}", _0)]
    RustdocParse(String),

    /// Failed to locate the workspace root via `cargo metadata`.
    #[display("cargo metadata error: {}", _0)]
    CargoMetadata(String),

    /// IO error (reading/writing files).
    #[display("IO error: {}", _0)]
    Io(String),

    /// CSV serialization error.
    #[display("CSV error: {}", _0)]
    Csv(String),
}

/// Wrapper error carrying kind + call site location.
#[derive(Debug, Clone, derive_more::Display, derive_more::Error)]
#[display("elicit_doc: {} at {}:{}", kind, file, line)]
pub struct ElicitDocError {
    pub kind: ElicitDocErrorKind,
    pub line: u32,
    pub file: &'static str,
}

impl ElicitDocError {
    #[track_caller]
    pub fn new(kind: ElicitDocErrorKind) -> Self {
        let loc = std::panic::Location::caller();
        Self {
            kind,
            line: loc.line(),
            file: loc.file(),
        }
    }

    #[track_caller]
    pub fn cargo_invocation(msg: impl Into<String>) -> Self {
        Self::new(ElicitDocErrorKind::CargoInvocation(msg.into()))
    }

    #[track_caller]
    pub fn rustdoc_missing(path: impl Into<String>) -> Self {
        Self::new(ElicitDocErrorKind::RustdocOutputMissing(path.into()))
    }

    #[track_caller]
    pub fn rustdoc_parse(msg: impl Into<String>) -> Self {
        Self::new(ElicitDocErrorKind::RustdocParse(msg.into()))
    }

    #[track_caller]
    pub fn cargo_metadata(msg: impl Into<String>) -> Self {
        Self::new(ElicitDocErrorKind::CargoMetadata(msg.into()))
    }

    #[track_caller]
    pub fn io(msg: impl Into<String>) -> Self {
        Self::new(ElicitDocErrorKind::Io(msg.into()))
    }

    #[track_caller]
    pub fn csv(msg: impl Into<String>) -> Self {
        Self::new(ElicitDocErrorKind::Csv(msg.into()))
    }
}
