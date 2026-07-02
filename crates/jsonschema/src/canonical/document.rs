//! Document intake: turn a raw schema `Value` into something canonicalizable.
//!
//! Front of the pipeline: detect the draft, build the resolver registry, validate the document, and decide whether
//! it must be preserved verbatim (`Raw`) rather than canonicalized.

use std::sync::Arc;

use referencing::{Draft, Registry};
use serde_json::Value;

use crate::{
    canonical::{
        intern::shared,
        ir::{Schema, SharedSchema},
        parse,
        schema::CanonicalSchema,
        CanonicalizationError, DefinitionMap,
    },
    compiler::formats_are_assertions_by_default,
    options::PatternEngineOptions,
};

/// Deepest document the recursive stages (metaschema validation, opaqueness scan, parse, the rewrite
/// pipeline) are guaranteed to handle without exhausting the stack; deeper documents stay `Raw`.
const MAX_SCHEMA_DEPTH: usize = 128;

/// Iterative depth measurement: must not recurse, it guards the recursive stages.
pub(crate) fn exceeds_depth_limit(value: &Value) -> bool {
    let mut stack = vec![(value, 1usize)];
    while let Some((node, depth)) = stack.pop() {
        if depth > MAX_SCHEMA_DEPTH {
            return true;
        }
        match node {
            Value::Object(map) => stack.extend(map.values().map(|child| (child, depth + 1))),
            Value::Array(items) => stack.extend(items.iter().map(|child| (child, depth + 1))),
            _ => {}
        }
    }
    false
}

fn ensure_schema_document_root(value: &Value) -> Result<(), CanonicalizationError> {
    match value {
        Value::Bool(_) | Value::Object(_) => Ok(()),
        other => Err(CanonicalizationError::InvalidSchemaType(other.to_string())),
    }
}

pub(crate) fn validate_schema_document(
    value: &Value,
    draft: Draft,
) -> Result<(), CanonicalizationError> {
    crate::compiler::validate_schema(draft, value).map_err(CanonicalizationError::from)
}

pub(crate) fn raw_schema(
    value: &Value,
    draft: Draft,
    pattern_options: PatternEngineOptions,
    validate_formats: bool,
) -> CanonicalSchema {
    CanonicalSchema::with_definitions(
        shared(Schema::Raw(raw_schema_text(value))),
        draft,
        pattern_options,
        validate_formats,
        Arc::new(DefinitionMap::new()),
    )
}

/// Outcome of the shared pre-flight: a schema that must be preserved raw, or the resolved settings to
/// canonicalize with.
pub(crate) enum PreparedRoot {
    Raw(CanonicalSchema),
    Canonicalize {
        draft: Draft,
        validate_formats: bool,
    },
}

/// Validate the root document and resolve the effective draft / format settings, shared by sync and async entry
/// points.
pub(crate) fn prepare_root(
    value: &Value,
    draft: Option<Draft>,
    registry: Option<&Registry<'_>>,
    validate_formats: Option<bool>,
    pattern_options: PatternEngineOptions,
) -> Result<PreparedRoot, CanonicalizationError> {
    ensure_schema_document_root(value)?;
    let draft = detect_draft(value, draft, registry)?;
    let validate_formats =
        validate_formats.unwrap_or_else(|| formats_are_assertions_by_default(draft));
    // Everything past this point recurses over the document (validation, opaqueness scan, parse),
    // so a depth beyond what those stacks tolerate is preserved verbatim instead.
    if exceeds_depth_limit(value) {
        return Ok(PreparedRoot::Raw(raw_schema(
            value,
            draft,
            pattern_options,
            validate_formats,
        )));
    }
    validate_schema_document(value, draft)?;
    if requires_opaque_preservation(value, draft) {
        return Ok(PreparedRoot::Raw(raw_schema(
            value,
            draft,
            pattern_options,
            validate_formats,
        )));
    }
    Ok(PreparedRoot::Canonicalize {
        draft,
        validate_formats,
    })
}

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

/// Whether this object node folds `unevaluated*` lossily: a non-`true` `unevaluatedItems`/`unevaluatedProperties`
/// paired with a sibling applicator (or `contains`, for items) whose evaluation scope the leaf can't model. Node-local.
fn folds_unevaluated_lossily(map: &serde_json::Map<String, Value>, draft: Draft) -> bool {
    let has_known_keyword =
        |keyword: &str| draft.is_known_keyword(keyword) && map.contains_key(keyword);
    let has_known_non_true_keyword = |keyword: &str| {
        draft.is_known_keyword(keyword)
            && map
                .get(keyword)
                .is_some_and(|value| !matches!(value, Value::Bool(true)))
    };
    let has_applicator = ["allOf", "anyOf", "oneOf", "if", "not", "$ref", "dependentSchemas"]
        .iter()
        .any(|keyword| has_known_keyword(keyword))
        // Schema-form `dependencies` is an in-place applicator too.
        || (draft.is_known_keyword("dependencies")
            && map
                .get("dependencies")
                .and_then(Value::as_object)
                .is_some_and(|deps| deps.values().any(Value::is_object)));
    (has_known_non_true_keyword("unevaluatedItems")
        && (has_applicator || has_known_keyword("contains")))
        || (has_known_non_true_keyword("unevaluatedProperties") && has_applicator)
}

/// Whether an external `$ref` target (or a subschema within it) folds `unevaluated*` lossily and so must be kept raw.
/// The *only* such reason for an external target: other opaque cases ($dynamicRef, numerics, ...) the parser handles.
pub(crate) fn external_target_folds_unevaluated(value: &Value, draft: Draft) -> bool {
    match value {
        Value::Object(map) => {
            folds_unevaluated_lossily(map, draft)
                || draft
                    .subresources_of(value)
                    .any(|schema| external_target_folds_unevaluated(schema, draft.detect(schema)))
        }
        _ => false,
    }
}

fn requires_opaque_preservation(value: &Value, draft: Draft) -> bool {
    match value {
        Value::Object(map) => {
            if map
                .get("$schema")
                .and_then(Value::as_str)
                .is_some_and(|uri| matches!(Draft::from_schema_uri(uri), Draft::Unknown))
            {
                return true;
            }
            if folds_unevaluated_lossily(map, draft) {
                return true;
            }
            if [
                "$recursiveRef",
                "$recursiveAnchor",
                "$dynamicRef",
                "$dynamicAnchor",
            ]
            .iter()
            .any(|keyword| draft.is_known_keyword(keyword) && map.contains_key(*keyword))
            {
                return true;
            }
            if [
                "minimum",
                "maximum",
                "exclusiveMinimum",
                "exclusiveMaximum",
                "multipleOf",
            ]
            .iter()
            .any(|keyword| {
                map.get(*keyword)
                    .is_some_and(|value| !parse::numeric_value_is_supported(value))
            }) {
                return true;
            }
            if [
                "minLength",
                "maxLength",
                "minItems",
                "maxItems",
                "minContains",
                "maxContains",
                "minProperties",
                "maxProperties",
            ]
            .iter()
            .any(|keyword| {
                map.get(*keyword)
                    .is_some_and(|value| !parse::cardinality_value_is_supported(value))
            }) {
                return true;
            }
            // Reuse referencing's draft-aware child traversal so instance-data containers (`const`, `examples`) stay
            // opaque while property names like `properties.const` are still reached as schemas.
            draft
                .subresources_of(value)
                .any(|schema| requires_opaque_preservation(schema, draft.detect(schema)))
        }
        _ => false,
    }
}

// Register the root document so same-document refs (anchors, pointers, nested `$id`) resolve through `referencing`
// rather than bespoke logic. `prepare` crawls external refs; if any can't be retrieved callers preserve the schema raw.
pub(crate) fn canonical_registry_builder<'a>(
    registry: Option<&'a Registry<'a>>,
    base_uri: &referencing::Uri<String>,
    resource: referencing::ResourceRef<'a>,
    draft: Draft,
) -> Result<referencing::RegistryBuilder<'a>, referencing::Error> {
    let builder = match registry {
        Some(base) => base.add(base_uri.as_str(), resource)?,
        None => Registry::new().add(base_uri.as_str(), resource)?,
    };
    Ok(builder.draft(draft))
}

/// Initial base URI for resolving relative refs: the document's own absolute `$id` (Draft 4 `id`), else the
/// caller-supplied `base_uri`, else synthetic `file:///schema`. Relative root ids are applied later, not here.
pub(crate) fn root_base_uri(
    value: &Value,
    draft: Draft,
    base_uri: Option<&str>,
) -> referencing::Uri<String> {
    let fallback_base = base_uri
        .and_then(|base| referencing::uri::from_str(base).ok())
        .unwrap_or_else(|| {
            referencing::uri::from_str("file:///schema").expect("static URI is always valid")
        });
    let id_keyword = if draft == Draft::Draft4 { "id" } else { "$id" };
    let Some(id) = value
        .as_object()
        .and_then(|map| map.get(id_keyword))
        .and_then(Value::as_str)
    else {
        return fallback_base;
    };
    let id = id
        .strip_suffix('#')
        .filter(|stripped| !stripped.is_empty())
        .unwrap_or(id);
    let Ok(reference) = referencing::UriRef::parse(id) else {
        return fallback_base;
    };
    if reference.has_scheme() {
        referencing::uri::from_str(id).unwrap_or(fallback_base)
    } else {
        fallback_base
    }
}

fn raw_schema_text(value: &Value) -> Arc<str> {
    serde_json::to_string(value)
        .expect("schema Value serializes")
        .into()
}

/// The `Raw` node a `$ref` resolves to when its target document/fragment must be preserved verbatim
/// (see [`requires_opaque_preservation`]) - the external-document analogue of the root's `PreparedRoot::Raw`.
pub(crate) fn raw_subschema(value: &Value) -> SharedSchema {
    shared(Schema::Raw(raw_schema_text(value)))
}
