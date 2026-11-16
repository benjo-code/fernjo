//! Typed import errors. Messages never contain secrets.

/// Why an import failed. `code()` is the stable identifier the UI switches on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("the file is encrypted with a custom key")]
    KeyNeeded,
    #[error("the key does not decrypt the file")]
    WrongKey,
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("the file is too large")]
    TooLarge,
    /// Nested, branched or (through the number of stored passwords) too costly; the text says which.
    #[error("the file is too complex: {0}")]
    TooComplex(String),
    #[error("the file is damaged: {0}")]
    Damaged(String),
}

impl ImportError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::KeyNeeded => "key_needed",
            Self::WrongKey => "wrong_key",
            Self::UnsupportedFormat(_) => "unsupported_format",
            Self::TooLarge => "too_large",
            Self::TooComplex(_) => "too_complex",
            Self::Damaged(_) => "damaged",
        }
    }
}
