//! Error types
use crate::{
    paths::Location,
    primitive_type::{PrimitiveType, PrimitiveTypesBitMap},
};
use serde_json::{Map, Number, Value};
use std::{
    borrow::Cow,
    error,
    fmt::{self, Formatter, Write},
    iter::{empty, once},
    string::FromUtf8Error,
};

/// An error that can occur during validation.
#[derive(Debug)]
pub struct ValidationError<'a> {
    /// Value of the property that failed validation.
    pub instance: Cow<'a, Value>,
    /// Type of validation error.
    pub kind: ValidationErrorKind,
    /// Path to the value that failed validation.
    pub instance_path: Location,
    /// Path to the JSON Schema keyword that failed validation.
    pub schema_path: Location,
}

/// An iterator over instances of [`ValidationError`] that represent validation error for the
/// input instance.
///
/// # Examples
///
/// ```rust
/// use serde_json::json;
///
/// let schema = json!({"maxLength": 5});
/// let instance = json!("foo");
/// if let Ok(validator) = jsonschema::validator_for(&schema) {
///     let errors = validator.iter_errors(&instance);
///     for error in errors {
///         println!("Validation error: {}", error)
///     }
/// }
/// ```
pub type ErrorIterator<'a> = Box<dyn Iterator<Item = ValidationError<'a>> + Sync + Send + 'a>;

// Empty iterator means no error happened
pub(crate) fn no_error<'a>() -> ErrorIterator<'a> {
    Box::new(empty())
}
// A wrapper for one error
pub(crate) fn error(instance: ValidationError) -> ErrorIterator {
    Box::new(once(instance))
}

/// Kinds of errors that may happen during validation
#[derive(Debug)]
#[allow(missing_docs)]
pub enum ValidationErrorKind {
    /// The input array contain more items than expected.
    AdditionalItems { limit: usize },
    /// Unexpected properties.
    AdditionalProperties { unexpected: Vec<String> },
    /// The input value is not valid under any of the schemas listed in the 'anyOf' keyword.
    AnyOf,
    /// Results from a [`fancy_regex::RuntimeError::BacktrackLimitExceeded`] variant when matching
    BacktrackLimitExceeded { error: fancy_regex::Error },
    /// The input value doesn't match expected constant.
    Constant { expected_value: Value },
    /// The input array doesn't contain items conforming to the specified schema.
    Contains,
    /// The input value does not respect the defined contentEncoding
    ContentEncoding { content_encoding: String },
    /// The input value does not respect the defined contentMediaType
    ContentMediaType { content_media_type: String },
    /// Custom error message for user-defined validation.
    Custom { message: String },
    /// The input value doesn't match any of specified options.
    Enum { options: Value },
    /// Value is too large.
    ExclusiveMaximum { limit: Value },
    /// Value is too small.
    ExclusiveMinimum { limit: Value },
    /// Everything is invalid for `false` schema.
    FalseSchema,
    /// When the input doesn't match to the specified format.
    Format { format: String },
    /// May happen in `contentEncoding` validation if `base64` encoded data is invalid.
    FromUtf8 { error: FromUtf8Error },
    /// Too many items in an array.
    MaxItems { limit: u64 },
    /// Value is too large.
    Maximum { limit: Value },
    /// String is too long.
    MaxLength { limit: u64 },
    /// Too many properties in an object.
    MaxProperties { limit: u64 },
    /// Too few items in an array.
    MinItems { limit: u64 },
    /// Value is too small.
    Minimum { limit: Value },
    /// String is too short.
    MinLength { limit: u64 },
    /// Not enough properties in an object.
    MinProperties { limit: u64 },
    /// When some number is not a multiple of another number.
    MultipleOf { multiple_of: f64 },
    /// Negated schema failed validation.
    Not { schema: Value },
    /// The given schema is valid under more than one of the schemas listed in the 'oneOf' keyword.
    OneOfMultipleValid,
    /// The given schema is not valid under any of the schemas listed in the 'oneOf' keyword.
    OneOfNotValid,
    /// When the input doesn't match to a pattern.
    Pattern { pattern: String },
    /// Object property names are invalid.
    PropertyNames {
        error: Box<ValidationError<'static>>,
    },
    /// When a required property is missing.
    Required { property: Value },
    /// When the input value doesn't match one or multiple required types.
    Type { kind: TypeKind },
    /// Unexpected items.
    UnevaluatedItems { unexpected: Vec<String> },
    /// Unexpected properties.
    UnevaluatedProperties { unexpected: Vec<String> },
    /// When the input array has non-unique elements.
    UniqueItems,
    /// Error during schema ref resolution.
    Referencing(referencing::Error),
}

#[derive(Debug)]
#[allow(missing_docs)]
pub enum TypeKind {
    Single(PrimitiveType),
    Multiple(PrimitiveTypesBitMap),
}

/// Shortcuts for creation of specific error kinds.
impl<'a> ValidationError<'a> {
    pub(crate) fn into_owned(self) -> ValidationError<'static> {
        ValidationError {
            instance_path: self.instance_path.clone(),
            instance: Cow::Owned(self.instance.into_owned()),
            kind: self.kind,
            schema_path: self.schema_path,
        }
    }

    pub(crate) const fn additional_items(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        limit: usize,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::AdditionalItems { limit },
            schema_path: location,
        }
    }
    pub(crate) const fn additional_properties(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        unexpected: Vec<String>,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::AdditionalProperties { unexpected },
            schema_path: location,
        }
    }
    pub(crate) const fn any_of(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::AnyOf,
            schema_path: location,
        }
    }
    pub(crate) const fn backtrack_limit(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        error: fancy_regex::Error,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::BacktrackLimitExceeded { error },
            schema_path: location,
        }
    }
    pub(crate) fn constant_array(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        expected_value: &[Value],
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::Constant {
                expected_value: Value::Array(expected_value.to_vec()),
            },
            schema_path: location,
        }
    }
    pub(crate) const fn constant_boolean(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        expected_value: bool,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::Constant {
                expected_value: Value::Bool(expected_value),
            },
            schema_path: location,
        }
    }
    pub(crate) const fn constant_null(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::Constant {
                expected_value: Value::Null,
            },
            schema_path: location,
        }
    }
    pub(crate) fn constant_number(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        expected_value: &Number,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::Constant {
                expected_value: Value::Number(expected_value.clone()),
            },
            schema_path: location,
        }
    }
    pub(crate) fn constant_object(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        expected_value: &Map<String, Value>,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::Constant {
                expected_value: Value::Object(expected_value.clone()),
            },
            schema_path: location,
        }
    }
    pub(crate) fn constant_string(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        expected_value: &str,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::Constant {
                expected_value: Value::String(expected_value.to_string()),
            },
            schema_path: location,
        }
    }
    pub(crate) const fn contains(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::Contains,
            schema_path: location,
        }
    }
    pub(crate) fn content_encoding(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        encoding: &str,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::ContentEncoding {
                content_encoding: encoding.to_string(),
            },
            schema_path: location,
        }
    }
    pub(crate) fn content_media_type(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        media_type: &str,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::ContentMediaType {
                content_media_type: media_type.to_string(),
            },
            schema_path: location,
        }
    }
    pub(crate) fn enumeration(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        options: &Value,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::Enum {
                options: options.clone(),
            },
            schema_path: location,
        }
    }
    pub(crate) const fn exclusive_maximum(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        limit: Value,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::ExclusiveMaximum { limit },
            schema_path: location,
        }
    }
    pub(crate) const fn exclusive_minimum(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        limit: Value,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::ExclusiveMinimum { limit },
            schema_path: location,
        }
    }
    pub(crate) const fn false_schema(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::FalseSchema,
            schema_path: location,
        }
    }
    pub(crate) fn format(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        format: impl Into<String>,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::Format {
                format: format.into(),
            },
            schema_path: location,
        }
    }
    pub(crate) fn from_utf8(error: FromUtf8Error) -> ValidationError<'a> {
        ValidationError {
            instance_path: Location::new(),
            instance: Cow::Owned(Value::Null),
            kind: ValidationErrorKind::FromUtf8 { error },
            schema_path: Location::new(),
        }
    }
    pub(crate) const fn max_items(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        limit: u64,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::MaxItems { limit },
            schema_path: location,
        }
    }
    pub(crate) const fn maximum(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        limit: Value,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::Maximum { limit },
            schema_path: location,
        }
    }
    pub(crate) const fn max_length(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        limit: u64,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::MaxLength { limit },
            schema_path: location,
        }
    }
    pub(crate) const fn max_properties(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        limit: u64,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::MaxProperties { limit },
            schema_path: location,
        }
    }
    pub(crate) const fn min_items(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        limit: u64,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::MinItems { limit },
            schema_path: location,
        }
    }
    pub(crate) const fn minimum(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        limit: Value,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::Minimum { limit },
            schema_path: location,
        }
    }
    pub(crate) const fn min_length(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        limit: u64,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::MinLength { limit },
            schema_path: location,
        }
    }
    pub(crate) const fn min_properties(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        limit: u64,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::MinProperties { limit },
            schema_path: location,
        }
    }
    pub(crate) const fn multiple_of(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        multiple_of: f64,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::MultipleOf { multiple_of },
            schema_path: location,
        }
    }
    pub(crate) const fn not(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        schema: Value,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::Not { schema },
            schema_path: location,
        }
    }
    pub(crate) const fn one_of_multiple_valid(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::OneOfMultipleValid,
            schema_path: location,
        }
    }
    pub(crate) const fn one_of_not_valid(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::OneOfNotValid,
            schema_path: location,
        }
    }
    pub(crate) const fn pattern(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        pattern: String,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::Pattern { pattern },
            schema_path: location,
        }
    }
    pub(crate) fn property_names(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        error: ValidationError<'a>,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::PropertyNames {
                error: Box::new(error.into_owned()),
            },
            schema_path: location,
        }
    }
    pub(crate) const fn required(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        property: Value,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::Required { property },
            schema_path: location,
        }
    }

    pub(crate) const fn single_type_error(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        type_name: PrimitiveType,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::Type {
                kind: TypeKind::Single(type_name),
            },
            schema_path: location,
        }
    }
    pub(crate) const fn multiple_type_error(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        types: PrimitiveTypesBitMap,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::Type {
                kind: TypeKind::Multiple(types),
            },
            schema_path: location,
        }
    }
    pub(crate) const fn unevaluated_items(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        unexpected: Vec<String>,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::UnevaluatedItems { unexpected },
            schema_path: location,
        }
    }
    pub(crate) const fn unevaluated_properties(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        unexpected: Vec<String>,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::UnevaluatedProperties { unexpected },
            schema_path: location,
        }
    }
    pub(crate) const fn unique_items(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::UniqueItems,
            schema_path: location,
        }
    }
    /// Create a new custom validation error.
    pub fn custom(
        location: Location,
        instance_path: Location,
        instance: &'a Value,
        message: impl Into<String>,
    ) -> ValidationError<'a> {
        ValidationError {
            instance_path,
            instance: Cow::Borrowed(instance),
            kind: ValidationErrorKind::Custom {
                message: message.into(),
            },
            schema_path: location,
        }
    }
}

impl error::Error for ValidationError<'_> {}
impl From<referencing::Error> for ValidationError<'_> {
    #[inline]
    fn from(err: referencing::Error) -> Self {
        ValidationError {
            instance_path: Location::new(),
            instance: Cow::Owned(Value::Null),
            kind: ValidationErrorKind::Referencing(err),
            schema_path: Location::new(),
        }
    }
}
impl From<FromUtf8Error> for ValidationError<'_> {
    #[inline]
    fn from(err: FromUtf8Error) -> Self {
        ValidationError::from_utf8(err)
    }
}

/// Textual representation of various validation errors.
impl fmt::Display for ValidationError<'_> {
    #[allow(clippy::too_many_lines)] // The function is long but it does formatting only
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ValidationErrorKind::Referencing(error) => error.fmt(f),
            ValidationErrorKind::BacktrackLimitExceeded { error } => error.fmt(f),
            ValidationErrorKind::Format { format } => {
                write!(f, r#"{} is not a "{}""#, self.instance, format)
            }
            ValidationErrorKind::AdditionalItems { limit } => {
                f.write_str("Additional items are not allowed (")?;
                let array = self.instance.as_array().expect("Always valid");
                let mut iter = array.iter().skip(*limit);

                if let Some(item) = iter.next() {
                    write!(f, "{}", item)?;
                }
                for item in iter {
                    f.write_str(", ")?;
                    write!(f, "{}", item)?;
                }

                let items_count = array.len() - limit;
                f.write_str(if items_count == 1 {
                    " was unexpected)"
                } else {
                    " were unexpected)"
                })
            }
            ValidationErrorKind::AdditionalProperties { unexpected } => {
                f.write_str("Additional properties are not allowed (")?;
                let mut iter = unexpected.iter();
                if let Some(prop) = iter.next() {
                    f.write_char('\'')?;
                    write!(f, "{}", prop)?;
                    f.write_char('\'')?;
                }
                for prop in iter {
                    f.write_str(", ")?;
                    f.write_char('\'')?;
                    write!(f, "{}", prop)?;
                    f.write_char('\'')?;
                }
                f.write_str(if unexpected.len() == 1 {
                    " was unexpected)"
                } else {
                    " were unexpected)"
                })
            }
            ValidationErrorKind::AnyOf => write!(
                f,
                "{} is not valid under any of the schemas listed in the 'anyOf' keyword",
                self.instance
            ),
            ValidationErrorKind::OneOfNotValid => write!(
                f,
                "{} is not valid under any of the schemas listed in the 'oneOf' keyword",
                self.instance
            ),
            ValidationErrorKind::Contains => write!(
                f,
                "None of {} are valid under the given schema",
                self.instance
            ),
            ValidationErrorKind::Constant { expected_value } => {
                write!(f, "{} was expected", expected_value)
            }
            ValidationErrorKind::ContentEncoding { content_encoding } => {
                write!(
                    f,
                    r#"{} is not compliant with "{}" content encoding"#,
                    self.instance, content_encoding
                )
            }
            ValidationErrorKind::ContentMediaType { content_media_type } => {
                write!(
                    f,
                    r#"{} is not compliant with "{}" media type"#,
                    self.instance, content_media_type
                )
            }
            ValidationErrorKind::FromUtf8 { error } => error.fmt(f),
            ValidationErrorKind::Enum { options } => {
                write!(f, "{} is not one of {}", self.instance, options)
            }
            ValidationErrorKind::ExclusiveMaximum { limit } => write!(
                f,
                "{} is greater than or equal to the maximum of {}",
                self.instance, limit
            ),
            ValidationErrorKind::ExclusiveMinimum { limit } => write!(
                f,
                "{} is less than or equal to the minimum of {}",
                self.instance, limit
            ),
            ValidationErrorKind::FalseSchema => {
                write!(f, "False schema does not allow {}", self.instance)
            }
            ValidationErrorKind::Maximum { limit } => write!(
                f,
                "{} is greater than the maximum of {}",
                self.instance, limit
            ),
            ValidationErrorKind::Minimum { limit } => {
                write!(f, "{} is less than the minimum of {}", self.instance, limit)
            }
            ValidationErrorKind::MaxLength { limit } => write!(
                f,
                "{} is longer than {} character{}",
                self.instance,
                limit,
                if *limit == 1 { "" } else { "s" }
            ),
            ValidationErrorKind::MinLength { limit } => write!(
                f,
                "{} is shorter than {} character{}",
                self.instance,
                limit,
                if *limit == 1 { "" } else { "s" }
            ),
            ValidationErrorKind::MaxItems { limit } => write!(
                f,
                "{} has more than {} item{}",
                self.instance,
                limit,
                if *limit == 1 { "" } else { "s" }
            ),
            ValidationErrorKind::MinItems { limit } => write!(
                f,
                "{} has less than {} item{}",
                self.instance,
                limit,
                if *limit == 1 { "" } else { "s" }
            ),
            ValidationErrorKind::MaxProperties { limit } => write!(
                f,
                "{} has more than {} propert{}",
                self.instance,
                limit,
                if *limit == 1 { "y" } else { "ies" }
            ),
            ValidationErrorKind::MinProperties { limit } => write!(
                f,
                "{} has less than {} propert{}",
                self.instance,
                limit,
                if *limit == 1 { "y" } else { "ies" }
            ),
            ValidationErrorKind::Not { schema } => {
                write!(f, "{} is not allowed for {}", schema, self.instance)
            }
            ValidationErrorKind::OneOfMultipleValid => write!(
                f,
                "{} is valid under more than one of the schemas listed in the 'oneOf' keyword",
                self.instance
            ),
            ValidationErrorKind::Pattern { pattern } => {
                write!(f, r#"{} does not match "{}""#, self.instance, pattern)
            }
            ValidationErrorKind::PropertyNames { error } => error.fmt(f),
            ValidationErrorKind::Required { property } => {
                write!(f, "{} is a required property", property)
            }
            ValidationErrorKind::MultipleOf { multiple_of } => {
                write!(f, "{} is not a multiple of {}", self.instance, multiple_of)
            }
            ValidationErrorKind::UnevaluatedItems { unexpected } => {
                f.write_str("Unevaluated items are not allowed (")?;
                let mut iter = unexpected.iter();
                if let Some(item) = iter.next() {
                    f.write_char('\'')?;
                    write!(f, "{}", item)?;
                    f.write_char('\'')?;
                }
                for item in iter {
                    f.write_str(", ")?;
                    f.write_char('\'')?;
                    write!(f, "{}", item)?;
                    f.write_char('\'')?;
                }
                f.write_str(if unexpected.len() == 1 {
                    " was unexpected)"
                } else {
                    " were unexpected)"
                })
            }
            ValidationErrorKind::UnevaluatedProperties { unexpected } => {
                f.write_str("Unevaluated properties are not allowed (")?;
                let mut iter = unexpected.iter();
                if let Some(prop) = iter.next() {
                    f.write_char('\'')?;
                    write!(f, "{}", prop)?;
                    f.write_char('\'')?;
                }
                for prop in iter {
                    f.write_str(", ")?;
                    f.write_char('\'')?;
                    write!(f, "{}", prop)?;
                    f.write_char('\'')?;
                }
                f.write_str(if unexpected.len() == 1 {
                    " was unexpected)"
                } else {
                    " were unexpected)"
                })
            }
            ValidationErrorKind::UniqueItems => {
                write!(f, "{} has non-unique elements", self.instance)
            }
            ValidationErrorKind::Type {
                kind: TypeKind::Single(type_),
            } => write!(f, r#"{} is not of type "{}""#, self.instance, type_),
            ValidationErrorKind::Type {
                kind: TypeKind::Multiple(types),
            } => {
                write!(f, "{} is not of types ", self.instance)?;
                let mut iter = types.into_iter();
                if let Some(t) = iter.next() {
                    f.write_char('"')?;
                    write!(f, "{}", t)?;
                    f.write_char('"')?;
                }
                for t in iter {
                    f.write_str(", ")?;
                    f.write_char('"')?;
                    write!(f, "{}", t)?;
                    f.write_char('"')?;
                }
                Ok(())
            }
            ValidationErrorKind::Custom { message } => f.write_str(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use test_case::test_case;

    #[test]
    fn single_type_error() {
        let instance = json!(42);
        let err = ValidationError::single_type_error(
            Location::new(),
            Location::new(),
            &instance,
            PrimitiveType::String,
        );
        assert_eq!(err.to_string(), r#"42 is not of type "string""#)
    }

    #[test]
    fn multiple_types_error() {
        let instance = json!(42);
        let err = ValidationError::multiple_type_error(
            Location::new(),
            Location::new(),
            &instance,
            vec![PrimitiveType::String, PrimitiveType::Number].into(),
        );
        assert_eq!(err.to_string(), r#"42 is not of types "number", "string""#)
    }

    #[test_case(true, &json!({"foo": {"bar": 42}}), "/foo/bar")]
    #[test_case(true, &json!({"foo": "a"}), "/foo")]
    #[test_case(false, &json!({"foo": {"bar": 42}}), "/foo/bar")]
    #[test_case(false, &json!({"foo": "a"}), "/foo")]
    fn instance_path_properties(additional_properties: bool, instance: &Value, expected: &str) {
        let schema = json!(
            {
                "additionalProperties": additional_properties,
                "type":"object",
                "properties":{
                   "foo":{
                      "type":"object",
                      "properties":{
                         "bar":{
                            "type":"string"
                         }
                      }
                   }
                }
            }
        );
        let validator = crate::validator_for(&schema).unwrap();
        let mut result = validator.iter_errors(instance);
        let error = result.next().expect("validation error");

        assert!(result.next().is_none());
        assert_eq!(error.instance_path.as_str(), expected);
    }

    #[test_case(true, &json!([1, {"foo": ["42"]}]), "/0")]
    #[test_case(true, &json!(["a", {"foo": [42]}]), "/1/foo/0")]
    #[test_case(false, &json!([1, {"foo": ["42"]}]), "/0")]
    #[test_case(false, &json!(["a", {"foo": [42]}]), "/1/foo/0")]
    fn instance_path_properties_and_arrays(
        additional_items: bool,
        instance: &Value,
        expected: &str,
    ) {
        let schema = json!(
            {
                "items": additional_items,
                "type": "array",
                "prefixItems": [
                    {
                        "type": "string"
                    },
                    {
                        "type": "object",
                        "properties": {
                            "foo": {
                                "type": "array",
                                "prefixItems": [
                                    {
                                        "type": "string"
                                    }
                                ]
                            }
                        }
                    }
                ]
            }
        );
        let validator = crate::validator_for(&schema).unwrap();
        let mut result = validator.iter_errors(instance);
        let error = result.next().expect("validation error");

        assert!(result.next().is_none());
        assert_eq!(error.instance_path.as_str(), expected);
    }

    #[test_case(true, &json!([[1, 2, 3], [4, "5", 6], [7, 8, 9]]), "/1/1")]
    #[test_case(false, &json!([[1, 2, 3], [4, "5", 6], [7, 8, 9]]), "/1/1")]
    #[test_case(true, &json!([[1, 2, 3], [4, 5, 6], 42]), "/2")]
    #[test_case(false, &json!([[1, 2, 3], [4, 5, 6], 42]), "/2")]
    fn instance_path_nested_arrays(additional_items: bool, instance: &Value, expected: &str) {
        let schema = json!(
            {
                "additionalItems": additional_items,
                "type": "array",
                "items": {
                    "type": "array",
                    "items": {
                        "type": "integer"
                    }
                }
            }
        );
        let validator = crate::validator_for(&schema).unwrap();
        let mut result = validator.iter_errors(instance);
        let error = result.next().expect("validation error");

        assert!(result.next().is_none());
        assert_eq!(error.instance_path.as_str(), expected);
    }

    #[test_case(true, &json!([1, "a"]), "/1")]
    #[test_case(false, &json!([1, "a"]), "/1")]
    #[test_case(true, &json!(123), "")]
    #[test_case(false, &json!(123), "")]
    fn instance_path_arrays(additional_items: bool, instance: &Value, expected: &str) {
        let schema = json!(
            {
                "additionalItems": additional_items,
                "type": "array",
                "items": {
                    "type": "integer"
                }
            }
        );
        let validator = crate::validator_for(&schema).unwrap();
        let mut result = validator.iter_errors(instance);
        let error = result.next().expect("validation error");

        assert!(result.next().is_none());
        assert_eq!(error.instance_path.as_str(), expected);
    }
}
