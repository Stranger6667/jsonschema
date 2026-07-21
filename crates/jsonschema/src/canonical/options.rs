//! Configuration and entry points for canonicalization.

use std::sync::Arc;

use referencing::{Draft, Registry};
use serde_json::Value;

use crate::{
    canonical::{
        context::CanonicalizationContext,
        ir::{RawJson, Schema, SchemaKind},
        parse,
        schema::CanonicalSchema,
        CanonicalizationError, DefinitionMap,
    },
    compiler::{formats_are_assertions_by_default, validate_schema},
    options::PatternEngineOptions,
};

/// Build a [`CanonicalizeOptions`] for configurable canonicalization.
#[must_use]
pub fn options() -> CanonicalizeOptions<'static> {
    CanonicalizeOptions::default()
}

/// Configurable canonicalization entry point. Construct via [`options`].
#[derive(Default)]
pub struct CanonicalizeOptions<'r> {
    registry: Option<&'r Registry<'r>>,
    pattern_options: PatternEngineOptions,
    draft: Option<Draft>,
    validate_formats: Option<bool>,
}

impl<'r> CanonicalizeOptions<'r> {
    /// Use a pre-built [`Registry`] for external `$ref` resolution.
    #[must_use]
    pub fn with_registry(mut self, registry: &'r Registry<'r>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Use this draft for canonicalization, overriding `$schema` detection.
    #[must_use]
    pub fn with_draft(mut self, draft: Draft) -> Self {
        self.draft = Some(draft);
        self
    }

    /// Set whether canonicalization treats `format` as a validation assertion.
    ///
    /// Left unset, it follows the draft default (Draft 4/6/7 assert known formats; 2019-09/2020-12 annotate).
    /// Asserting lets incompatible format intersections like `date`/`uuid` collapse to `false`.
    #[must_use]
    pub fn should_validate_formats(mut self, enabled: bool) -> Self {
        self.validate_formats = Some(enabled);
        self
    }

    /// Run canonicalization with the configured options.
    ///
    /// # Errors
    ///
    /// Same as [`crate::canonicalize`].
    pub fn canonicalize(self, value: &Value) -> Result<CanonicalSchema, CanonicalizationError> {
        build(
            value,
            self.draft,
            self.registry,
            self.validate_formats,
            self.pattern_options,
        )
    }
}

/// Validate the document and reduce it to a [`CanonicalSchema`].
fn build(
    value: &Value,
    draft: Option<Draft>,
    registry: Option<&Registry<'_>>,
    validate_formats: Option<bool>,
    pattern_options: PatternEngineOptions,
) -> Result<CanonicalSchema, CanonicalizationError> {
    // Only a boolean or object is a schema document.
    match value {
        Value::Bool(_) | Value::Object(_) => {}
        other => return Err(CanonicalizationError::InvalidSchemaType(other.to_string())),
    }
    let draft = detect_draft(value, draft, registry)?;
    let validate_formats =
        validate_formats.unwrap_or_else(|| formats_are_assertions_by_default(draft));
    validate_schema(draft, value)?;
    let context = CanonicalizationContext::new(draft, pattern_options);
    let inner = match parse::parse(value, &context)? {
        Some(schema) => schema,
        None => Schema::new(SchemaKind::Raw(RawJson::new(value.clone()))),
    };
    Ok(CanonicalSchema::new(
        inner,
        draft,
        pattern_options,
        validate_formats,
        Arc::new(DefinitionMap::new()),
    ))
}

/// Resolve the draft: an explicit override, else detected from `$schema`.
fn detect_draft<'r>(
    value: &Value,
    draft: Option<Draft>,
    registry: Option<&'r Registry<'r>>,
) -> Result<Draft, CanonicalizationError> {
    let mut options = crate::options();
    if let Some(draft) = draft {
        options = options.with_draft(draft);
    }
    if let Some(registry) = registry {
        options = options.with_registry(registry);
    }
    options
        .draft_for(value)
        .map_err(|error| CanonicalizationError::InvalidSchemaType(error.to_string()))
}
