//! Configuration and entry points for canonicalization.

use std::{
    collections::{BTreeSet, HashSet},
    sync::Arc,
};

use referencing::{Draft, Registry, Retrieve, Uri};
use serde_json::Value;

use crate::{
    canonical::{
        context::{CanonicalizationContext, SharedRegexes},
        emptiness,
        ir::{RawJson, RawReason, Schema, SchemaKind},
        parse, refold,
        schema::CanonicalSchema,
        CanonicalizationError, DefinitionMap, ROOT_DEFINITION_KEY,
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
    regexes: SharedRegexes,
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

    /// Pointers to the subschemas that admit no value.
    ///
    /// A pointer left out is not proven satisfiable: an unmodeled document reports nothing, like
    /// [`Satisfiability::Unknown`](crate::canonical::Satisfiability).
    ///
    /// # Errors
    ///
    /// Same as [`crate::canonicalize`], for the document itself.
    pub fn unsatisfiable_pointers(&self) -> Result<HashSet<String>, CanonicalizationError> {
        let Some((registry, base_uri)) = &self.resolution else {
            return Ok(HashSet::new());
        };
        let resolver = registry.resolver(base_uri.clone());
        let context =
            CanonicalizationContext::new(self.draft, self.pattern_options, self.validate_formats)
                .sharing_regexes(Arc::clone(&self.regexes));
        let Some(parsed) = parse::parse_tracking_nodes(self.document, &context, &resolver)? else {
            return Ok(HashSet::new());
        };
        let mut pointers = HashSet::new();
        collect_unsatisfiable_pointers(self.document, &mut String::new(), &parsed, &mut pointers);
        Ok(pointers)
    }

    fn reduce(&self, target: &Value) -> Result<CanonicalSchema, CanonicalizationError> {
        let opaque = |target: &Value, reason: RawReason, pointer: Option<Arc<str>>| {
            CanonicalSchema::new(
                Schema::new(SchemaKind::Raw(RawJson::new(
                    target.clone(),
                    reason,
                    pointer,
                ))),
                self.draft,
                self.pattern_options,
                self.validate_formats,
                Arc::new(DefinitionMap::new()),
                Arc::new(BTreeSet::new()),
            )
        };
        let Some((registry, base_uri)) = &self.resolution else {
            return Ok(opaque(target, RawReason::UnknownDialect, None));
        };
        let resolver = registry.resolver(base_uri.clone());
        let context =
            CanonicalizationContext::new(self.draft, self.pattern_options, self.validate_formats)
                .sharing_regexes(Arc::clone(&self.regexes));
        let Some(parsed) = parse::parse(target, &context, &resolver)? else {
            let reason = raw_reason(&context);
            // Only an unmodeled construct sits at one node; a run out of allowance gave up on the
            // document as a whole.
            let pointer = (reason == RawReason::Unmodeled)
                .then(|| context.declined_at())
                .flatten()
                .and_then(|address| pointer_to(target, &mut String::new(), address));
            return Ok(opaque(target, reason, pointer));
        };
        let parsed = emptiness::fold_definitions(parsed, target, &context, &resolver)?;
        // Folded now every body is known, so this entry point and the set operations agree.
        let parsed = refold::through_targets(parsed, &context);
        let (inner, definitions, local) = (
            parsed.root,
            Arc::new(parsed.definitions),
            Arc::new(parsed.local_definitions),
        );
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
            regexes: SharedRegexes::default(),
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
        regexes: SharedRegexes::default(),
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

/// Whether the body `key` names is unsatisfiable, reading through a chain of pointers as
/// [`CanonicalSchema::satisfiability`] does.
fn names_unsatisfiable_body(parsed: &parse::ParseOutput, key: &str) -> bool {
    let mut key = key;
    let mut walked: Vec<&str> = Vec::new();
    loop {
        if walked.contains(&key) {
            return false;
        }
        walked.push(key);
        match parsed.parsed_definitions.get(key) {
            Some(parse::ParsedNode::Unsatisfiable) => return true,
            Some(parse::ParsedNode::Reference(next)) => {
                key = next.as_ref();
                continue;
            }
            None => {}
        }
        let body = if key == ROOT_DEFINITION_KEY {
            &parsed.root
        } else {
            match parsed.definitions.get(key) {
                Some(body) => body,
                None => return false,
            }
        };
        match body.kind() {
            SchemaKind::False => return true,
            SchemaKind::Reference(next) => key = next.as_ref(),
            SchemaKind::MultiType(_)
            | SchemaKind::TypedGroup { .. }
            | SchemaKind::String(_)
            | SchemaKind::Integer(_)
            | SchemaKind::Number(_)
            | SchemaKind::Array(_)
            | SchemaKind::Object(_)
            | SchemaKind::Const(_)
            | SchemaKind::Enum(_)
            | SchemaKind::Not(_)
            | SchemaKind::AllOf(_)
            | SchemaKind::AnyOf(_)
            | SchemaKind::OneOf(_)
            | SchemaKind::True
            | SchemaKind::Raw(_) => return false,
        }
    }
}

/// Walk `value` alongside what the parse made of each node, naming the unsatisfiable ones by pointer.
/// What stopped the run, read off the context the parse left behind. An exhausted allowance also
/// records an approximation, so it is asked about first.
fn raw_reason(context: &CanonicalizationContext) -> RawReason {
    if context.outgrew_distribution() {
        RawReason::OutgrewIntersections
    } else if context.saw_inexact_intersection() {
        RawReason::InexactIntersection
    } else if context.outgrew_cases() {
        RawReason::OutgrewCases
    } else {
        RawReason::Unmodeled
    }
}

/// The pointer naming the node at `address`, or `None` where the parse declined on a schema it
/// rewrote, which the document no longer holds.
fn pointer_to(value: &Value, pointer: &mut String, address: usize) -> Option<Arc<str>> {
    if std::ptr::from_ref(value) as usize == address {
        return Some(Arc::from(pointer.as_str()));
    }
    let restore = pointer.len();
    let mut children = |children: Box<dyn Iterator<Item = (String, &Value)> + '_>| {
        for (segment, child) in children {
            pointer.push('/');
            pointer.push_str(&segment);
            if let Some(found) = pointer_to(child, pointer, address) {
                return Some(found);
            }
            pointer.truncate(restore);
        }
        None
    };
    match value {
        Value::Object(map) => children(Box::new(map.iter().map(|(key, child)| {
            let mut segment = String::new();
            referencing::write_escaped_str(&mut segment, key);
            (segment, child)
        }))),
        Value::Array(items) => children(Box::new(
            items
                .iter()
                .enumerate()
                .map(|(index, child)| (index.to_string(), child)),
        )),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn collect_unsatisfiable_pointers(
    value: &Value,
    pointer: &mut String,
    parsed: &parse::ParseOutput,
    out: &mut HashSet<String>,
) {
    let empty = match parsed.parsed_nodes.get(&std::ptr::from_ref(value)) {
        Some(parse::ParsedNode::Unsatisfiable) => true,
        Some(parse::ParsedNode::Reference(key)) => names_unsatisfiable_body(parsed, key),
        None => false,
    };
    if empty {
        out.insert(pointer.clone());
    }
    let restore = pointer.len();
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                pointer.push('/');
                referencing::write_escaped_str(pointer, key);
                collect_unsatisfiable_pointers(child, pointer, parsed, out);
                pointer.truncate(restore);
            }
        }
        Value::Array(items) => {
            let mut index_buffer = itoa::Buffer::new();
            for (index, child) in items.iter().enumerate() {
                pointer.push('/');
                pointer.push_str(index_buffer.format(index));
                collect_unsatisfiable_pointers(child, pointer, parsed, out);
                pointer.truncate(restore);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}
