use referencing::Draft;

use crate::ValidationError;

/// Why two operands of a set operation cannot be combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OperandMismatch {
    /// The operands were canonicalized under different drafts.
    Drafts {
        /// Draft of the receiver.
        left: Draft,
        /// Draft of the argument.
        right: Draft,
    },
    /// One operand asserts `format`, the other annotates it.
    FormatAssertions,
    /// The operands were canonicalized with different pattern engines.
    PatternEngine,
    /// The operands resolve references through different definition maps.
    Definitions,
}

impl std::fmt::Display for OperandMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Drafts { left, right } => {
                write!(f, "operands canonicalized under {left:?} and {right:?}")
            }
            Self::FormatAssertions => f.write_str("operands disagree on whether `format` asserts"),
            Self::PatternEngine => {
                f.write_str("operands canonicalized with different pattern engines")
            }
            Self::Definitions => f.write_str("operands carry different definition maps"),
        }
    }
}

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
    /// Operands of a set operation cannot be combined.
    IncompatibleOperands(OperandMismatch),
    /// A set operation reached a schema the canonical form does not model.
    UnmodeledOperand,
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
            Self::IncompatibleOperands(mismatch) => mismatch.fmt(f),
            Self::UnmodeledOperand => f.write_str("operand is not modeled in canonical form"),
        }
    }
}

impl std::error::Error for CanonicalizationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ReferenceResolution(error) => Some(error),
            Self::ValidationError(error) => Some(error),
            Self::InvalidSchemaType(_)
            | Self::InvalidPattern { .. }
            | Self::IncompatibleOperands(_)
            | Self::UnmodeledOperand => None,
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
