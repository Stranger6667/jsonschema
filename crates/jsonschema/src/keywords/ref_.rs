use crate::{evaluation::InstancePosition, LazyInstance};
use std::borrow::Cow;

use crate::{
    compiler::{self, CachedTarget, TargetIdentity, TargetNode},
    keywords::{BoxedValidator, CompilationResult},
    node::{PendingSchemaNode, SchemaNode},
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
    /// Location of this `$ref` keyword, e.g. `/$defs/Order/properties/foo/$ref`.
    /// Used for building the `tracker` prefix.
    ref_location: Location,
    /// The resource base of the `$ref` target.
    /// E.g., `/$defs/Item` when `$ref` points to `#/$defs/Item`.
    /// Used for computing validator suffixes at runtime.
    ref_target_base: Location,
    /// Absolute URI of the `$ref` target, if its resource has one.
    target_location: Option<Arc<Uri<String>>>,
}

impl<F: Json> Validate<F> for RefValidator<F> {
    fn collect_object_part<'a>(&'a self, part: &mut super::all_of::ObjectPart<'a, F>) -> bool {
        self.inner.collect_object_part(part)
    }

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
        let child_tracker = RefTracker::new(&self.ref_location, &self.ref_target_base, tracker);
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
        let child_tracker = RefTracker::new(&self.ref_location, &self.ref_target_base, tracker);
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
        self.evaluate_with_location(
            instance,
            location,
            ctx.instance_position(location),
            tracker,
            ctx,
        )
    }

    fn evaluate_with_location(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: InstancePosition,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let child_tracker = RefTracker::new(&self.ref_location, &self.ref_target_base, tracker);
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
    /// Per 2020-12 Core 12.3.2, a keyword's absolute location holds no `$ref` or `$dynamicRef`
    /// as a non-terminal component.
    fn canonical_location(&self) -> Option<(&Location, Option<&Arc<Uri<String>>>)> {
        Some((&self.ref_target_base, self.target_location.as_ref()))
    }
}

/// Like `RefValidator` but holds a concrete target instead of `BoxedValidator<F>`,
/// eliminating one layer of vtable dispatch on every validation call.
/// Used for non-recursive refs: a compiled node, or a placeholder for a deferred one.
struct DirectRefValidator<T> {
    inner: T,
    ref_location: Location,
    ref_target_base: Location,
    target_location: Option<Arc<Uri<String>>>,
}

impl<F: Json, T: Validate<F>> Validate<F> for DirectRefValidator<T> {
    fn owned_ref_target(&self) -> Option<&crate::node::SchemaNode<F>> {
        self.inner.as_owned_node()
    }

    fn collect_object_part<'a>(&'a self, part: &mut super::all_of::ObjectPart<'a, F>) -> bool {
        self.inner.collect_object_part(part)
    }

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
        let child_tracker = RefTracker::new(&self.ref_location, &self.ref_target_base, tracker);
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
        let child_tracker = RefTracker::new(&self.ref_location, &self.ref_target_base, tracker);
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
        self.evaluate_with_location(
            instance,
            location,
            ctx.instance_position(location),
            tracker,
            ctx,
        )
    }

    fn evaluate_with_location(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: InstancePosition,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let child_tracker = RefTracker::new(&self.ref_location, &self.ref_target_base, tracker);
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
    Some(pointer_uri(base, location))
}

/// URI of the schema at `location` within the resource `base` names.
fn pointer_uri(base: &Uri<String>, location: &Location) -> Arc<Uri<String>> {
    let mut fragment = uri::EncodedBuffer::new();
    fragment.encode_str::<uri::Path>(location.as_str());
    Arc::new(base.with_fragment(Some(fragment.as_estr())))
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

/// What a `$ref` holds: a node it owns, or one the tree owns elsewhere.
enum RefTarget<F: Json> {
    Node(SchemaNode<F>),
    Pending(PendingSchemaNode<F>),
}

impl<F: Json> RefTarget<F> {
    fn into_validator(
        self,
        ref_location: Location,
        ref_target_base: Location,
        target_location: Option<Arc<Uri<String>>>,
    ) -> BoxedValidator<F> {
        match self {
            RefTarget::Node(inner) => Box::new(DirectRefValidator {
                inner,
                ref_location,
                ref_target_base,
                target_location,
            }),
            RefTarget::Pending(inner) => Box::new(DirectRefValidator {
                inner,
                ref_location,
                ref_target_base,
                target_location,
            }),
        }
    }
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
    let dynamic = keyword == "$dynamicRef";
    let ((alias, ref_target_base), resolved) =
        match ctx.resolve_ref_target(reference, dynamic, extract_ref_target_base) {
            Ok(target) => target,
            Err(error) => return Some(Err(ValidationError::from(error))),
        };

    let ref_location = ctx.location().join(keyword);
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
                // The resource's own URI, which a document registered under another one keeps.
                let target = target_location(&resolved.resolver().base_uri(), &ref_target_base);
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

    // Every reference naming this schema shares its node; a `$dynamicRef` names the schema it
    // lands on in this dynamic scope.
    let identity = TargetIdentity::new(resolved.contents(), ref_target_base.clone());
    if let Some(target) = ctx.lookup_cached_target(&identity, resolved.resolver()) {
        let target = match target {
            CachedTarget::Node(TargetNode::Owned(node)) => RefTarget::Node(node),
            CachedTarget::Node(TargetNode::Shared(node)) => {
                RefTarget::Pending(PendingSchemaNode::pointing_at(&node))
            }
            CachedTarget::Placeholder(placeholder) => RefTarget::Pending(placeholder),
        };
        return Some(Ok(target.into_validator(
            ref_location,
            ref_target_base,
            target_location,
        )));
    }

    ctx.mark_seen(&identity);

    if ctx.defers_ref_target(&resolved, &identity) {
        let placeholder = ctx.defer_ref_target(resolved, ref_target_base.clone(), identity);
        return Some(Ok(Box::new(DirectRefValidator {
            inner: placeholder,
            ref_location,
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
        ctx.compile_ref_target(|| {
            compiler::compile_with_identity(&inner_ctx, resource_ref, &identity)
        })
        .map(|target| {
            let target = match target {
                TargetNode::Owned(node) => RefTarget::Node(node),
                TargetNode::Shared(node) => {
                    RefTarget::Pending(PendingSchemaNode::pointing_at(&node))
                }
            };
            target.into_validator(ref_location, ref_target_base, target_location)
        })
        .map_err(ValidationError::to_owned),
    )
}

/// Compile the subschema at the escaped JSON Pointer `pointer` as a `$ref` to it from the document
/// root `ctx` is at compiles it, so the `$ref`s in it that name it share its node.
pub(crate) fn compile_pointer_target<F: Json>(
    ctx: &compiler::Context<F>,
    pointer: &str,
) -> Result<SchemaNode<F>, ValidationError<'static>> {
    let mut fragment = uri::EncodedBuffer::new();
    fragment.encode_str::<uri::Path>(pointer);
    let reference = format!("#{}", fragment.as_str());
    let ((alias, target_base), resolved) =
        ctx.resolve_ref_target(&reference, false, extract_ref_target_base)?;
    let resource_start = embedded_resource_start(ctx, &alias, &target_base, &resolved)?;
    let identity = TargetIdentity::new(resolved.contents(), target_base.clone());
    ctx.mark_seen(&identity);
    let (contents, resolver, draft) = resolved.into_inner();
    let resource_ref = draft.create_resource_ref(contents);
    let vocabularies = resolver.find_vocabularies(draft, contents);
    let inner_ctx = ctx
        .with_resolver_and_draft(resolver, resource_ref.draft(), vocabularies, target_base)?
        .with_resource_start(resource_start);
    compiler::compile_with_identity(&inner_ctx, resource_ref, &identity)
        .map(TargetNode::into_node)
        .map_err(ValidationError::to_owned)
}

fn compile_recursive_validator<'a, F: Json>(
    ctx: &compiler::Context<F>,
    reference: &str,
) -> CompilationResult<'a, F> {
    let ref_location = ctx.location().join("$recursiveRef");
    let (_, ref_target_base) = ctx
        .ref_target(reference, extract_ref_target_base)
        .map_err(ValidationError::from)?;
    let resolved = ctx
        .lookup_recursive_reference()
        .map_err(ValidationError::from)?;
    let target_location = target_location(&resolved.resolver().base_uri(), &ref_target_base);
    // Shares the node of every reference naming the resource it lands on in this dynamic scope.
    let identity = TargetIdentity::new(resolved.contents(), ref_target_base.clone());

    if let Some(validator) = ctx.lookup_maybe_recursive(&identity, resolved.resolver()) {
        return Ok(Box::new(RefValidator {
            inner: validator,
            ref_location,
            ref_target_base,
            target_location,
        }));
    }

    ctx.mark_seen(&identity);

    let (contents, resolver, draft) = resolved.into_inner();
    let vocabularies = resolver.find_vocabularies(draft, contents);
    let resource_ref = draft.create_resource_ref(contents);
    let target_base = ref_target_base.clone();
    let inner_ctx =
        ctx.with_resolver_and_draft(resolver, resource_ref.draft(), vocabularies, target_base)?;
    compiler::compile_with_identity(&inner_ctx, resource_ref, &identity)
        .map(|target| {
            Box::new(RefValidator {
                inner: target.into_validator(),
                ref_location,
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
    use crate::{error::ValidationErrorKind, tests_util, ValidationError};
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
    #[test_case(&repeated_reference(), &json!({"a": "x", "b": 42}), "/properties/b/$ref/type"; "repeated reference")]
    #[test_case(&recursive_reference(), &json!({"child": {"child": 1}}), "/$ref/properties/child/$ref/properties/child/$ref/type"; "recursive reference")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        // For $ref tests, check tracker (includes $ref traversals)
        tests_util::assert_evaluation_path(schema, instance, expected);
    }

    // The second `$ref` to a definition reuses the node compiled for the first.
    fn repeated_reference() -> Value {
        json!({
            "properties": {"a": {"$ref": "#/definitions/foo"}, "b": {"$ref": "#/definitions/foo"}},
            "definitions": {"foo": {"type": "string"}}
        })
    }

    fn recursive_reference() -> Value {
        json!({
            "$ref": "#/definitions/node",
            "definitions": {"node": {"type": "object", "properties": {"child": {"$ref": "#/definitions/node"}}}}
        })
    }

    #[test_case(&repeated_reference(), &json!({"a": "x", "b": 42}), "/definitions/foo/type"; "repeated reference")]
    #[test_case(&recursive_reference(), &json!({"child": {"child": 1}}), "/definitions/node/type"; "recursive reference")]
    #[test_case(&repeated_reference(), &json!({"a": "x", "b": "y"}), ""; "repeated reference accepted")]
    fn reused_target_schema_location(schema: &Value, instance: &Value, expected: &str) {
        if expected.is_empty() {
            tests_util::is_valid(schema, instance);
        } else {
            tests_util::is_not_valid(schema, instance);
            tests_util::assert_schema_location(schema, instance, expected);
        }
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
            "$id": "https://example.com/root",
            "properties": {"a": {"$ref": "https://example.com/alias"}}
        }),
        &[("https://example.com/alias", json!({"$id": "https://example.com/real", "type": "string"}))],
        &[
            ("", "https://example.com/root#"),
            ("/properties", "https://example.com/root#/properties"),
            ("/properties/a", "https://example.com/root#/properties/a"),
            ("/properties/a/$ref", "https://example.com/real#"),
            ("/properties/a/$ref/type", "https://example.com/real#/type"),
        ],
        &["https://example.com/real#/type"]
    ; "resource registered under another URI than its id")]
    #[test_case(
        &json!({
            "$id": "https://example.com/root",
            "properties": {"a": {"$ref": "https://example.com/alias#/$defs/s"}}
        }),
        &[(
            "https://example.com/alias",
            json!({"$id": "https://example.com/real", "$defs": {"s": {"type": "string"}}})
        )],
        &[
            ("", "https://example.com/root#"),
            ("/properties", "https://example.com/root#/properties"),
            ("/properties/a", "https://example.com/root#/properties/a"),
            ("/properties/a/$ref", "https://example.com/real#/$defs/s"),
            ("/properties/a/$ref/type", "https://example.com/real#/$defs/s/type"),
        ],
        &["https://example.com/real#/$defs/s/type"]
    ; "pointer into a resource registered under another URI than its id")]
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

    // A keyword in a resource under the default base has no absolute location, as in a schema
    // without `$id`, even when a resource with one reaches it.
    #[test_case(
        &json!({"$id": "http://example.com/root", "properties": {"x": {"$ref": "json-schema:///other"}}}),
        &json!({"x": "a"}),
        None
    ; "reference")]
    #[test_case(
        &json!({"$id": "http://example.com/root", "$ref": "json-schema:///other"}),
        &json!("a"),
        None
    ; "reference at the root")]
    #[test_case(
        &json!({"$id": "http://example.com/root", "items": {"$ref": "json-schema:///other#/$defs/s"}}),
        &json!([0]),
        None
    ; "pointer below the resource root")]
    #[test_case(
        &json!({"$id": "http://example.com/root", "properties": {"x": {"$ref": "json-schema:///never"}}}),
        &json!({"x": 1}),
        None
    ; "false resource")]
    #[test_case(
        &json!({"$id": "http://example.com/root", "properties": {"x": {"$id": "json-schema:///inner", "maxLength": 0}}}),
        &json!({"x": "a"}),
        None
    ; "embedded resource")]
    #[test_case(
        &json!({"$id": "http://example.com/root", "properties": {"x": {"$ref": "http://example.com/other"}}}),
        &json!({"x": "a"}),
        Some("http://example.com/other#/maxLength")
    ; "resource with an absolute URI")]
    fn resource_under_default_base(schema: &Value, instance: &Value, expected: Option<&str>) {
        let other = json!({"maxLength": 0, "$defs": {"s": {"minimum": 1}}});
        let registry = crate::Registry::new()
            .add("json-schema:///other", &other)
            .expect("Valid resource")
            .add("http://example.com/other", &other)
            .expect("Valid resource")
            .add("json-schema:///never", json!(false))
            .expect("Valid resource")
            .prepare()
            .expect("Registry should prepare");
        let validator = crate::options()
            .with_registry(&registry)
            .build(schema)
            .expect("Invalid schema");
        let error = validator.validate(instance).expect_err("Invalid instance");
        let errors: Vec<Option<String>> = validator
            .iter_errors(instance)
            .map(|error| error.absolute_keyword_location().map(ToString::to_string))
            .collect();
        let evaluation: Vec<Option<String>> = validator
            .evaluate(instance)
            .iter_errors()
            .map(|entry| entry.absolute_keyword_location.map(ToString::to_string))
            .collect();
        let expected = expected.map(str::to_string);
        assert_eq!(
            (
                error.absolute_keyword_location().map(ToString::to_string),
                errors,
                evaluation
            ),
            (expected.clone(), vec![expected.clone()], vec![expected])
        );
    }

    // A rejected name's own error is located at the keyword that rejected it.
    #[test]
    fn property_name_in_resource_under_default_base() {
        let other = json!({"maxLength": 0});
        let registry = crate::Registry::new()
            .add("json-schema:///other", &other)
            .expect("Valid resource")
            .prepare()
            .expect("Registry should prepare");
        let validator = crate::options()
            .with_registry(&registry)
            .build(&json!({"$id": "http://example.com/root", "propertyNames": {"$ref": "json-schema:///other"}}))
            .expect("Invalid schema");
        let instance = json!({"x": 1});
        let name_location = |error: &ValidationError<'_>| match error.kind() {
            ValidationErrorKind::PropertyNames { error } => {
                error.absolute_keyword_location().map(ToString::to_string)
            }
            kind => panic!("Unexpected error kind: {kind:?}"),
        };
        let error = validator.validate(&instance).expect_err("Invalid instance");
        let errors: Vec<Option<String>> = validator
            .iter_errors(&instance)
            .map(|error| name_location(&error))
            .collect();
        assert_eq!((name_location(&error), errors), (None, vec![None]));
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
                    "responses": {
                        "200": {
                            "$defs": {"x": {"$anchor": "num", "type": "integer"}},
                            "$ref": "#num"
                        }
                    },
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
        // Chained local `$ref`s ending in an external resource retrieve that resource.
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
        // A local `$ref` to a schema with an external `$ref` under `properties`, not a direct
        // chain.
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

    // A `$ref` pointer through a numeric object key (an OpenAPI response code) reaches the
    // references nested under its target
    #[test_case("#/paths/~1x/responses/200", json!({"/x": {"responses": {"200": {"$ref": "ext.yaml#/components/schemas/c3"}}}}); "numeric key")]
    #[test_case("#/paths/~1x/responses/0", json!({"/x": {"responses": {"0": {"$ref": "ext.yaml#/components/schemas/c3"}}}}); "zero key")]
    #[test_case("ext.yaml#/responses/200", json!({}); "external numeric key")]
    #[test_case("#/paths/~1x/responses/ok", json!({"/x": {"responses": {"ok": {"$ref": "ext.yaml#/components/schemas/c3"}}}}); "string key")]
    #[allow(clippy::needless_pass_by_value)]
    fn test_ref_through_numeric_object_key(reference: &str, paths: Value) {
        let schema = json!({
            "$id": "file:///tmp",
            "$ref": reference,
            "paths": paths
        });

        let validator = crate::options()
            .with_retriever(IndirectExternalRetrieve)
            .build(&schema)
            .expect("Failed to build validator - external resource was not discovered");

        assert!(validator.is_valid(&json!(42)));
        assert!(!validator.is_valid(&json!("string")));
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
        // An external `$ref` whose target holds local `$ref`s resolves them against that file's
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

    // A registered resource's `$id` is the base for its references, however the resource is reached
    #[test_case("child.json", json!({"$id": "nested/child.json", "$ref": "../leaf.json"}); "relative id")]
    #[test_case("child.json", json!({"$id": "file:///schemas/nested/child.json", "$ref": "../leaf.json"}); "absolute id")]
    #[test_case("child.json#/$defs/name", json!({"$id": "nested/child.json", "$defs": {"name": {"$ref": "../leaf.json"}}}); "pointer")]
    #[test_case("child.json#name", json!({"$id": "nested/child.json", "$defs": {"name": {"$anchor": "name", "$ref": "../leaf.json"}}}); "anchor")]
    #[test_case("nested/child.json", json!({"$id": "nested/child.json", "$ref": "../leaf.json"}); "reached by id")]
    fn registered_resource_id_is_reference_base(reference: &str, child: Value) {
        let registry = crate::Registry::new()
            .add("file:///schemas/child.json", child)
            .expect("Invalid resource")
            .add("file:///schemas/leaf.json", json!({"type": "string"}))
            .expect("Invalid resource")
            .prepare()
            .expect("Invalid registry");
        let validator = crate::options()
            .with_base_uri("file:///schemas/root.json")
            .with_registry(&registry)
            .build(&json!({"properties": {"name": {"$ref": reference}}}))
            .expect("Invalid schema");

        assert!(validator.is_valid(&json!({"name": "hello"})));
        assert!(!validator.is_valid(&json!({"name": 1})));
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

    /// The same shape with the `$ref` directly on the subresource: it is not a self-reference.
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

    // Dynamic scope `a` -> `b` -> `c`: `a` accepts booleans, `b` strings, `c` integers, so the value
    // under `x` shows where the `$recursiveRef` in `c` lands.
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/a", "$recursiveAnchor": true, "anyOf": [{"type": "boolean"}, {"$ref": "b"}], "$defs": {"b": {"$id": "https://example.com/b", "anyOf": [{"type": "string"}, {"$ref": "c"}]}, "c": {"$id": "https://example.com/c", "$recursiveAnchor": true, "anyOf": [{"type": "integer"}, {"type": "object", "additionalProperties": {"$recursiveRef": "#"}}]}}}),
        &[true, true, true, true, false];
        "middle without the anchor, references"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/a", "$recursiveAnchor": true, "anyOf": [{"type": "boolean"}, {"$id": "https://example.com/b", "$recursiveAnchor": false, "anyOf": [{"type": "string"}, {"$id": "https://example.com/c", "$recursiveAnchor": true, "anyOf": [{"type": "integer"}, {"type": "object", "additionalProperties": {"$recursiveRef": "#"}}]}]}]}),
        &[true, true, true, true, false];
        "middle with the anchor false, inline"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/a", "$recursiveAnchor": true, "anyOf": [{"type": "boolean"}, {"$ref": "b"}], "$defs": {"b": {"$id": "https://example.com/b", "$recursiveAnchor": false, "anyOf": [{"type": "string"}, {"$id": "https://example.com/c", "$recursiveAnchor": true, "anyOf": [{"type": "integer"}, {"type": "object", "additionalProperties": {"$recursiveRef": "#"}}]}]}}}),
        &[true, true, true, true, false];
        "middle with the anchor false, reference then inline"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/a", "$recursiveAnchor": true, "anyOf": [{"type": "boolean"}, {"$id": "https://example.com/b", "anyOf": [{"type": "string"}, {"$ref": "c"}], "$defs": {"c": {"$id": "https://example.com/c", "$recursiveAnchor": true, "anyOf": [{"type": "integer"}, {"type": "object", "additionalProperties": {"$recursiveRef": "#"}}]}}}]}),
        &[true, true, true, true, false];
        "middle without the anchor, inline then reference"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/a", "$recursiveAnchor": true, "anyOf": [{"type": "boolean"}, {"$ref": "b"}], "$defs": {"b": {"$id": "https://example.com/b", "anyOf": [{"type": "string"}, {"$ref": "c"}]}, "c": {"$id": "https://example.com/c", "anyOf": [{"type": "integer"}, {"type": "object", "additionalProperties": {"$recursiveRef": "#"}}]}}}),
        &[false, false, true, false, false];
        "target without the anchor"
    )]
    fn recursive_reference_lands_on_outermost_anchored_resource(schema: &Value, expected: &[bool]) {
        let validator = crate::validator_for(schema).expect("Invalid schema");
        let results: Vec<bool> = [
            json!({"x": true}),
            json!({"x": "s"}),
            json!({"x": 1}),
            json!({"x": {"y": true}}),
            json!({"x": []}),
        ]
        .iter()
        .map(|instance| validator.is_valid(instance))
        .collect();
        assert_eq!(results, expected);
    }

    // Scope `a` -> `b` -> `c`: only `a` evaluates `y` and the first item, so `unevaluated*` beside
    // the `$recursiveRef` in `c` accepts them only when it lands on `a`.
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/a", "$recursiveAnchor": true, "properties": {"y": true}, "$ref": "b", "$defs": {"b": {"$id": "https://example.com/b", "$ref": "c"}, "c": {"$id": "https://example.com/c", "$recursiveAnchor": true, "properties": {"x": {"$recursiveRef": "#", "unevaluatedProperties": false}}}}}),
        &json!({"x": {"y": 1}}),
        &json!({"x": {"z": 1}});
        "unevaluatedProperties"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/a", "$recursiveAnchor": true, "items": [true], "$ref": "b", "$defs": {"b": {"$id": "https://example.com/b", "$ref": "c"}, "c": {"$id": "https://example.com/c", "$recursiveAnchor": true, "properties": {"x": {"$recursiveRef": "#", "unevaluatedItems": false}}}}}),
        &json!({"x": [1]}),
        &json!({"x": [1, 2]});
        "unevaluatedItems"
    )]
    fn recursive_reference_past_unanchored_resource_evaluates_outermost(
        schema: &Value,
        valid: &Value,
        invalid: &Value,
    ) {
        let validator = crate::validator_for(schema).expect("Invalid schema");
        assert_eq!(
            [validator.is_valid(valid), validator.is_valid(invalid)],
            [true, false]
        );
    }

    // One `c` reached from `a` directly and through `b`, which has no `$recursiveAnchor`; `a`
    // accepts booleans and objects only.
    fn recursive_target_shared_by_two_scopes(through_middle: &str) -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2019-09/schema",
            "$id": "https://example.com/a",
            "$recursiveAnchor": true,
            "anyOf": [
                {"type": "boolean"},
                {
                    "type": "object",
                    "properties": {through_middle: {"$ref": "b"}, "m": {"$ref": "c"}},
                    "additionalProperties": false
                }
            ],
            "$defs": {
                "b": {"$id": "https://example.com/b", "anyOf": [{"type": "string"}, {"$ref": "c"}]},
                "c": {
                    "$id": "https://example.com/c",
                    "$recursiveAnchor": true,
                    "anyOf": [
                        {"type": "integer"},
                        {"type": "object", "additionalProperties": {"$recursiveRef": "#"}}
                    ]
                }
            }
        })
    }

    #[test_case("a" ; "through the middle compiled first")]
    #[test_case("z" ; "through the middle compiled last")]
    fn recursive_target_shared_by_two_scopes_lands_on_outermost_anchored_resource(
        through_middle: &str,
    ) {
        let validator =
            crate::validator_for(&recursive_target_shared_by_two_scopes(through_middle))
                .expect("Invalid schema");
        let results: Vec<bool> = [
            json!({"m": {"x": true}}),
            json!({"m": {"x": 1}}),
            json!({through_middle: {"x": true}}),
            json!({through_middle: {"x": 1}}),
        ]
        .iter()
        .map(|instance| validator.is_valid(instance))
        .collect();
        assert_eq!(results, vec![true, false, true, false]);
    }

    // `inner` is the resource every reference names; `outer` is the outermost `item` in scope
    fn same_name_dynamic_anchors(root: &Value) -> Value {
        let mut schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/root",
            "$defs": {
                "outer": {"$dynamicAnchor": "item", "type": "null"},
                "inner": {"$id": "inner", "$dynamicAnchor": "item", "type": "string"}
            }
        });
        let keywords = schema.as_object_mut().expect("An object");
        keywords.extend(root.as_object().expect("An object").clone());
        schema
    }

    #[test_case(&json!({"$ref": "inner#item"}), &json!("a"), &json!(null); "ref is static")]
    #[test_case(&json!({"$dynamicRef": "inner#item"}), &json!(null), &json!("a"); "dynamic ref follows the scope")]
    #[test_case(
        &json!({"allOf": [
            {"properties": {"static": {"$ref": "inner#item"}}},
            {"properties": {"dynamic": {"$dynamicRef": "inner#item"}}}
        ]}),
        &json!({"static": "a", "dynamic": null}),
        &json!({"static": null});
        "ref before dynamic ref, static side"
    )]
    #[test_case(
        &json!({"allOf": [
            {"properties": {"static": {"$ref": "inner#item"}}},
            {"properties": {"dynamic": {"$dynamicRef": "inner#item"}}}
        ]}),
        &json!({"static": "a", "dynamic": null}),
        &json!({"dynamic": "a"});
        "ref before dynamic ref, dynamic side"
    )]
    #[test_case(
        &json!({"allOf": [
            {"properties": {"dynamic": {"$dynamicRef": "inner#item"}}},
            {"properties": {"static": {"$ref": "inner#item"}}}
        ]}),
        &json!({"static": "a", "dynamic": null}),
        &json!({"static": null});
        "dynamic ref before ref, static side"
    )]
    #[test_case(
        &json!({"allOf": [
            {"properties": {"dynamic": {"$dynamicRef": "inner#item"}}},
            {"properties": {"static": {"$ref": "inner#item"}}}
        ]}),
        &json!({"static": "a", "dynamic": null}),
        &json!({"dynamic": "a"});
        "dynamic ref before ref, dynamic side"
    )]
    fn ref_to_dynamic_anchor_ignores_dynamic_scope(root: &Value, valid: &Value, invalid: &Value) {
        let schema = same_name_dynamic_anchors(root);
        tests_util::is_valid(&schema, valid);
        tests_util::is_not_valid(&schema, invalid);
    }

    #[test_case("$ref", &json!({"inner": 1}), &json!({"outer": 1}); "ref, properties")]
    #[test_case("$ref", &json!([1, 2]), &json!([1, 2, 3]); "ref, items")]
    #[test_case("$dynamicRef", &json!({"outer": 1}), &json!({"inner": 1}); "dynamic ref, properties")]
    #[test_case("$dynamicRef", &json!([1]), &json!([1, 2]); "dynamic ref, items")]
    fn unevaluated_sees_what_the_reference_target_evaluates(
        keyword: &str,
        valid: &Value,
        invalid: &Value,
    ) {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/root",
            keyword: "inner#item",
            "unevaluatedProperties": false,
            "unevaluatedItems": false,
            "$defs": {
                "outer": {
                    "$dynamicAnchor": "item",
                    "properties": {"outer": true},
                    "prefixItems": [true]
                },
                "inner": {
                    "$id": "inner",
                    "$dynamicAnchor": "item",
                    "properties": {"inner": true},
                    "prefixItems": [true, true]
                }
            }
        });
        tests_util::is_valid(&schema, valid);
        tests_util::is_not_valid(&schema, invalid);
    }

    // `cycle` is a `$ref` target first reached with only `leaf` binding `node`; `wrap` enters the
    // dynamic scope on the way back to it and binds `node` from then on. `hop1` leads back through
    // enough nested `$ref`s for `cycle` to compile in a later round.
    fn dynamic_anchor_entering_scope_inside_ref_cycle(back_to_cycle: &str) -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/root",
            "$ref": "start",
            "$defs": {
                "start": {
                    "$id": "start",
                    "$ref": "#/$defs/cycle",
                    "$defs": {
                        "cycle": {
                            "properties": {"next": {"$ref": "wrap"}},
                            "additionalProperties": {"$dynamicRef": "leaf#node"}
                        },
                        "copy": {
                            "properties": {"next": {"$ref": "wrap"}},
                            "additionalProperties": {"$dynamicRef": "leaf#node"}
                        }
                    }
                },
                "leaf": {"$id": "leaf", "$dynamicAnchor": "node", "type": "integer"},
                "wrap": {
                    "$id": "wrap",
                    "$dynamicAnchor": "node",
                    "type": "object",
                    "$ref": back_to_cycle,
                    "$defs": {
                        "hop1": {"$ref": "#/$defs/hop2"},
                        "hop2": {"$ref": "#/$defs/hop3"},
                        "hop3": {"$ref": "#/$defs/hop4"},
                        "hop4": {"$ref": "#/$defs/hop5"},
                        "hop5": {"$ref": "#/$defs/hop6"},
                        "hop6": {"$ref": "#/$defs/hop7"},
                        "hop7": {"$ref": "#/$defs/hop8"},
                        "hop8": {"$ref": "start#/$defs/cycle"}
                    }
                }
            }
        })
    }

    #[test_case("start#/$defs/cycle", &json!({"a": 1}), true; "cycle, leaf binds before wrap")]
    #[test_case("start#/$defs/cycle", &json!({"a": {}}), false; "cycle, leaf rejects before wrap")]
    #[test_case("start#/$defs/cycle", &json!({"next": {"a": {}}}), true; "cycle, wrap binds")]
    #[test_case("start#/$defs/cycle", &json!({"next": {"a": 1}}), false; "cycle, wrap rejects")]
    #[test_case("start#/$defs/cycle", &json!({"next": {"next": {"a": 1}}}), false; "cycle, wrap rejects twice in")]
    #[test_case("#/$defs/hop1", &json!({"a": 1}), true; "later round, leaf binds before wrap")]
    #[test_case("#/$defs/hop1", &json!({"next": {"a": {}}}), true; "later round, wrap binds")]
    #[test_case("#/$defs/hop1", &json!({"next": {"a": 1}}), false; "later round, wrap rejects")]
    #[test_case("#/$defs/hop1", &json!({"next": {"next": {"a": 1}}}), false; "later round, wrap rejects twice in")]
    #[test_case("start#/$defs/copy", &json!({"a": 1}), true; "unrolled, leaf binds before wrap")]
    #[test_case("start#/$defs/copy", &json!({"a": {}}), false; "unrolled, leaf rejects before wrap")]
    #[test_case("start#/$defs/copy", &json!({"next": {"a": {}}}), true; "unrolled, wrap binds")]
    #[test_case("start#/$defs/copy", &json!({"next": {"a": 1}}), false; "unrolled, wrap rejects")]
    #[test_case("start#/$defs/copy", &json!({"next": {"next": {"a": 1}}}), false; "unrolled, wrap rejects twice in")]
    fn dynamic_anchor_bound_inside_ref_cycle(back_to_cycle: &str, instance: &Value, valid: bool) {
        let schema = dynamic_anchor_entering_scope_inside_ref_cycle(back_to_cycle);
        if valid {
            tests_util::is_valid(&schema, instance);
        } else {
            tests_util::is_not_valid(&schema, instance);
        }
    }

    // `caller` declares `b`, which no resource around it does. Its `$dynamicRef` lands in another
    // resource, below which `caller` stays in the dynamic scope, so `leaf`'s `$dynamicRef "#b"`
    // binds `caller`'s `b` rather than its own.
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/root", "$ref": "caller", "$defs": {"a": {"$dynamicAnchor": "a", "$ref": "leaf"}, "caller": {"$id": "caller", "$dynamicRef": "#a", "$defs": {"a": {"$dynamicAnchor": "a", "type": "null"}, "b": {"$dynamicAnchor": "b", "type": "string"}}}, "leaf": {"$id": "leaf", "$dynamicRef": "#b", "$defs": {"b": {"$dynamicAnchor": "b", "type": "integer"}}}}}),
        &json!("x"),
        &json!(1);
        "landed target holds a reference"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/root", "$ref": "caller", "$defs": {"a": {"$dynamicAnchor": "a", "$dynamicRef": "leaf#b"}, "caller": {"$id": "caller", "$dynamicRef": "#a", "$defs": {"a": {"$dynamicAnchor": "a", "type": "null"}, "b": {"$dynamicAnchor": "b", "type": "string"}}}, "leaf": {"$id": "leaf", "$defs": {"b": {"$dynamicAnchor": "b", "type": "integer"}}}}}),
        &json!("x"),
        &json!(1);
        "landed target holds a dynamic reference"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/root", "$ref": "middle", "$defs": {"middle": {"$id": "middle", "$ref": "caller", "$defs": {"a": {"$dynamicAnchor": "a", "$ref": "leaf"}}}, "caller": {"$id": "caller", "$dynamicRef": "#a", "$defs": {"a": {"$dynamicAnchor": "a", "type": "null"}, "b": {"$dynamicAnchor": "b", "type": "string"}}}, "leaf": {"$id": "leaf", "$dynamicRef": "#b", "$defs": {"b": {"$dynamicAnchor": "b", "type": "integer"}}}}}),
        &json!("x"),
        &json!(1);
        "lands in an intermediate resource"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/root", "$ref": "caller", "$defs": {"a": {"$dynamicAnchor": "a", "$ref": "leaf"}, "caller": {"$id": "caller", "$dynamicRef": "anchors#a", "$defs": {"b": {"$dynamicAnchor": "b", "type": "string"}}}, "anchors": {"$id": "anchors", "$defs": {"a": {"$dynamicAnchor": "a", "type": "null"}}}, "leaf": {"$id": "leaf", "$dynamicRef": "#b", "$defs": {"b": {"$dynamicAnchor": "b", "type": "integer"}}}}}),
        &json!("x"),
        &json!(1);
        "initial target in another resource"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/x", "$dynamicAnchor": "node", "properties": {"c": {"$id": "inner", "$dynamicAnchor": "node", "$defs": {"leaf": {"$dynamicAnchor": "leaf", "type": "number"}}, "properties": {"c": {"$dynamicRef": "#node"}}}, "d": {"$dynamicRef": "leaf#leaf"}}, "$defs": {"leaf": {"$id": "leaf", "$dynamicAnchor": "leaf", "type": "string"}}}),
        &json!({"c": {"c": {"d": 1}}}),
        &json!({"c": {"c": {"d": "s"}}});
        "caller entered inline"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/root", "$ref": "caller", "unevaluatedProperties": false, "$defs": {"a": {"$dynamicAnchor": "a", "$ref": "leaf"}, "caller": {"$id": "caller", "$dynamicRef": "#a", "$defs": {"a": {"$dynamicAnchor": "a", "type": "null"}, "b": {"$dynamicAnchor": "b", "properties": {"s": true}}}}, "leaf": {"$id": "leaf", "$dynamicRef": "#b", "$defs": {"b": {"$dynamicAnchor": "b", "properties": {"i": true}}}}}}),
        &json!({"s": 1}),
        &json!({"i": 1});
        "unevaluatedProperties"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/root", "unevaluatedProperties": false, "allOf": [{"$id": "caller", "$dynamicRef": "#a", "$defs": {"a": {"$dynamicAnchor": "a", "type": "null"}, "b": {"$dynamicAnchor": "b", "properties": {"s": true}}}}], "$defs": {"a": {"$dynamicAnchor": "a", "$ref": "leaf"}, "leaf": {"$id": "leaf", "$dynamicRef": "#b", "$defs": {"b": {"$dynamicAnchor": "b", "properties": {"i": true}}}}}}),
        &json!({"s": 1}),
        &json!({"i": 1});
        "unevaluatedProperties, caller entered inline"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/root", "$ref": "caller", "unevaluatedItems": false, "$defs": {"a": {"$dynamicAnchor": "a", "$ref": "leaf"}, "caller": {"$id": "caller", "$dynamicRef": "#a", "$defs": {"a": {"$dynamicAnchor": "a", "type": "null"}, "b": {"$dynamicAnchor": "b", "prefixItems": [true]}}}, "leaf": {"$id": "leaf", "$dynamicRef": "#b", "$defs": {"b": {"$dynamicAnchor": "b", "prefixItems": [true, true]}}}}}),
        &json!([1]),
        &json!([1, 2]);
        "unevaluatedItems"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/root", "unevaluatedItems": false, "allOf": [{"$id": "caller", "$dynamicRef": "#a", "$defs": {"a": {"$dynamicAnchor": "a", "type": "null"}, "b": {"$dynamicAnchor": "b", "prefixItems": [true]}}}], "$defs": {"a": {"$dynamicAnchor": "a", "$ref": "leaf"}, "leaf": {"$id": "leaf", "$dynamicRef": "#b", "$defs": {"b": {"$dynamicAnchor": "b", "prefixItems": [true, true]}}}}}),
        &json!([1]),
        &json!([1, 2]);
        "unevaluatedItems, caller entered inline"
    )]
    fn dynamic_ref_keeps_its_resource_in_dynamic_scope(
        schema: &Value,
        valid: &Value,
        invalid: &Value,
    ) {
        tests_util::is_valid(schema, valid);
        tests_util::is_not_valid(schema, invalid);
    }

    fn reported_error(error: &ValidationError<'_>) -> Value {
        json!({
            "message": error.to_string(),
            "instancePath": error.instance_path().as_str(),
            "schemaPath": error.schema_path().as_str(),
            "evaluationPath": error.evaluation_path().as_str(),
            "absoluteKeywordLocation": error.absolute_keyword_location().map(Uri::as_str),
        })
    }

    // What every validation mode reports for `instance`.
    fn reported(validator: &crate::Validator, instance: &Value) -> Value {
        json!({
            "isValid": validator.is_valid(instance),
            "validate": validator.validate(instance).err().as_ref().map(reported_error),
            "iterErrors": validator
                .iter_errors(instance)
                .map(|error| reported_error(&error))
                .collect::<Vec<_>>(),
            "evaluate": validator.evaluate(instance).list(),
        })
    }

    fn instances_for_ref_cycles() -> [Value; 7] {
        [
            json!({}),
            json!({"a": {}}),
            json!({"a": {"b": {}}}),
            json!({"a": {"x": 1}}),
            json!({"b": {}}),
            json!({"b": {"x": 1}}),
            json!(1),
        ]
    }

    // Every way of naming one schema is one node, so a `$ref` cycle through it reports what it
    // reports when every reference names it in one way.
    #[test_case(
        crate::Draft::Draft201909,
        &json!({"$id": "https://example.com/root", "$ref": "cycle", "$defs": {"cycle": {"$id": "cycle", "minProperties": 1, "allOf": [{"$ref": "#"}], "additionalProperties": {"$ref": "#"}}}}),
        &json!({"$id": "https://example.com/root", "$ref": "cycle", "$defs": {"cycle": {"$id": "cycle", "minProperties": 1, "allOf": [{"$ref": "cycle"}], "additionalProperties": {"$ref": "cycle"}}}})
        ; "resource and root fragment, draft 2019-09"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$id": "https://example.com/root", "$ref": "cycle#", "$defs": {"cycle": {"$id": "cycle", "minProperties": 1, "allOf": [{"$ref": "#"}], "additionalProperties": {"$ref": "cycle"}}}}),
        &json!({"$id": "https://example.com/root", "$ref": "cycle", "$defs": {"cycle": {"$id": "cycle", "minProperties": 1, "allOf": [{"$ref": "cycle"}], "additionalProperties": {"$ref": "cycle"}}}})
        ; "resource with empty fragment"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$id": "https://example.com/root", "$ref": "https://example.com/cycle", "$defs": {"cycle": {"$id": "cycle", "minProperties": 1, "allOf": [{"$ref": "cycle"}], "additionalProperties": {"$ref": "#"}}}}),
        &json!({"$id": "https://example.com/root", "$ref": "cycle", "$defs": {"cycle": {"$id": "cycle", "minProperties": 1, "allOf": [{"$ref": "cycle"}], "additionalProperties": {"$ref": "cycle"}}}})
        ; "absolute and relative uri"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$id": "https://example.com/root", "$ref": "cycle", "$defs": {"cycle": {"$id": "cycle", "$anchor": "node", "minProperties": 1, "allOf": [{"$ref": "#node"}], "additionalProperties": {"$ref": "#"}}}}),
        &json!({"$id": "https://example.com/root", "$ref": "cycle", "$defs": {"cycle": {"$id": "cycle", "$anchor": "node", "minProperties": 1, "allOf": [{"$ref": "cycle"}], "additionalProperties": {"$ref": "cycle"}}}})
        ; "anchor and root fragment"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$id": "https://example.com/root", "$ref": "cycle#node", "$defs": {"cycle": {"$id": "cycle", "$anchor": "node", "minProperties": 1, "allOf": [{"$ref": "#"}], "additionalProperties": {"$ref": "#node"}}}}),
        &json!({"$id": "https://example.com/root", "$ref": "cycle", "$defs": {"cycle": {"$id": "cycle", "$anchor": "node", "minProperties": 1, "allOf": [{"$ref": "cycle"}], "additionalProperties": {"$ref": "cycle"}}}})
        ; "anchor from another resource"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$ref": "#/$defs/cycle", "$defs": {"cycle": {"$anchor": "node", "minProperties": 1, "allOf": [{"$ref": "#node"}], "additionalProperties": {"$ref": "#/$defs/cycle"}}}}),
        &json!({"$ref": "#/$defs/cycle", "$defs": {"cycle": {"$anchor": "node", "minProperties": 1, "allOf": [{"$ref": "#/$defs/cycle"}], "additionalProperties": {"$ref": "#/$defs/cycle"}}}})
        ; "anchor and pointer"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$ref": "#/$defs/cycle", "$defs": {"cycle": {"minProperties": 1, "allOf": [{"$ref": "#/%24defs/cycle"}], "additionalProperties": {"$ref": "#/$defs/%63ycle"}}}}),
        &json!({"$ref": "#/$defs/cycle", "$defs": {"cycle": {"minProperties": 1, "allOf": [{"$ref": "#/$defs/cycle"}], "additionalProperties": {"$ref": "#/$defs/cycle"}}}})
        ; "percent-encoded pointer"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$ref": "#/$defs/a%23%25b", "$defs": {"a#%b": {"minProperties": 1, "allOf": [{"$ref": "#/$defs/a%23%25%62"}], "additionalProperties": {"$ref": "#/$defs/a%23%25b"}}}}),
        &json!({"$ref": "#/$defs/a%23%25b", "$defs": {"a#%b": {"minProperties": 1, "allOf": [{"$ref": "#/$defs/a%23%25b"}], "additionalProperties": {"$ref": "#/$defs/a%23%25b"}}}})
        ; "percent-encoded number sign and percent"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$id": "https://example.com/root", "minProperties": 1, "allOf": [{"$ref": "#"}], "additionalProperties": {"$ref": "https://example.com/root"}}),
        &json!({"$id": "https://example.com/root", "minProperties": 1, "allOf": [{"$ref": "#"}], "additionalProperties": {"$ref": "#"}})
        ; "root and its uri"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$id": "https://example.com/root", "$ref": "cycle", "$defs": {"cycle": {"$id": "cycle", "not": {"$ref": "#"}}}}),
        &json!({"$id": "https://example.com/root", "$ref": "cycle", "$defs": {"cycle": {"$id": "cycle", "not": {"$ref": "cycle"}}}})
        ; "negation of the cycle"
    )]
    #[test_case(
        crate::Draft::Draft7,
        &json!({"$id": "https://example.com/root", "allOf": [{"$ref": "cycle"}], "definitions": {"cycle": {"$id": "cycle", "minProperties": 1, "allOf": [{"$ref": "#"}], "additionalProperties": {"$ref": "#"}}}}),
        &json!({"$id": "https://example.com/root", "allOf": [{"$ref": "cycle"}], "definitions": {"cycle": {"$id": "cycle", "minProperties": 1, "allOf": [{"$ref": "cycle"}], "additionalProperties": {"$ref": "cycle"}}}})
        ; "resource and root fragment, draft 7"
    )]
    #[test_case(
        crate::Draft::Draft7,
        &json!({"$id": "https://example.com/root", "allOf": [{"$ref": "#node"}], "definitions": {"cycle": {"$id": "#node", "minProperties": 1, "allOf": [{"$ref": "#/definitions/cycle"}], "additionalProperties": {"$ref": "#node"}}}}),
        &json!({"$id": "https://example.com/root", "allOf": [{"$ref": "#/definitions/cycle"}], "definitions": {"cycle": {"$id": "#node", "minProperties": 1, "allOf": [{"$ref": "#/definitions/cycle"}], "additionalProperties": {"$ref": "#/definitions/cycle"}}}})
        ; "plain-name id and pointer, draft 7"
    )]
    #[test_case(
        crate::Draft::Draft6,
        &json!({"$id": "https://example.com/root", "allOf": [{"$ref": "cycle"}], "definitions": {"cycle": {"$id": "cycle", "minProperties": 1, "allOf": [{"$ref": "#"}], "additionalProperties": {"$ref": "https://example.com/cycle#"}}}}),
        &json!({"$id": "https://example.com/root", "allOf": [{"$ref": "cycle"}], "definitions": {"cycle": {"$id": "cycle", "minProperties": 1, "allOf": [{"$ref": "cycle"}], "additionalProperties": {"$ref": "cycle"}}}})
        ; "resource and root fragment, draft 6"
    )]
    #[test_case(
        crate::Draft::Draft4,
        &json!({"id": "https://example.com/root", "allOf": [{"$ref": "cycle"}], "definitions": {"cycle": {"id": "cycle", "minProperties": 1, "allOf": [{"$ref": "#"}], "additionalProperties": {"$ref": "#"}}}}),
        &json!({"id": "https://example.com/root", "allOf": [{"$ref": "cycle"}], "definitions": {"cycle": {"id": "cycle", "minProperties": 1, "allOf": [{"$ref": "cycle"}], "additionalProperties": {"$ref": "cycle"}}}})
        ; "resource and root fragment, draft 4"
    )]
    #[test_case(
        crate::Draft::Draft4,
        &json!({"id": "https://example.com/root", "allOf": [{"$ref": "#node"}], "definitions": {"cycle": {"id": "#node", "minProperties": 1, "allOf": [{"$ref": "https://example.com/root#/definitions/cycle"}], "additionalProperties": {"$ref": "#node"}}}}),
        &json!({"id": "https://example.com/root", "allOf": [{"$ref": "#/definitions/cycle"}], "definitions": {"cycle": {"id": "#node", "minProperties": 1, "allOf": [{"$ref": "#/definitions/cycle"}], "additionalProperties": {"$ref": "#/definitions/cycle"}}}})
        ; "plain-name id and pointer, draft 4"
    )]
    #[test_case(
        crate::Draft::Draft201909,
        &json!({"$id": "https://example.com/root", "$ref": "cycle", "$defs": {"cycle": {"$id": "cycle", "minProperties": 1, "allOf": [{"$recursiveRef": "#"}], "additionalProperties": {"$ref": "#"}}}}),
        &json!({"$id": "https://example.com/root", "$ref": "cycle#", "$defs": {"cycle": {"$id": "cycle", "minProperties": 1, "allOf": [{"$recursiveRef": "#"}], "additionalProperties": {"$ref": "#"}}}})
        ; "recursive reference without recursive anchor"
    )]
    #[test_case(
        crate::Draft::Draft201909,
        &json!({"$id": "https://example.com/root", "$ref": "tree", "$defs": {"tree": {"$id": "tree", "$recursiveAnchor": true, "minProperties": 1, "allOf": [{"$recursiveRef": "#"}], "additionalProperties": {"$recursiveRef": "#"}}}}),
        &json!({"$id": "https://example.com/root", "$ref": "tree#", "$defs": {"tree": {"$id": "tree", "$recursiveAnchor": true, "minProperties": 1, "allOf": [{"$recursiveRef": "#"}], "additionalProperties": {"$recursiveRef": "#"}}}})
        ; "recursive reference to its own recursive anchor"
    )]
    #[test_case(
        crate::Draft::Draft201909,
        &json!({"$id": "https://example.com/root", "$ref": "tree", "$defs": {"tree": {"$id": "tree", "$recursiveAnchor": true, "not": {"$recursiveRef": "#"}}}}),
        &json!({"$id": "https://example.com/root", "$ref": "tree#", "$defs": {"tree": {"$id": "tree", "$recursiveAnchor": true, "not": {"$recursiveRef": "#"}}}})
        ; "negation of a recursive reference to its own recursive anchor"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$id": "https://example.com/root", "$ref": "cycle", "$defs": {"cycle": {"$id": "cycle", "minProperties": 1, "allOf": [{"$dynamicRef": "#"}], "additionalProperties": {"$ref": "#"}}}}),
        &json!({"$id": "https://example.com/root", "$ref": "cycle#", "$defs": {"cycle": {"$id": "cycle", "minProperties": 1, "allOf": [{"$dynamicRef": "#"}], "additionalProperties": {"$ref": "#"}}}})
        ; "dynamic reference without fragment"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$id": "https://example.com/root", "$ref": "cycle", "$defs": {"cycle": {"$id": "cycle", "$anchor": "node", "minProperties": 1, "allOf": [{"$dynamicRef": "#node"}], "additionalProperties": {"$ref": "#"}}}}),
        &json!({"$id": "https://example.com/root", "$ref": "cycle#node", "$defs": {"cycle": {"$id": "cycle", "$anchor": "node", "minProperties": 1, "allOf": [{"$dynamicRef": "#node"}], "additionalProperties": {"$ref": "#node"}}}})
        ; "dynamic reference to an anchor"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$id": "https://example.com/root", "$dynamicRef": "tree", "$defs": {"tree": {"$id": "tree", "$dynamicAnchor": "node", "minProperties": 1, "allOf": [{"$dynamicRef": "#node"}], "additionalProperties": {"$dynamicRef": "#"}}}}),
        &json!({"$id": "https://example.com/root", "$dynamicRef": "tree#node", "$defs": {"tree": {"$id": "tree", "$dynamicAnchor": "node", "minProperties": 1, "allOf": [{"$dynamicRef": "#node"}], "additionalProperties": {"$dynamicRef": "#node"}}}})
        ; "dynamic reference to its own dynamic anchor"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$id": "https://example.com/root", "$dynamicRef": "tree", "$defs": {"tree": {"$id": "tree", "$dynamicAnchor": "node", "not": {"$dynamicRef": "#node"}}}}),
        &json!({"$id": "https://example.com/root", "$dynamicRef": "tree#node", "$defs": {"tree": {"$id": "tree", "$dynamicAnchor": "node", "not": {"$dynamicRef": "#node"}}}})
        ; "negation of a dynamic reference to its own dynamic anchor"
    )]
    fn ref_target_named_in_different_ways(
        draft: crate::Draft,
        schema: &Value,
        named_one_way: &Value,
    ) {
        let options = crate::options().with_draft(draft);
        let validator = options.build(schema).expect("Valid schema");
        let expected = options.build(named_one_way).expect("Valid schema");
        for instance in instances_for_ref_cycles() {
            assert_eq!(
                reported(&validator, &instance),
                reported(&expected, &instance),
                "{instance}"
            );
        }
    }

    // A pointer from the outer document into an embedded resource and `#` inside that resource name
    // one schema at two locations, and `#/` names the member with an empty name: each is a node of
    // its own.
    #[test_case(
        &json!({"$id": "https://example.com/root", "$ref": "#/$defs/cycle", "$defs": {"cycle": {"$id": "cycle", "minProperties": 1, "allOf": [{"$ref": "#"}], "additionalProperties": {"$ref": "root#/$defs/cycle"}}}}),
        &json!({}),
        &json!([
            {"message": "{} has less than 1 property", "instancePath": "", "schemaPath": "/$defs/cycle/minProperties", "evaluationPath": "/$ref/minProperties", "absoluteKeywordLocation": "https://example.com/cycle#/minProperties"},
            {"message": "{} has less than 1 property", "instancePath": "", "schemaPath": "/minProperties", "evaluationPath": "/$ref/allOf/0/$ref/minProperties", "absoluteKeywordLocation": "https://example.com/cycle#/minProperties"},
            {"message": "{} has less than 1 property", "instancePath": "", "schemaPath": "/minProperties", "evaluationPath": "/$ref/allOf/0/$ref/allOf/0/$ref/minProperties", "absoluteKeywordLocation": "https://example.com/cycle#/minProperties"}
        ])
        ; "pointer into an embedded resource and its root fragment"
    )]
    #[test_case(
        &json!({"$id": "https://example.com/root", "": {"maxProperties": 0}, "minProperties": 1, "allOf": [{"$ref": "#"}], "additionalProperties": {"$ref": "#/"}}),
        &json!({"a": {"b": 1}}),
        &json!([
            {"message": "{\"b\":1} has more than 0 properties", "instancePath": "/a", "schemaPath": "//maxProperties", "evaluationPath": "/additionalProperties/$ref/maxProperties", "absoluteKeywordLocation": "https://example.com/root#//maxProperties"},
            {"message": "{\"b\":1} has more than 0 properties", "instancePath": "/a", "schemaPath": "//maxProperties", "evaluationPath": "/allOf/0/$ref/additionalProperties/$ref/maxProperties", "absoluteKeywordLocation": "https://example.com/root#//maxProperties"}
        ])
        ; "root fragment and member with an empty name"
    )]
    fn ref_targets_at_different_locations_stay_apart(
        schema: &Value,
        instance: &Value,
        expected: &Value,
    ) {
        let validator = crate::validator_for(schema).expect("Valid schema");
        let errors: Vec<Value> = validator
            .iter_errors(instance)
            .map(|error| reported_error(&error))
            .collect();
        assert_eq!(json!(errors), *expected);
    }

    // A subschema applied inline and as a `$ref` target is one node, whose errors report the
    // evaluation path of the route that reached it.
    #[test_case(
        &json!({"$id": "http://example.com/a.json", "properties": {"foo": {"$id": "http://example.com/b.json", "definitions": {"inner": {"properties": {"bar": {"type": "string"}}}}, "allOf": [{"$ref": "#/definitions/inner"}]}}, "$ref": "#/properties/foo/allOf/0"}),
        &json!([
            {"message": "1 is not of type \"string\"", "instancePath": "/foo/bar", "schemaPath": "/definitions/inner/properties/bar/type", "evaluationPath": "/properties/foo/allOf/0/$ref/properties/bar/type", "absoluteKeywordLocation": "http://example.com/b.json#/definitions/inner/properties/bar/type"},
            {"message": "1 is not of type \"string\"", "instancePath": "/bar", "schemaPath": "/definitions/inner/properties/bar/type", "evaluationPath": "/$ref/$ref/properties/bar/type", "absoluteKeywordLocation": "http://example.com/b.json#/definitions/inner/properties/bar/type"}
        ])
        ; "pointer into an embedded resource"
    )]
    #[test_case(
        &json!({"$schema": "http://json-schema.org/draft-07/schema#", "$id": "http://example.com/a.json", "properties": {"foo": {"$id": "http://example.com/b.json", "definitions": {"inner": {"properties": {"bar": {"type": "string"}}}}, "allOf": [{"$ref": "#/definitions/inner"}]}}, "allOf": [{"$ref": "#/properties/foo/allOf/0"}]}),
        &json!([
            {"message": "1 is not of type \"string\"", "instancePath": "/foo/bar", "schemaPath": "/definitions/inner/properties/bar/type", "evaluationPath": "/properties/foo/allOf/0/$ref/properties/bar/type", "absoluteKeywordLocation": "http://example.com/b.json#/definitions/inner/properties/bar/type"},
            {"message": "1 is not of type \"string\"", "instancePath": "/bar", "schemaPath": "/definitions/inner/properties/bar/type", "evaluationPath": "/allOf/0/$ref/$ref/properties/bar/type", "absoluteKeywordLocation": "http://example.com/b.json#/definitions/inner/properties/bar/type"}
        ])
        ; "pointer into an embedded resource in Draft 7"
    )]
    #[test_case(
        &json!({"$ref": "#/$defs/a", "$defs": {"a": {"$ref": "#/$defs/a/properties/foo", "properties": {"foo": {"properties": {"bar": {"$ref": "#/$defs/s"}}}}}, "s": {"type": "string"}}}),
        &json!([
            {"message": "1 is not of type \"string\"", "instancePath": "/foo/bar", "schemaPath": "/$defs/s/type", "evaluationPath": "/$ref/properties/foo/properties/bar/$ref/type", "absoluteKeywordLocation": null},
            {"message": "1 is not of type \"string\"", "instancePath": "/bar", "schemaPath": "/$defs/s/type", "evaluationPath": "/$ref/$ref/properties/bar/$ref/type", "absoluteKeywordLocation": null}
        ])
        ; "pointer below a reference target"
    )]
    #[test_case(
        &json!({"$ref": "#/$defs/a", "$defs": {"a": {"allOf": [{"properties": {"bar": {"$ref": "#/$defs/s"}}}], "properties": {"foo": {"$ref": "#/$defs/a/allOf/0"}}}, "s": {"type": "string"}}}),
        &json!([
            {"message": "1 is not of type \"string\"", "instancePath": "/foo/bar", "schemaPath": "/$defs/s/type", "evaluationPath": "/$ref/properties/foo/$ref/properties/bar/$ref/type", "absoluteKeywordLocation": null},
            {"message": "1 is not of type \"string\"", "instancePath": "/bar", "schemaPath": "/$defs/s/type", "evaluationPath": "/$ref/allOf/0/properties/bar/$ref/type", "absoluteKeywordLocation": null}
        ])
        ; "pointer to a subschema applied inline first"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "type": "object", "$ref": "#/$defs/a", "$defs": {"a": {"$ref": "#/$defs/a/properties/foo", "properties": {"foo": {"properties": {"bar": {"$recursiveRef": "#"}}}}}}}),
        &json!([
            {"message": "1 is not of type \"object\"", "instancePath": "/foo/bar", "schemaPath": "/type", "evaluationPath": "/$ref/properties/foo/properties/bar/$recursiveRef/type", "absoluteKeywordLocation": null},
            {"message": "1 is not of type \"object\"", "instancePath": "/bar", "schemaPath": "/type", "evaluationPath": "/$ref/$ref/properties/bar/$recursiveRef/type", "absoluteKeywordLocation": null}
        ])
        ; "recursive reference below a reference target"
    )]
    fn subschema_applied_inline_and_by_reference_reports_each_route(
        schema: &Value,
        expected: &Value,
    ) {
        let validator = crate::validator_for(schema).expect("Valid schema");
        let instance = json!({"bar": 1, "foo": {"bar": 1}});
        let errors: Vec<Value> = validator
            .iter_errors(&instance)
            .map(|error| reported_error(&error))
            .collect();
        assert_eq!(json!(errors), *expected);
        assert_eq!(
            validator
                .validate(&instance)
                .err()
                .as_ref()
                .map(reported_error),
            expected.get(0).cloned()
        );
        let evaluation =
            serde_json::to_value(validator.evaluate(&instance).list()).expect("Serializable");
        let evaluated: Vec<Value> = evaluation["details"]
            .as_array()
            .expect("List output has details")
            .iter()
            .filter(|unit| unit.get("errors").is_some())
            .map(|unit| json!([unit["instanceLocation"], unit["evaluationPath"]]))
            .collect();
        let routes: Vec<Value> = expected
            .as_array()
            .expect("Errors are a list")
            .iter()
            .map(|error| json!([error["instancePath"], error["evaluationPath"]]))
            .collect();
        assert_eq!(evaluated, routes);
    }

    const SELF_REFERENCING_ID: &str = "https://example.com/x";

    fn definitions_keyword(draft: crate::Draft) -> &'static str {
        if draft < crate::Draft::Draft201909 {
            "definitions"
        } else {
            "$defs"
        }
    }

    fn with_id(schema: &Value, draft: crate::Draft) -> Value {
        let keyword = if draft == crate::Draft::Draft4 {
            "id"
        } else {
            "$id"
        };
        let mut schema = schema.clone();
        schema
            .as_object_mut()
            .expect("An object schema")
            .insert(keyword.to_string(), json!(SELF_REFERENCING_ID));
        schema
    }

    // `schema` with every `$ref` naming its root by `#` naming it by `pointer`.
    fn naming_root_by(schema: &Value, pointer: &str) -> Value {
        match schema {
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .map(|(keyword, value)| {
                        let value = if keyword == "$ref" && value == "#" {
                            json!(pointer)
                        } else {
                            naming_root_by(value, pointer)
                        };
                        (keyword.clone(), value)
                    })
                    .collect(),
            ),
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|item| naming_root_by(item, pointer))
                    .collect(),
            ),
            _ => schema.clone(),
        }
    }

    // `value` without the `definition` that schema paths start from and the `$ref` that evaluation
    // paths start from.
    fn strip_wrapper(value: &mut Value, definition: &str) {
        match value {
            Value::Object(object) => {
                for (keyword, value) in object.iter_mut() {
                    let prefix = match keyword.as_str() {
                        "schemaPath" | "schemaLocation" => definition,
                        "evaluationPath" => "/$ref",
                        _ => {
                            strip_wrapper(value, definition);
                            continue;
                        }
                    };
                    if let Some(rest) = value.as_str().and_then(|path| path.strip_prefix(prefix)) {
                        *value = json!(rest);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    strip_wrapper(item, definition);
                }
            }
            _ => {}
        }
    }

    // `reported` of a root whose `$ref` reaches the schema at `/{definitions}/x`, located as if that
    // schema were the root: the root's own unit goes, and the `$ref` and the definition leave every
    // path.
    fn located_as_root(mut reported: Value, definitions: &str) -> Value {
        reported["evaluate"]["details"]
            .as_array_mut()
            .expect("List output details")
            .retain(|unit| unit["evaluationPath"] != "");
        strip_wrapper(&mut reported, &format!("/{definitions}/x"));
        reported
    }

    fn assert_reported_alike(draft: crate::Draft, root: &Value, wrapped: &Value) {
        let options = crate::options().with_draft(draft);
        let as_root = options.build(root).expect("Valid schema");
        let as_ref_target = options.build(wrapped).expect("Valid schema");
        for instance in [
            json!({}),
            json!({"a": 1, "b": 2}),
            json!({"c": {"c": 1}}),
            json!({"c": {"c": {}}}),
            json!(0),
            json!(1),
        ] {
            assert_eq!(
                reported(&as_root, &instance),
                located_as_root(
                    reported(&as_ref_target, &instance),
                    definitions_keyword(draft)
                ),
                "{draft:?} {root} {instance}"
            );
        }
    }

    // The root is the node a `$ref` to it names, so a schema reports the same as the root and below
    // a root that only references it, by pointer or by its `$id`.
    #[test_case(&json!({"not": {"$ref": "#"}}); "under not")]
    #[test_case(&json!({"minProperties": 2, "allOf": [{"$ref": "#"}]}); "from allOf")]
    #[test_case(
        &json!({"type": "object", "properties": {"c": {"$ref": "#"}}, "patternProperties": {"^c": {"$ref": "#"}}, "allOf": [{"$ref": "#"}]});
        "from allOf and two keywords for one member"
    )]
    #[test_case(
        &json!({"$dynamicAnchor": "node", "minProperties": 2, "allOf": [{"$ref": "#"}]});
        "from allOf beside a dynamic anchor"
    )]
    #[test_case(&json!({"$dynamicAnchor": "node", "not": {"$ref": "#"}}); "under not beside a dynamic anchor")]
    fn root_referencing_itself_reports_as_a_ref_target(schema: &Value) {
        for draft in [
            crate::Draft::Draft4,
            crate::Draft::Draft7,
            crate::Draft::Draft201909,
            crate::Draft::Draft202012,
        ] {
            let definitions = definitions_keyword(draft);
            let pointer = format!("#/{definitions}/x");
            assert_reported_alike(
                draft,
                schema,
                &json!({"$ref": pointer, definitions: {"x": naming_root_by(schema, &pointer)}}),
            );
            let identified = with_id(schema, draft);
            assert_reported_alike(
                draft,
                &identified,
                &json!({"$ref": SELF_REFERENCING_ID, definitions: {"x": identified}}),
            );
        }
    }

    #[test_case(
        crate::Draft::Draft201909,
        &json!({"$recursiveAnchor": true, "minimum": 1, "allOf": [{"$recursiveRef": "#"}]});
        "recursive reference from allOf"
    )]
    #[test_case(
        crate::Draft::Draft201909,
        &json!({"$recursiveAnchor": true, "not": {"$recursiveRef": "#"}});
        "recursive reference under not"
    )]
    #[test_case(
        crate::Draft::Draft201909,
        &json!({"$recursiveAnchor": true, "type": "object", "properties": {"c": {"$recursiveRef": "#"}}, "patternProperties": {"^c": {"$recursiveRef": "#"}}, "allOf": [{"$recursiveRef": "#"}]});
        "recursive reference from allOf and two keywords for one member"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$dynamicAnchor": "node", "minimum": 1, "allOf": [{"$dynamicRef": "#node"}]});
        "dynamic reference from allOf"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$dynamicAnchor": "node", "not": {"$dynamicRef": "#node"}});
        "dynamic reference under not"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$dynamicAnchor": "node", "type": "object", "properties": {"c": {"$dynamicRef": "#node"}}, "patternProperties": {"^c": {"$dynamicRef": "#node"}}, "allOf": [{"$dynamicRef": "#node"}]});
        "dynamic reference from allOf and two keywords for one member"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$ref": "#", "minimum": 1, "allOf": [{"$ref": "#"}]});
        "from allOf beside a reference to itself"
    )]
    #[test_case(
        crate::Draft::Draft201909,
        &json!({"$recursiveAnchor": true, "type": "object", "properties": {"c": {"$id": "inner", "$recursiveAnchor": true, "properties": {"c": {"$recursiveRef": "#"}}}}});
        "recursive reference from an embedded resource"
    )]
    #[test_case(
        crate::Draft::Draft202012,
        &json!({"$dynamicAnchor": "node", "type": "object", "properties": {"c": {"$id": "inner", "$dynamicAnchor": "node", "properties": {"c": {"$dynamicRef": "#node"}}}}});
        "dynamic reference from an embedded resource"
    )]
    fn root_with_id_referencing_itself_reports_as_a_ref_target(
        draft: crate::Draft,
        schema: &Value,
    ) {
        let identified = with_id(schema, draft);
        assert_reported_alike(
            draft,
            &identified,
            &json!({"$ref": SELF_REFERENCING_ID, "$defs": {"x": identified}}),
        );
    }

    fn not_an_object_at(instance_path: &str, evaluation_path: &str) -> Value {
        json!([{
            "message": "1 is not of type \"object\"",
            "instancePath": instance_path,
            "schemaPath": "/type",
            "evaluationPath": evaluation_path,
            "absoluteKeywordLocation": "https://example.com/x#/type",
        }])
    }

    // A resource `x` holds stays in the dynamic scope below it, entered inline or by a pointer, so
    // a reference there lands on `x`, the outermost resource with the anchor.
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/x", "$recursiveAnchor": true, "type": "object", "properties": {"c": {"$id": "inner", "$recursiveAnchor": true, "properties": {"c": {"$recursiveRef": "#"}}}}}),
        &json!({"c": {"c": 1}}),
        &not_an_object_at("/c/c", "/properties/c/properties/c/$recursiveRef/type");
        "recursive reference"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/x", "$recursiveAnchor": true, "type": "object", "properties": {"c": {"$id": "inner", "$recursiveAnchor": true, "properties": {"c": {"$recursiveRef": "#"}}}}}),
        &json!({"c": {"c": {"c": 1}}}),
        &json!([]);
        "recursive reference, valid"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$ref": "https://example.com/x", "$defs": {"x": {"$id": "https://example.com/x", "$recursiveAnchor": true, "type": "object", "properties": {"c": {"$id": "inner", "$recursiveAnchor": true, "properties": {"c": {"$recursiveRef": "#"}}}}}}}),
        &json!({"c": {"c": 1}}),
        &not_an_object_at("/c/c", "/$ref/properties/c/properties/c/$recursiveRef/type");
        "recursive reference, resource reached by reference"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/x", "$dynamicAnchor": "node", "type": "object", "properties": {"c": {"$id": "inner", "$dynamicAnchor": "node", "properties": {"c": {"$dynamicRef": "#node"}}}}}),
        &json!({"c": {"c": 1}}),
        &not_an_object_at("/c/c", "/properties/c/properties/c/$dynamicRef/type");
        "dynamic reference"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/x", "$dynamicAnchor": "node", "type": "object", "properties": {"c": {"$id": "inner", "$dynamicAnchor": "node", "properties": {"c": {"$dynamicRef": "#node"}}}}}),
        &json!({"c": {"c": {"c": 1}}}),
        &json!([]);
        "dynamic reference, valid"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$ref": "https://example.com/x", "$defs": {"x": {"$id": "https://example.com/x", "$dynamicAnchor": "node", "type": "object", "properties": {"c": {"$id": "inner", "$dynamicAnchor": "node", "properties": {"c": {"$dynamicRef": "#node"}}}}}}}),
        &json!({"c": {"c": 1}}),
        &not_an_object_at("/c/c", "/$ref/properties/c/properties/c/$dynamicRef/type");
        "dynamic reference, resource reached by reference"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/x", "$dynamicAnchor": "node", "type": "object", "properties": {"a": {"$id": "middle", "properties": {"b": {"$id": "inner", "$dynamicAnchor": "node", "properties": {"c": {"$dynamicRef": "#node"}}}}}}}),
        &json!({"a": {"b": {"c": 1}}}),
        &not_an_object_at("/a/b/c", "/properties/a/properties/b/properties/c/$dynamicRef/type");
        "dynamic reference past a resource without the anchor"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/x", "$dynamicAnchor": "node", "type": "object", "properties": {"c": {"$id": "middle", "$ref": "leaf"}}, "$defs": {"leaf": {"$id": "leaf", "$dynamicAnchor": "node", "properties": {"c": {"$dynamicRef": "#node"}}}}}),
        &json!({"c": {"c": 1}}),
        &not_an_object_at("/c/c", "/properties/c/$ref/properties/c/$dynamicRef/type");
        "dynamic reference, resource held inline then a reference"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$id": "https://example.com/w", "$ref": "x", "$defs": {"x": {"$id": "x", "$recursiveAnchor": true, "type": "object", "properties": {"c": {"$ref": "#/$defs/inner"}}, "$defs": {"inner": {"$id": "inner", "$recursiveAnchor": true, "properties": {"c": {"$recursiveRef": "#"}}}}}}}),
        &json!({"c": {"c": 1}}),
        &not_an_object_at("/c/c", "/$ref/properties/c/$ref/properties/c/$recursiveRef/type");
        "recursive reference, resource reached by a pointer"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/w", "$ref": "x", "$defs": {"x": {"$id": "x", "$dynamicAnchor": "node", "type": "object", "properties": {"c": {"$ref": "#/$defs/inner"}}, "$defs": {"inner": {"$id": "inner", "$dynamicAnchor": "node", "properties": {"c": {"$dynamicRef": "#node"}}}}}}}),
        &json!({"c": {"c": 1}}),
        &not_an_object_at("/c/c", "/$ref/properties/c/$ref/properties/c/$dynamicRef/type");
        "dynamic reference, resource reached by a pointer"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/x", "$dynamicAnchor": "node", "type": "object", "properties": {"c": {"items": {"$id": "inner", "$ref": "middle"}}}, "$defs": {"middle": {"$id": "middle", "$dynamicRef": "leaf#node"}, "leaf": {"$id": "leaf", "$dynamicAnchor": "node"}}}),
        &json!({"c": [1]}),
        &not_an_object_at("/c/0", "/properties/c/items/$ref/$dynamicRef/type");
        "dynamic reference behind a reference from a resource under items"
    )]
    #[test_case(
        &json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$id": "https://example.com/x", "$dynamicAnchor": "node", "type": "object", "unevaluatedProperties": {"$id": "inner", "$dynamicRef": "middle#node"}, "$defs": {"middle": {"$id": "middle", "$dynamicAnchor": "node"}}}),
        &json!({"a": 1}),
        &json!([{
            "message": "Unevaluated properties are not allowed ('a' was unexpected)",
            "instancePath": "",
            "schemaPath": "/unevaluatedProperties",
            "evaluationPath": "/unevaluatedProperties",
            "absoluteKeywordLocation": "https://example.com/x#/unevaluatedProperties",
        }]);
        "dynamic reference in a resource under unevaluatedProperties"
    )]
    fn embedded_resource_keeps_the_enclosing_one_in_dynamic_scope(
        schema: &Value,
        instance: &Value,
        expected: &Value,
    ) {
        let validator = crate::validator_for(schema).expect("Valid schema");
        let errors: Vec<_> = validator
            .iter_errors(instance)
            .map(|error| reported_error(&error))
            .collect();
        assert_eq!(json!(errors), *expected);
        assert_eq!(
            validator
                .validate(instance)
                .err()
                .as_ref()
                .map(reported_error),
            expected.get(0).cloned()
        );
        assert_eq!(validator.is_valid(instance), errors.is_empty());
    }

    // A `$ref` from outside `x` to a resource `x` holds inline leaves `x` out of the dynamic scope.
    #[test_case(&json!({"d": {"c": 1}}); "number")]
    #[test_case(&json!({"d": {"c": {"c": 1}}}); "nested")]
    fn embedded_resource_reached_from_outside_its_holder(instance: &Value) {
        let validator = crate::validator_for(&json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/w",
            "properties": {"d": {"$ref": "inner"}},
            "$defs": {"x": {
                "$id": "x",
                "$dynamicAnchor": "node",
                "type": "object",
                "properties": {"c": {
                    "$id": "inner",
                    "$dynamicAnchor": "node",
                    "properties": {"c": {"$dynamicRef": "#node"}}
                }}
            }}
        }))
        .expect("Valid schema");
        assert_eq!(validator.iter_errors(instance).count(), 0);
        assert!(validator.is_valid(instance));
    }

    // A root registered under another URI than its `$id` is one resource, which a `$dynamicRef`
    // into it locates by its `$id`.
    #[test]
    fn dynamic_anchor_in_a_root_registered_under_another_uri() {
        let validator = crate::options()
            .with_base_uri("https://example.com/retrieved")
            .build(&json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$id": "https://example.com/x",
                "properties": {"c": {"$dynamicRef": "#node"}},
                "$defs": {"n": {"$dynamicAnchor": "node", "type": "object"}}
            }))
            .expect("Valid schema");
        let errors: Vec<_> = validator
            .iter_errors(&json!({"c": 1}))
            .map(|error| reported_error(&error))
            .collect();
        assert_eq!(
            json!(errors),
            json!([{
                "message": "1 is not of type \"object\"",
                "instancePath": "/c",
                "schemaPath": "/$defs/n/type",
                "evaluationPath": "/properties/c/$dynamicRef/type",
                "absoluteKeywordLocation": "https://example.com/x#/$defs/n/type",
            }])
        );
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
