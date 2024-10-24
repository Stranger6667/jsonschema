use crate::{
    compiler,
    content_encoding::{
        ContentEncodingCheckType, ContentEncodingConverterType,
        DEFAULT_CONTENT_ENCODING_CHECKS_AND_CONVERTERS,
    },
    content_media_type::{ContentMediaTypeCheckType, DEFAULT_CONTENT_MEDIA_TYPE_CHECKS},
    keywords::{custom::KeywordFactory, format::Format},
    paths::Location,
    retriever::DefaultRetriever,
    Keyword, ValidationError, Validator,
};
use ahash::AHashMap;
use referencing::{uri, Draft, Resource, Retrieve};
use serde_json::Value;
use std::{fmt, sync::Arc};

/// Configuration options for JSON Schema validation.
#[derive(Clone)]
pub struct ValidationOptions {
    pub(crate) draft: Option<Draft>,
    content_media_type_checks: AHashMap<&'static str, Option<ContentMediaTypeCheckType>>,
    content_encoding_checks_and_converters:
        AHashMap<&'static str, Option<(ContentEncodingCheckType, ContentEncodingConverterType)>>,
    /// Retriever for external resources
    pub(crate) retriever: Arc<dyn Retrieve>,
    /// Additional resources that should be addressable during validation.
    pub(crate) resources: AHashMap<String, Resource>,
    formats: AHashMap<String, Arc<dyn Format>>,
    validate_formats: Option<bool>,
    pub(crate) validate_schema: bool,
    ignore_unknown_formats: bool,
    keywords: AHashMap<String, Arc<dyn KeywordFactory>>,
}

impl Default for ValidationOptions {
    fn default() -> Self {
        ValidationOptions {
            draft: None,
            content_media_type_checks: AHashMap::default(),
            content_encoding_checks_and_converters: AHashMap::default(),
            retriever: Arc::new(DefaultRetriever),
            resources: AHashMap::default(),
            formats: AHashMap::default(),
            validate_formats: None,
            validate_schema: true,
            ignore_unknown_formats: true,
            keywords: AHashMap::default(),
        }
    }
}

impl ValidationOptions {
    /// Return the draft version, or the default if not set.
    pub(crate) fn draft(&self) -> Draft {
        self.draft.unwrap_or_default()
    }
    pub(crate) fn draft_for(&self, contents: &Value) -> Draft {
        // Preference:
        //  - Explicitly set
        //  - Autodetected
        //  - Default
        if let Some(draft) = self.draft {
            draft
        } else {
            let default = Draft::default();
            match default.detect(contents) {
                Ok(draft) => draft,
                Err(referencing::Error::UnknownSpecification { specification }) => {
                    // Try to retrieve the specification and detect its draft
                    if let Ok(Ok(retrieved)) = uri::from_str(&specification)
                        .map(|uri| self.retriever.retrieve(&uri.borrow()))
                    {
                        default.detect(&retrieved).unwrap_or_default()
                    } else {
                        default
                    }
                }
                _ => default,
            }
        }
    }
    /// Build a JSON Schema validator using the current options.
    ///
    /// # Example
    ///
    /// ```rust
    /// use serde_json::json;
    ///
    /// let schema = json!({"type": "string"});
    /// let validator = jsonschema::options()
    ///     .build(&schema)
    ///     .expect("A valid schema");
    ///
    /// assert!(validator.is_valid(&json!("Hello")));
    /// assert!(!validator.is_valid(&json!(42)));
    /// ```
    pub fn build(&self, schema: &Value) -> Result<Validator, ValidationError<'static>> {
        compiler::build_validator(self.clone(), schema)
    }
    /// Sets the JSON Schema draft version.
    ///
    /// ```rust
    /// use jsonschema::Draft;
    ///
    /// let options = jsonschema::options()
    ///     .with_draft(Draft::Draft4);
    /// ```
    #[inline]
    pub fn with_draft(&mut self, draft: Draft) -> &mut Self {
        self.draft = Some(draft);
        self
    }

    pub(crate) fn get_content_media_type_check(
        &self,
        media_type: &str,
    ) -> Option<ContentMediaTypeCheckType> {
        if let Some(value) = self.content_media_type_checks.get(media_type) {
            *value
        } else {
            DEFAULT_CONTENT_MEDIA_TYPE_CHECKS.get(media_type).copied()
        }
    }
    /// Add support for a custom content media type validation.
    ///
    /// # Example
    ///
    /// ```rust
    /// fn check_custom_media_type(instance_string: &str) -> bool {
    ///     instance_string.starts_with("custom:")
    /// }
    ///
    /// let options = jsonschema::options()
    ///     .with_content_media_type("application/custom", check_custom_media_type);
    /// ```
    pub fn with_content_media_type(
        &mut self,
        media_type: &'static str,
        media_type_check: ContentMediaTypeCheckType,
    ) -> &mut Self {
        self.content_media_type_checks
            .insert(media_type, Some(media_type_check));
        self
    }
    /// Set a retriever to fetch external resources.
    pub fn with_retriever(&mut self, retriever: impl Retrieve + 'static) -> &mut Self {
        self.retriever = Arc::new(retriever);
        self
    }
    /// Remove support for a specific content media type validation.
    pub fn without_content_media_type_support(&mut self, media_type: &'static str) -> &mut Self {
        self.content_media_type_checks.insert(media_type, None);
        self
    }

    #[inline]
    fn content_encoding_check_and_converter(
        &self,
        content_encoding: &str,
    ) -> Option<(ContentEncodingCheckType, ContentEncodingConverterType)> {
        if let Some(value) = self
            .content_encoding_checks_and_converters
            .get(content_encoding)
        {
            *value
        } else {
            DEFAULT_CONTENT_ENCODING_CHECKS_AND_CONVERTERS
                .get(content_encoding)
                .copied()
        }
    }

    pub(crate) fn content_encoding_check(
        &self,
        content_encoding: &str,
    ) -> Option<ContentEncodingCheckType> {
        if let Some((check, _)) = self.content_encoding_check_and_converter(content_encoding) {
            Some(check)
        } else {
            None
        }
    }

    pub(crate) fn get_content_encoding_convert(
        &self,
        content_encoding: &str,
    ) -> Option<ContentEncodingConverterType> {
        if let Some((_, converter)) = self.content_encoding_check_and_converter(content_encoding) {
            Some(converter)
        } else {
            None
        }
    }
    /// Add support for a custom content encoding.
    ///
    /// # Arguments
    ///
    /// * `encoding`: Name of the content encoding (e.g., "base64")
    /// * `check`: Validates the input string (return `true` if valid)
    /// * `converter`: Converts the input string, returning:
    ///   - `Err(ValidationError)`: For supported errors
    ///   - `Ok(None)`: If input is invalid
    ///   - `Ok(Some(content))`: If valid, with decoded content
    ///
    /// # Example
    ///
    /// ```rust
    /// use jsonschema::ValidationError;
    ///
    /// fn check(s: &str) -> bool {
    ///     s.starts_with("valid:")
    /// }
    ///
    /// fn convert(s: &str) -> Result<Option<String>, ValidationError<'static>> {
    ///     if s.starts_with("valid:") {
    ///         Ok(Some(s[6..].to_string()))
    ///     } else {
    ///         Ok(None)
    ///     }
    /// }
    ///
    /// let options = jsonschema::options()
    ///     .with_content_encoding("custom", check, convert);
    /// ```
    pub fn with_content_encoding(
        &mut self,
        encoding: &'static str,
        check: ContentEncodingCheckType,
        converter: ContentEncodingConverterType,
    ) -> &mut Self {
        self.content_encoding_checks_and_converters
            .insert(encoding, Some((check, converter)));
        self
    }
    /// Remove support for a specific content encoding.
    ///
    /// # Example
    ///
    /// ```rust
    /// let options = jsonschema::options()
    ///     .without_content_encoding_support("base64");
    /// ```
    pub fn without_content_encoding_support(
        &mut self,
        content_encoding: &'static str,
    ) -> &mut Self {
        self.content_encoding_checks_and_converters
            .insert(content_encoding, None);
        self
    }
    /// Add a custom schema, allowing it to be referenced by the specified URI during validation.
    ///
    /// This enables the use of additional in-memory schemas alongside the main schema being validated.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use serde_json::json;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use jsonschema::Resource;
    ///
    /// let extra = Resource::from_contents(json!({"minimum": 5}))?;
    ///
    /// let validator = jsonschema::options()
    ///     .with_resource("urn:minimum-schema", extra)
    ///     .build(&json!({"$ref": "urn:minimum-schema"}))?;
    /// assert!(validator.is_valid(&json!(5)));
    /// assert!(!validator.is_valid(&json!(4)));
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_resource(&mut self, uri: impl Into<String>, resource: Resource) -> &mut Self {
        self.resources.insert(uri.into(), resource);
        self
    }
    /// Add custom schemas, allowing them to be referenced by the specified URI during validation.
    ///
    /// This enables the use of additional in-memory schemas alongside the main schema being validated.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use serde_json::json;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use jsonschema::Resource;
    ///
    /// let validator = jsonschema::options()
    ///     .with_resources([
    ///         (
    ///             "urn:minimum-schema",
    ///             Resource::from_contents(json!({"minimum": 5}))?,
    ///         ),
    ///         (
    ///             "urn:maximum-schema",
    ///             Resource::from_contents(json!({"maximum": 10}))?,
    ///         ),
    ///       ].into_iter())
    ///     .build(&json!({"$ref": "urn:minimum-schema"}))?;
    /// assert!(validator.is_valid(&json!(5)));
    /// assert!(!validator.is_valid(&json!(4)));
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_resources(
        &mut self,
        pairs: impl Iterator<Item = (impl Into<String>, Resource)>,
    ) -> &mut Self {
        for (uri, resource) in pairs {
            self.resources.insert(uri.into(), resource);
        }
        self
    }
    /// Register a custom format validator.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use serde_json::json;
    /// fn my_format(s: &str) -> bool {
    ///    // Your awesome format check!
    ///    s.ends_with("42!")
    /// }
    /// # fn foo() {
    /// let schema = json!({"type": "string", "format": "custom"});
    /// let validator = jsonschema::options()
    ///     .with_format("custom", my_format)
    ///     .build(&schema)
    ///     .expect("Valid schema");
    ///
    /// assert!(!validator.is_valid(&json!("foo")));
    /// assert!(validator.is_valid(&json!("foo42!")));
    /// # }
    /// ```
    pub fn with_format<N, F>(&mut self, name: N, format: F) -> &mut Self
    where
        N: Into<String>,
        F: Fn(&str) -> bool + Send + Sync + 'static,
    {
        self.formats.insert(name.into(), Arc::new(format));
        self
    }
    pub(crate) fn get_format(&self, format: &str) -> Option<(&String, &Arc<dyn Format>)> {
        self.formats.get_key_value(format)
    }
    /// Disable schema validation during compilation.
    ///
    /// Used internally to prevent infinite recursion when validating meta-schemas.
    /// **Note**: Manually-crafted `ValidationError`s may still occur during compilation.
    #[inline]
    pub(crate) fn without_schema_validation(&mut self) -> &mut Self {
        self.validate_schema = false;
        self
    }
    /// Set whether to validate formats.
    ///
    /// Default behavior depends on the draft version. This method overrides
    /// the default, enabling or disabling format validation regardless of draft.
    #[inline]
    pub fn should_validate_formats(&mut self, yes: bool) -> &mut Self {
        self.validate_formats = Some(yes);
        self
    }
    pub(crate) fn validate_formats(&self) -> Option<bool> {
        self.validate_formats
    }
    /// Set whether to ignore unknown formats.
    ///
    /// By default, unknown formats are silently ignored. Set to `false` to report
    /// unrecognized formats as validation errors.
    pub fn should_ignore_unknown_formats(&mut self, yes: bool) -> &mut Self {
        self.ignore_unknown_formats = yes;
        self
    }

    pub(crate) const fn are_unknown_formats_ignored(&self) -> bool {
        self.ignore_unknown_formats
    }
    /// Register a custom keyword validator.
    ///
    /// ## Example
    ///
    /// ```rust
    /// # use jsonschema::{
    /// #    paths::{LazyLocation, Location},
    /// #    ErrorIterator, Keyword, ValidationError,
    /// # };
    /// # use serde_json::{json, Map, Value};
    /// # use std::iter::once;
    ///
    /// struct MyCustomValidator;
    ///
    /// impl Keyword for MyCustomValidator {
    ///     fn validate<'i>(
    ///         &self,
    ///         instance: &'i Value,
    ///         location: &LazyLocation,
    ///     ) -> Result<(), ValidationError<'i>> {
    ///         // ... validate instance ...
    ///         if !instance.is_object() {
    ///             return Err(ValidationError::custom(
    ///                 Location::new(),
    ///                 location.into(),
    ///                 instance,
    ///                 "Boom!",
    ///             ));
    ///         } else {
    ///             Ok(())
    ///         }
    ///     }
    ///     fn is_valid(&self, instance: &Value) -> bool {
    ///         // ... determine if instance is valid ...
    ///         true
    ///     }
    /// }
    ///
    /// // You can create a factory function, or use a closure to create new validator instances.
    /// fn custom_validator_factory<'a>(
    ///     parent: &'a Map<String, Value>,
    ///     value: &'a Value,
    ///     path: Location,
    /// ) -> Result<Box<dyn Keyword>, ValidationError<'a>> {
    ///     Ok(Box::new(MyCustomValidator))
    /// }
    ///
    /// let validator = jsonschema::options()
    ///     .with_keyword("my-type", custom_validator_factory)
    ///     .with_keyword("my-type-with-closure", |_, _, _| Ok(Box::new(MyCustomValidator)))
    ///     .build(&json!({ "my-type": "my-schema"}))
    ///     .expect("A valid schema");
    ///
    /// assert!(validator.is_valid(&json!({ "a": "b"})));
    /// ```
    pub fn with_keyword<N, F>(&mut self, name: N, factory: F) -> &mut Self
    where
        N: Into<String>,
        F: for<'a> Fn(
                &'a serde_json::Map<String, Value>,
                &'a Value,
                Location,
            ) -> Result<Box<dyn Keyword>, ValidationError<'a>>
            + Send
            + Sync
            + 'static,
    {
        self.keywords.insert(name.into(), Arc::new(factory));
        self
    }

    pub(crate) fn get_keyword_factory(&self, name: &str) -> Option<&Arc<dyn KeywordFactory>> {
        self.keywords.get(name)
    }
}

impl fmt::Debug for ValidationOptions {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("CompilationConfig")
            .field("draft", &self.draft)
            .field("content_media_type", &self.content_media_type_checks.keys())
            .field(
                "content_encoding",
                &self.content_encoding_checks_and_converters.keys(),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    fn custom(s: &str) -> bool {
        s.ends_with("42!")
    }

    #[test]
    fn custom_format() {
        let schema = json!({"type": "string", "format": "custom"});
        let validator = crate::options()
            .with_format("custom", custom)
            .should_validate_formats(true)
            .build(&schema)
            .expect("Valid schema");
        assert!(!validator.is_valid(&json!("foo")));
        assert!(validator.is_valid(&json!("foo42!")));
    }
}
