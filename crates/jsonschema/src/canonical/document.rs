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

/// Deepest `Raw`-preserved document: round-tripping re-parses the stored text and drops the decoded
/// tree, both recursing per nesting level, so past this bound canonicalization is rejected instead
/// of handing out a schema whose emit exhausts the stack.
const MAX_RAW_SCHEMA_DEPTH: usize = 8192;

/// Iterative depth measurement: must not recurse, it guards the recursive stages.
fn depth_exceeds(value: &Value, limit: usize) -> bool {
    let mut stack = vec![(value, 1usize)];
    while let Some((node, depth)) = stack.pop() {
        if depth > limit {
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

pub(crate) fn exceeds_depth_limit(value: &Value) -> bool {
    depth_exceeds(value, MAX_SCHEMA_DEPTH)
}

pub(crate) fn exceeds_raw_depth_limit(value: &Value) -> bool {
    depth_exceeds(value, MAX_RAW_SCHEMA_DEPTH)
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
    raw_schema_from_text(
        raw_schema_text(value),
        draft,
        pattern_options,
        validate_formats,
    )
}

pub(crate) fn raw_schema_from_text(
    text: Arc<str>,
    draft: Draft,
    pattern_options: PatternEngineOptions,
    validate_formats: bool,
) -> CanonicalSchema {
    CanonicalSchema::with_definitions(
        shared(Schema::Raw(text)),
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
    // so a depth beyond what those stacks tolerate is preserved verbatim instead — up to the bound
    // the `Raw` round-trip itself can rebuild.
    if exceeds_depth_limit(value) {
        if exceeds_raw_depth_limit(value) {
            return Err(CanonicalizationError::DepthLimitExceeded);
        }
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

/// Whether a schema node (root or external `$ref` target) or any subschema within it must be preserved as
/// `Raw`: an unrecognized meta-schema, a lossy `unevaluated*` fold, a dynamic/recursive ref, or an
/// out-of-range numeric/cardinality bound.
pub(crate) fn requires_opaque_preservation(value: &Value, draft: Draft) -> bool {
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
            // `const`/`enum` numbers past the expansion cap emit in scientific normal form, which
            // the runtime validator cannot compare exactly; such documents stay raw.
            if ["const", "enum"].iter().any(|keyword| {
                map.get(*keyword)
                    .is_some_and(|value| !finite_value_spelling_is_exact(value))
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

/// Whether every number nested in an instance-data value keeps a plain canonical spelling.
#[cfg(feature = "arbitrary-precision")]
fn finite_value_spelling_is_exact(value: &Value) -> bool {
    match value {
        Value::Number(number) => {
            crate::canonical::json::number_spelling_stays_plain(number.as_str())
        }
        Value::Array(items) => items.iter().all(finite_value_spelling_is_exact),
        Value::Object(map) => map.values().all(finite_value_spelling_is_exact),
        _ => true,
    }
}

#[cfg(not(feature = "arbitrary-precision"))]
fn finite_value_spelling_is_exact(_value: &Value) -> bool {
    // Default-build numbers are `i64`/`u64`/`f64`; their canonical spellings never go scientific.
    true
}

/// Whether `value` resolves a `$dynamicRef`/`$recursiveRef` anywhere in its subtree: a reference that binds
/// against its own document's dynamic scope, so relocating the fragment into a referrer loses the anchor it targets.
/// Explicit work stack: external `$ref` targets are probed before any depth gate, so the scan must handle
/// documents of any nesting depth.
pub(crate) fn contains_dynamic_scope_ref(value: &Value, draft: Draft) -> bool {
    let mut stack = vec![(value, draft)];
    while let Some((node, draft)) = stack.pop() {
        let Value::Object(map) = node else {
            continue;
        };
        if ["$dynamicRef", "$recursiveRef"]
            .iter()
            .any(|keyword| draft.is_known_keyword(keyword) && map.contains_key(*keyword))
        {
            return true;
        }
        stack.extend(
            draft
                .subresources_of(node)
                .map(|schema| (schema, draft.detect(schema))),
        );
    }
    false
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
    // Referencing owns the id rules: the draft's keyword (`id` vs `$id`), the legacy
    // `$ref`-sibling suppression, and trailing-`#` trimming.
    let resource = draft.create_resource_ref(value);
    let Some(id) = resource.id() else {
        return fallback_base;
    };
    let Ok(reference) = referencing::UriRef::parse(id) else {
        return fallback_base;
    };
    if reference.has_scheme() {
        referencing::uri::from_str(id).unwrap_or(fallback_base)
    } else {
        fallback_base
    }
}

// Iterative to avoid the stack overflow `serde_json::to_string` hits on documents past
// `MAX_SCHEMA_DEPTH` (the ones routed here). Scalars and keys reuse serde_json for identical output;
// only the container walk is explicit.
fn raw_schema_text(value: &Value) -> Arc<str> {
    enum Step<'a> {
        Value(&'a Value),
        Literal(&'static str),
        Key(&'a str),
    }
    let mut out = String::new();
    let mut stack = vec![Step::Value(value)];
    while let Some(step) = stack.pop() {
        match step {
            Step::Literal(text) => out.push_str(text),
            Step::Key(key) => {
                out.push_str(&serde_json::to_string(key).expect("string serializes"));
                out.push(':');
            }
            Step::Value(Value::Array(items)) => {
                out.push('[');
                stack.push(Step::Literal("]"));
                for (index, item) in items.iter().enumerate().rev() {
                    stack.push(Step::Value(item));
                    if index > 0 {
                        stack.push(Step::Literal(","));
                    }
                }
            }
            Step::Value(Value::Object(map)) => {
                out.push('{');
                stack.push(Step::Literal("}"));
                for (index, (key, item)) in map.iter().enumerate().rev() {
                    stack.push(Step::Value(item));
                    stack.push(Step::Key(key.as_str()));
                    if index > 0 {
                        stack.push(Step::Literal(","));
                    }
                }
            }
            Step::Value(scalar) => {
                out.push_str(&serde_json::to_string(scalar).expect("scalar serializes"));
            }
        }
    }
    Arc::from(out)
}

/// The `Raw` node a `$ref` resolves to when its target document/fragment must be preserved verbatim
/// (see [`requires_opaque_preservation`]) - the external-document analogue of the root's `PreparedRoot::Raw`.
pub(crate) fn raw_subschema(value: &Value) -> SharedSchema {
    shared(Schema::Raw(raw_schema_text(value)))
}
