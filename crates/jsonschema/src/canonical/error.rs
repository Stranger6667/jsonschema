use crate::ValidationError;

/// Why a schema document could not be canonicalized.
#[derive(Debug)]
#[non_exhaustive]
pub enum CanonicalizationError {
    /// Schema root is neither a boolean nor an object.
    InvalidSchemaType(String),
    /// Meta-schema validation failed.
    ValidationError(ValidationError<'static>),
    /// A `$ref` cycle that never crosses a typed operator (no base case).
    UnguardedRecursion(String),
    /// A recursive schema with no finite instance (recursion in a required position).
    InfiniteRecursion(String),
    /// A `pattern` / `patternProperties` regex failed to compile.
    InvalidPattern { pointer: String, message: String },
    /// A schema literal value cannot be represented as canonical JSON.
    InvalidJsonValue(String),
    /// A `$ref` chain nested deeper than the parser's bound; the schema is preserved verbatim instead.
    RefDepthLimitExceeded,
}

impl std::fmt::Display for CanonicalizationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSchemaType(value) => {
                write!(f, "schema must be a boolean or object, got: {value}")
            }
            Self::ValidationError(error) => write!(f, "schema validation failed: {error}"),
            Self::UnguardedRecursion(name) => write!(f, "unguarded recursion at: {name}"),
            Self::InfiniteRecursion(name) => write!(f, "infinite recursion at: {name}"),
            Self::InvalidPattern { pointer, message } => {
                write!(f, "invalid pattern at {pointer}: {message}")
            }
            Self::InvalidJsonValue(message) => write!(f, "invalid JSON value: {message}"),
            Self::RefDepthLimitExceeded => write!(f, "$ref chain exceeds the depth limit"),
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

#[cfg(test)]
mod tests {
    use serde_json::json;
    use test_case::test_case;

    use super::CanonicalizationError;

    #[test_case(
        &CanonicalizationError::InvalidSchemaType("42".to_string()),
        "schema must be a boolean or object, got: 42";
        "invalid_schema_type"
    )]
    #[test_case(
        &CanonicalizationError::UnguardedRecursion("#/$defs/a".to_string()),
        "unguarded recursion at: #/$defs/a";
        "unguarded_recursion"
    )]
    #[test_case(
        &CanonicalizationError::InfiniteRecursion("#/$defs/a".to_string()),
        "infinite recursion at: #/$defs/a";
        "infinite_recursion"
    )]
    #[test_case(
        &CanonicalizationError::InvalidPattern {
            pointer: "#/pattern".to_string(),
            message: "unbalanced parenthesis".to_string(),
        },
        "invalid pattern at #/pattern: unbalanced parenthesis";
        "invalid_pattern"
    )]
    #[test_case(
        &CanonicalizationError::InvalidJsonValue("NaN is not representable".to_string()),
        "invalid JSON value: NaN is not representable";
        "invalid_json_value"
    )]
    fn display_and_no_source(error: &CanonicalizationError, expected: &str) {
        assert_eq!(error.to_string(), expected);
        assert!(std::error::Error::source(error).is_none());
    }

    #[test]
    fn validation_error_display_and_source() {
        let error =
            crate::canonicalize(&json!({"type": 123})).expect_err("invalid schema must error");
        assert!(error.to_string().starts_with("schema validation failed:"));
        assert!(std::error::Error::source(&error).is_some());
    }
}
