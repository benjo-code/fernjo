//! Typed import errors. Messages never contain secrets.

/// Why an import failed. `code()` is the stable identifier the UI switches on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("the file is encrypted with a custom password")]
    KeyNeeded,
    #[error("the password does not decrypt the file")]
    WrongKey,
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("the file is too large")]
    TooLarge,
    #[error("the file is nested or branched too deeply")]
    TooComplex,
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
            Self::TooComplex => "too_complex",
            Self::Damaged(_) => "damaged",
        }
    }
}
