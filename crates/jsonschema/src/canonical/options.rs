//! Configuration and entry points for canonicalization.

use std::{collections::BTreeSet, sync::Arc};

use referencing::{Draft, Registry, Retrieve, Uri};
use serde_json::Value;

use crate::{
    canonical::{
        context::CanonicalizationContext,
        emptiness,
        ir::{RawJson, Schema, SchemaKind},
        parse, refold,
        schema::CanonicalSchema,
        CanonicalizationError, DefinitionMap,
    },
    compiler::{
        formats_are_assertions_by_default, normalize_base_uri, resolve_base_uri, validate_schema,
    },
    options::{PatternEngineOptions, PatternOptions},
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
    retriever: Option<Arc<dyn Retrieve>>,
    base_uri: Option<String>,
    pattern_options: PatternEngineOptions,
    draft: Option<Draft>,
    validate_formats: Option<bool>,
}

impl<'r> CanonicalizeOptions<'r> {
    /// Use a pre-built [`Registry`] for dialect and `$ref` resolution.
    #[must_use]
    pub fn with_registry(mut self, registry: &'r Registry<'r>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Fetch external resources that are not present in the registry.
    #[must_use]
    pub fn with_retriever(mut self, retriever: impl Retrieve + 'static) -> Self {
        self.retriever = Some(Arc::new(retriever));
        self
    }

    /// Refuse to fetch any reference that is not already in the registry.
    #[must_use]
    pub fn offline(mut self) -> Self {
        self.retriever = Some(Arc::new(crate::retriever::OfflineRetriever));
        self
    }

    /// Use this URI as the base for resolving relative references in the root schema.
    ///
    /// Takes precedence over the root `$id`.
    #[must_use]
    pub fn with_base_uri(mut self, base_uri: impl Into<String>) -> Self {
        self.base_uri = Some(base_uri.into());
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

    /// Select the regular-expression engine used for `pattern` compilation and membership.
    #[must_use]
    #[allow(clippy::needless_pass_by_value)]
    pub fn with_pattern_options<E>(mut self, options: PatternOptions<E>) -> Self {
        self.pattern_options = options.inner;
        self
    }

    /// Run canonicalization with the configured options.
    ///
    /// # Errors
    ///
    /// Same as [`crate::canonicalize`].
    pub fn canonicalize(self, value: &Value) -> Result<CanonicalSchema, CanonicalizationError> {
        self.prepare(value)?.canonicalize()
    }

    /// Prepare `value` for canonicalizing its subschemas.
    ///
    /// The document decides the draft, the base URI and what `#` means, so a subschema selected
    /// from a prepared document resolves its references as it does in place. Resolving the draft,
    /// validating the document and indexing it depends only on the document, so preparing once
    /// pays for it once however many subschemas are then selected.
    ///
    /// # Examples
    ///
    /// ```
    /// use jsonschema::canonical::options;
    /// use serde_json::json;
    ///
    /// let document = json!({
    ///     "$defs": {
    ///         "Named": {"type": "object", "required": ["name"]},
    ///         "Pet": {"allOf": [
    ///             {"$ref": "#/$defs/Named"},
    ///             {"properties": {"age": {"type": "integer", "minimum": 0}}}
    ///         ]}
    ///     }
    /// });
    ///
    /// let prepared = options().prepare(&document)?;
    /// assert_eq!(
    ///     prepared.canonicalize_at("/$defs/Pet")?.to_json_schema(),
    ///     json!({
    ///         "$schema": "https://json-schema.org/draft/2020-12/schema",
    ///         "type": "object",
    ///         "properties": {"age": {"type": "integer", "minimum": 0}},
    ///         "required": ["name"]
    ///     })
    /// );
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Same as [`canonicalize`](Self::canonicalize), for the document itself.
    pub fn prepare<'a>(
        self,
        value: &'a Value,
    ) -> Result<PreparedDocument<'a>, CanonicalizationError>
    where
        'r: 'a,
    {
        prepare(value, &self)
    }
}

/// A document indexed once, ready to canonicalize any number of its subschemas.
///
/// Built by [`CanonicalizeOptions::prepare`].
pub struct PreparedDocument<'a> {
    document: &'a Value,
    draft: Draft,
    pattern_options: PatternEngineOptions,
    validate_formats: bool,
    // `None` when the draft is unknown: nothing resolves, and every selection stays verbatim.
    resolution: Option<(Registry<'a>, Uri<String>)>,
}

impl PreparedDocument<'_> {
    /// The draft the document was read under.
    #[must_use]
    pub fn draft(&self) -> Draft {
        self.draft
    }

    /// Canonicalize the document itself.
    ///
    /// # Errors
    ///
    /// Same as [`crate::canonicalize`].
    pub fn canonicalize(&self) -> Result<CanonicalSchema, CanonicalizationError> {
        self.reduce(self.document)
    }

    /// Canonicalize the subschema at `pointer`, in the document's context.
    ///
    /// # Errors
    ///
    /// Same as [`crate::canonicalize`], plus [`CanonicalizationError::PointerNotFound`] when
    /// `pointer` names nothing.
    pub fn canonicalize_at(&self, pointer: &str) -> Result<CanonicalSchema, CanonicalizationError> {
        let target = referencing::pointer(self.document, pointer)
            .ok_or_else(|| CanonicalizationError::PointerNotFound(pointer.to_string()))?;
        match target {
            Value::Bool(_) | Value::Object(_) => self.reduce(target),
            other @ (Value::Null | Value::Number(_) | Value::String(_) | Value::Array(_)) => {
                Err(CanonicalizationError::InvalidSchemaType(other.to_string()))
            }
        }
    }

    fn reduce(&self, target: &Value) -> Result<CanonicalSchema, CanonicalizationError> {
        let opaque = |target: &Value| {
            CanonicalSchema::new(
                Schema::new(SchemaKind::Raw(RawJson::new(target.clone()))),
                self.draft,
                self.pattern_options,
                self.validate_formats,
                Arc::new(DefinitionMap::new()),
                Arc::new(BTreeSet::new()),
            )
        };
        let Some((registry, base_uri)) = &self.resolution else {
            return Ok(opaque(target));
        };
        let resolver = registry.resolver(base_uri.clone());
        let context =
            CanonicalizationContext::new(self.draft, self.pattern_options, self.validate_formats);
        let (inner, definitions, local) = match parse::parse(target, &context, &resolver)? {
            Some(parsed) => {
                let parsed = emptiness::fold_definitions(parsed, target, &context, &resolver)?;
                // Folded now every body is known, so this entry point and the set operations agree.
                let parsed = refold::through_targets(parsed, &context);
                (
                    parsed.root,
                    Arc::new(parsed.definitions),
                    Arc::new(parsed.local_definitions),
                )
            }
            None => return Ok(opaque(target)),
        };
        Ok(CanonicalSchema::new(
            inner,
            self.draft,
            self.pattern_options,
            self.validate_formats,
            definitions,
            local,
        ))
    }
}

/// Validate the document and index it for reference resolution.
fn prepare<'a, 'r: 'a>(
    value: &'a Value,
    options: &CanonicalizeOptions<'r>,
) -> Result<PreparedDocument<'a>, CanonicalizationError> {
    // Only a boolean or object is a schema document.
    match value {
        Value::Bool(_) | Value::Object(_) => {}
        other @ (Value::Null | Value::Number(_) | Value::String(_) | Value::Array(_)) => {
            return Err(CanonicalizationError::InvalidSchemaType(other.to_string()))
        }
    }
    let pattern_options = options.pattern_options;
    let draft = detect_draft(value, options.draft, options.registry)?;
    if draft == Draft::Unknown {
        return Ok(PreparedDocument {
            document: value,
            draft,
            pattern_options,
            validate_formats: options.validate_formats.unwrap_or(false),
            resolution: None,
        });
    }
    let validate_formats = options
        .validate_formats
        .unwrap_or_else(|| formats_are_assertions_by_default(draft));
    validate_schema(draft, value)?;
    let resource = draft.create_resource_ref(value);
    let base_uri = resolve_base_uri(options.base_uri.as_ref(), resource.id())?;
    let mut builder = match options.registry {
        Some(registry) => registry.add(base_uri.as_str(), resource)?,
        None => Registry::new().add(base_uri.as_str(), resource)?,
    };
    if let Some(retriever) = &options.retriever {
        builder = builder.retriever(Arc::clone(retriever));
    }
    let registry = builder.draft(draft).prepare()?;
    let base_uri = normalize_base_uri(&registry, &base_uri);
    Ok(PreparedDocument {
        document: value,
        draft,
        pattern_options,
        validate_formats,
        resolution: Some((registry, base_uri)),
    })
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
        .map_err(CanonicalizationError::from)
}
