use std::fmt;

/// A validation failure at an untrusted protocol boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationError {
    field: String,
    kind: ValidationErrorKind,
}

impl ValidationError {
    pub fn new(field: impl Into<String>, kind: ValidationErrorKind) -> Self {
        Self {
            field: field.into(),
            kind,
        }
    }

    pub fn field(&self) -> &str {
        &self.field
    }

    pub fn kind(&self) -> &ValidationErrorKind {
        &self.kind
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid {}: {}", self.field, self.kind)
    }
}

impl std::error::Error for ValidationError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidationErrorKind {
    Empty,
    TooLong { max: usize, actual: usize },
    TooMany { max: usize, actual: usize },
    InvalidFormat,
    OutOfRange,
    Unsupported,
    Expired,
    Replay,
    NonMonotonicSequence,
    SenderMismatch,
    StaleIncarnation,
    InvalidLifecycleTransition,
    SignatureMissing,
    SignatureInvalid,
}

impl fmt::Display for ValidationErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("must not be empty"),
            Self::TooLong { max, actual } => {
                write!(formatter, "is {actual} bytes; maximum is {max}")
            }
            Self::TooMany { max, actual } => {
                write!(formatter, "contains {actual} entries; maximum is {max}")
            }
            Self::InvalidFormat => formatter.write_str("has an invalid format"),
            Self::OutOfRange => formatter.write_str("is outside the permitted range"),
            Self::Unsupported => formatter.write_str("is not supported"),
            Self::Expired => formatter.write_str("has expired"),
            Self::Replay => formatter.write_str("has already been processed"),
            Self::NonMonotonicSequence => {
                formatter.write_str("is not newer than the sender sequence")
            }
            Self::SenderMismatch => formatter.write_str("does not match the authenticated sender"),
            Self::StaleIncarnation => {
                formatter.write_str("belongs to a stale Tentacle incarnation")
            }
            Self::InvalidLifecycleTransition => {
                formatter.write_str("is not a permitted lifecycle transition")
            }
            Self::SignatureMissing => formatter.write_str("requires a signature"),
            Self::SignatureInvalid => formatter.write_str("signature verification failed"),
        }
    }
}

/// Serialization errors are intentionally kept distinct from domain validation failures.
#[derive(Debug)]
pub enum EnvelopeDecodeError {
    Oversized { max: usize, actual: usize },
    Json(serde_json::Error),
    Validation(ValidationError),
}

impl fmt::Display for EnvelopeDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Oversized { max, actual } => {
                write!(
                    formatter,
                    "Council envelope is {actual} bytes; maximum is {max}"
                )
            }
            Self::Json(error) => write!(formatter, "invalid Council JSON: {error}"),
            Self::Validation(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for EnvelopeDecodeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::Validation(error) => Some(error),
            Self::Oversized { .. } => None,
        }
    }
}

impl From<serde_json::Error> for EnvelopeDecodeError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<ValidationError> for EnvelopeDecodeError {
    fn from(value: ValidationError) -> Self {
        Self::Validation(value)
    }
}
