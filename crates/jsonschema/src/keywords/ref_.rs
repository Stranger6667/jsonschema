use crate::LazyInstance;
use std::borrow::Cow;

use crate::{
    compiler::{self, TargetNode},
    keywords::{BoxedValidator, CompilationResult},
    paths::{LazyLocation, Location, RefTracker},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
    Json, ValidationError,
};
use referencing::{unescape_segment, uri, Resolved, Uri};
use serde_json::{Map, Value};
use std::sync::Arc;

/// Tracks `$ref` traversals for recursive references where the target is behind `BoxedValidator<F>`
/// (either a `PendingSchemaNode` or a cached node returned by `lookup_maybe_recursive`).
struct RefValidator<F: Json> {
    inner: BoxedValidator<F>,
    /// Path of this `$ref` keyword relative to its resource base.
    /// E.g., `/properties/foo/$ref` (not the full canonical path).
    /// Used for building the `tracker` prefix.
    ref_suffix: Location,
    /// The resource base of the `$ref` target.
    /// E.g., `/$defs/Item` when `$ref` points to `#/$defs/Item`.
    /// Used for computing validator suffixes at runtime.
    ref_target_base: Location,
    /// Absolute URI of the `$ref` target, if its resource has one.
    target_location: Option<Arc<Uri<String>>>,
}

impl<F: Json> Validate<F> for RefValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        self.inner.is_valid(instance, ctx)
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        let child_tracker = RefTracker::new(&self.ref_suffix, &self.ref_target_base, tracker);
        self.inner
            .validate(instance, location, Some(&child_tracker), ctx)
    }

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        let child_tracker = RefTracker::new(&self.ref_suffix, &self.ref_target_base, tracker);
        self.inner
            .collect_errors(instance, location, Some(&child_tracker), ctx, errors);
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        self.evaluate_with_location(instance, location, &location.into(), tracker, ctx)
    }

    fn evaluate_with_location(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: &Location,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let child_tracker = RefTracker::new(&self.ref_suffix, &self.ref_target_base, tracker);
        self.inner.evaluate_with_location(
            instance,
            location,
            instance_location,
            Some(&child_tracker),
            ctx,
        )
    }

    /// Returns `ref_target_base` for `schema_path` output.
    ///
    /// Per JSON Schema 2020-12 Core Section 12.4.2, `schema_path` "MUST NOT include
    /// by-reference applicators such as `$ref` or `$dynamicRef`".
    fn canonical_location(&self) -> Option<(&Location, Option<&Arc<Uri<String>>>)> {
        Some((&self.ref_target_base, self.target_location.as_ref()))
    }
}

/// Like `RefValidator` but holds a concrete target instead of `BoxedValidator<F>`,
/// eliminating one layer of vtable dispatch on every validation call.
/// Used for non-recursive refs: a compiled node, or a placeholder for a deferred one.
struct DirectRefValidator<T> {
    inner: T,
    ref_suffix: Location,
    ref_target_base: Location,
    target_location: Option<Arc<Uri<String>>>,
}

impl<F: Json, T: Validate<F>> Validate<F> for DirectRefValidator<T> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        self.inner.is_valid(instance, ctx)
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        let child_tracker = RefTracker::new(&self.ref_suffix, &self.ref_target_base, tracker);
        self.inner
            .validate(instance, location, Some(&child_tracker), ctx)
    }

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        let child_tracker = RefTracker::new(&self.ref_suffix, &self.ref_target_base, tracker);
        self.inner
            .collect_errors(instance, location, Some(&child_tracker), ctx, errors);
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        self.evaluate_with_location(instance, location, &location.into(), tracker, ctx)
    }

    fn evaluate_with_location(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: &Location,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let child_tracker = RefTracker::new(&self.ref_suffix, &self.ref_target_base, tracker);
        self.inner.evaluate_with_location(
            instance,
            location,
            instance_location,
            Some(&child_tracker),
            ctx,
        )
    }

    fn canonical_location(&self) -> Option<(&Location, Option<&Arc<Uri<String>>>)> {
        Some((&self.ref_target_base, self.target_location.as_ref()))
    }
}

/// Extract `ref_target_base` from a resolved URI fragment.
///
/// JSON Pointer fragments (starting with `/`) become the location path.
/// Anchor fragments (plain names like `#node`) resolve to root.
fn extract_ref_target_base(alias: &referencing::Uri<String>) -> Location {
    if let Some(fragment) = alias.fragment() {
        let fragment = fragment.as_str();
        if fragment.starts_with('/') {
            // Fragment is URI percent-encoded (RFC 3986); Location stores JSON Pointers (RFC 6901).
            let decoded = percent_encoding::percent_decode_str(fragment).decode_utf8_lossy();
            return Location::from_escaped(&decoded);
        }
    }
    Location::new()
}

/// Absolute URI of the target at `location` within the resource `base` names.
///
/// `None` when that resource has no URI of its own, i.e. only the default base URI.
fn target_location(base: &Uri<String>, location: &Location) -> Option<Arc<Uri<String>>> {
    if base.scheme().as_str() == compiler::DEFAULT_SCHEME {
        return None;
    }
    let mut fragment = uri::EncodedBuffer::new();
    fragment.encode_str::<uri::Path>(location.as_str());
    Some(Arc::new(base.with_fragment(Some(fragment.as_estr()))))
}

/// Whether `alias` names its target by an anchor rather than a JSON Pointer.
fn is_named(alias: &referencing::Uri<String>) -> bool {
    alias.fragment().is_some_and(|fragment| {
        !fragment.as_str().is_empty() && !fragment.as_str().starts_with('/')
    })
}

/// Location of a named target within its resource.
fn named_target_base<F: Json>(
    ctx: &compiler::Context<F>,
    resolved: &Resolved<'_>,
) -> Result<Location, referencing::Error> {
    let resolver = resolved.resolver();
    let resource = resolver.lookup("")?;
    Ok(ctx
        .anchor_location(resource.contents(), resolved.contents())
        .expect("A named target lives in its resolver's resource"))
}

/// Length of the `pointer` prefix leading to the innermost `$id` subschema it passes through.
///
/// Zero when the pointer stays in the resource `alias` names.
fn embedded_resource_start<F: Json>(
    ctx: &compiler::Context<F>,
    alias: &Uri<String>,
    pointer: &Location,
    resolved: &Resolved<'_>,
) -> Result<usize, referencing::Error> {
    let target_base = resolved.resolver().base_uri();
    if alias
        .as_str()
        .strip_prefix(target_base.as_str())
        .is_some_and(|fragment| fragment.is_empty() || fragment.starts_with('#'))
    {
        return Ok(0);
    }
    let alias_base = alias
        .as_str()
        .split_once('#')
        .map_or(alias.as_str(), |(base, _)| base);
    let resource = resolved.resolver().lookup("")?.contents();
    let document = ctx.lookup(alias_base)?;
    let mut node = document.contents();
    let mut offset = 0;
    let mut start = 0;
    for segment in pointer.as_str().split('/').skip(1) {
        offset += 1 + segment.len();
        let next = match node {
            Value::Object(map) => map.get(unescape_segment(segment).as_ref()),
            Value::Array(items) => segment.parse::<usize>().ok().and_then(|idx| items.get(idx)),
            _ => None,
        };
        let Some(next) = next else {
            break;
        };
        node = next;
        if std::ptr::eq(node, resource) {
            start = offset;
        }
    }
    Ok(start)
}

fn compile_reference_validator<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &Map<String, Value>,
    reference: &str,
    keyword: &str,
) -> Option<CompilationResult<'a, F>> {
    // An empty reference ("" per RFC 3986) targets the enclosing resource - skip to avoid
    // infinite recursion.
    if reference.is_empty() {
        return None;
    }
    let (alias, ref_target_base) = match ctx
        .ref_target(reference, extract_ref_target_base)
        .map_err(ValidationError::from)
    {
        Ok(target) => target,
        Err(error) => return Some(Err(error)),
    };

    let ref_suffix = ctx.suffix().join(keyword);

    let resolved = match ctx.lookup(reference) {
        Ok(resolved) => resolved,
        Err(error) => return Some(Err(ValidationError::from(error))),
    };
    // A pointer counts from the resource the reference names, an anchor from the one it is in.
    let (ref_target_base, target_location, resource_start) = if is_named(&alias) {
        match named_target_base(ctx, &resolved) {
            Ok(location) => {
                let target = target_location(&resolved.resolver().base_uri(), &location);
                (location, target, 0)
            }
            Err(error) => return Some(Err(ValidationError::from(error))),
        }
    } else {
        match embedded_resource_start(ctx, &alias, &ref_target_base, &resolved) {
            Ok(0) => {
                let target = target_location(&alias, &ref_target_base);
                (ref_target_base, target, 0)
            }
            Ok(start) => {
                let within = Location::from_escaped(&ref_target_base.as_str()[start..]);
                let target = target_location(&resolved.resolver().base_uri(), &within);
                (ref_target_base, target, start)
            }
            Err(error) => return Some(Err(ValidationError::from(error))),
        }
    };

    // Direct self-reference - skip to avoid infinite recursion. This compares node identity
    // rather than URIs because the location pointer is relative to the enclosing document
    // while the base URI may come from an `$id`-bearing subresource; composing the two
    // yields a URI that does not denote this schema and can collide with the target's.
    if resolved
        .contents()
        .as_object()
        .is_some_and(|target| std::ptr::eq(target, parent))
    {
        return None;
    }

    match ctx.lookup_maybe_recursive(reference, resolved.resolver()) {
        Ok(Some(validator)) => {
            return Some(Ok(Box::new(RefValidator {
                inner: validator,
                ref_suffix,
                ref_target_base,
                target_location,
            })));
        }
        Ok(None) => {}
        Err(error) => return Some(Err(error)),
    }

    if let Err(error) = ctx.mark_seen(reference) {
        return Some(Err(ValidationError::from(error)));
    }

    if ctx.defers_ref_target(&resolved, &alias) {
        let placeholder = ctx.defer_ref_target(resolved, ref_target_base.clone(), alias);
        return Some(Ok(Box::new(DirectRefValidator {
            inner: placeholder,
            ref_suffix,
            ref_target_base,
            target_location,
        })));
    }

    let (contents, resolver, draft) = resolved.into_inner();
    let vocabularies = resolver.find_vocabularies(draft, contents);
    let resource_ref = draft.create_resource_ref(contents);
    let inner_ctx = match ctx.with_resolver_and_draft(
        resolver,
        resource_ref.draft(),
        vocabularies,
        ref_target_base.clone(),
    ) {
        Ok(inner_ctx) => inner_ctx.with_resource_start(resource_start),
        Err(error) => return Some(Err(error)),
    };
    Some(
        ctx.compile_ref_target(|| compiler::compile_with_alias(&inner_ctx, resource_ref, alias))
            .map(|target| match target {
                TargetNode::Owned(node) => Box::new(DirectRefValidator {
                    inner: node,
                    ref_suffix,
                    ref_target_base,
                    target_location,
                }) as Box<dyn Validate<F>>,
                TargetNode::Shared(_) => Box::new(RefValidator {
                    inner: target.into_validator(),
                    ref_suffix,
                    ref_target_base,
                    target_location,
                }),
            })
            .map_err(ValidationError::to_owned),
    )
}

fn compile_recursive_validator<'a, F: Json>(
    ctx: &compiler::Context<F>,
    reference: &str,
) -> CompilationResult<'a, F> {
    let ref_suffix = ctx.suffix().join("$recursiveRef");
    let (alias, ref_target_base) = ctx
        .ref_target(reference, extract_ref_target_base)
        .map_err(ValidationError::from)?;
    let resolved = ctx
        .lookup_recursive_reference()
        .map_err(ValidationError::from)?;
    let target_location = target_location(&resolved.resolver().base_uri(), &ref_target_base);

    match ctx.lookup_maybe_recursive(reference, resolved.resolver()) {
        Ok(Some(validator)) => {
            return Ok(Box::new(RefValidator {
                inner: validator,
                ref_suffix,
                ref_target_base,
                target_location,
            }));
        }
        Ok(None) => {}
        Err(error) => return Err(error),
    }

    if let Err(error) = ctx.mark_seen(reference) {
        return Err(ValidationError::from(error));
    }

    let (contents, resolver, draft) = resolved.into_inner();
    let vocabularies = resolver.find_vocabularies(draft, contents);
    let resource_ref = draft.create_resource_ref(contents);
    let target_base = ref_target_base.clone();
    let inner_ctx =
        ctx.with_resolver_and_draft(resolver, resource_ref.draft(), vocabularies, target_base)?;
    compiler::compile_with_alias(&inner_ctx, resource_ref, alias)
        .map(|target| {
            Box::new(RefValidator {
                inner: target.into_validator(),
                ref_suffix,
                ref_target_base,
                target_location,
            }) as Box<dyn Validate<F>>
        })
        .map_err(ValidationError::to_owned)
}

fn invalid_reference<'a, F: Json>(
    ctx: &compiler::Context<F>,
    keyword: &str,
    schema: &'a Value,
) -> ValidationError<'a> {
    let location = ctx.location().join(keyword);
    ValidationError::single_type_error(
        location.clone(),
        location.clone(),
        location,
        LazyInstance::Ready(Cow::Borrowed(schema)),
        JsonType::String,
    )
}

#[inline]
pub(crate) fn compile_impl<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
    keyword: &str,
) -> Option<CompilationResult<'a, F>> {
    if let Some(reference) = schema.as_str() {
        compile_reference_validator(ctx, parent, reference, keyword)
    } else {
        Some(Err(invalid_reference(ctx, keyword, schema)))
    }
}

#[inline]
pub(crate) fn compile_dynamic_ref<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    compile_impl(ctx, parent, schema, "$dynamicRef")
}

#[inline]
pub(crate) fn compile_ref<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    compile_impl(ctx, parent, schema, "$ref")
}

#[inline]
pub(crate) fn compile_recursive_ref<'a, F: Json>(
    ctx: &compiler::Context<F>,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    Some(
        schema
            .as_str()
            .ok_or_else(|| invalid_reference(ctx, "$recursiveRef", schema))
            .and_then(|reference| compile_recursive_validator(ctx, reference)),
    )
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use ahash::HashMap;
    use referencing::{Retrieve, Uri};
    use serde_json::{json, Value};
    use test_case::test_case;

    struct MyRetrieve;

    impl Retrieve for MyRetrieve {
        fn retrieve(
            &self,
            uri: &Uri<String>,
        ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
            match uri.path().as_str() {
                "/indirection" => Ok(json!({
                    "$id": "/indirection",
                    "baz": {
                        "$ref": "/types#/foo"
                    }
                })),
                "/types" => Ok(json!({
                    "$id": "/types",
                    "foo": {
                        "$id": "#/foo",
                        "$ref": "#/bar"
                    },
                    "bar": {
                        "type": "integer"
                    }
                })),
                _ => panic!("Not found"),
            }
        }
    }

    fn no_validation_meta() -> Value {
        json!({
            "$id": "json-schema:///meta/no-validation",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$vocabulary": {
                "https://json-schema.org/draft/2020-12/vocab/core": true,
                "https://json-schema.org/draft/2020-12/vocab/applicator": true,
                "https://json-schema.org/draft/2020-12/vocab/validation": false
            }
        })
    }

    fn build_no_validation(schema: &Value, extra: &[(&str, Value)]) -> crate::Validator {
        let meta = no_validation_meta();
        let mut registry = crate::Registry::new()
            .add("json-schema:///meta/no-validation", &meta)
            .unwrap();
        for (uri, resource) in extra {
            registry = registry.add(*uri, resource).unwrap();
        }
        let registry = registry.prepare().unwrap();
        crate::options()
            .with_registry(&registry)
            .build(schema)
            .unwrap()
    }

    // A `$ref` re-entered while it is being validated counts as satisfied, so the sibling `$ref`
    // cannot be served a result taken outside that state.
    #[test]
    fn self_reference_under_not_agrees_across_modes() {
        let schema = json!({"allOf": [{"not": {"$ref": "#"}}, {"$ref": "#"}]});
        tests_util::is_not_valid(&schema, &json!([]));
    }

    #[test]
    fn ref_same_document_inherits_disabled_validation_vocabulary() {
        let schema = json!({
            "$schema": "json-schema:///meta/no-validation",
            "$defs": {"t": {"type": "integer"}},
            "$ref": "#/$defs/t"
        });
        assert!(build_no_validation(&schema, &[]).is_valid(&json!("x")));
    }

    #[test]
    fn ref_cross_resource_uses_target_validation_enabled() {
        let target = json!({
            "$id": "https://example.com/on",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$defs": {"t": {"type": "integer"}}
        });
        let schema = json!({
            "$schema": "json-schema:///meta/no-validation",
            "$ref": "https://example.com/on#/$defs/t"
        });
        let validator = build_no_validation(&schema, &[("https://example.com/on", target)]);
        assert!(!validator.is_valid(&json!("x")));
    }

    #[test]
    fn ref_cross_resource_uses_target_validation_disabled() {
        let target = json!({
            "$id": "https://example.com/off",
            "$schema": "json-schema:///meta/no-validation",
            "$defs": {"t": {"type": "integer"}}
        });
        let schema = json!({
            "$schema": "json-schema:///meta/no-validation",
            "$ref": "https://example.com/off#/$defs/t"
        });
        let validator = build_no_validation(&schema, &[("https://example.com/off", target)]);
        assert!(validator.is_valid(&json!("x")));
    }

    #[test]
    fn custom_retrieve_can_load_remote() {
        let retriever = MyRetrieve;
        let uri = Uri::try_from("https://example.com/types".to_string()).expect("valid uri");
        let value: Value = retriever
            .retrieve(&uri)
            .expect("should load the remote document");
        let bar = value
            .get("bar")
            .and_then(|schema| schema.get("type"))
            .cloned();
        assert_eq!(bar, Some(json!("integer")));
    }

    struct TestRetrieve {
        storage: HashMap<String, Value>,
    }

    impl Retrieve for TestRetrieve {
        fn retrieve(
            &self,
            uri: &Uri<String>,
        ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
            self.storage
                .get(uri.path().as_str())
                .cloned()
                .ok_or_else(|| "Document not found".into())
        }
    }

    struct NestedRetrieve;

    impl Retrieve for NestedRetrieve {
        fn retrieve(
            &self,
            uri: &Uri<String>,
        ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
            match uri.as_str() {
                "foo://schema_2.json" => Ok(json!({
                    "$id": "foo://schema_2.json",
                    "type": "string"
                })),
                _ => panic!("Unexpected URI: {}", uri.path()),
            }
        }
    }

    struct FragmentRetrieve;

    impl Retrieve for FragmentRetrieve {
        fn retrieve(
            &self,
            uri: &Uri<String>,
        ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
            match uri.path().as_str() {
                "/tmp/schemas/one.json" => Ok(json!({
                    "$defs": {
                        "obj": {
                            "$ref": "other.json#/$defs/obj"
                        }
                    }
                })),
                "/tmp/schemas/other.json" => Ok(json!({
                    "$defs": {
                        "obj": {
                            "type": "number"
                        }
                    }
                })),
                _ => panic!("Unexpected URI: {}", uri.path()),
            }
        }
    }

    #[test_case(
        &json!({
            "properties": {
                "foo": {"$ref": "#/definitions/foo"}
            },
            "definitions": {
                "foo": {"type": "string"}
            }
        }),
        &json!({"foo": 42}),
        "/properties/foo/$ref/type"
    )]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        // For $ref tests, check tracker (includes $ref traversals)
        tests_util::assert_evaluation_path(schema, instance, expected);
    }

    #[test]
    fn multiple_errors_locations() {
        let instance = json!({
            "things": [
                { "code": "CC" },
                { "code": "CC" },
            ]
        });
        let schema = json!({
                "type": "object",
                "properties": {
                    "things": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "code": {
                                    "type": "string",
                                    "$ref": "#/$defs/codes"
                                }
                            },
                            "required": ["code"]
                        }
                    }
                },
                "required": ["things"],
                "$defs": { "codes": { "enum": ["AA", "BB"] } }
        });
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        let mut iter = validator.iter_errors(&instance);
        // tracker includes $ref traversals
        let expected = "/properties/things/items/properties/code/$ref/enum";
        assert_eq!(
            iter.next()
                .expect("Should be present")
                .evaluation_path()
                .to_string(),
            expected
        );
        assert_eq!(
            iter.next()
                .expect("Should be present")
                .evaluation_path()
                .to_string(),
            expected
        );
    }

    #[test]
    fn test_relative_base_uri() {
        let schema = json!({
            "$id": "/root",
            "$ref": "#/foo",
            "foo": {
                "$id": "#/foo",
                "$ref": "#/bar"
            },
            "bar": {
                "$id": "#/bar",
                "type": "integer"
            },
        });
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        assert!(validator.is_valid(&json!(2)));
        assert!(!validator.is_valid(&json!("a")));
    }

    #[test_case(
        &json!({
            "$id": "https://example.com/schema.json",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "foo": {
                    "type": "array",
                    "items": { "$ref": "#/$defs/item" }
                }
            },
            "$defs": {
                "item": {
                    "type": "object",
                    "required": ["name", "value"],
                    "properties": {
                        "name": { "type": "string" },
                        "value": { "type": "boolean" }
                    }
                }
            }
        }),
        &json!({
            "foo": [{"name": "item1", "value": true}]
        }),
        vec![
            ("", "/properties"),
            ("/foo", "/properties/foo/items"),
            // schemaLocation is the canonical location WITHOUT $ref (per JSON Schema spec)
            // The $ref resolves to $defs/item, so properties keyword is at /$defs/item/properties
            ("/foo/0", "/$defs/item/properties"),
        ]
    ; "standard $ref")]
    #[test_case(
        &json!({
            "$id": "https://example.com/schema.json",
            "$schema": "https://json-schema.org/draft/2019-09/schema",
            "$recursiveAnchor": true,
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "child": { "$recursiveRef": "#" }
            }
        }),
        &json!({
            "name": "parent",
            "child": {
                "name": "child",
                "child": { "name": "grandchild" }
            }
        }),
        vec![
            ("", "/properties"),
            // schemaLocation is the canonical location WITHOUT $recursiveRef (per JSON Schema spec)
            // $recursiveRef resolves to root (where $recursiveAnchor is), so properties is at /properties
            ("/child", "/properties"),
            // Same for nested - still resolves to root's /properties
            ("/child/child", "/properties"),
        ]
    ; "$recursiveRef")]
    fn keyword_locations(schema: &Value, instance: &Value, expected: Vec<(&str, &str)>) {
        let validator = crate::validator_for(schema).expect("Invalid schema");
        for (pointer, keyword_location) in expected {
            tests_util::assert_keyword_location(&validator, instance, pointer, keyword_location);
        }
    }

    // A reference node reports the location of its target, in the target's own resource.
    #[test_case(
        &json!({
            "$id": "https://example.com/root",
            "properties": {"a": {"$ref": "https://example.com/leaf"}},
            "$defs": {"leaf": {"$id": "https://example.com/leaf", "type": "string"}}
        }),
        &json!({"a": 1}),
        &[
            ("", "https://example.com/root#"),
            ("/properties", "https://example.com/root#/properties"),
            ("/properties/a", "https://example.com/root#/properties/a"),
            ("/properties/a/$ref", "https://example.com/leaf#"),
            ("/properties/a/$ref/type", "https://example.com/leaf#/type"),
        ]
    ; "$ref to another resource")]
    #[test_case(
        &json!({
            "properties": {"a": {"$ref": "https://example.com/leaf"}},
            "$defs": {"leaf": {"$id": "https://example.com/leaf", "type": "string"}}
        }),
        &json!({"a": 1}),
        &[
            ("", ""),
            ("/properties", "/properties"),
            ("/properties/a", "/properties/a"),
            ("/properties/a/$ref", "https://example.com/leaf#"),
            ("/properties/a/$ref/type", "https://example.com/leaf#/type"),
        ]
    ; "$ref to another resource from a root without $id")]
    #[test_case(
        &json!({
            "$id": "https://example.com/root",
            "properties": {"a": {"$ref": "https://example.com/leaf#/$defs/inner"}},
            "$defs": {
                "leaf": {
                    "$id": "https://example.com/leaf",
                    "$defs": {"inner": {"type": "string"}}
                }
            }
        }),
        &json!({"a": 1}),
        &[
            ("", "https://example.com/root#"),
            ("/properties", "https://example.com/root#/properties"),
            ("/properties/a", "https://example.com/root#/properties/a"),
            ("/properties/a/$ref", "https://example.com/leaf#/$defs/inner"),
            ("/properties/a/$ref/type", "https://example.com/leaf#/$defs/inner/type"),
        ]
    ; "$ref to a pointer in another resource")]
    #[test_case(
        &json!({
            "$id": "https://example.com/root",
            "properties": {"a": {"$ref": "https://example.com/leaf#string"}},
            "$defs": {
                "leaf": {
                    "$id": "https://example.com/leaf",
                    "$defs": {"inner": {"$anchor": "string", "type": "string"}}
                }
            }
        }),
        &json!({"a": 1}),
        &[
            ("", "https://example.com/root#"),
            ("/properties", "https://example.com/root#/properties"),
            ("/properties/a", "https://example.com/root#/properties/a"),
            ("/properties/a/$ref", "https://example.com/leaf#/$defs/inner"),
            ("/properties/a/$ref/type", "https://example.com/leaf#/$defs/inner/type"),
        ]
    ; "$ref to an anchor in another resource")]
    #[test_case(
        &json!({
            "$id": "https://example.com/root",
            "properties": {"a": {"$ref": "#/$defs/leaf"}},
            "$defs": {"leaf": {"type": "string"}}
        }),
        &json!({"a": 1}),
        &[
            ("", "https://example.com/root#"),
            ("/properties", "https://example.com/root#/properties"),
            ("/properties/a", "https://example.com/root#/properties/a"),
            ("/properties/a/$ref", "https://example.com/root#/$defs/leaf"),
            ("/properties/a/$ref/type", "https://example.com/root#/$defs/leaf/type"),
        ]
    ; "$ref within the same resource")]
    #[test_case(
        &json!({
            "$id": "https://example.com/root",
            "properties": {"a": {"$ref": "#/$defs/f"}},
            "$defs": {"f": false}
        }),
        &json!({"a": 1}),
        &[
            ("", "https://example.com/root#"),
            ("/properties", "https://example.com/root#/properties"),
            ("/properties/a", "https://example.com/root#/properties/a"),
            ("/properties/a/$ref", "https://example.com/root#/$defs/f"),
        ]
    ; "$ref to a boolean schema")]
    #[test_case(
        &json!({
            "$id": "https://example.com/root",
            "properties": {"a": {"$ref": "https://example.com/leaf#/$defs/f"}},
            "$defs": {"leaf": {"$id": "https://example.com/leaf", "$defs": {"f": false}}}
        }),
        &json!({"a": 1}),
        &[
            ("", "https://example.com/root#"),
            ("/properties", "https://example.com/root#/properties"),
            ("/properties/a", "https://example.com/root#/properties/a"),
            ("/properties/a/$ref", "https://example.com/leaf#/$defs/f"),
        ]
    ; "$ref to a boolean schema in another resource")]
    #[test_case(
        &json!({
            "$id": "https://example.com/root",
            "properties": {"a": {"$dynamicRef": "https://example.com/leaf#node"}},
            "$defs": {
                "leaf": {
                    "$id": "https://example.com/leaf",
                    "$dynamicAnchor": "node",
                    "type": "string"
                }
            }
        }),
        &json!({"a": 1}),
        &[
            ("", "https://example.com/root#"),
            ("/properties", "https://example.com/root#/properties"),
            ("/properties/a", "https://example.com/root#/properties/a"),
            ("/properties/a/$dynamicRef", "https://example.com/leaf#"),
            ("/properties/a/$dynamicRef/type", "https://example.com/leaf#/type"),
        ]
    ; "$dynamicRef to another resource")]
    #[test_case(
        &json!({
            "$schema": "https://json-schema.org/draft/2019-09/schema",
            "$id": "https://example.com/root",
            "$recursiveAnchor": true,
            "type": "object",
            "properties": {"a": {"$ref": "https://example.com/tree"}},
            "$defs": {
                "tree": {
                    "$id": "https://example.com/tree",
                    "$recursiveAnchor": true,
                    "properties": {"child": {"$recursiveRef": "#"}}
                }
            }
        }),
        &json!({"a": {"child": 1}}),
        &[
            ("", "https://example.com/root#"),
            ("/type", "https://example.com/root#/type"),
            ("/properties", "https://example.com/root#/properties"),
            ("/properties/a", "https://example.com/root#/properties/a"),
            ("/properties/a/$ref", "https://example.com/tree#"),
            ("/properties/a/$ref/properties", "https://example.com/tree#/properties"),
            ("/properties/a/$ref/properties/child", "https://example.com/tree#/properties/child"),
            ("/properties/a/$ref/properties/child/$recursiveRef", "https://example.com/root#"),
            ("/properties/a/$ref/properties/child/$recursiveRef/type", "https://example.com/root#/type"),
            ("/properties/a/$ref/properties/child/$recursiveRef/properties", "https://example.com/root#/properties"),
        ]
    ; "$recursiveRef to another resource")]
    fn reference_node_schema_location(schema: &Value, instance: &Value, expected: &[(&str, &str)]) {
        let validator = crate::validator_for(schema).expect("Invalid schema");
        let output = serde_json::to_value(validator.evaluate(instance).list())
            .expect("Output should serialize");
        let locations: Vec<(&str, &str)> = output["details"]
            .as_array()
            .expect("List output has details")
            .iter()
            .map(|node| {
                (
                    node["evaluationPath"].as_str().expect("Evaluation path"),
                    node["schemaLocation"].as_str().expect("Schema location"),
                )
            })
            .collect();
        assert_eq!(locations, expected);
    }

    // A JSON Pointer that enters a subschema with its own `$id` counts from that subschema.
    #[test_case(
        &json!({
            "$id": "https://example.com/root",
            "properties": {"a": {"$ref": "#/$defs/leaf/properties/x"}},
            "$defs": {
                "leaf": {
                    "$id": "https://example.com/leaf",
                    "properties": {"x": {"type": "string"}}
                }
            }
        }),
        &[],
        &[
            ("", "https://example.com/root#"),
            ("/properties", "https://example.com/root#/properties"),
            ("/properties/a", "https://example.com/root#/properties/a"),
            ("/properties/a/$ref", "https://example.com/leaf#/properties/x"),
            ("/properties/a/$ref/type", "https://example.com/leaf#/properties/x/type"),
        ],
        &["https://example.com/leaf#/properties/x/type"]
    ; "pointer below an embedded resource")]
    #[test_case(
        &json!({
            "$id": "https://example.com/root",
            "properties": {"a": {"$ref": "#/$defs/leaf"}},
            "$defs": {"leaf": {"$id": "https://example.com/leaf", "type": "string"}}
        }),
        &[],
        &[
            ("", "https://example.com/root#"),
            ("/properties", "https://example.com/root#/properties"),
            ("/properties/a", "https://example.com/root#/properties/a"),
            ("/properties/a/$ref", "https://example.com/leaf#"),
            ("/properties/a/$ref/type", "https://example.com/leaf#/type"),
        ],
        &["https://example.com/leaf#/type"]
    ; "pointer to an embedded resource")]
    #[test_case(
        &json!({
            "properties": {"a": {"$ref": "#/$defs/leaf/properties/x"}},
            "$defs": {
                "leaf": {
                    "$id": "https://example.com/leaf",
                    "properties": {"x": {"type": "string"}}
                }
            }
        }),
        &[],
        &[
            ("", ""),
            ("/properties", "/properties"),
            ("/properties/a", "/properties/a"),
            ("/properties/a/$ref", "https://example.com/leaf#/properties/x"),
            ("/properties/a/$ref/type", "https://example.com/leaf#/properties/x/type"),
        ],
        &["https://example.com/leaf#/properties/x/type"]
    ; "pointer below an embedded resource from a root without $id")]
    #[test_case(
        &json!({
            "$id": "https://example.com/root",
            "properties": {"a": {"$ref": "#/$defs/leaf/$defs/inner/properties/x"}},
            "$defs": {
                "leaf": {
                    "$id": "https://example.com/leaf",
                    "$defs": {
                        "inner": {
                            "$id": "https://example.com/inner",
                            "properties": {"x": {"type": "string"}}
                        }
                    }
                }
            }
        }),
        &[],
        &[
            ("", "https://example.com/root#"),
            ("/properties", "https://example.com/root#/properties"),
            ("/properties/a", "https://example.com/root#/properties/a"),
            ("/properties/a/$ref", "https://example.com/inner#/properties/x"),
            ("/properties/a/$ref/type", "https://example.com/inner#/properties/x/type"),
        ],
        &["https://example.com/inner#/properties/x/type"]
    ; "pointer below two embedded resources")]
    #[test_case(
        &json!({"$ref": "https://example.com/ext#/$defs/n"}),
        &[(
            "https://example.com/ext",
            json!({
                "$id": "https://example.com/ext",
                "$defs": {"n": {"$id": "https://example.com/inner", "type": "string"}}
            }),
        )],
        &[
            ("", ""),
            ("/$ref", "https://example.com/inner#"),
            ("/$ref/type", "https://example.com/inner#/type"),
        ],
        &["https://example.com/inner#/type"]
    ; "pointer into a registry resource to an embedded resource")]
    #[test_case(
        &json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "$id": "https://example.com/root",
            "properties": {"a": {"$ref": "#/definitions/leaf/properties/x"}},
            "definitions": {
                "leaf": {
                    "$id": "https://example.com/leaf",
                    "properties": {"x": {"type": "string"}}
                }
            }
        }),
        &[],
        &[
            ("", "https://example.com/root#"),
            ("/properties", "https://example.com/root#/properties"),
            ("/properties/a", "https://example.com/root#/properties/a"),
            ("/properties/a/$ref", "https://example.com/leaf#/properties/x"),
            ("/properties/a/$ref/type", "https://example.com/leaf#/properties/x/type"),
        ],
        &["https://example.com/leaf#/properties/x/type"]
    ; "Draft 7 pointer below an embedded resource")]
    #[test_case(
        &json!({
            "$schema": "https://json-schema.org/draft/2019-09/schema",
            "$id": "https://example.com/root",
            "properties": {"a": {"$ref": "#/$defs/leaf/properties/x"}},
            "$defs": {
                "leaf": {
                    "$id": "https://example.com/leaf",
                    "properties": {"x": {"type": "string"}}
                }
            }
        }),
        &[],
        &[
            ("", "https://example.com/root#"),
            ("/properties", "https://example.com/root#/properties"),
            ("/properties/a", "https://example.com/root#/properties/a"),
            ("/properties/a/$ref", "https://example.com/leaf#/properties/x"),
            ("/properties/a/$ref/type", "https://example.com/leaf#/properties/x/type"),
        ],
        &["https://example.com/leaf#/properties/x/type"]
    ; "Draft 2019-09 pointer below an embedded resource")]
    fn pointer_through_embedded_resource(
        schema: &Value,
        resources: &[(&str, Value)],
        expected_evaluation: &[(&str, &str)],
        expected_errors: &[&str],
    ) {
        let mut registry = crate::Registry::new();
        for (uri, resource) in resources {
            registry = registry.add(*uri, resource).expect("Valid resource");
        }
        let registry = registry.prepare().expect("Registry should prepare");
        let validator = crate::options()
            .with_registry(&registry)
            .build(schema)
            .expect("Invalid schema");
        let instance = json!({"a": 1});
        let output = serde_json::to_value(validator.evaluate(&instance).list())
            .expect("Output should serialize");
        let evaluation: Vec<(&str, &str)> = output["details"]
            .as_array()
            .expect("List output has details")
            .iter()
            .map(|node| {
                (
                    node["evaluationPath"].as_str().expect("Evaluation path"),
                    node["schemaLocation"].as_str().expect("Schema location"),
                )
            })
            .collect();
        let errors: Vec<String> = validator
            .iter_errors(&instance)
            .map(|error| {
                error
                    .absolute_keyword_location()
                    .expect("Absolute keyword location")
                    .to_string()
            })
            .collect();
        assert_eq!(
            (evaluation, errors),
            (
                expected_evaluation.to_vec(),
                expected_errors.iter().map(ToString::to_string).collect()
            )
        );
    }

    // Annotations collected at a reference node carry the target's location.
    #[test_case(
        &json!({
            "$id": "https://example.com/root",
            "properties": {"a": {"$ref": "https://example.com/leaf"}},
            "$defs": {"leaf": {"$id": "https://example.com/leaf", "x-note": 1}}
        }),
        ("https://example.com/leaf#", Some("https://example.com/leaf#"))
    ; "another resource")]
    #[test_case(
        &json!({
            "properties": {"a": {"$ref": "https://example.com/leaf"}},
            "$defs": {"leaf": {"$id": "https://example.com/leaf", "x-note": 1}}
        }),
        ("https://example.com/leaf#", Some("https://example.com/leaf#"))
    ; "another resource from a root without $id")]
    #[test_case(
        &json!({
            "$id": "https://example.com/root",
            "properties": {"a": {"$ref": "#/$defs/leaf"}},
            "$defs": {"leaf": {"x-note": 1}}
        }),
        ("https://example.com/root#/$defs/leaf", Some("https://example.com/root#/$defs/leaf"))
    ; "same resource")]
    #[test_case(
        &json!({
            "properties": {"a": {"$ref": "#/$defs/leaf"}},
            "$defs": {"leaf": {"x-note": 1}}
        }),
        ("/$defs/leaf", None)
    ; "same resource without $id")]
    fn reference_node_annotation_location(schema: &Value, expected: (&str, Option<&str>)) {
        let validator = crate::validator_for(schema).expect("Invalid schema");
        let evaluation = validator.evaluate(&json!({"a": "x"}));
        let locations: Vec<(&str, Option<&str>)> = evaluation
            .iter_annotations()
            .filter(|entry| entry.annotations.value() == &json!({"x-note": 1}))
            .map(|entry| {
                (
                    entry.schema_location,
                    entry.absolute_keyword_location.map(Uri::as_str),
                )
            })
            .collect();
        assert_eq!(locations, vec![expected]);
    }

    #[test]
    fn test_resolving_finds_references_in_referenced_resources() {
        let schema = json!({"$ref": "/indirection#/baz"});

        let validator = crate::options()
            .with_retriever(MyRetrieve)
            .build(&schema)
            .expect("Failed to build validator");

        assert!(validator.is_valid(&json!(2)));
        assert!(!validator.is_valid(&json!("")));
    }

    #[test_case(
        &json!({"$ref": "/doc#/definitions/foo"}),
        &json!({
            "$id": "/doc",
            "definitions": {
                "foo": {"type": "integer"}
            }
        }),
        None
        ; "basic_fragment"
    )]
    #[test_case(
        &json!({"$ref": "/doc1#/definitions/foo"}),
        &json!({
            "$id": "/doc1",
            "definitions": {
                "foo": {"$ref": "#/definitions/bar"},
                "bar": {"type": "integer"}
            }
        }),
        None
        ; "intermediate_reference"
    )]
    #[test_case(
        &json!({"$ref": "/doc2#/refs/first"}),
        &json!({
            "$id": "/doc2",
            "refs": {
                "first": {"$ref": "/doc3#/refs/second"}
            }
        }),
        Some(&json!({
            "/doc3": {
                "$id": "/doc3",
                "refs": {
                    "second": {"type": "integer"}
                }
            }
        }))
        ; "multiple_documents"
    )]
    #[test_case(
        &json!({"$ref": "/doc4#/defs/foo"}),
        &json!({
            "$id": "/doc4",
            "defs": {
                "foo": {
                    "$id": "#/defs/foo",
                    "$ref": "#/defs/bar"
                },
                "bar": {"type": "integer"}
            }
        }),
        None
        ; "id_and_fragment"
    )]
    #[test_case(
        &json!({"$ref": "/doc5#/outer"}),
        &json!({
            "$id": "/doc5",
            "outer": {
                "$ref": "#/middle",
            },
            "middle": {
                "$id": "#/middle",
                "$ref": "#/inner"
            },
            "inner": {"type": "integer"}
        }),
        None
        ; "nested_references"
    )]
    fn test_fragment_resolution(schema: &Value, root: &Value, extra: Option<&Value>) {
        let mut storage = HashMap::default();

        let doc_path = schema["$ref"]
            .as_str()
            .and_then(|r| r.split('#').next())
            .expect("Invalid $ref");

        storage.insert(doc_path.to_string(), root.clone());

        if let Some(extra) = extra {
            for (path, document) in extra.as_object().unwrap() {
                storage.insert(path.clone(), document.clone());
            }
        }

        let retriever = TestRetrieve { storage };

        let validator = crate::options()
            .with_retriever(retriever)
            .build(schema)
            .expect("Invalid schema");

        assert!(validator.is_valid(&json!(42)));
        assert!(!validator.is_valid(&json!("string")));
    }

    #[test]
    fn test_infinite_loop() {
        let validator = crate::validator_for(&json!({"$ref": "#"})).expect("Invalid schema");
        assert!(validator.is_valid(&json!(42)));
    }

    #[test]
    fn test_nested_external_reference() {
        let schema = json!({
            "$id": "foo://schema_1.json",
            "$ref": "#/$defs/a/b",
            "$defs": {
                "a": {
                    "b": {
                        "description": "nested schema with external ref",
                        "$ref": "foo://schema_2.json"
                    }
                }
            }
        });

        let validator = crate::options()
            .with_retriever(NestedRetrieve)
            .build(&schema)
            .expect("Failed to build validator");

        assert!(validator.is_valid(&json!("test")));
        assert!(!validator.is_valid(&json!(42)));
    }

    #[test]
    fn test_relative_reference_with_fragment() {
        let schema = json!({
            "$id": "file:///tmp/schemas/root.json",
            "$ref": "one.json#/$defs/obj"
        });

        let validator = crate::options()
            .with_retriever(FragmentRetrieve)
            .build(&schema)
            .expect("Failed to build validator");

        assert!(validator.is_valid(&json!(42)));
        assert!(!validator.is_valid(&json!("string")));
    }

    #[test]
    fn test_missing_file() {
        let schema = json!({"$ref": "./virtualNetwork.json"});
        let error = crate::validator_for(&schema).expect_err("Should fail");
        assert_eq!(
            error.to_string(),
            "Resource './virtualNetwork.json' is not present in a registry and retrieving it failed: No base URI is available"
        );
    }

    #[test]
    fn test_empty_ref_no_stack_overflow() {
        // Empty string is a same-document reference per RFC 3986, should behave like $ref: "#"
        let schema = json!({"$ref": ""});
        let instance = json!(-1);

        // Should compile without error and validate without stack overflow
        let validator = crate::validator_for(&schema).expect("Should compile");
        assert!(validator.is_valid(&instance));
    }

    struct IndirectExternalRetrieve;

    impl Retrieve for IndirectExternalRetrieve {
        fn retrieve(
            &self,
            uri: &Uri<String>,
        ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
            match uri.as_str() {
                "file:///ext.yaml" => Ok(json!({
                    "components": {
                        "schemas": {
                            "c3": {
                                "type": "integer"
                            }
                        }
                    }
                })),
                _ => Err(format!("Unexpected URI: {uri}").into()),
            }
        }
    }

    #[test]
    fn test_indirect_local_refs_to_external_resource() {
        // GH-892: Chained local $refs where the final ref points to an external resource
        // should properly discover and retrieve the external resource.
        //
        // The chain is:
        //   root $ref -> #/components/schemas/c1
        //   c1 $ref   -> #/components/schemas/c2
        //   c2 $ref   -> ext.yaml#/components/schemas/c3  (EXTERNAL)
        let schema = json!({
            "$id": "file:///tmp",
            "$ref": "#/components/schemas/c1",
            "components": {
                "schemas": {
                    "c1": {
                        "$ref": "#/components/schemas/c2"
                    },
                    "c2": {
                        "$ref": "ext.yaml#/components/schemas/c3"
                    }
                }
            }
        });

        let validator = crate::options()
            .with_retriever(IndirectExternalRetrieve)
            .build(&schema)
            .expect("Failed to build validator - external resource was not discovered");

        assert!(validator.is_valid(&json!(42)));
        assert!(!validator.is_valid(&json!("string")));
    }

    #[test]
    fn test_local_ref_with_nested_external_ref_in_properties() {
        // GH-892 follow-up: Local $ref points to a schema that has an external $ref
        // nested within properties (not a direct $ref chain).
        //
        // The structure is:
        //   root $ref -> #/components/schemas/c1
        //   c1 is a full schema with type/properties
        //   c1.properties.p contains an external $ref
        let schema = json!({
            "$id": "file:///tmp",
            "$ref": "#/components/schemas/c1",
            "components": {
                "schemas": {
                    "c1": {
                        "type": "object",
                        "properties": {
                            "p": {
                                "$ref": "ext.yaml#/components/schemas/c3"
                            }
                        }
                    }
                }
            }
        });

        let validator = crate::options()
            .with_retriever(IndirectExternalRetrieve)
            .build(&schema)
            .expect("Failed to build validator - external resource was not discovered");

        assert!(validator.is_valid(&json!({"p": 42})));
        assert!(!validator.is_valid(&json!({"p": "string"})));
    }

    struct CrossFileRetrieve;

    impl Retrieve for CrossFileRetrieve {
        fn retrieve(
            &self,
            uri: &Uri<String>,
        ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
            match uri.as_str() {
                "file:///tmp/json" => Ok(json!({
                    "components": {
                        "schemas": {
                            "c1": {
                                "type": "array",
                                "items": {
                                    "$ref": "#/components/schemas/c2"
                                }
                            },
                            "c2": {
                                "$ref": "ext.json#/components/schemas/c3"
                            }
                        }
                    }
                })),
                "file:///tmp/ext.json" => Ok(json!({
                    "components": {
                        "schemas": {
                            "c3": {
                                "type": "integer"
                            }
                        }
                    }
                })),
                _ => Err(format!("Unexpected URI: {uri}").into()),
            }
        }
    }

    #[test]
    fn test_cross_file_local_ref_resolution() {
        // GH-892: External ref with fragment pointing to a schema that has local refs.
        // The local refs within the external file need to resolve against that file's
        // document root, not the original schema's root.
        //
        // Structure:
        //   root $ref -> file:///tmp/json#/components/schemas/c1
        //   /tmp/json has:
        //     c1.items.$ref -> #/components/schemas/c2 (local ref within /tmp/json)
        //     c2.$ref -> ext.json#/components/schemas/c3 (external ref)
        let schema = json!({
            "$ref": "file:///tmp/json#/components/schemas/c1"
        });

        let validator = crate::options()
            .with_retriever(CrossFileRetrieve)
            .build(&schema)
            .expect("Failed to build validator - external resource was not discovered");

        assert!(validator.is_valid(&json!([1, 2, 3])));
        assert!(!validator.is_valid(&json!(["a", "b"])));
    }

    #[test]
    fn test_circular_local_refs_compile() {
        let schema = json!({
            "$defs": {
                "a": {"$ref": "#/$defs/b"},
                "b": {"$ref": "#/$defs/a"}
            },
            "$ref": "#/$defs/a"
        });
        let validator = crate::validator_for(&schema).expect("Should compile");

        // A pure $ref cycle is equivalent to `true` schema
        for instance in [
            json!(42),
            json!("string"),
            json!(null),
            json!({"nested": [1, 2, 3]}),
        ] {
            assert!(validator.is_valid(&instance));
            assert!(validator.validate(&instance).is_ok());
            assert_eq!(validator.iter_errors(&instance).count(), 0);
            assert!(validator.evaluate(&instance).flag().valid);
        }
    }

    #[test]
    fn test_circular_refs_with_constraints() {
        let schema = json!({
            "$defs": {
                "node": {
                    "type": "object",
                    "properties": {
                        "value": {"type": "integer"},
                        "next": {"$ref": "#/$defs/node"}
                    }
                }
            },
            "$ref": "#/$defs/node"
        });
        let validator = crate::validator_for(&schema).expect("Should compile");

        let valid = json!({"value": 1, "next": {"value": 2, "next": {"value": 3}}});
        assert!(validator.is_valid(&valid));
        assert!(validator.validate(&valid).is_ok());
        assert_eq!(validator.iter_errors(&valid).count(), 0);

        let invalid = json!({"value": "not an int"});
        assert!(!validator.is_valid(&invalid));
        assert!(validator.validate(&invalid).is_err());
        assert!(validator.iter_errors(&invalid).count() > 0);

        let invalid_nested = json!({"value": 1, "next": {"value": "bad"}});
        assert!(!validator.is_valid(&invalid_nested));
        assert!(validator.validate(&invalid_nested).is_err());
        assert!(validator.iter_errors(&invalid_nested).count() > 0);
    }

    #[test]
    fn test_longer_circular_chain() {
        let schema = json!({
            "$defs": {
                "a": {"$ref": "#/$defs/b"},
                "b": {"$ref": "#/$defs/c"},
                "c": {"$ref": "#/$defs/a"}
            },
            "$ref": "#/$defs/a"
        });
        let validator = crate::validator_for(&schema).expect("Should compile");

        let instance = json!({"any": "value"});
        assert!(validator.is_valid(&instance));
        assert!(validator.validate(&instance).is_ok());
        assert_eq!(validator.iter_errors(&instance).count(), 0);
        assert!(validator.evaluate(&instance).flag().valid);
    }

    #[test]
    fn test_dependencies_with_array_form() {
        // Tests NodeValidators::Array branch via dependencies with array form
        let schema = json!({
            "dependencies": {
                "foo": ["bar", "baz"]
            }
        });
        let validator = crate::validator_for(&schema).expect("Should compile");

        let valid = json!({"foo": 1, "bar": 2, "baz": 3});
        assert!(validator.is_valid(&valid));
        assert!(validator.validate(&valid).is_ok());
        assert_eq!(validator.iter_errors(&valid).count(), 0);
        assert!(validator.evaluate(&valid).flag().valid);

        let invalid = json!({"foo": 1});
        assert!(!validator.is_valid(&invalid));
        assert!(validator.validate(&invalid).is_err());
        assert!(validator.iter_errors(&invalid).count() > 0);
        assert!(!validator.evaluate(&invalid).flag().valid);
    }

    #[test]
    fn test_dependent_required_array_form() {
        // Tests NodeValidators::Array branch via dependentRequired (Draft 2019-09+)
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2019-09/schema",
            "dependentRequired": {
                "foo": ["bar"]
            }
        });
        let validator = crate::validator_for(&schema).expect("Should compile");

        let valid = json!({"foo": 1, "bar": 2});
        assert!(validator.is_valid(&valid));
        assert!(validator.validate(&valid).is_ok());
        assert_eq!(validator.iter_errors(&valid).count(), 0);
        assert!(validator.evaluate(&valid).flag().valid);

        let invalid = json!({"foo": 1});
        assert!(!validator.is_valid(&invalid));
        assert!(validator.validate(&invalid).is_err());
        assert!(validator.iter_errors(&invalid).count() > 0);
    }

    #[test]
    fn evaluation_path_through_ref() {
        // Test that tracker correctly includes $ref traversals
        let schema = json!({
            "properties": {
                "foo": {"$ref": "#/$defs/item"}
            },
            "$defs": {
                "item": {"type": "string"}
            }
        });
        let instance = json!({"foo": 42});
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        let error = validator.validate(&instance).expect_err("Should fail");

        // schema_path is the canonical location (where the keyword actually is)
        assert_eq!(error.schema_path().as_str(), "/$defs/item/type");

        // tracker includes the $ref traversal
        assert_eq!(
            error.evaluation_path().as_str(),
            "/properties/foo/$ref/type"
        );
    }

    #[test]
    fn evaluation_path_nested_refs() {
        // Test nested $ref traversals
        let schema = json!({
            "$ref": "#/$defs/wrapper",
            "$defs": {
                "wrapper": {
                    "properties": {
                        "value": {"$ref": "#/$defs/item"}
                    }
                },
                "item": {"type": "integer"}
            }
        });
        let instance = json!({"value": "not an integer"});
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        let error = validator.validate(&instance).expect_err("Should fail");

        // schema_path is canonical
        assert_eq!(error.schema_path().as_str(), "/$defs/item/type");

        // tracker shows full traversal through both $refs
        assert_eq!(
            error.evaluation_path().as_str(),
            "/$ref/properties/value/$ref/type"
        );
    }

    #[test]
    fn evaluation_path_recursive_ref() {
        // $recursiveRef should appear in evaluation path
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2019-09/schema",
            "$recursiveAnchor": true,
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "child": {"$recursiveRef": "#"}
            }
        });
        let instance = json!({
            "name": "parent",
            "child": {
                "name": 42
            }
        });
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        let error = validator.validate(&instance).expect_err("Should fail");

        // schema_path is canonical (at root, since $recursiveRef resolves to root)
        assert_eq!(error.schema_path().as_str(), "/properties/name/type");

        // tracker includes the $recursiveRef traversal
        assert_eq!(
            error.evaluation_path().as_str(),
            "/properties/child/$recursiveRef/properties/name/type"
        );
    }

    #[test]
    fn evaluation_path_recursive_ref_deep() {
        // Multiple levels of $recursiveRef
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2019-09/schema",
            "$recursiveAnchor": true,
            "type": "object",
            "properties": {
                "value": {"type": "integer"},
                "child": {"$recursiveRef": "#"}
            }
        });
        let instance = json!({
            "value": 1,
            "child": {
                "value": 2,
                "child": {
                    "value": "not an int"
                }
            }
        });
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        let error = validator.validate(&instance).expect_err("Should fail");

        // schema_path is canonical (at root, since $recursiveRef resolves to root)
        assert_eq!(error.schema_path().as_str(), "/properties/value/type");

        // tracker shows the full traversal through both $recursiveRef
        assert_eq!(
            error.evaluation_path().as_str(),
            "/properties/child/$recursiveRef/properties/child/$recursiveRef/properties/value/type"
        );
    }

    #[test]
    fn evaluation_path_dynamic_ref() {
        // $dynamicRef should appear in evaluation path but NOT in schema_path
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$dynamicAnchor": "node",
            "type": "object",
            "properties": {
                "data": {"type": "string"},
                "child": {"$dynamicRef": "#node"}
            }
        });
        let instance = json!({
            "data": "parent",
            "child": {
                "data": 123
            }
        });
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        let error = validator.validate(&instance).expect_err("Should fail");

        // schema_path is the canonical location (at root, since #node anchor is at root)
        assert_eq!(error.schema_path().as_str(), "/properties/data/type");

        // tracker includes the $dynamicRef traversal
        assert_eq!(
            error.evaluation_path().as_str(),
            "/properties/child/$dynamicRef/properties/data/type"
        );
    }

    #[test_case(
        &json!({"child": {"v": 1}}),
        None;
        "target relative ref accepts integer"
    )]
    #[test_case(
        &json!({"child": {"v": "x"}}),
        Some((
            "\"x\" is not of type \"integer\"",
            "/$defs/leaf/properties/v/type",
            "https://example.com/root.json#/$defs/leaf/properties/v/type",
        ));
        "target relative ref rejects string"
    )]
    #[test_case(
        &json!({"child": "str"}),
        None;
        "target permits non-object"
    )]
    fn cross_document_dynamic_ref_uses_target_base(
        instance: &Value,
        expected_error: Option<(&str, &str, &str)>,
    ) {
        let tree = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/tree.json",
            "$dynamicAnchor": "node",
            "type": "object",
            "properties": {"child": {"$dynamicRef": "#node"}},
            "$defs": {"leaf": {"type": "string"}}
        });
        let root = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/root.json",
            "$ref": "tree.json",
            "$defs": {
                "node": {"$dynamicAnchor": "node", "$ref": "#/$defs/leaf"},
                "leaf": {"properties": {"v": {"type": "integer"}}}
            }
        });
        let registry = crate::Registry::new()
            .add("https://example.com/tree.json", &tree)
            .expect("Invalid resource")
            .prepare()
            .expect("Invalid registry");
        let validator = crate::options()
            .with_registry(&registry)
            .build(&root)
            .expect("Invalid schema");

        assert_eq!(validator.is_valid(instance), expected_error.is_none());
        let result = validator.validate(instance);
        match expected_error {
            Some((message, schema_path, absolute_keyword_location)) => {
                let error = result.expect_err("Should fail");
                assert_eq!(error.to_string(), message);
                assert_eq!(error.schema_path().as_str(), schema_path);
                assert_eq!(
                    error
                        .absolute_keyword_location()
                        .expect("Absolute keyword location")
                        .as_str(),
                    absolute_keyword_location
                );
            }
            None => assert!(result.is_ok()),
        }
    }

    #[test]
    fn evaluation_path_triple_nested_ref() {
        // Three levels of $ref
        let schema = json!({
            "$ref": "#/$defs/level1",
            "$defs": {
                "level1": {
                    "$ref": "#/$defs/level2"
                },
                "level2": {
                    "$ref": "#/$defs/level3"
                },
                "level3": {
                    "type": "boolean"
                }
            }
        });
        let instance = json!("not a boolean");
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        let error = validator.validate(&instance).expect_err("Should fail");

        assert_eq!(error.schema_path().as_str(), "/$defs/level3/type");
        assert_eq!(error.evaluation_path().as_str(), "/$ref/$ref/$ref/type");
    }

    #[test]
    fn evaluation_path_ref_in_allof() {
        // $ref inside allOf
        let schema = json!({
            "allOf": [
                {"$ref": "#/$defs/stringType"},
                {"minLength": 5}
            ],
            "$defs": {
                "stringType": {"type": "string"}
            }
        });
        let instance = json!(42);
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        let error = validator.validate(&instance).expect_err("Should fail");

        assert_eq!(error.evaluation_path().as_str(), "/allOf/0/$ref/type");
    }

    #[test]
    fn evaluation_path_ref_in_anyof() {
        // $ref inside anyOf - all branches fail
        let schema = json!({
            "anyOf": [
                {"$ref": "#/$defs/intType"},
                {"$ref": "#/$defs/boolType"}
            ],
            "$defs": {
                "intType": {"type": "integer"},
                "boolType": {"type": "boolean"}
            }
        });
        let instance = json!("string");
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        let errors: Vec<_> = validator.iter_errors(&instance).collect();

        // anyOf produces a single error containing nested errors
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].evaluation_path().as_str(), "/anyOf");
    }

    #[test_case(
        "age", "positiveInt",
        json!({"type": "integer", "minimum": 0}),
        json!(-5),
        "minimum"
        ; "minimum"
    )]
    #[test_case(
        "email", "emailPattern",
        json!({"type": "string", "pattern": "^.+@.+$"}),
        json!("not-an-email"),
        "pattern"
        ; "pattern"
    )]
    #[test_case(
        "user", "userType",
        json!({"type": "object", "required": ["name"]}),
        json!({}),
        "required"
        ; "required"
    )]
    #[test_case(
        "status", "statusEnum",
        json!({"enum": ["active", "inactive"]}),
        json!("unknown"),
        "enum"
        ; "enum_keyword"
    )]
    #[test_case(
        "version", "versionConst",
        json!({"const": "1.0"}),
        json!("2.0"),
        "const"
        ; "const_keyword"
    )]
    #[test_case(
        "code", "shortString",
        json!({"type": "string", "maxLength": 3}),
        json!("toolong"),
        "maxLength"
        ; "maxLength"
    )]
    #[test_case(
        "tags", "uniqueArray",
        json!({"type": "array", "uniqueItems": true}),
        json!(["a", "b", "a"]),
        "uniqueItems"
        ; "uniqueItems"
    )]
    #[allow(clippy::needless_pass_by_value)]
    fn evaluation_path_ref_keyword(
        prop: &str,
        def_name: &str,
        definition: Value,
        instance_value: Value,
        expected_keyword: &str,
    ) {
        let schema = json!({
            "properties": {
                (prop): {"$ref": format!("#/$defs/{def_name}")}
            },
            "$defs": {
                (def_name): definition
            }
        });
        let instance = json!({ (prop): instance_value });
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        let error = validator.validate(&instance).expect_err("Should fail");

        assert_eq!(
            error.evaluation_path().as_str(),
            format!("/properties/{prop}/$ref/{expected_keyword}")
        );
    }

    #[test]
    fn evaluation_path_multiple_errors_different_refs() {
        // Multiple errors through different $refs
        let schema = json!({
            "properties": {
                "name": {"$ref": "#/$defs/stringType"},
                "age": {"$ref": "#/$defs/intType"}
            },
            "$defs": {
                "stringType": {"type": "string"},
                "intType": {"type": "integer"}
            }
        });
        let instance = json!({"name": 123, "age": "not an int"});
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        let errors: Vec<_> = validator.iter_errors(&instance).collect();

        assert_eq!(errors.len(), 2);

        let paths: Vec<_> = errors
            .iter()
            .map(|e| e.evaluation_path().to_string())
            .collect();

        assert!(paths.contains(&"/properties/name/$ref/type".to_string()));
        assert!(paths.contains(&"/properties/age/$ref/type".to_string()));
    }

    // A named target reports the same `schema_path` as a JSON Pointer to it
    #[test_case(
        &json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$defs": {"model": {"$anchor": "model", "properties": {"name": {"type": "string"}}}},
            "properties": {"named": {"$ref": "#model"}, "pointer": {"$ref": "#/$defs/model"}}
        }),
        "/$defs/model/properties/name/type";
        "draft2020 anchor"
    )]
    #[test_case(
        &json!({
            "$schema": "https://json-schema.org/draft/2019-09/schema",
            "$defs": {"model": {"$anchor": "model", "properties": {"name": {"type": "string"}}}},
            "properties": {"named": {"$ref": "#model"}, "pointer": {"$ref": "#/$defs/model"}}
        }),
        "/$defs/model/properties/name/type";
        "draft2019 anchor"
    )]
    #[test_case(
        &json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "definitions": {"model": {"$id": "#model", "properties": {"name": {"type": "string"}}}},
            "properties": {"named": {"$ref": "#model"}, "pointer": {"$ref": "#/definitions/model"}}
        }),
        "/definitions/model/properties/name/type";
        "draft7 fragment id"
    )]
    #[test_case(
        &json!({
            "$schema": "http://json-schema.org/draft-04/schema#",
            "definitions": {"model": {"id": "#model", "properties": {"name": {"type": "string"}}}},
            "properties": {"named": {"$ref": "#model"}, "pointer": {"$ref": "#/definitions/model"}}
        }),
        "/definitions/model/properties/name/type";
        "draft4 fragment id"
    )]
    #[test_case(
        &json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$defs": {"model": {"$dynamicAnchor": "model", "properties": {"name": {"type": "string"}}}},
            "properties": {"named": {"$dynamicRef": "#model"}, "pointer": {"$ref": "#/$defs/model"}}
        }),
        "/$defs/model/properties/name/type";
        "draft2020 dynamic anchor"
    )]
    #[test_case(
        &json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/root.json",
            "$defs": {
                "inner": {
                    "$id": "inner.json",
                    "$defs": {"model": {"$anchor": "model", "properties": {"name": {"type": "string"}}}}
                }
            },
            "properties": {"named": {"$ref": "inner.json#model"}, "pointer": {"$ref": "inner.json#/$defs/model"}}
        }),
        "/$defs/model/properties/name/type";
        "anchor in embedded resource"
    )]
    fn schema_path_of_named_target(schema: &Value, expected: &str) {
        let validator = crate::validator_for(schema).expect("Invalid schema");
        for property in ["named", "pointer"] {
            let instance = json!({property: {"name": 42}});
            let error = validator.validate(&instance).expect_err("Should fail");
            assert_eq!(error.schema_path().as_str(), expected, "{property}");
            let errors: Vec<_> = validator
                .iter_errors(&instance)
                .map(|error| error.schema_path().to_string())
                .collect();
            assert_eq!(errors, vec![expected.to_string()], "{property}");
        }
    }

    #[test]
    fn evaluation_path_ref_with_anchor() {
        // $ref using $anchor
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "properties": {
                "data": {"$ref": "#myAnchor"}
            },
            "$defs": {
                "myDef": {
                    "$anchor": "myAnchor",
                    "type": "number"
                }
            }
        });
        let instance = json!({"data": "not a number"});
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        let error = validator.validate(&instance).expect_err("Should fail");

        assert_eq!(
            error.evaluation_path().as_str(),
            "/properties/data/$ref/type"
        );
    }

    #[test]
    fn evaluation_path_items_with_ref() {
        // $ref inside items
        let schema = json!({
            "type": "array",
            "items": {"$ref": "#/$defs/itemType"},
            "$defs": {
                "itemType": {"type": "string"}
            }
        });
        let instance = json!([1, 2, 3]);
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        let errors: Vec<_> = validator.iter_errors(&instance).collect();

        assert_eq!(errors.len(), 3);
        for error in &errors {
            assert_eq!(error.evaluation_path().as_str(), "/items/$ref/type");
        }
    }

    #[test]
    fn evaluation_path_additional_properties_with_ref() {
        // additionalProperties with $ref
        let schema = json!({
            "type": "object",
            "additionalProperties": {"$ref": "#/$defs/valueType"},
            "$defs": {
                "valueType": {"type": "integer"}
            }
        });
        let instance = json!({"a": "not int", "b": "also not int"});
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        let errors: Vec<_> = validator.iter_errors(&instance).collect();

        assert_eq!(errors.len(), 2);
        for error in &errors {
            assert_eq!(
                error.evaluation_path().as_str(),
                "/additionalProperties/$ref/type"
            );
        }
    }

    #[test]
    fn schema_path_with_json_pointer_escaped_key() {
        // $defs key contains special chars that need JSON Pointer escaping
        let schema = json!({
            "properties": {
                "data": {"$ref": "#/$defs/type~1name"}
            },
            "$defs": {
                "type/name": {"type": "string"}
            }
        });
        let instance = json!({"data": 42});
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        let error = validator.validate(&instance).expect_err("Should fail");

        // schema_path should have the unescaped key (type/name), re-escaped properly
        assert_eq!(error.schema_path().as_str(), "/$defs/type~1name/type");
    }

    #[test]
    fn schema_path_with_url_encoded_key() {
        // $defs key contains characters that get percent-encoded in the URI fragment
        // (here: a literal space). The $ref value uses URL-encoded form ("%20")
        // because JSON Schema $ref values are URI-Reference per RFC 3986.
        // schema_path is JSON-Pointer-encoded (RFC 6901), so the space must be
        // percent-decoded before being stored as a Location segment.
        let schema = json!({
            "properties": {
                "data": {"$ref": "#/$defs/Request%20class"}
            },
            "$defs": {
                "Request class": {"type": "string"}
            }
        });
        let instance = json!({"data": 42});
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        let error = validator.validate(&instance).expect_err("Should fail");

        // JSON Pointer form holds the literal space ' ', not the URI-encoded "%20".
        assert_eq!(error.schema_path().as_str(), "/$defs/Request class/type",);
    }

    /// A fragment-only JSON Pointer `$ref` written inside an `$id`-bearing subresource must
    /// resolve within that subresource, even when its pointer coincides with the pointer that
    /// reached the subresource from the enclosing document.
    ///
    /// Cross-checked against `jsonschema` (Python) and `@hyperjump/json-schema`: both resolve
    /// these to the inner `integer` definition on every draft where the shape is expressible.
    #[test_case("https://json-schema.org/draft/2020-12/schema", "$defs" ; "draft 2020-12")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", "$defs" ; "draft 2019-09")]
    #[test_case("http://json-schema.org/draft-07/schema#", "definitions" ; "draft 7")]
    #[test_case("http://json-schema.org/draft-06/schema#", "definitions" ; "draft 6")]
    #[test_case("http://json-schema.org/draft-04/schema#", "definitions" ; "draft 4")]
    fn pointer_ref_inside_nested_id_resolves_in_that_resource(meta: &str, defs: &str) {
        // The inner `$ref` shadows the outer definition name. Reached through `properties` so
        // the shape is also expressible on drafts where `$ref` suppresses its siblings.
        let schema = json!({
            "$schema": meta,
            "id": "https://example.com/outer",
            "$id": "https://example.com/outer",
            "properties": {"value": {"$ref": format!("#/{defs}/wrapper")}},
            defs: {
                "target": {"type": "string"},
                "wrapper": {
                    "id": "https://example.com/inner",
                    "$id": "https://example.com/inner",
                    "allOf": [{"$ref": format!("#/{defs}/target")}],
                    defs: {"target": {"type": "integer"}}
                }
            }
        });
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        assert!(validator.is_valid(&json!({"value": 1})));
        assert!(!validator.is_valid(&json!({"value": "text"})));
    }

    /// The same shape with the `$ref` sitting directly on the subresource, where the composed
    /// "current location" URI used to collide with the reference's own target URI and the `$ref`
    /// was silently dropped as a self-reference.
    #[test_case("https://json-schema.org/draft/2020-12/schema" ; "draft 2020-12")]
    #[test_case("https://json-schema.org/draft/2019-09/schema" ; "draft 2019-09")]
    fn shadowed_pointer_ref_on_nested_id_root_is_not_a_self_reference(meta: &str) {
        let schema = json!({
            "$schema": meta,
            "$id": "https://example.com/outer",
            "$ref": "#/$defs/target",
            "$defs": {
                "target": {
                    "$id": "https://example.com/inner",
                    "$ref": "#/$defs/target",
                    "$defs": {"target": {"type": "integer"}}
                }
            }
        });
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        assert!(validator.is_valid(&json!(1)));
        assert!(!validator.is_valid(&json!("text")));
        assert!(!validator.is_valid(&json!({})));
    }

    /// Writing the same reference absolutely must not change the outcome.
    #[test]
    fn absolute_form_of_shadowed_pointer_ref_agrees() {
        let schema = json!({
            "$id": "https://example.com/outer",
            "$ref": "#/$defs/target",
            "$defs": {
                "target": {
                    "$id": "https://example.com/inner",
                    "$ref": "https://example.com/inner#/$defs/target",
                    "$defs": {"target": {"type": "integer"}}
                }
            }
        });
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        assert!(validator.is_valid(&json!(1)));
        assert!(!validator.is_valid(&json!("text")));
    }

    /// Three nested resources, to exercise the resolution stack rather than a single hop.
    #[test]
    fn shadowed_pointer_ref_through_three_nested_resources() {
        let schema = json!({
            "$id": "https://example.com/first",
            "$ref": "#/$defs/target",
            "$defs": {
                "target": {
                    "$id": "https://example.com/second",
                    "$ref": "#/$defs/target",
                    "$defs": {
                        "target": {
                            "$id": "https://example.com/third",
                            "$ref": "#/$defs/target",
                            "$defs": {"target": {"type": "integer"}}
                        }
                    }
                }
            }
        });
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        assert!(validator.is_valid(&json!(1)));
        assert!(!validator.is_valid(&json!("text")));
    }

    /// Without the nested `$id` the inner `$ref` really does target its own enclosing schema,
    /// which stays an unconstrained self-loop.
    #[test]
    fn pointer_ref_without_nested_id_remains_a_self_reference() {
        let schema = json!({
            "$id": "https://example.com/outer",
            "$ref": "#/$defs/target",
            "$defs": {
                "target": {
                    "$ref": "#/$defs/target",
                    "$defs": {"target": {"type": "integer"}}
                }
            }
        });
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        assert!(validator.is_valid(&json!(1)));
        assert!(validator.is_valid(&json!("text")));
    }

    /// The same shape reached through a `Registry` rather than inline, which enters resolution
    /// by a different path.
    #[test]
    fn shadowed_pointer_ref_inside_nested_id_via_registry() {
        let registry = referencing::Registry::new()
            .add(
                "https://example.com/outer",
                referencing::Resource::from_contents(json!({
                    "$id": "https://example.com/outer",
                    "$ref": "#/$defs/target",
                    "$defs": {
                        "target": {
                            "$id": "https://example.com/inner",
                            "$ref": "#/$defs/target",
                            "$defs": {"target": {"type": "integer"}}
                        }
                    }
                })),
            )
            .expect("Invalid resource")
            .prepare()
            .expect("Invalid registry");
        let validator = crate::options()
            .with_registry(&registry)
            .build(&json!({"$ref": "https://example.com/outer"}))
            .expect("Invalid schema");
        assert!(validator.is_valid(&json!(1)));
        assert!(!validator.is_valid(&json!("text")));
    }

    /// An `$anchor` declared inside a nested `$id` resource uses the same base URI machinery
    /// through a different lookup path, and must not be shadowed by a same-named outer anchor.
    #[test]
    fn anchor_ref_inside_nested_id_resolves_in_that_resource() {
        let schema = json!({
            "$id": "https://example.com/outer",
            "$ref": "#/$defs/wrapper",
            "$defs": {
                "wrapper": {
                    "$id": "https://example.com/inner",
                    "$ref": "#target",
                    "$defs": {"target": {"$anchor": "target", "type": "integer"}}
                },
                "decoy": {"$anchor": "target", "type": "string"}
            }
        });
        let validator = crate::validator_for(&schema).expect("Invalid schema");
        assert!(validator.is_valid(&json!(1)));
        assert!(!validator.is_valid(&json!("text")));
    }

    #[test_case(crate::Draft::Draft4, "id"; "draft4")]
    #[test_case(crate::Draft::Draft6, "$id"; "draft6")]
    #[test_case(crate::Draft::Draft7, "$id"; "draft7")]
    fn distinct_fragment_ids_keep_distinct_cached_targets(draft: crate::Draft, id: &str) {
        let schema = json!({
            id: "https://example.com/model.json",
            "type": "object",
            "definitions": {
                "directive": {id: "#directive", "$ref": "#/definitions/text"},
                "model": {
                    id: "#model",
                    "type": "object",
                    "properties": {"name": {"$ref": "#/definitions/text"}},
                    "required": ["name"]
                },
                "text": {"type": "string"}
            },
            "properties": {
                "$schema": {"$ref": "#directive"},
                "alias": {"$ref": "#/definitions/model"},
                "model": {"$ref": "#model"}
            },
            "required": ["model"]
        });
        let validator = crate::options()
            .with_draft(draft)
            .build(&schema)
            .expect("Valid fragment-ID schema");
        for instance in [
            json!({"model": {"name": "actual model"}}),
            json!({"$schema": "a reference", "model": {"name": "actual model"}}),
            json!({"alias": {"name": "same target"}, "model": {"name": "actual model"}}),
        ] {
            tests_util::is_valid_with(&validator, &instance);
        }
        for instance in [
            json!({"model": "not a model object"}),
            json!({"model": {"name": 42}}),
            json!({"model": {}}),
            json!({"$schema": {}, "model": {"name": "actual model"}}),
            json!({"alias": {"name": 42}, "model": {"name": "actual model"}}),
        ] {
            tests_util::is_not_valid_with(&validator, &instance);
        }
    }

    #[test_case(crate::Draft::Draft4, "id", "id", "#", "definitions"; "draft4")]
    #[test_case(crate::Draft::Draft6, "$id", "$id", "#", "definitions"; "draft6")]
    #[test_case(crate::Draft::Draft7, "$id", "$id", "#", "definitions"; "draft7")]
    #[test_case(crate::Draft::Draft201909, "$id", "$anchor", "", "$defs"; "draft2019")]
    #[test_case(crate::Draft::Draft202012, "$id", "$anchor", "", "$defs"; "draft2020")]
    fn distinct_recursive_anchors_keep_their_own_children(
        draft: crate::Draft,
        id: &str,
        anchor: &str,
        prefix: &str,
        definitions: &str,
    ) {
        let schema = json!({
            id: "https://example.com/recursive.json",
            "type": "object",
            definitions: {
                "left": {
                    anchor: format!("{prefix}left"),
                    "type": "object",
                    "properties": {
                        "tag": {"enum": ["left"]},
                        "next": {"$ref": "#right"}
                    },
                    "required": ["tag"],
                    "additionalProperties": false
                },
                "right": {
                    anchor: format!("{prefix}right"),
                    "type": "object",
                    "properties": {
                        "tag": {"enum": ["right"]},
                        "next": {"$ref": "#left"}
                    },
                    "required": ["tag"],
                    "additionalProperties": false
                }
            },
            "properties": {
                "left": {"$ref": "#left"},
                "right": {"$ref": "#right"},
                "viaPointer": {"$ref": format!("#/{definitions}/left")}
            }
        });
        let validator = crate::options()
            .with_draft(draft)
            .build(&schema)
            .expect("Valid recursive anchor schema");
        for instance in [
            json!({"left": {"tag": "left"}, "right": {"tag": "right"}}),
            json!({"left": {"tag": "left", "next": {"tag": "right", "next": {"tag": "left"}}}}),
            json!({"viaPointer": {"tag": "left", "next": {"tag": "right"}}}),
        ] {
            tests_util::is_valid_with(&validator, &instance);
        }
        for instance in [
            json!({"left": {"tag": "right"}}),
            json!({"right": {"tag": "left"}}),
            json!({"left": {"tag": "left", "next": {"tag": "left"}}}),
            json!({"right": {"tag": "right", "next": {"tag": "right"}}}),
            json!({"viaPointer": {"tag": "right"}}),
            json!({"viaPointer": {"tag": "left", "next": {"tag": "left"}}}),
        ] {
            tests_util::is_not_valid_with(&validator, &instance);
        }
    }

    fn dynamic_anchor_via_pointer(entry: &str) -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/root",
            "$defs": {
                "sub": {
                    "$id": "sub",
                    "$defs": {
                        "bind": {"$dynamicAnchor": "T", "type": "number"},
                        "middle": {"$ref": "https://example.com/shared"}
                    }
                },
                "shared": {
                    "$id": "shared",
                    "$defs": {"fallback": {"$dynamicAnchor": "T", "type": "string"}},
                    "items": {"$dynamicRef": "#T"}
                }
            },
            "properties": {
                entry: {"$ref": "sub#/$defs/middle"},
                "b": {"$ref": "shared"}
            }
        })
    }

    fn dynamic_anchor_beside_stray_id(entry: &str) -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/root",
            "$defs": {
                "holder": {
                    "$id": "holder",
                    "$defs": {
                        "bind": {"id": "stray", "$dynamicAnchor": "T", "type": "number"},
                        "use": {"$ref": "https://example.com/shared"}
                    }
                },
                "shared": {
                    "$id": "shared",
                    "$defs": {"fallback": {"$dynamicAnchor": "T", "type": "string"}},
                    "items": {"$dynamicRef": "#T"}
                }
            },
            "properties": {
                entry: {"$ref": "holder#/$defs/use"},
                "b": {"$ref": "shared"}
            }
        })
    }

    fn recursive_anchor_via_pointer(entry: &str) -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2019-09/schema",
            "$id": "https://example.com/root",
            "$defs": {
                "sub": {
                    "$id": "sub",
                    "$recursiveAnchor": true,
                    "type": "number",
                    "$defs": {"middle": {"$ref": "https://example.com/shared"}}
                },
                "shared": {
                    "$id": "shared",
                    "$recursiveAnchor": true,
                    "anyOf": [
                        {"type": "string"},
                        {"type": "array", "items": {"$recursiveRef": "#"}}
                    ]
                }
            },
            "properties": {
                entry: {"$ref": "sub#/$defs/middle"},
                "b": {"$ref": "shared"}
            }
        })
    }

    #[test_case(dynamic_anchor_via_pointer, "a" ; "dynamic anchor, entry compiled first")]
    #[test_case(dynamic_anchor_via_pointer, "z" ; "dynamic anchor, entry compiled last")]
    #[test_case(dynamic_anchor_beside_stray_id, "a" ; "stray id, entry compiled first")]
    #[test_case(dynamic_anchor_beside_stray_id, "z" ; "stray id, entry compiled last")]
    #[test_case(recursive_anchor_via_pointer, "a" ; "recursive anchor, entry compiled first")]
    #[test_case(recursive_anchor_via_pointer, "z" ; "recursive anchor, entry compiled last")]
    fn anchor_binding_does_not_depend_on_property_order(schema: fn(&str) -> Value, entry: &str) {
        let validator = crate::validator_for(&schema(entry)).expect("Invalid schema");
        let results: Vec<bool> = [
            json!({entry: [1]}),
            json!({entry: ["s"]}),
            json!({"b": [1]}),
            json!({"b": ["s"]}),
        ]
        .iter()
        .map(|instance| validator.is_valid(instance))
        .collect();
        assert_eq!(results, vec![true, false, false, true]);
    }

    #[test_case("$ref"; "ref")]
    #[test_case("$dynamicRef"; "dynamic ref")]
    fn malformed(keyword: &str) {
        tests_util::assert_compile_error(
            &json!({keyword: 5}),
            "5 is not of type \"string\"",
            &format!("/{keyword}"),
        );
    }

    #[test]
    fn malformed_draft7() {
        tests_util::assert_compile_error_with(
            &crate::options().with_draft(crate::Draft::Draft7),
            &json!({"$ref": 5}),
            "5 is not of type \"string\"",
            "/$ref",
        );
    }
}
