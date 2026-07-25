use crate::ValidationError;

/// Why a schema document could not be canonicalized.
#[derive(Debug)]
#[non_exhaustive]
pub enum CanonicalizationError {
    /// Schema root is neither a boolean nor an object.
    InvalidSchemaType(String),
    /// A schema reference could not be resolved.
    ReferenceResolution(referencing::Error),
    /// Meta-schema validation failed.
    ValidationError(ValidationError<'static>),
    /// A `pattern` value is not a valid regular expression.
    InvalidPattern {
        /// The offending pattern.
        pattern: String,
    },
}

impl std::fmt::Display for CanonicalizationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSchemaType(value) => {
                write!(f, "schema must be a boolean or object, got: {value}")
            }
            Self::ReferenceResolution(error) => error.fmt(f),
            Self::ValidationError(error) => write!(f, "schema validation failed: {error}"),
            Self::InvalidPattern { pattern } => {
                write!(f, "invalid regular expression: {pattern:?}")
            }
        }
    }
}

impl std::error::Error for CanonicalizationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ReferenceResolution(error) => Some(error),
            Self::ValidationError(error) => Some(error),
            Self::InvalidSchemaType(_) | Self::InvalidPattern { .. } => None,
        }
    }
}

impl From<ValidationError<'static>> for CanonicalizationError {
    fn from(error: ValidationError<'static>) -> Self {
        Self::ValidationError(error)
    }
}

impl From<referencing::Error> for CanonicalizationError {
    fn from(error: referencing::Error) -> Self {
        Self::ReferenceResolution(error)
    }
}
