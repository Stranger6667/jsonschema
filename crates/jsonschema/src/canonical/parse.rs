//! JSON Schema -> IR parser.
//!
//! In-document `$ref` targets inline; refs on a guarded cycle become `Recursive(name)`, unresolved externals
//! `Reference(uri)`. Unguarded recursion (cycles through composition keywords only) has no fixed point and is rejected.

#![allow(
    clippy::unnecessary_fallible_conversions,
    reason = "`try_from(f64)` is infallible without `arbitrary-precision`, genuinely fallible with it."
)]

use std::{borrow::Cow, str::FromStr, sync::Arc};

use ahash::{AHashMap, AHashSet};
use num_traits::One;
#[cfg(not(feature = "arbitrary-precision"))]
use num_traits::ToPrimitive;
use referencing::{Draft, Resolver};
use serde_json::{Map, Number, Value};

#[cfg(feature = "arbitrary-precision")]
use crate::ext::numeric::bignum::{try_parse_bigfraction, try_parse_bigint};
use crate::{
    canonical::{
        document::{
            contains_dynamic_scope_ref, raw_subschema, requires_opaque_preservation,
            validate_schema_document,
        },
        error::CanonicalizationError,
        intern::shared,
        ir::{
            ArrayLeaf, BooleanBounds, BoundCardinality, BoundFraction, BoundInteger, CanonicalJson,
            ContainsClause, ContentFacet, IfThenElse, IntegerLeaf, LengthBounds, NumberBounds,
            NumberLeaf, ObjectConstraint, ObjectLeaf, ObjectRequirement, OneOf,
            PropertyNameMatcher, Schema, SchemaNode, SharedSchema, StringLeaf,
        },
        numeric::{number_bounds_to_integer, number_multiple_of_to_integer},
        recursion::{check_infinite_recursion, check_unguarded_recursion},
    },
    options::PatternEngineOptions,
    JsonType, JsonTypeSet,
};

#[cfg(not(feature = "arbitrary-precision"))]
const MAX_EXACT_F64_INTEGER: f64 = 9_007_199_254_740_992.0;

/// Root IR plus the symbolic-reference definitions discovered during parse.
#[derive(Debug)]
pub(crate) struct ParseOutput {
    pub(crate) root: SharedSchema,
    /// Reference uri -> definition body (pre-pipeline IR; the caller canonicalizes each). Bodies may themselves
    /// hold `Reference`/`Recursive` leaves pointing at other keys.
    pub(crate) definitions: AHashMap<Arc<str>, SharedSchema>,
}

pub(crate) fn parse_graph(
    value: &Value,
    draft: Draft,
    resolver: Option<Resolver<'_>>,
    inline_budget: usize,
    pattern_options: PatternEngineOptions,
) -> Result<ParseOutput, CanonicalizationError> {
    let mut ctx = ParseContext::new(value, draft, resolver, inline_budget, false);
    if ctx.has_refs {
        check_unguarded_recursion(value, ctx.draft, ctx.resolver.as_ref())?;
    }
    let root_key = ValueIdentity::of(value);
    ctx.local_memo.in_progress.insert(root_key);
    let root = parse_inner(value, &mut ctx);
    ctx.local_memo.in_progress.remove(&root_key);
    let root = root?;
    // The root frame is `parse_graph`, which never calls `register_definition`, so a `#`-style self-ref cycling back
    // to the root left its key in `cyclic` with no body. Register the root body for those keys.
    for key in std::mem::take(&mut ctx.root_self_refs) {
        ctx.cross
            .definitions
            .entry(key)
            .or_insert_with(|| Arc::clone(&root));
    }
    if !ctx.cross.cyclic.is_empty() {
        check_infinite_recursion(
            &root,
            &ctx.cross.definitions,
            &ctx.cross.cyclic,
            pattern_options,
        )?;
    }
    Ok(ParseOutput {
        root,
        definitions: std::mem::take(&mut ctx.cross.definitions),
    })
}

/// Address-based identity of a resolved `&Value` target, so `#` under a nested `$id` scope does not collide with the
/// root's `#`, and the same relative ref under different bases keys differently.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct ValueIdentity(usize);

impl ValueIdentity {
    pub(super) fn of(value: &Value) -> Self {
        Self(std::ptr::from_ref::<Value>(value) as usize)
    }
}

/// Parsed-body cache plus the in-flight set that turns a re-entrant parse into a [`Schema::Recursive`] leaf.
struct CycleMemo<K> {
    cache: AHashMap<K, SharedSchema>,
    in_progress: AHashSet<K>,
}

impl<K> Default for CycleMemo<K> {
    fn default() -> Self {
        Self {
            cache: AHashMap::new(),
            in_progress: AHashSet::new(),
        }
    }
}

/// State shared across documents: external targets resolve into the same memo, and cycles/definitions found in any
/// document are recorded against the root parse.
#[derive(Default)]
struct CrossDocumentState {
    external_memo: CycleMemo<ValueIdentity>,
    /// Reference keys hit while in progress: they sit on a cycle and produced a [`Schema::Recursive`] leaf. Their
    /// bodies are recorded in `definitions` so emit and `definitions()` resolve them.
    cyclic: AHashSet<Arc<str>>,
    /// Reference uri -> definition body, for every symbolic (`Recursive`/over-budget `Reference`) target. Built during parse.
    definitions: AHashMap<Arc<str>, SharedSchema>,
}

struct ParseContext<'a, 'r> {
    draft: Draft,
    /// Canonical fragment (`#/$defs/foo`) -> definition body.
    defs: AHashMap<Arc<str>, &'a Value>,
    definition_memo: CycleMemo<Arc<str>>,
    local_memo: CycleMemo<ValueIdentity>,
    has_refs: bool,
    resolver: Option<Resolver<'r>>,
    /// Reference keys that cycled back to the root (`#`-style self-refs); the root body is registered for them after
    /// parse, since the root's own frame never calls [`register_definition`].
    root_self_refs: AHashSet<Arc<str>>,
    /// Identity of the document root, to recognize self-references in [`inline_local_target`].
    root_key: ValueIdentity,
    /// Whether the resolver already sits at the root's own `$id` scope (true for `lookup`-created contexts), else a
    /// relative root `$id` resolves against its own base twice.
    root_scope_established: bool,
    cross: CrossDocumentState,
    /// Inline a resolvable acyclic ref only when its parsed node-count fits this budget, else keep it symbolic.
    /// `usize::MAX` inlines everything.
    inline_budget: usize,
    /// Nested `$ref` resolutions on the parse stack; bounds chain length so a long chain can't overflow.
    ref_depth: usize,
}

/// Maximum nested `$ref` resolutions; a chain past this is preserved verbatim rather than recursed into.
const MAX_REF_DEPTH: usize = 128;

impl<'a, 'r> ParseContext<'a, 'r> {
    fn new(
        value: &'a Value,
        draft: Draft,
        resolver: Option<Resolver<'r>>,
        inline_budget: usize,
        root_scope_established: bool,
    ) -> Self {
        // `draft` is already resolved by the caller: explicit override or detected at the root,
        // registry-resolved for external documents. Re-detecting would defeat the override.
        let mut defs: AHashMap<Arc<str>, &'a Value> = AHashMap::new();
        if let Value::Object(root) = value {
            for (registry, prefix) in [("$defs", "#/$defs/"), ("definitions", "#/definitions/")] {
                if let Some(Value::Object(entries)) = root.get(registry) {
                    for (name, schema) in entries {
                        // RFC 6901 escaping: `~` -> `~0`, `/` -> `~1`.
                        let key: Arc<str> = build_def_pointer(prefix, name).into();
                        defs.insert(key, schema);
                    }
                }
            }
        }
        Self {
            draft,
            defs,
            definition_memo: CycleMemo::default(),
            local_memo: CycleMemo::default(),
            has_refs: contains_ref(value),
            resolver,
            root_self_refs: AHashSet::new(),
            root_key: ValueIdentity::of(value),
            root_scope_established,
            cross: CrossDocumentState::default(),
            inline_budget,
            ref_depth: 0,
        }
    }

    /// Read a keyword only when the active draft recognizes it.
    fn get<'m>(&self, map: &'m Map<String, Value>, key: &str) -> Option<&'m Value> {
        if self.draft.is_known_keyword(key) {
            map.get(key)
        } else {
            None
        }
    }
}

fn parse_inner(
    value: &Value,
    ctx: &mut ParseContext<'_, '_>,
) -> Result<SharedSchema, CanonicalizationError> {
    match value {
        Value::Bool(true) => Ok(shared(Schema::True)),
        Value::Bool(false) => Ok(shared(Schema::False)),
        Value::Object(map) if map.is_empty() => Ok(shared(Schema::True)),
        Value::Object(map) => parse_object_schema(value, map, ctx),
        other => Err(CanonicalizationError::InvalidSchemaType(other.to_string())),
    }
}

fn parse_object_schema(
    value: &Value,
    map: &Map<String, Value>,
    ctx: &mut ParseContext<'_, '_>,
) -> Result<SharedSchema, CanonicalizationError> {
    // An `$id` here opens a new base scope for relative refs inside; restore on exit.
    let restore = enter_id_scope(value, map, ctx);
    let result = parse_object_schema_inner(map, ctx);
    if let Some(restore) = restore {
        ctx.resolver = Some(restore.resolver);
        if let Some(defs) = restore.defs {
            ctx.defs = defs;
        }
    }
    result
}

struct IdScopeRestore<'a, 'r> {
    resolver: Resolver<'r>,
    defs: Option<AHashMap<Arc<str>, &'a Value>>,
}

/// Evolve `ctx.resolver` into this object's `$id` base, returning the previous resolver to restore; `None` when unchanged.
fn enter_id_scope<'a, 'r>(
    value: &Value,
    map: &Map<String, Value>,
    ctx: &mut ParseContext<'a, 'r>,
) -> Option<IdScopeRestore<'a, 'r>> {
    // A `lookup`-produced context already sits at its root's `$id` scope; re-entering would resolve a relative `$id` twice.
    if ctx.root_scope_established && ValueIdentity::of(value) == ctx.root_key {
        return None;
    }
    let id = map
        .get(crate::bundler::id_keyword(ctx.draft))
        .and_then(Value::as_str)?;
    // Plain-name anchors (`#foo`) are not subresource ids.
    if id.starts_with('#') {
        return None;
    }
    let resolver = ctx.resolver.as_ref()?;
    let evolved = resolver
        .in_subresource(ctx.draft.create_resource_ref(value))
        .ok()?;
    let previous = ctx.resolver.replace(evolved)?;
    // `defs` keys are root-scope-local pointers; inside a nested resource scope the same pointer means a different
    // target, so empty the map until the scope exits.
    let defs = (ValueIdentity::of(value) != ctx.root_key).then(|| std::mem::take(&mut ctx.defs));
    Some(IdScopeRestore {
        resolver: previous,
        defs,
    })
}

fn parse_object_schema_inner(
    map: &Map<String, Value>,
    ctx: &mut ParseContext<'_, '_>,
) -> Result<SharedSchema, CanonicalizationError> {
    if matches!(ctx.draft, Draft::Draft4 | Draft::Draft6 | Draft::Draft7) {
        if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
            return resolve_reference(reference, ctx);
        }
    }
    let mut facets: Vec<SharedSchema> = Vec::new();
    if let Some(branches) = map.get("allOf").and_then(Value::as_array) {
        facets.push(shared(Schema::AllOf(parse_branches(branches, ctx)?)));
    }
    if let Some(branches) = map.get("anyOf").and_then(Value::as_array) {
        facets.push(shared(Schema::AnyOf(parse_branches(branches, ctx)?)));
    }
    if let Some(branches) = map.get("oneOf").and_then(Value::as_array) {
        facets.push(shared(Schema::OneOf(OneOf(parse_branches(branches, ctx)?))));
    }
    if let Some(value) = map.get("not") {
        facets.push(shared(Schema::Not(parse_inner(value, ctx)?)));
    }
    if let Some(condition) = ctx.get(map, "if") {
        facets.push(shared(Schema::IfThenElse(IfThenElse {
            condition: parse_inner(condition, ctx)?,
            then_branch: ctx
                .get(map, "then")
                .map(|value| parse_inner(value, ctx))
                .transpose()?,
            else_branch: ctx
                .get(map, "else")
                .map(|value| parse_inner(value, ctx))
                .transpose()?,
        })));
    }
    if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
        facets.push(resolve_reference(reference, ctx)?);
    }
    // `$recursiveRef` is 2019-09's dynamic ref; model it like `$dynamicRef`.
    if let Some(reference) = ctx
        .get(map, "$dynamicRef")
        .or_else(|| ctx.get(map, "$recursiveRef"))
        .and_then(Value::as_str)
    {
        facets.push(shared(Schema::DynamicRef(Arc::from(reference))));
    }
    if let Some(value) = ctx.get(map, "const") {
        facets.push(shared(Schema::Const(CanonicalJson::try_from_value(value)?)));
    }
    if let Some(Value::Array(values)) = map.get("enum") {
        facets.push(shared(Schema::Enum(
            values
                .iter()
                .map(CanonicalJson::try_from_value)
                .collect::<Result<Vec<_>, _>>()?,
        )));
    }
    match map.get("type") {
        Some(Value::String(name)) => facets.push(parse_single_type(name, map, ctx)?),
        Some(Value::Array(names)) => facets.push(parse_type_list(names, map, ctx)?),
        Some(_) => {
            unreachable!("meta-validation guarantees `type` is a string or array of simple types")
        }
        None => {
            if let Some(facet) = parse_untyped_facets(map, ctx)? {
                facets.push(facet);
            }
        }
    }
    Ok(match facets.len() {
        0 => shared(Schema::True),
        1 => facets.into_iter().next().expect("len == 1"),
        _ => shared(Schema::AllOf(facets)),
    })
}

/// Kind-restricted keywords (e.g. `maximum`, `items`) without an explicit `type` constrain only their kind, passing other values.
fn parse_untyped_facets(
    map: &Map<String, Value>,
    ctx: &mut ParseContext<'_, '_>,
) -> Result<Option<SharedSchema>, CanonicalizationError> {
    let kinds = kinds_with_restricted_keywords(map, ctx.draft);
    if kinds.is_empty() {
        return Ok(None);
    }
    if kinds.len() == 1 {
        let kind = kinds.iter().next().expect("len == 1");
        let body = shared(build_typed_leaf(kind, map, ctx)?);
        return Ok(Some(shared(Schema::TypeGuard { ty: kind, body })));
    }
    // Keywords for several kinds: every JSON type is admitted, each constrained by its own keywords.
    Ok(Some(parse_kind_list(JsonTypeSet::all().iter(), map, ctx)?))
}

/// Kinds with at least one kind-restricted keyword present in `map` under `draft`.
fn kinds_with_restricted_keywords(map: &Map<String, Value>, draft: Draft) -> JsonTypeSet {
    let mut kinds = JsonTypeSet::empty();
    for (kind, keywords) in [
        (JsonType::Number, NUMERIC_KEYWORDS),
        (JsonType::String, STRING_KEYWORDS),
        (JsonType::Array, ARRAY_KEYWORDS),
        (JsonType::Object, OBJECT_KEYWORDS),
    ] {
        if keywords
            .iter()
            .any(|key| draft.is_known_keyword(key) && map.contains_key(*key))
        {
            kinds = kinds.insert(kind);
        }
    }
    kinds
}

const NUMERIC_KEYWORDS: &[&str] = &[
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
];
const STRING_KEYWORDS: &[&str] = &[
    "minLength",
    "maxLength",
    "pattern",
    "format",
    "contentEncoding",
    "contentMediaType",
    "contentSchema",
];
const ARRAY_KEYWORDS: &[&str] = &[
    "items",
    "prefixItems",
    "additionalItems",
    "minItems",
    "maxItems",
    "uniqueItems",
    "contains",
    "minContains",
    "maxContains",
    "unevaluatedItems",
];
const OBJECT_KEYWORDS: &[&str] = &[
    "properties",
    "patternProperties",
    "additionalProperties",
    "required",
    "minProperties",
    "maxProperties",
    "dependencies",
    "dependentRequired",
    "dependentSchemas",
    "propertyNames",
    "unevaluatedProperties",
];

/// Percent-encode a `$ref` fragment so it parses as a URI and matches the identically encoded definition keys; borrows when no escaping is needed.
fn normalize_ref_fragment(reference: &str) -> Cow<'_, str> {
    let Some((base, fragment)) = reference.split_once('#') else {
        return Cow::Borrowed(reference);
    };
    match referencing::uri::encode_fragment(fragment) {
        Cow::Borrowed(_) => Cow::Borrowed(reference),
        Cow::Owned(encoded) => Cow::Owned(format!("{base}#{encoded}")),
    }
}

/// Shared flow of the three ref-resolution paths: re-entrant -> `Recursive` leaf, cached -> reuse, fresh -> parse/memoize/register.
/// `memo` is re-selected through `ctx` on each access so `parse` can borrow `ctx` mutably.
fn parse_cycle_aware<'a, 'r, K>(
    ctx: &mut ParseContext<'a, 'r>,
    memo: for<'c> fn(&'c mut ParseContext<'a, 'r>) -> &'c mut CycleMemo<K>,
    memo_key: K,
    symbolic_key: Arc<str>,
    on_cycle: impl FnOnce(&mut ParseContext<'a, 'r>, &Arc<str>),
    parse: impl FnOnce(&mut ParseContext<'a, 'r>) -> Result<SharedSchema, CanonicalizationError>,
) -> Result<SharedSchema, CanonicalizationError>
where
    K: std::hash::Hash + Eq + Clone,
{
    if memo(ctx).in_progress.contains(&memo_key) {
        ctx.cross.cyclic.insert(Arc::clone(&symbolic_key));
        on_cycle(ctx, &symbolic_key);
        return Ok(shared(Schema::Recursive(symbolic_key)));
    }
    if let Some(cached) = memo(ctx).cache.get(&memo_key) {
        let cached = Arc::clone(cached);
        return inline_or_reference(ctx, &symbolic_key, cached);
    }
    if ctx.ref_depth >= MAX_REF_DEPTH {
        return Err(CanonicalizationError::RefDepthLimitExceeded);
    }
    memo(ctx).in_progress.insert(memo_key.clone());
    ctx.ref_depth += 1;
    let parsed = parse(ctx);
    ctx.ref_depth -= 1;
    memo(ctx).in_progress.remove(&memo_key);
    let parsed = parsed?;
    memo(ctx).cache.insert(memo_key, Arc::clone(&parsed));
    register_definition(ctx, &symbolic_key, &parsed);
    inline_or_reference(ctx, &symbolic_key, parsed)
}

fn resolve_reference(
    reference: &str,
    ctx: &mut ParseContext<'_, '_>,
) -> Result<SharedSchema, CanonicalizationError> {
    let normalized = normalize_ref_fragment(reference);
    let reference = normalized.as_ref();
    let entry = ctx
        .defs
        .get_key_value(reference)
        .map(|(key, body)| (Arc::clone(key), *body));
    if let Some((key, body)) = entry {
        return parse_cycle_aware(
            ctx,
            |ctx| &mut ctx.definition_memo,
            Arc::clone(&key),
            key,
            |_, _| {},
            move |ctx| parse_inner(body, ctx),
        );
    }
    // Same-document fragment (`#`, `#/pointer`, `#anchor`): the resolver handles anchors and percent-decoded pointers.
    // Inline the target rather than emit a `$ref` the canonicalizer may later rewrite away.
    if reference.starts_with('#') {
        let resolved = {
            ctx.resolver
                .as_ref()
                .and_then(|resolver| resolver.lookup(reference).ok())
        };
        if let Some(resolved) = resolved {
            let (target, target_resolver, _) = resolved.into_inner();
            let key = local_symbolic_ref_key(reference, target, ctx);
            // A pointer can walk through a subresource `$id` that changes the base: the target's relative refs must
            // resolve against that base, and root-scope-local `defs` pointers no longer apply inside it.
            let scope_changed = ctx.resolver.as_ref().is_some_and(|current| {
                current.base_uri().as_str() != target_resolver.base_uri().as_str()
            });
            if scope_changed {
                let previous_resolver = ctx.resolver.replace(target_resolver);
                let previous_defs = std::mem::take(&mut ctx.defs);
                let result = inline_local_target(key, target, ctx);
                ctx.resolver = previous_resolver;
                ctx.defs = previous_defs;
                return result;
            }
            return inline_local_target(key, target, ctx);
        }
    }
    if ctx.resolver.is_some() {
        if let Some(resolved) = resolve_external(reference, ctx)? {
            return Ok(resolved);
        }
    }
    let uri = referencing::uri::from_str(reference).map_err(|error| {
        CanonicalizationError::InvalidSchemaType(format!("invalid $ref URI {reference:?}: {error}"))
    })?;
    Ok(shared(Schema::Reference(uri)))
}

/// Parse a located target inline, keyed by `key` for cycle detection and reuse.
fn inline_local_target(
    key: Arc<str>,
    target: &Value,
    ctx: &mut ParseContext<'_, '_>,
) -> Result<SharedSchema, CanonicalizationError> {
    let target_identity = ValueIdentity::of(target);
    parse_cycle_aware(
        ctx,
        |ctx| &mut ctx.local_memo,
        target_identity,
        key,
        move |ctx, key| {
            if target_identity == ctx.root_key {
                ctx.root_self_refs.insert(Arc::clone(key));
            }
        },
        move |ctx| {
            // A pointer can land in a non-schema location (an unknown keyword's value) that root
            // meta-validation and the opaqueness pre-scan never descend into; screen such targets
            // like external ones.
            if target_identity != ctx.root_key {
                validate_schema_document(target, ctx.draft)?;
                if requires_opaque_preservation(target, ctx.draft) {
                    return Ok(raw_subschema(target));
                }
            }
            parse_inner(target, ctx)
        },
    )
}

fn local_symbolic_ref_key(reference: &str, target: &Value, ctx: &ParseContext<'_, '_>) -> Arc<str> {
    if ValueIdentity::of(target) == ctx.root_key {
        // External document roots must stay resource-qualified; bare `#` would point at the emitted outer root.
        if ctx.root_scope_established {
            if let Some(resolver) = ctx.resolver.as_ref() {
                return external_target_uri(resolver, reference);
            }
        }
        return Arc::from(reference);
    }
    match ctx.resolver.as_ref() {
        Some(resolver) => external_target_uri(resolver, reference),
        None => Arc::from(reference),
    }
}

/// Record a parsed definition body so emit and `definitions()` can resolve the symbolic refs pointing at it. Only
/// cyclic refs (which produced a `Recursive` leaf) need an entry; a non-cyclic ref is inlined and carries no symbolic node.
fn register_definition(ctx: &mut ParseContext<'_, '_>, key: &Arc<str>, body: &SharedSchema) {
    if ctx.cross.cyclic.contains(key) {
        ctx.cross
            .definitions
            .entry(Arc::clone(key))
            .or_insert_with(|| Arc::clone(body));
    }
}

/// Inline `body` when it fits the budget, else keep the ref symbolic as a [`Schema::Reference`] and register the target for `definitions()`.
fn inline_or_reference(
    ctx: &mut ParseContext<'_, '_>,
    key: &Arc<str>,
    body: SharedSchema,
) -> Result<SharedSchema, CanonicalizationError> {
    if ctx.inline_budget == usize::MAX || !node_count_exceeds_budget(&body, ctx.inline_budget) {
        return Ok(body);
    }
    let uri = referencing::uri::from_str(key).map_err(|error| {
        CanonicalizationError::InvalidSchemaType(format!("invalid $ref URI {key:?}: {error}"))
    })?;
    ctx.cross.definitions.entry(Arc::clone(key)).or_insert(body);
    Ok(shared(Schema::Reference(uri)))
}

/// Whether the total schema nodes in a shared graph exceed `budget`.
fn node_count_exceeds_budget(node: &SharedSchema, budget: usize) -> bool {
    let mut count = 0;
    let mut seen = AHashSet::new();
    node_count_exceeds_budget_inner(node, budget, &mut count, &mut seen)
}

fn node_count_exceeds_budget_inner(
    node: &SharedSchema,
    budget: usize,
    count: &mut usize,
    seen: &mut AHashSet<*const SchemaNode>,
) -> bool {
    if !seen.insert(Arc::as_ptr(node)) {
        return false;
    }
    *count += 1;
    if *count > budget {
        return true;
    }
    let mut exceeded = false;
    node.as_schema().for_each_child(|child| {
        if !exceeded {
            exceeded = node_count_exceeds_budget_inner(child, budget, count, seen);
        }
    });
    exceeded
}

/// `Ok(None)` when the resolver doesn't know the URI (caller falls back to [`Schema::Reference`]).
fn resolve_external(
    reference: &str,
    ctx: &mut ParseContext<'_, '_>,
) -> Result<Option<SharedSchema>, CanonicalizationError> {
    let (contents, new_resolver, resolved_draft, target_uri) = {
        let resolver = ctx
            .resolver
            .as_ref()
            .expect("resolve_external invoked only when resolver is set");
        // Resolve to an absolute uri against the current lexical base so the same target keys identically whether
        // reached top-level or via a cycle.
        let target_uri = external_target_uri(resolver, reference);
        match resolver.lookup(reference) {
            Ok(resolved) => {
                let (contents, new_resolver, draft) = resolved.into_inner();
                (contents, new_resolver, draft, target_uri)
            }
            Err(_) => return Ok(None),
        }
    };
    // A `$dynamicRef`/`$recursiveRef` in the target binds against the target document's dynamic scope; inlining or
    // raw-preserving the fragment into the referrer detaches it from its anchor, so keep the reference symbolic.
    if contains_dynamic_scope_ref(contents, resolved_draft) {
        return Ok(None);
    }
    // Key on target identity, not the raw ref string (see `external_memo`).
    let identity = ValueIdentity::of(contents);
    parse_cycle_aware(
        ctx,
        |ctx| &mut ctx.cross.external_memo,
        identity,
        target_uri,
        |_, _| {},
        move |ctx| {
            // Meta-validate each fresh external target (once per identity) so externals fail like the meta-validated root.
            validate_schema_document(contents, resolved_draft)?;
            // Screen external targets through the same opaque-preservation gate as the root: a dynamic ref or an
            // out-of-range numeric/cardinality bound must be kept verbatim, not canonicalized.
            if requires_opaque_preservation(contents, resolved_draft) {
                return Ok(raw_subschema(contents));
            }
            parse_external_document(contents, resolved_draft, new_resolver, ctx)
        },
    )
    .map(Some)
}

/// Absolute uri of an external target: resolve the document part against the current lexical base, re-attach any fragment.
fn external_target_uri(resolver: &Resolver<'_>, reference: &str) -> Arc<str> {
    let base = resolver.base_uri();
    let resolve_doc = |uri_part: &str| {
        resolver
            .resolve_uri(&base.borrow(), uri_part)
            .map_or_else(|_| uri_part.to_string(), |uri| uri.as_str().to_string())
    };
    let resolved = match reference.split_once('#') {
        Some((uri_part, fragment)) => {
            let doc = if uri_part.is_empty() {
                base.as_str().to_string()
            } else {
                resolve_doc(uri_part)
            };
            format!("{doc}#{fragment}")
        }
        None => resolve_doc(reference),
    };
    Arc::from(resolved)
}

/// Parse a resolved external document with its own document-local `$defs`, sharing the outer cross-document cache and cycle set.
fn parse_external_document<'r>(
    value: &'r Value,
    draft: Draft,
    resolver: Resolver<'r>,
    outer: &mut ParseContext<'_, '_>,
) -> Result<SharedSchema, CanonicalizationError> {
    // Use the resolved document's own draft: a cross-draft `$ref` processes the target's keywords by its dialect, not the referrer's.
    let mut sub = ParseContext::new(value, draft, Some(resolver), outer.inline_budget, true);
    // Cross-document state is global: lend it to the sub-context so cycles found in the target are recorded on the outer context.
    std::mem::swap(&mut sub.cross, &mut outer.cross);
    let parsed = if sub.has_refs {
        check_unguarded_recursion(value, sub.draft, sub.resolver.as_ref())
            .and_then(|()| parse_inner(value, &mut sub))
    } else {
        parse_inner(value, &mut sub)
    };
    std::mem::swap(&mut sub.cross, &mut outer.cross);
    parsed
}

/// Build `"{prefix}{token}"` with `token` escaped per RFC 6901.
pub(super) fn build_def_pointer(prefix: &str, token: &str) -> String {
    let mut escaped = String::with_capacity(token.len());
    referencing::write_escaped_str(&mut escaped, token);
    // Generated from a raw JSON object key, so `%` must be encoded as data, not treated as an existing URI escape sequence.
    let mut buffer = String::with_capacity(prefix.len() + escaped.len());
    buffer.push_str(prefix);
    referencing::uri::encode_to(&escaped, &mut buffer);
    buffer
}

fn parse_branches(
    values: &[Value],
    ctx: &mut ParseContext<'_, '_>,
) -> Result<Vec<SharedSchema>, CanonicalizationError> {
    values.iter().map(|value| parse_inner(value, ctx)).collect()
}

/// Parse a subschema, dropping `true` (the vacuous default of every keyword this is used for).
fn parse_non_true(
    value: &Value,
    ctx: &mut ParseContext<'_, '_>,
) -> Result<Option<SharedSchema>, CanonicalizationError> {
    let parsed = parse_inner(value, ctx)?;
    Ok((!matches!(parsed.as_schema(), Schema::True)).then_some(parsed))
}

fn parse_single_type(
    name: &str,
    map: &Map<String, Value>,
    ctx: &mut ParseContext<'_, '_>,
) -> Result<SharedSchema, CanonicalizationError> {
    let kind = JsonType::from_str(name).expect("meta-validation guarantees a known JSON type name");
    Ok(shared(build_typed_leaf(kind, map, ctx)?))
}

fn parse_type_list(
    names: &[Value],
    map: &Map<String, Value>,
    ctx: &mut ParseContext<'_, '_>,
) -> Result<SharedSchema, CanonicalizationError> {
    let kinds = names
        .iter()
        .map(|entry| {
            let name = entry
                .as_str()
                .expect("meta-validation guarantees type entries are strings");
            JsonType::from_str(name).expect("meta-validation guarantees a known JSON type name")
        })
        .collect::<Vec<_>>();
    parse_kind_list(kinds, map, ctx)
}

/// `AnyOf` over one `TypedGroup` per kind, each constrained by `map`'s keywords for that kind.
fn parse_kind_list(
    kinds: impl IntoIterator<Item = JsonType>,
    map: &Map<String, Value>,
    ctx: &mut ParseContext<'_, '_>,
) -> Result<SharedSchema, CanonicalizationError> {
    let branches = kinds
        .into_iter()
        .map(|kind| {
            let body = shared(build_typed_leaf(kind, map, ctx)?);
            Ok(shared(Schema::TypedGroup { ty: kind, body }))
        })
        .collect::<Result<Vec<_>, CanonicalizationError>>()?;
    Ok(shared(Schema::AnyOf(branches)))
}

fn build_typed_leaf(
    kind: JsonType,
    map: &Map<String, Value>,
    ctx: &mut ParseContext<'_, '_>,
) -> Result<Schema, CanonicalizationError> {
    match kind {
        JsonType::Null => Ok(Schema::Null),
        JsonType::Boolean => Ok(Schema::Boolean(BooleanBounds::Any)),
        JsonType::Integer => parse_integer_leaf(map),
        JsonType::Number => Ok(Schema::Number(NumberLeaf {
            bounds: parse_number_bounds(map)?,
            multiple_of: bigfraction_bound(map, "multipleOf")?,
            not_multiple_of: Vec::new(),
        })),
        JsonType::String => parse_string_leaf(map),
        JsonType::Array => parse_array_leaf(map, ctx),
        JsonType::Object => parse_object_leaf(map, ctx),
    }
}

/// Keywords producing a facet besides the typed leaf in [`parse_object_schema_inner`]; when any is present the string pin
/// must take the full object-schema path.
const FACET_KEYWORDS: &[&str] = &[
    "allOf",
    "anyOf",
    "oneOf",
    "not",
    "if",
    "$ref",
    "$dynamicRef",
    "$recursiveRef",
    "const",
    "enum",
];

/// Property names are always strings; pin `"type": "string"` when only string keywords appear so kind-restricted dispatch works.
fn parse_property_names(
    value: &Value,
    ctx: &mut ParseContext<'_, '_>,
) -> Result<SharedSchema, CanonicalizationError> {
    if let Value::Object(map) = value {
        let has_string_keyword = STRING_KEYWORDS.iter().any(|key| map.contains_key(*key));
        if !map.contains_key("type") && has_string_keyword {
            if FACET_KEYWORDS.iter().all(|key| !map.contains_key(*key)) {
                return Ok(shared(build_typed_leaf(JsonType::String, map, ctx)?));
            }
            // Other facets are present: re-parse the whole map with the pin injected.
            let mut with_type = map.clone();
            with_type.insert("type".to_string(), Value::String("string".to_string()));
            return parse_inner(&Value::Object(with_type), ctx);
        }
    }
    parse_inner(value, ctx)
}

/// Reject patterns that are not valid regexes, matching `format: regex` (`to_rust_regex` accepts lookaround/backrefs but rejects malformed syntax like `[`).
fn ensure_valid_regex(pattern: &str, pointer: &str) -> Result<(), CanonicalizationError> {
    if jsonschema_regex::to_rust_regex(pattern).is_err() {
        return Err(CanonicalizationError::InvalidPattern {
            pointer: pointer.to_string(),
            message: format!("{pattern:?} is not a valid regular expression"),
        });
    }
    Ok(())
}

fn parse_string_leaf(map: &Map<String, Value>) -> Result<Schema, CanonicalizationError> {
    let min_length = cardinality_bound(map, "minLength")?.filter(|value| !value.is_zero());
    let patterns = match map.get("pattern").and_then(Value::as_str) {
        Some(pattern) => {
            ensure_valid_regex(pattern, "/pattern")?;
            vec![Arc::from(pattern)]
        }
        None => Vec::new(),
    };
    Ok(Schema::String(StringLeaf {
        // `minLength: 0` is the type-default; drop it so the leaf compares equal to one parsed without `minLength`.
        min_length,
        max_length: cardinality_bound(map, "maxLength")?,
        patterns,
        not_patterns: Vec::new(),
        format: map.get("format").and_then(Value::as_str).map(Arc::from),
        content: parse_content_facet(map)?.into_iter().collect(),
    }))
}

fn parse_content_facet(
    map: &Map<String, Value>,
) -> Result<Option<ContentFacet>, CanonicalizationError> {
    let content_encoding = map
        .get("contentEncoding")
        .and_then(Value::as_str)
        .map(Arc::from);
    let content_media_type = map
        .get("contentMediaType")
        .and_then(Value::as_str)
        .map(Arc::from);
    let content_schema = map
        .get("contentSchema")
        .map(CanonicalJson::try_from_value)
        .transpose()?;
    if content_encoding.is_none() && content_media_type.is_none() && content_schema.is_none() {
        return Ok(None);
    }
    Ok(Some(ContentFacet {
        content_encoding,
        content_media_type,
        content_schema,
    }))
}

/// Exact fraction for a numeric bound keyword. `Err` when the number is past the carrier, routing
/// the document to raw preservation - the backstop keeping parse in sync with `document`'s
/// pre-scan. Non-number values (Draft 4 boolean exclusives) are not bounds and stay `None`.
fn bigfraction_bound(
    map: &Map<String, Value>,
    keyword: &str,
) -> Result<Option<BoundFraction>, CanonicalizationError> {
    match map.get(keyword) {
        Some(value @ Value::Number(_)) => match parse_bigfraction_value(value) {
            Some(fraction) => Ok(Some(fraction)),
            None => Err(unsupported_bound(keyword, value)),
        },
        _ => Ok(None),
    }
}

/// Exact count for a cardinality bound keyword; `Err` past the carrier, like [`bigfraction_bound`].
fn cardinality_bound(
    map: &Map<String, Value>,
    keyword: &str,
) -> Result<Option<BoundCardinality>, CanonicalizationError> {
    cardinality_bound_value(keyword, map.get(keyword))
}

/// [`cardinality_bound`] on an optionally-fetched value, for draft-gated keyword lookups.
fn cardinality_bound_value(
    keyword: &str,
    value: Option<&Value>,
) -> Result<Option<BoundCardinality>, CanonicalizationError> {
    match value {
        Some(value @ Value::Number(_)) => match parse_cardinality_bound(value) {
            Some(bound) => Ok(Some(bound)),
            None => Err(unsupported_bound(keyword, value)),
        },
        _ => Ok(None),
    }
}

fn unsupported_bound(keyword: &str, value: &Value) -> CanonicalizationError {
    CanonicalizationError::InvalidJsonValue(format!("unsupported `{keyword}` value: {value}"))
}

fn parse_cardinality_bound(value: &Value) -> Option<BoundCardinality> {
    let fraction = parse_bigfraction_value(value)?;
    if fraction.is_nan() || fraction.is_infinite() || fraction.is_sign_negative() {
        return None;
    }
    if !fraction.denom()?.is_one() {
        return None;
    }
    #[cfg(not(feature = "arbitrary-precision"))]
    {
        Some(BoundCardinality::from(*fraction.numer()?))
    }
    #[cfg(feature = "arbitrary-precision")]
    {
        Some(BoundCardinality::from(Clone::clone(fraction.numer()?)))
    }
}

pub(crate) fn cardinality_value_is_supported(value: &Value) -> bool {
    let Some(number) = value.as_number() else {
        return true;
    };
    // Negative integers are unconstrained bounds, and integers up to 2^53 convert exactly in both modes; skip the bignum path.
    if number.as_i64().is_some_and(i64::is_negative)
        || number.as_u64().is_some_and(|integer| integer <= (1 << 53))
    {
        return true;
    }
    // The default build materialises a count through `f64` (`parse_cardinality_bound`), so a non-negative integer above
    // 2^53 would round and emit an off-by-one bound; preserve those raw (`arbitrary-precision` parses them exactly).
    #[cfg(not(feature = "arbitrary-precision"))]
    {
        if number.as_u64().is_some() {
            return false;
        }
    }
    let Some(fraction) = parse_bigfraction_value(value) else {
        return false;
    };
    if fraction.is_sign_negative() {
        return true;
    }
    let Some(denominator) = fraction.denom() else {
        return false;
    };
    if !denominator.is_one() {
        return true;
    }
    #[cfg(not(feature = "arbitrary-precision"))]
    {
        fraction
            .numer()
            .is_some_and(|numerator| numerator.to_u64().is_some())
    }
    #[cfg(feature = "arbitrary-precision")]
    {
        true
    }
}

fn parse_array_leaf(
    map: &Map<String, Value>,
    ctx: &mut ParseContext<'_, '_>,
) -> Result<Schema, CanonicalizationError> {
    let min_items =
        cardinality_bound(map, "minItems")?.unwrap_or_else(|| BoundCardinality::from(0u64));
    let mut prefix = match ctx.get(map, "prefixItems") {
        Some(Value::Array(entries)) => entries
            .iter()
            .map(|value| parse_inner(value, ctx))
            .collect::<Result<Vec<_>, _>>()?,
        _ => Vec::new(),
    };
    // Pre-2020-12 `items`-as-array (with `additionalItems`) and 2020-12 `prefixItems` + `items` both flow here.
    // `tail_evaluated` records whether a tail keyword evaluates indices past the prefix (even `items: true` does, making `unevaluatedItems` vacuous).
    let (mut tail, tail_evaluated) = match map.get("items") {
        Some(Value::Array(entries)) => {
            if prefix.is_empty() {
                prefix = entries
                    .iter()
                    .map(|value| parse_inner(value, ctx))
                    .collect::<Result<Vec<_>, _>>()?;
            }
            match map.get("additionalItems") {
                Some(value) => (parse_inner(value, ctx)?, true),
                None => (shared(Schema::True), false),
            }
        }
        Some(value) => (parse_inner(value, ctx)?, true),
        None => (shared(Schema::True), false),
    };
    let contains = match ctx.get(map, "contains") {
        Some(value) => {
            let min_contains = cardinality_bound_value("minContains", ctx.get(map, "minContains"))?
                .unwrap_or_else(|| BoundCardinality::from(1u64));
            let max_contains = cardinality_bound_value("maxContains", ctx.get(map, "maxContains"))?;
            vec![ContainsClause {
                schema: parse_inner(value, ctx)?,
                min_contains,
                max_contains,
            }]
        }
        None => Vec::new(),
    };
    // The opaque-preservation guard already diverted every `contains`-bearing/applicator case, so here `unevaluatedItems`
    // is exactly the `items` tail: fold it in when the tail is not already evaluated.
    if contains.is_empty() && !tail_evaluated {
        if let Some(value) = ctx.get(map, "unevaluatedItems") {
            if let Some(parsed) = parse_non_true(value, ctx)? {
                tail = parsed;
            }
        }
    }
    Ok(Schema::Array(ArrayLeaf {
        prefix,
        tail,
        length: LengthBounds {
            minimum: min_items,
            maximum: cardinality_bound(map, "maxItems")?,
        },
        unique_items: matches!(map.get("uniqueItems"), Some(Value::Bool(true))),
        repeated_items: false,
        contains,
    }))
}

fn parse_object_leaf(
    map: &Map<String, Value>,
    ctx: &mut ParseContext<'_, '_>,
) -> Result<Schema, CanonicalizationError> {
    let mut constraints = Vec::new();
    let mut requirements = Vec::new();

    if let Some(Value::Object(properties)) = map.get("properties") {
        for (name, sub) in properties {
            constraints.push(ObjectConstraint {
                matcher: PropertyNameMatcher::NamedProperty(Arc::from(name.as_str())),
                schema: parse_inner(sub, ctx)?,
            });
        }
    }
    if let Some(Value::Object(patterns)) = map.get("patternProperties") {
        for (pattern, sub) in patterns {
            ensure_valid_regex(pattern, "/patternProperties")?;
            constraints.push(ObjectConstraint {
                matcher: PropertyNameMatcher::PatternProperty(Arc::from(pattern.as_str())),
                schema: parse_inner(sub, ctx)?,
            });
        }
    }
    // With no annotation-contributing applicator (diverted by the opaque-preservation guard), `unevaluatedProperties`
    // is exactly `additionalProperties`; `additionalProperties` wins when both are present.
    if let Some(value) = map
        .get("additionalProperties")
        .or_else(|| ctx.get(map, "unevaluatedProperties"))
    {
        // `additionalProperties: true` is the default; a catch-all that admits everything adds no information.
        if let Some(parsed) = parse_non_true(value, ctx)? {
            constraints.push(ObjectConstraint {
                matcher: PropertyNameMatcher::AdditionalProperties,
                schema: parsed,
            });
        }
    }

    if let Some(Value::Array(required)) = map.get("required") {
        for name in required.iter().filter_map(Value::as_str) {
            requirements.push(ObjectRequirement::RequiredProperty(Arc::from(name)));
        }
    }
    if let Some(count) = cardinality_bound(map, "minProperties")? {
        // `minProperties: 0` is the type-default; drop so canonical objects are equal.
        if !count.is_zero() {
            requirements.push(ObjectRequirement::MinProperties(count));
        }
    }
    if let Some(count) = cardinality_bound(map, "maxProperties")? {
        requirements.push(ObjectRequirement::MaxProperties(count));
    }
    // Draft-7 `dependencies` and the post-2019-09 `dependentRequired`/`dependentSchemas` split all lower to the same IR;
    // the value's shape disambiguates.
    let dependency_sources = [
        ctx.get(map, "dependencies"),
        ctx.get(map, "dependentRequired"),
        ctx.get(map, "dependentSchemas"),
    ];
    for source in dependency_sources.into_iter().flatten() {
        let Value::Object(entries) = source else {
            continue;
        };
        for (trigger, value) in entries {
            match value {
                Value::Array(required) => {
                    let names: Vec<Arc<str>> = required
                        .iter()
                        .filter_map(Value::as_str)
                        .map(Arc::from)
                        .collect();
                    // Empty list means "if trigger is present, require nothing" - vacuous.
                    if names.is_empty() {
                        continue;
                    }
                    requirements.push(ObjectRequirement::DependentPropertiesRequirement {
                        property: Arc::from(trigger.as_str()),
                        required_properties: names,
                    });
                }
                schema => {
                    // `dependentSchemas: {trigger: true}` adds no constraint.
                    let Some(parsed) = parse_non_true(schema, ctx)? else {
                        continue;
                    };
                    requirements.push(ObjectRequirement::DependentSchemaRequirement {
                        property: Arc::from(trigger.as_str()),
                        schema: parsed,
                    });
                }
            }
        }
    }

    // `propertyNames: true` is the spec default (all property names allowed).
    let property_names = ctx
        .get(map, "propertyNames")
        .map(|value| parse_property_names(value, ctx))
        .transpose()?
        .filter(|parsed| !matches!(parsed.as_schema(), Schema::True));
    Ok(Schema::Object(ObjectLeaf {
        requirements,
        constraints,
        property_names,
        // Lowered into `additionalProperties` above; never retained on the leaf.
    }))
}

/// Bounds and `multipleOf` parse as fractions then lift to integers so fractional inputs (`"minimum": 1.5`) round inward.
fn parse_integer_leaf(map: &Map<String, Value>) -> Result<Schema, CanonicalizationError> {
    let number_bounds = parse_number_bounds(map)?;
    let multiple_of_fraction = bigfraction_bound(map, "multipleOf")?;
    let lifted = number_bounds_to_integer(&number_bounds).and_then(|bounds| {
        Some((
            bounds,
            number_multiple_of_to_integer(multiple_of_fraction.as_ref())?.into_modulus(),
        ))
    });
    let Some((bounds, multiple_of)) = lifted else {
        // Fraction parts past the integer carrier: keep the exact constraints on the number
        // carrier under the integer pin.
        return Ok(Schema::TypedGroup {
            ty: JsonType::Integer,
            body: shared(Schema::Number(NumberLeaf {
                bounds: number_bounds,
                multiple_of: multiple_of_fraction,
                not_multiple_of: Vec::new(),
            })),
        });
    };
    Ok(Schema::Integer(IntegerLeaf {
        bounds,
        multiple_of,
        not_multiple_of: Vec::new(),
    }))
}

fn parse_number_bounds(map: &Map<String, Value>) -> Result<NumberBounds, CanonicalizationError> {
    let mut minimum = bigfraction_bound(map, "minimum")?;
    let mut maximum = bigfraction_bound(map, "maximum")?;
    let mut exclusive_minimum = matches!(map.get("exclusiveMinimum"), Some(Value::Bool(true)));
    let mut exclusive_maximum = matches!(map.get("exclusiveMaximum"), Some(Value::Bool(true)));
    if let Some(value) = bigfraction_bound(map, "exclusiveMinimum")? {
        // At equality the exclusive bound (`x > v`) is stricter than inclusive `minimum` (`x >= v`), so it must win.
        if minimum
            .as_ref()
            .is_none_or(|current| value.cmp(current).is_ge())
        {
            minimum = Some(value);
            exclusive_minimum = true;
        }
    }
    if let Some(value) = bigfraction_bound(map, "exclusiveMaximum")? {
        if maximum
            .as_ref()
            .is_none_or(|current| value.cmp(current).is_le())
        {
            maximum = Some(value);
            exclusive_maximum = true;
        }
    }
    Ok(NumberBounds {
        minimum,
        maximum,
        exclusive_minimum,
        exclusive_maximum,
    })
}

fn bigint_from_number(number: &Number) -> Option<BoundInteger> {
    #[cfg(feature = "arbitrary-precision")]
    {
        if let Some(bigint) = try_parse_bigint(number) {
            return Some(BoundInteger::from(bigint));
        }
    }
    if let Some(value) = number.as_i64() {
        return Some(BoundInteger::from(value));
    }
    // A remaining integer is a `u64` above `i64::MAX`: exact under `arbitrary-precision`, out of
    // carrier range in the default build.
    #[cfg(feature = "arbitrary-precision")]
    if let Some(value) = number.as_u64() {
        return Some(BoundInteger::from(value));
    }
    None
}

fn parse_bigfraction_value(value: &Value) -> Option<BoundFraction> {
    let number = value.as_number()?;
    let fraction = bigfraction_from_number(number)?;
    if fraction.is_nan() || fraction.is_infinite() {
        return None;
    }
    Some(fraction)
}

pub(crate) fn numeric_value_is_supported(value: &Value) -> bool {
    if !value.is_number() {
        return true;
    }
    #[cfg(feature = "arbitrary-precision")]
    {
        let number = value.as_number().expect("checked above");
        // A value whose canonical spelling is scientific (plain expansion past the digit cap) has
        // no exact in-cap emission; checked before parsing, which would materialize the expansion.
        crate::canonical::json::number_spelling_stays_plain(&number.to_string())
            && parse_bigfraction_value(value).is_some()
    }
    #[cfg(not(feature = "arbitrary-precision"))]
    {
        let number = value.as_number().expect("checked above");
        if number.as_i64().is_some() {
            return true;
        }
        if let Some(value) = number.as_u64() {
            return i64::try_from(value).is_ok();
        }
        if number
            .as_f64()
            .is_some_and(|value| value.abs() > MAX_EXACT_F64_INTEGER)
        {
            return false;
        }
        let text = number.to_string();
        if text.bytes().any(|byte| byte == b'e' || byte == b'E') {
            return false;
        }
        parse_bigfraction_value(value).is_some()
    }
}

pub(crate) fn bigfraction_from_number(number: &Number) -> Option<BoundFraction> {
    #[cfg(feature = "arbitrary-precision")]
    {
        if let Some(fraction) = try_parse_bigfraction(number) {
            return Some(BoundFraction::from(fraction));
        }
    }
    #[cfg(not(feature = "arbitrary-precision"))]
    {
        // Integer-representable values must not detour through `f64`: near +/-2^63 the rounding
        // shifts the bound by one.
        if let Some(value) = number.as_i64() {
            return Some(BoundFraction::from(value));
        }
        if let Some(value) = number.as_u64() {
            return Some(BoundFraction::from(value));
        }
        if let Some(value) = number.as_f64() {
            // A magnitude past the `u64` carrier converts to a NaN/infinite fraction, not an error.
            return BoundFraction::try_from(value)
                .ok()
                .filter(|fraction| !fraction.is_nan() && !fraction.is_infinite());
        }
    }
    bigint_from_number(number).map(BoundFraction::from)
}

fn contains_ref(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            if map.contains_key("$ref") || map.contains_key("$dynamicRef") {
                return true;
            }
            map.values().any(contains_ref)
        }
        Value::Array(items) => items.iter().any(contains_ref),
        _ => false,
    }
}
