use crate::ValidationError;

/// Why a schema document could not be canonicalized.
#[derive(Debug)]
#[non_exhaustive]
pub enum CanonicalizationError {
    /// Schema root is neither a boolean nor an object.
    InvalidSchemaType(String),
    /// Meta-schema validation failed.
    ValidationError(ValidationError<'static>),
}

impl std::fmt::Display for CanonicalizationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSchemaType(value) => {
                write!(f, "schema must be a boolean or object, got: {value}")
            }
            Self::ValidationError(error) => write!(f, "schema validation failed: {error}"),
        }
    }
}

impl std::error::Error for CanonicalizationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ValidationError(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ValidationError<'static>> for CanonicalizationError {
    fn from(error: ValidationError<'static>) -> Self {
        Self::ValidationError(error)
    }
}
