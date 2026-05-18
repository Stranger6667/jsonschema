//! IR -> JSON Schema emit. Inverse of `crate::canonical::parse`.
//!
//! Static `$ref`s left symbolic (recursion / over-budget) emit as `$ref`, with their targets reattached as
//! `$defs`/`definitions` by `attach_definitions`. Schemas carrying a `Schema::DynamicRef` are preserved as
//! `Schema::Raw` (see `canonicalize_with_resolver`), so dynamic refs never reach emit.

#![cfg_attr(
    not(feature = "arbitrary-precision"),
    allow(clippy::trivially_copy_pass_by_ref)
)]

use std::{borrow::Cow, sync::Arc};

use ahash::{AHashMap, AHashSet};
use num_traits::One;
use referencing::Draft;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::{
    canonical::{
        collect_all_symbolic_refs,
        ir::{
            ArrayLeaf, BooleanBounds, BoundCardinality, BoundFraction, BoundInteger, CanonicalJson,
            ContainsClause, ContentFacet, IfThenElse, IntegerBounds, IntegerLeaf, LengthBounds,
            NumberBounds, NumberLeaf, ObjectConstraint, ObjectLeaf, ObjectRequirement, OneOf,
            PropertyNameMatcher, Schema, SchemaNode, SharedSchema, StringLeaf,
        },
        DefinitionMap,
    },
    JsonType, JsonTypeSet,
};

/// Cardinality bound -> `Value::Number`, the shape every length/count keyword emits.
fn cardinality_value(value: &BoundCardinality) -> Value {
    Value::Number(value.to_number())
}

/// `{key: value}` with a runtime key (`json!` takes only literal keys).
fn single_entry(key: impl Into<String>, value: Value) -> Value {
    let mut map = Map::with_capacity(1);
    map.insert(key.into(), value);
    Value::Object(map)
}

/// Emit a canonical schema, reattaching a `$defs`/`definitions` block resolving every symbolic static `$ref`.
/// External (absolute-uri) targets are bundled under a synthesized `$defs` entry so the schema is self-contained.
#[must_use]
pub(crate) fn to_json_schema(
    root: &SharedSchema,
    draft: Draft,
    validate_formats: bool,
    definitions: &DefinitionMap,
) -> Value {
    let external = external_renames(root, definitions, draft);
    let shared = shared_definitions(root, definitions, &external, draft);
    let definition_refs = definitions
        .iter()
        .filter_map(|(uri, body)| {
            let stripped = strip_synthetic_root(uri);
            stripped
                .starts_with("#/")
                .then(|| (Arc::clone(body), stripped.to_string()))
        })
        .collect();
    let ctx = EmitContext {
        draft,
        validate_formats,
        external,
        shared: shared
            .iter()
            .map(|(node, pointer)| (Arc::as_ptr(node), pointer.clone()))
            .collect(),
        definition_refs,
    };
    // A root that unfolds a definition emits as a `$ref` to it, else each round trip re-inlines one level deeper.
    let root_reference = definitions.iter().find_map(|(uri, body)| {
        let stripped = strip_synthetic_root(uri);
        (stripped.starts_with("#/") && (Arc::ptr_eq(body, root) || body == root))
            .then(|| stripped.to_string())
    });
    let value = match root_reference {
        Some(reference) => {
            let reference = single_entry("$ref", Value::String(reference));
            match schema_uri(draft) {
                Some(uri) => with_schema_uri(reference, uri),
                None => reference,
            }
        }
        None => emit_root(root.as_schema(), &ctx),
    };
    let value = attach_definitions(value, root, &ctx, definitions);
    attach_shared_definitions(value, &ctx, &shared)
}

/// Duplication cost above which a shared subtree is emitted as a synthetic definition instead of
/// inlining per occurrence - a compact IR DAG must not unfold into an exponentially larger document.
const SHARED_EMIT_COST_LIMIT: u64 = 256;

/// `(node, "#/<container>/<name>")` for nodes whose inline duplication would dominate the output,
/// in deterministic first-visit order so a round-tripped DAG emits identical names.
fn shared_definitions(
    root: &SharedSchema,
    definitions: &DefinitionMap,
    external: &AHashMap<Box<str>, String>,
    draft: Draft,
) -> Vec<(SharedSchema, String)> {
    fn walk(
        node: &SharedSchema,
        refcount: &mut AHashMap<*const SchemaNode, u64>,
        order: &mut Vec<SharedSchema>,
    ) {
        let count = refcount.entry(Arc::as_ptr(node)).or_insert(0);
        *count += 1;
        if *count > 1 {
            return;
        }
        order.push(Arc::clone(node));
        node.as_schema()
            .for_each_child(|child| walk(child, refcount, order));
    }
    let mut refcount: AHashMap<*const SchemaNode, u64> = AHashMap::new();
    let mut order: Vec<SharedSchema> = Vec::new();
    let mut roots: AHashSet<*const SchemaNode> = AHashSet::new();
    roots.insert(Arc::as_ptr(root));
    walk(root, &mut refcount, &mut order);
    for body in definitions.values() {
        roots.insert(Arc::as_ptr(body));
        walk(body, &mut refcount, &mut order);
    }
    // Existing definition leaf names and external synthesized names are reserved.
    let mut used = existing_definition_leaf_names(definitions);
    used.extend(
        external
            .values()
            .filter_map(|pointer| pointer.rsplit('/').next())
            .map(str::to_string),
    );
    let container = defs_container(draft);
    let mut counter = 0_usize;
    let mut out = Vec::new();
    for node in order {
        if roots.contains(&Arc::as_ptr(&node)) {
            continue;
        }
        let count = refcount[&Arc::as_ptr(&node)];
        if count < 2 || (count - 1) * u64::from(node.size) < SHARED_EMIT_COST_LIMIT {
            continue;
        }
        let name = loop {
            let candidate = format!("shared{counter}");
            counter += 1;
            if used.insert(candidate.clone()) {
                break candidate;
            }
        };
        out.push((node, format!("#/{container}/{name}")));
    }
    out
}

/// Merge the extracted bodies into the defs container. Bodies emit one level inside the node, so `emit_node`'s
/// interception skips the body itself while nested extracted nodes still become `$ref`s.
fn attach_shared_definitions(
    value: Value,
    ctx: &EmitContext,
    shared: &[(SharedSchema, String)],
) -> Value {
    let Value::Object(mut map) = value else {
        // A non-object root has no children, so nothing can be extracted.
        return value;
    };
    if shared.is_empty() {
        return Value::Object(map);
    }
    let container = map
        .entry(defs_container(ctx.draft))
        .or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(entries) = container {
        for (node, pointer) in shared {
            let name = pointer.rsplit('/').next().expect("pointer has segments");
            entries.insert(name.to_string(), emit(node.as_schema(), ctx));
        }
    }
    Value::Object(map)
}

/// Reattach synthesized definitions into an emitted root object so each body's `$ref` resolves. Same-document keys go
/// at their pointer path, external keys at the synthetic `$defs` pointer, root plain-names as the identifier keyword.
fn attach_definitions(
    value: Value,
    root: &SharedSchema,
    ctx: &EmitContext,
    definitions: &DefinitionMap,
) -> Value {
    let Value::Object(mut map) = value else {
        // A non-object root (bool/scalar) cannot host definitions; leave it as-is.
        return value;
    };
    if definitions.is_empty() {
        return Value::Object(map);
    }
    for (uri, body) in definitions {
        let stripped = strip_synthetic_root(uri);
        if let Some((keyword, identifier)) =
            root_plain_name_identifier(stripped, root, body, ctx.draft)
        {
            map.entry(keyword.to_string())
                .or_insert_with(|| Value::String(identifier));
            continue;
        }
        // A bare plain-name anchor (`#name`) whose body is not the emit root can't put `$id`/`$anchor` on the root,
        // so bundle the body under a synthetic definition carrying the anchor keyword for the `$ref` to resolve to.
        if let Some((keyword, identifier)) = plain_name_anchor_identifier(stripped, ctx.draft) {
            let anchored = anchored_definition(emit(body.as_schema(), ctx), keyword, identifier);
            let container = defs_container(ctx.draft);
            let name = unique_container_key(&map, container, &stripped[1..]);
            insert_at_pointer(
                &mut map,
                &[Cow::Borrowed(container), Cow::Owned(name)],
                anchored,
            );
            continue;
        }
        let Some(fragment) = stripped.strip_prefix("#/").or_else(|| {
            ctx.external
                .get(stripped)
                .and_then(|p| p.strip_prefix("#/"))
        }) else {
            continue;
        };
        // Resolvers percent-decode the fragment before splitting/unescaping (see `ResourceRef::pointer`), so the def
        // must be keyed by the decoded path or the emitted `$ref` would dangle.
        let decoded = percent_encoding::percent_decode_str(fragment).decode_utf8_lossy();
        let segments: Vec<Cow<'_, str>> = decoded
            .split('/')
            .map(referencing::unescape_segment)
            .collect();
        insert_at_pointer(&mut map, &segments, emit(body.as_schema(), ctx));
    }
    Value::Object(map)
}

/// Draft keyword + value anchoring a schema at the bare plain-name fragment `uri` (`#name`).
/// `$anchor` (2019-09+), or `id`/`$id` with a `#name` fragment (Draft 4 / 6-7); `None` for pointers, root `#`, or no keyword.
fn plain_name_anchor_identifier(uri: &str, draft: Draft) -> Option<(&'static str, String)> {
    let anchor = uri.strip_prefix('#')?;
    if anchor.is_empty() || anchor.starts_with('/') {
        return None;
    }
    match draft {
        Draft::Draft4 => Some(("id", format!("#{anchor}"))),
        Draft::Draft6 | Draft::Draft7 => Some(("$id", format!("#{anchor}"))),
        _ if draft.is_known_keyword("$anchor") => Some(("$anchor", anchor.to_string())),
        _ => None,
    }
}

/// The plain-name anchor identifier, but only when `body` is the emit root - then the keyword sits on the
/// root object itself rather than on a bundled definition.
fn root_plain_name_identifier(
    uri: &str,
    root: &SharedSchema,
    body: &SharedSchema,
    draft: Draft,
) -> Option<(&'static str, String)> {
    if !(Arc::ptr_eq(body, root) || body == root) {
        return None;
    }
    plain_name_anchor_identifier(uri, draft)
}

/// Place `body` at the nested pointer `segments`, creating intermediate objects. No-op if a path segment collides with
/// an existing non-object value.
fn insert_at_pointer(map: &mut Map<String, Value>, segments: &[Cow<'_, str>], body: Value) {
    let Some((leaf, parents)) = segments.split_last() else {
        return;
    };
    let mut current = map;
    for segment in parents {
        let entry = current
            .entry(segment.as_ref().to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        match entry {
            Value::Object(object) => current = object,
            _ => return,
        }
    }
    current.insert(leaf.as_ref().to_owned(), body);
}

/// Attach `keyword: identifier` (`$anchor`/`$id`/`id`) to an emitted definition body so a plain-name `$ref` resolves
/// to it. A boolean/scalar body is wrapped in `allOf` so the keyword has an object to live on.
fn anchored_definition(value: Value, keyword: &'static str, identifier: String) -> Value {
    match value {
        Value::Object(mut map) => {
            map.entry(keyword.to_string())
                .or_insert_with(|| Value::String(identifier));
            Value::Object(map)
        }
        other => {
            let mut map = Map::with_capacity(2);
            map.insert(keyword.to_string(), Value::String(identifier));
            map.insert("allOf".into(), Value::Array(vec![other]));
            Value::Object(map)
        }
    }
}

/// A `base` key unique within `map[container]`, suffixing `_1`, `_2`, ... on collision.
fn unique_container_key(map: &Map<String, Value>, container: &str, base: &str) -> String {
    let taken = |name: &str| {
        map.get(container)
            .and_then(Value::as_object)
            .is_some_and(|entries| entries.contains_key(name))
    };
    if !taken(base) {
        return base.to_string();
    }
    let mut suffix = 1;
    loop {
        let candidate = format!("{base}_{suffix}");
        if !taken(&candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

pub(crate) struct EmitContext {
    draft: Draft,
    validate_formats: bool,
    /// External (absolute-uri) ref target -> synthetic same-document pointer (`#/$defs/<name>`) where its body is bundled.
    external: AHashMap<Box<str>, String>,
    /// Heavily-shared node -> synthetic same-document pointer; `emit_node` emits a `$ref` instead of
    /// unfolding the subtree per occurrence.
    shared: AHashMap<*const SchemaNode, String>,
    /// Same-document definition body -> its `#/...` pointer; a nested node equal to a definition body emits a `$ref`
    /// to it, so round trips don't re-inline. Keyed by structural equality since set ops rebuild equal subtrees distinctly.
    definition_refs: AHashMap<SharedSchema, String>,
}

impl EmitContext {
    /// `$ref` target for a symbolic reference uri: the bundled synthetic pointer for an external target, else the
    /// (root-stripped) uri itself.
    fn ref_target(&self, uri: &str) -> String {
        let stripped = strip_synthetic_root(uri);
        match self.external.get(stripped) {
            Some(pointer) => pointer.clone(),
            None => stripped.to_string(),
        }
    }
}

fn emit_root(schema: &Schema, ctx: &EmitContext) -> Value {
    if matches!(schema, Schema::Raw(_)) {
        return emit(schema, ctx);
    }
    let value = emit(schema, ctx);
    match schema_uri(ctx.draft) {
        Some(uri) => with_schema_uri(value, uri),
        None => value,
    }
}

fn emit_node(node: &SharedSchema, ctx: &EmitContext) -> Value {
    if let Some(pointer) = ctx.shared.get(&Arc::as_ptr(node)) {
        return single_entry("$ref", Value::String(pointer.clone()));
    }
    // An inlined copy of a definition body emits as a `$ref` back to it (matching `Schema::Recursive`), else a forward
    // ref to a recursive definition re-inlines one level deeper each round trip.
    if let Some(pointer) = ctx.definition_refs.get(node) {
        return single_entry("$ref", Value::String(pointer.clone()));
    }
    emit(node.as_schema(), ctx)
}

fn schema_uri(draft: Draft) -> Option<&'static str> {
    match draft {
        Draft::Draft4 => Some("http://json-schema.org/draft-04/schema#"),
        Draft::Draft6 => Some("http://json-schema.org/draft-06/schema#"),
        Draft::Draft7 => Some("http://json-schema.org/draft-07/schema#"),
        Draft::Draft201909 => Some("https://json-schema.org/draft/2019-09/schema"),
        Draft::Draft202012 => Some("https://json-schema.org/draft/2020-12/schema"),
        // `Draft::Unknown` (unrecognised `$schema`) has no canonical meta-schema; `negate`/`intersect` can emit a
        // non-`Raw` root under it, so omit `$schema` rather than panic.
        _ => None,
    }
}

fn with_schema_uri(value: Value, uri: &'static str) -> Value {
    let mut map = match value {
        Value::Object(map) => map,
        Value::Bool(true) => Map::new(),
        Value::Bool(false) => {
            let mut map = Map::new();
            map.insert("not".into(), Value::Object(Map::new()));
            map
        }
        // `emit` yields only objects or booleans; a bare-scalar root comes solely from `Schema::Raw`, which
        // `emit_root` emits without `$schema`.
        other => unreachable!("with_schema_uri on a bare-scalar root: {other:?}"),
    };
    map.insert("$schema".into(), Value::String(uri.into()));
    Value::Object(map)
}

fn emit(schema: &Schema, ctx: &EmitContext) -> Value {
    let draft = ctx.draft;
    match schema {
        Schema::True if matches!(draft, Draft::Draft4) => Value::Object(Map::new()),
        Schema::True => Value::Bool(true),
        Schema::False if matches!(draft, Draft::Draft4) => json!({"not": {}}),
        Schema::False => Value::Bool(false),
        Schema::Null => json!({"type": "null"}),
        Schema::Boolean(bounds) => emit_boolean(bounds, draft),
        Schema::Integer(leaf) => Value::Object(emit_integer(leaf, draft)),
        Schema::Number(leaf) => emit_number(leaf, draft),
        Schema::String(leaf) => emit_string(leaf, ctx),
        Schema::Array(leaf) => emit_array(leaf, ctx),
        Schema::Object(leaf) => emit_object(leaf, ctx),
        Schema::AllOf(branches) => emit_composition("allOf", branches, ctx),
        Schema::AnyOf(branches) => emit_any_of(branches, ctx),
        Schema::OneOf(OneOf(branches)) => emit_composition("oneOf", branches, ctx),
        Schema::Not(inner) => json!({"not": emit_node(inner, ctx)}),
        Schema::IfThenElse(node) => emit_if_then_else(node, ctx),
        Schema::TypedGroup { ty: kind, body } => emit_typed_group(*kind, body, ctx),
        Schema::TypeGuard { ty: kind, body } => emit_type_guard(*kind, body, ctx),
        // `null` is the only JSON value whose type contains exactly one inhabitant, so `{"const": null}` is identical
        // to `{"type": "null"}` - prefer the type form.
        Schema::Const(value) if value.as_str() == "null" => json!({"type": "null"}),
        Schema::Const(value) if matches!(draft, Draft::Draft4) => {
            json!({"enum": [value.to_value()]})
        }
        Schema::Const(value) => json!({"const": value.to_value()}),
        Schema::Enum(values) => emit_enum(values),
        Schema::Reference(uri) => json!({"$ref": ctx.ref_target(uri.as_str())}),
        // A static cycle: emit the `$ref` back to its definition uri. The matching `$defs`/`definitions` entry is
        // reattached by `attach_definitions`.
        Schema::Recursive(uri) => json!({"$ref": ctx.ref_target(uri)}),
        // Schemas carrying a dynamic ref are preserved as `Raw` before emit, via `canonicalize_with_resolver`.
        Schema::DynamicRef(_) => {
            unreachable!("DynamicRef is preserved as Raw before emit")
        }
        Schema::Raw(value) => decode_raw_json(value),
        Schema::MultiType(set) => emit_multi_type(*set),
    }
}

fn decode_raw_json(value: &str) -> Value {
    let mut deserializer = serde_json::Deserializer::from_str(value);
    deserializer.disable_recursion_limit();
    Value::deserialize(&mut deserializer).expect("Raw schema holds valid JSON")
}

// Refs in an id-less document resolve against the synthetic `json-schema:///` root (an internal artifact, not user
// input). Strip it so `#/...` round-trips; leave real `$id`s and external URLs absolute.
pub(crate) fn strip_synthetic_root(uri: &str) -> &str {
    match uri.strip_prefix("json-schema:///") {
        Some("") => "#",
        Some(rest) => rest,
        None => uri,
    }
}

/// Map each external (absolute-uri) definition key to a synthetic same-document `$defs` pointer so emit can bundle
/// its body and rewrite refs. Same-document keys, root `#`, and bare anchors resolve in place and get no entry.
fn external_renames(
    root: &SharedSchema,
    definitions: &DefinitionMap,
    draft: Draft,
) -> AHashMap<Box<str>, String> {
    let mut external = AHashMap::new();
    // Seed with existing and referenced same-document leaf names so synthesized names never bind a pre-existing ref.
    let mut used = existing_definition_leaf_names(definitions);
    reserve_same_document_ref_leaf_names(root, definitions, &mut used);
    let container = defs_container(draft);
    for uri in definitions.keys() {
        let stripped = strip_synthetic_root(uri);
        if stripped.starts_with('#') {
            // Same-document pointer, root, or bare anchor: resolved in place.
            continue;
        }
        let name = unique_name(stripped, &mut used);
        external.insert(Box::from(stripped), format!("#/{container}/{name}"));
    }
    external
}

/// Unescaped leaf name of a same-document (`#/...`) pointer, or `None` for the root, bare anchors, or external uris.
fn same_document_leaf_name(uri: &str) -> Option<String> {
    strip_synthetic_root(uri)
        .strip_prefix("#/")
        .and_then(|fragment| fragment.rsplit('/').next())
        .map(|leaf| referencing::unescape_segment(leaf).into_owned())
}

/// Leaf names of same-document definition keys, reserved so synthesized names never collide.
fn existing_definition_leaf_names(definitions: &DefinitionMap) -> AHashSet<String> {
    definitions
        .keys()
        .filter_map(|uri| same_document_leaf_name(uri))
        .collect()
}

/// Leaf names every same-document `$ref` points at, reserved so synthesized names never rebind a pre-existing ref.
fn reserve_same_document_ref_leaf_names(
    root: &SharedSchema,
    definitions: &DefinitionMap,
    used: &mut AHashSet<String>,
) {
    used.extend(
        collect_all_symbolic_refs(root, definitions)
            .iter()
            .filter_map(|uri| same_document_leaf_name(uri)),
    );
}

/// Draft-appropriate keyword for bundled definitions.
fn defs_container(draft: Draft) -> &'static str {
    match draft {
        Draft::Draft4 | Draft::Draft6 | Draft::Draft7 => "definitions",
        _ => "$defs",
    }
}

/// A readable, pointer-safe, unique `$defs` key derived from an external uri.
fn unique_name(uri: &str, used: &mut AHashSet<String>) -> String {
    let tail = uri.rsplit('/').next().unwrap_or(uri);
    let tail = tail.split(['#', '?']).next().unwrap_or(tail);
    let base: String = tail
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        .collect();
    let base = if base.is_empty() {
        "external".to_string()
    } else {
        base
    };
    let mut candidate = base.clone();
    let mut suffix = 1;
    while !used.insert(candidate.clone()) {
        candidate = format!("{base}_{suffix}");
        suffix += 1;
    }
    candidate
}

fn emit_multi_type(set: JsonTypeSet) -> Value {
    // `set.iter()` yields in canonical order (null, boolean, integer, ...).
    let names: Vec<Value> = set
        .iter()
        .map(|ty| Value::String(ty.as_str().to_string()))
        .collect();
    single_entry("type", Value::Array(names))
}

fn emit_boolean(bounds: &BooleanBounds, draft: Draft) -> Value {
    match bounds {
        BooleanBounds::Any => json!({"type": "boolean"}),
        BooleanBounds::JustTrue if matches!(draft, Draft::Draft4) => json!({"enum": [true]}),
        BooleanBounds::JustFalse if matches!(draft, Draft::Draft4) => json!({"enum": [false]}),
        BooleanBounds::JustTrue => json!({"const": true}),
        BooleanBounds::JustFalse => json!({"const": false}),
    }
}

fn emit_integer(leaf: &IntegerLeaf, draft: Draft) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("type".into(), Value::String("integer".into()));
    insert_int_bounds(&mut map, &leaf.bounds, draft);
    if let Some(multiple_of) = &leaf.multiple_of {
        map.insert("multipleOf".into(), Value::Number(multiple_of.to_number()));
    }
    insert_not_multiple_of(
        &mut map,
        "integer",
        leaf.not_multiple_of
            .iter()
            .map(|q| Value::Number(q.to_number())),
    );
    map
}

/// `allOf` of `{"not": {"type": T, "multipleOf": q}}` branches. Inner `type` is REQUIRED: a bare `{multipleOf: q}`
/// canonicalizes to "non-numbers OR multiples", whose `Not` is no clean leaf and the collapse absorber won't re-fold.
fn insert_not_multiple_of(
    map: &mut Map<String, Value>,
    type_name: &str,
    moduli: impl Iterator<Item = Value>,
) {
    let all_of: Vec<Value> = moduli
        .map(|q| json!({"not": {"type": type_name, "multipleOf": q}}))
        .collect();
    if !all_of.is_empty() {
        map.insert("allOf".into(), Value::Array(all_of));
    }
}

fn emit_number(leaf: &NumberLeaf, draft: Draft) -> Value {
    // `number ∧ ¬multipleOf(q)` with no other facets is exactly `{"not": {"multipleOf": q}}`: the
    // type-less form passes non-numbers, so its complement re-pins `number`. Keep the compact form.
    if leaf.bounds == NumberBounds::default() && leaf.multiple_of.is_none() {
        if let [modulus] = leaf.not_multiple_of.as_slice() {
            return json!({"not": {"multipleOf": modulus.to_json_value()}});
        }
    }
    let mut map = Map::new();
    map.insert("type".into(), Value::String("number".into()));
    insert_number_bounds(&mut map, &leaf.bounds, draft);
    if let Some(multiple_of) = &leaf.multiple_of {
        map.insert("multipleOf".into(), multiple_of.to_json_value());
    }
    insert_not_multiple_of(
        &mut map,
        "number",
        leaf.not_multiple_of
            .iter()
            .map(BoundFraction::to_json_value),
    );
    Value::Object(map)
}

fn emit_string(leaf: &StringLeaf, ctx: &EmitContext) -> Value {
    let mut map = Map::new();
    let mut all_of = Vec::new();
    map.insert("type".into(), Value::String("string".into()));
    if let Some(value) = &leaf.min_length {
        map.insert("minLength".into(), cardinality_value(value));
    }
    if let Some(value) = &leaf.max_length {
        map.insert("maxLength".into(), cardinality_value(value));
    }
    match leaf.patterns.as_slice() {
        [] => {}
        [only] => {
            map.insert("pattern".into(), Value::String(only.to_string()));
        }
        many => {
            // `pattern` takes one regex; emit multiples as `allOf`. Branches stay type-less so each re-parses to a
            // string guard - a pinned branch would wrongly reject non-strings the guard exempts.
            all_of.extend(
                many.iter()
                    .map(|pattern| json!({"pattern": pattern.as_ref()})),
            );
        }
    }
    // Inner `type` REQUIRED so the parsed `Not` inner is a clean `String{patterns:[r]}` leaf the absorber re-folds.
    all_of.extend(
        leaf.not_patterns
            .iter()
            .map(|pattern| json!({"not": {"type": "string", "pattern": pattern.as_ref()}})),
    );
    if let Some(format) = &leaf.format {
        let draft_default_asserts = crate::compiler::formats_are_assertions_by_default(ctx.draft);
        match (ctx.validate_formats, draft_default_asserts) {
            (true, false) => {
                map.insert("format".into(), Value::String(format.to_string()));
                all_of.push(format_assertion_schema(format, ctx.draft));
            }
            (true, true) | (false, false) => {
                map.insert("format".into(), Value::String(format.to_string()));
            }
            (false, true) => {}
        }
    }
    if let Some((first, rest)) = leaf.content.split_first() {
        insert_content_facet(&mut map, first);
        all_of.extend(rest.iter().map(content_facet_to_value));
    }
    if !all_of.is_empty() {
        map.insert("allOf".into(), Value::Array(all_of));
    }
    Value::Object(map)
}

fn format_assertion_schema(format: &str, draft: Draft) -> Value {
    let draft = if crate::keywords::format::is_known_format(Draft::Draft7, format) {
        Draft::Draft7
    } else {
        draft
    };

    match schema_uri(draft) {
        Some(uri) => json!({"$schema": uri, "format": format}),
        None => json!({"format": format}),
    }
}

fn content_facet_to_value(facet: &ContentFacet) -> Value {
    let mut map = Map::new();
    insert_content_facet(&mut map, facet);
    Value::Object(map)
}

fn insert_content_facet(map: &mut Map<String, Value>, facet: &ContentFacet) {
    if let Some(encoding) = &facet.content_encoding {
        map.insert(
            "contentEncoding".into(),
            Value::String(encoding.to_string()),
        );
    }
    if let Some(media_type) = &facet.content_media_type {
        map.insert(
            "contentMediaType".into(),
            Value::String(media_type.to_string()),
        );
    }
    if let Some(schema) = &facet.content_schema {
        map.insert("contentSchema".into(), schema.to_value());
    }
}

fn emit_array(leaf: &ArrayLeaf, ctx: &EmitContext) -> Value {
    let draft = ctx.draft;
    let mut map = Map::new();
    map.insert("type".into(), Value::String("array".into()));
    let tail_is_true = matches!(leaf.tail.as_schema(), Schema::True);
    if leaf.prefix.is_empty() {
        if !tail_is_true {
            map.insert("items".into(), emit_node(&leaf.tail, ctx));
        }
    } else {
        // Pre-2020-12: tuple in `items`, additional schema in `additionalItems`; 2020-12: tuple in
        // `prefixItems`, additional schema in `items`.
        let (tuple_key, tail_key) = if matches!(
            draft,
            Draft::Draft4 | Draft::Draft6 | Draft::Draft7 | Draft::Draft201909
        ) {
            ("items", "additionalItems")
        } else {
            ("prefixItems", "items")
        };
        let prefix: Vec<Value> = leaf
            .prefix
            .iter()
            .map(|child| emit_node(child, ctx))
            .collect();
        map.insert(tuple_key.into(), Value::Array(prefix));
        if !tail_is_true {
            map.insert(tail_key.into(), emit_node(&leaf.tail, ctx));
        }
    }
    insert_length_bounds(&mut map, &leaf.length);
    if leaf.unique_items {
        map.insert("uniqueItems".into(), Value::Bool(true));
    }
    // `repeated_items` and multiple `contains` clauses both need to land in `allOf`; accumulate
    // them so neither overwrites the other.
    let mut allof_branches: Vec<Value> = Vec::new();
    if leaf.repeated_items {
        // Inner `type` REQUIRED so the parsed `Not` inner is a clean `Array{uniqueItems}`
        // leaf the collapse absorber re-folds.
        allof_branches.push(json!({"not": {"type": "array", "uniqueItems": true}}));
    }
    insert_contains(&mut map, &mut allof_branches, &leaf.contains, ctx);
    if !allof_branches.is_empty() {
        map.insert("allOf".into(), Value::Array(allof_branches));
    }
    Value::Object(map)
}

fn emit_object(leaf: &ObjectLeaf, ctx: &EmitContext) -> Value {
    let mut map = Map::new();
    map.insert("type".into(), Value::String("object".into()));
    insert_constraints(&mut map, &leaf.constraints, ctx);
    insert_requirements(&mut map, &leaf.requirements, ctx);
    if let Some(value) = &leaf.property_names {
        map.insert("propertyNames".into(), emit_node(value, ctx));
    }
    Value::Object(map)
}

fn insert_constraints(
    map: &mut Map<String, Value>,
    constraints: &[ObjectConstraint],
    ctx: &EmitContext,
) {
    let mut properties: Vec<(String, Value)> = Vec::new();
    let mut pattern_properties: Vec<(String, Value)> = Vec::new();
    let mut additional: Option<Value> = None;
    for constraint in constraints {
        match &constraint.matcher {
            PropertyNameMatcher::NamedProperty(name) => {
                properties.push((name.to_string(), emit_node(&constraint.schema, ctx)));
            }
            PropertyNameMatcher::PatternProperty(pattern) => {
                pattern_properties.push((pattern.to_string(), emit_node(&constraint.schema, ctx)));
            }
            PropertyNameMatcher::AdditionalProperties => {
                additional = Some(emit_node(&constraint.schema, ctx));
            }
        }
    }
    properties.sort_by(|left, right| left.0.cmp(&right.0));
    pattern_properties.sort_by(|left, right| left.0.cmp(&right.0));
    if !properties.is_empty() {
        map.insert(
            "properties".into(),
            Value::Object(properties.into_iter().collect()),
        );
    }
    if !pattern_properties.is_empty() {
        map.insert(
            "patternProperties".into(),
            Value::Object(pattern_properties.into_iter().collect()),
        );
    }
    if let Some(value) = additional {
        map.insert("additionalProperties".into(), value);
    }
}

fn insert_requirements(
    map: &mut Map<String, Value>,
    requirements: &[ObjectRequirement],
    ctx: &EmitContext,
) {
    let draft = ctx.draft;
    let mut required: Vec<String> = Vec::new();
    let mut dependent_required: Map<String, Value> = Map::new();
    let mut dependent_schemas: Map<String, Value> = Map::new();
    let mut existential: Vec<(PropertyNameMatcher, Value)> = Vec::new();
    for requirement in requirements {
        match requirement {
            ObjectRequirement::RequiredProperty(name) => required.push(name.to_string()),
            ObjectRequirement::PatternPropertyRequirement { matcher, schema } => {
                existential.push((matcher.clone(), emit_node(schema, ctx)));
            }
            ObjectRequirement::DependentPropertiesRequirement {
                property,
                required_properties,
            } => {
                let array: Vec<Value> = required_properties
                    .iter()
                    .map(|name| Value::String(name.to_string()))
                    .collect();
                dependent_required.insert(property.to_string(), Value::Array(array));
            }
            ObjectRequirement::DependentSchemaRequirement { property, schema } => {
                dependent_schemas.insert(property.to_string(), emit_node(schema, ctx));
            }
            ObjectRequirement::MinProperties(value) => {
                map.insert("minProperties".into(), cardinality_value(value));
            }
            ObjectRequirement::MaxProperties(value) => {
                map.insert("maxProperties".into(), cardinality_value(value));
            }
        }
    }
    if !existential.is_empty() {
        // No direct keyword for "some key matching M satisfies S"; emit as `not(<scope>: not(S))`,
        // where the scope is `patternProperties{p}` / `properties{n}` / `additionalProperties`.
        let all_of: Vec<Value> = existential
            .into_iter()
            .map(|(matcher, value)| {
                let not_value = single_entry("not", value);
                let scope = match matcher {
                    PropertyNameMatcher::PatternProperty(pattern) => single_entry(
                        "patternProperties",
                        single_entry(pattern.to_string(), not_value),
                    ),
                    PropertyNameMatcher::NamedProperty(name) => {
                        single_entry("properties", single_entry(name.to_string(), not_value))
                    }
                    PropertyNameMatcher::AdditionalProperties => {
                        single_entry("additionalProperties", not_value)
                    }
                };
                single_entry("not", scope)
            })
            .collect();
        map.insert("allOf".into(), Value::Array(all_of));
    }
    if !required.is_empty() {
        required.sort();
        required.dedup();
        map.insert(
            "required".into(),
            Value::Array(required.into_iter().map(Value::String).collect()),
        );
    }
    if matches!(draft, Draft::Draft4 | Draft::Draft6 | Draft::Draft7) {
        let mut dependencies = dependent_required;
        dependencies.extend(dependent_schemas);
        if !dependencies.is_empty() {
            map.insert("dependencies".into(), Value::Object(dependencies));
        }
    } else {
        if !dependent_required.is_empty() {
            map.insert(
                "dependentRequired".into(),
                Value::Object(dependent_required),
            );
        }
        if !dependent_schemas.is_empty() {
            map.insert("dependentSchemas".into(), Value::Object(dependent_schemas));
        }
    }
}

fn insert_length_bounds(map: &mut Map<String, Value>, bounds: &LengthBounds) {
    if !bounds.minimum.is_zero() {
        map.insert("minItems".into(), cardinality_value(&bounds.minimum));
    }
    if let Some(value) = &bounds.maximum {
        map.insert("maxItems".into(), cardinality_value(value));
    }
}

fn insert_contains(
    map: &mut Map<String, Value>,
    allof_branches: &mut Vec<Value>,
    contains: &[ContainsClause],
    ctx: &EmitContext,
) {
    if contains.is_empty() {
        return;
    }
    // `contains` takes a single subschema; emit multiples as `allOf` branches.
    if contains.len() == 1 {
        emit_single_contains_clause(map, &contains[0], ctx);
        return;
    }
    for clause in contains {
        let mut clause_map = Map::new();
        emit_single_contains_clause(&mut clause_map, clause, ctx);
        allof_branches.push(Value::Object(clause_map));
    }
}

fn emit_single_contains_clause(
    map: &mut Map<String, Value>,
    clause: &ContainsClause,
    ctx: &EmitContext,
) {
    map.insert("contains".into(), emit_node(&clause.schema, ctx));
    if !clause.min_contains.is_one() {
        map.insert(
            "minContains".into(),
            cardinality_value(&clause.min_contains),
        );
    }
    if let Some(value) = &clause.max_contains {
        map.insert("maxContains".into(), cardinality_value(value));
    }
}

/// When every `AnyOf` branch is an unconstrained typed leaf, fold into `{type: [...]}`, so `anyOf` of bare types
/// and `{type: [...]}` produce identical output (they reduce to the same IR).
fn emit_any_of(branches: &[SharedSchema], ctx: &EmitContext) -> Value {
    let mut combined = JsonTypeSet::empty();
    for branch in branches {
        let Some(set) = branch.as_schema().as_type_set() else {
            return emit_composition("anyOf", branches, ctx);
        };
        combined = combined.union(set);
    }
    emit_multi_type(combined)
}

/// Emit a standalone `Enum`; collapse to `type:[...]` when the value set saturates one or more JSON types.
fn emit_enum(values: &[CanonicalJson]) -> Value {
    if let Some(set) = Schema::Enum(values.to_vec()).as_type_set() {
        return emit_multi_type(set);
    }
    json!({
        "enum": values.iter().map(CanonicalJson::to_value).collect::<Vec<_>>()
    })
}

fn emit_composition(keyword: &'static str, branches: &[SharedSchema], ctx: &EmitContext) -> Value {
    single_entry(
        keyword,
        Value::Array(
            branches
                .iter()
                .map(|branch| emit_node(branch, ctx))
                .collect(),
        ),
    )
}

fn emit_if_then_else(node: &IfThenElse, ctx: &EmitContext) -> Value {
    let mut map = Map::new();
    map.insert("if".into(), emit_node(&node.condition, ctx));
    if let Some(branch) = &node.then_branch {
        map.insert("then".into(), emit_node(branch, ctx));
    }
    if let Some(branch) = &node.else_branch {
        map.insert("else".into(), emit_node(branch, ctx));
    }
    Value::Object(map)
}

fn emit_typed_group(kind: JsonType, body: &SharedSchema, ctx: &EmitContext) -> Value {
    if matches!(body.as_schema(), Schema::True) {
        return json!({"type": kind.to_string()});
    }
    // The canonicalize pipeline (collapse_typed_group) guarantees `body` is not a bare typed leaf of the same kind, so
    // the `allOf` shape is the natural one.
    #[rustfmt::skip]
    debug_assert!(!body.as_schema().is_typed_leaf_of(kind), "TypedGroup with same-kind body must have been unwrapped by collapse_typed_group; got {:?}", body.as_schema());
    json!({
        "allOf": [
            {"type": kind.to_string()},
            emit_node(body, ctx),
        ]
    })
}

fn emit_type_guard(kind: JsonType, body: &SharedSchema, ctx: &EmitContext) -> Value {
    match body.as_schema() {
        Schema::True => Value::Bool(true),
        Schema::False => emit_type_complement(kind),
        Schema::Integer(leaf) if kind == JsonType::Number => {
            emit_number_guarded_integer(leaf, ctx.draft)
        }
        body_schema
            if body_schema
                .pinned_kind()
                .is_some_and(|body_kind| kind.covers(body_kind)) =>
        {
            emit_without_type(body, ctx)
        }
        _ => json!({
            "anyOf": [
                emit_type_complement(kind),
                emit_node(body, ctx),
            ]
        }),
    }
}

fn emit_number_guarded_integer(leaf: &IntegerLeaf, draft: Draft) -> Value {
    let mut map = emit_integer(leaf, draft);
    map.remove("type");
    map.entry("multipleOf")
        .or_insert_with(|| Value::Number(BoundInteger::from(1).to_number()));
    Value::Object(map)
}

fn emit_without_type(node: &SharedSchema, ctx: &EmitContext) -> Value {
    // The compact `{"not": {"multipleOf": q}}` re-pins `number` on its own, so without the outer `type` pin it drops
    // the guard's exemption for non-numbers; a guard body keeps the self-scoping form.
    if let Schema::Number(leaf) = node.as_schema() {
        if leaf.bounds == NumberBounds::default() && leaf.multiple_of.is_none() {
            if let [modulus] = leaf.not_multiple_of.as_slice() {
                return json!({
                    "allOf": [{"not": {"type": "number", "multipleOf": modulus.to_json_value()}}]
                });
            }
        }
    }
    // Never emit through the shared-`$ref` interception: a `$ref` target keeps its `type` pin (stripping the key is a
    // no-op on `{"$ref": ...}`), dropping the guard's exemption. Inlining keeps the type-less shape parsing back to it.
    let emitted = emit(node.as_schema(), ctx);
    let Value::Object(mut map) = emitted else {
        return emitted;
    };
    map.remove("type");
    if map.is_empty() {
        Value::Bool(true)
    } else {
        Value::Object(map)
    }
}

fn emit_type_complement(kind: JsonType) -> Value {
    let single = JsonTypeSet::from(kind);
    if let Some(complement) = Schema::type_set_complement(single) {
        emit_multi_type(complement)
    } else {
        // `Integer` complement: "non-integer numbers" isn't a type tag.
        json!({"not": {"type": kind.to_string()}})
    }
}

fn insert_int_bounds(map: &mut Map<String, Value>, bounds: &IntegerBounds, draft: Draft) {
    insert_bound(
        map,
        BoundSide::Min,
        bounds.minimum.as_ref(),
        bounds.exclusive_minimum,
        draft,
        |v| Value::Number(v.to_number()),
    );
    insert_bound(
        map,
        BoundSide::Max,
        bounds.maximum.as_ref(),
        bounds.exclusive_maximum,
        draft,
        |v| Value::Number(v.to_number()),
    );
}

fn insert_number_bounds(map: &mut Map<String, Value>, bounds: &NumberBounds, draft: Draft) {
    insert_bound(
        map,
        BoundSide::Min,
        bounds.minimum.as_ref(),
        bounds.exclusive_minimum,
        draft,
        BoundFraction::to_json_value,
    );
    insert_bound(
        map,
        BoundSide::Max,
        bounds.maximum.as_ref(),
        bounds.exclusive_maximum,
        draft,
        BoundFraction::to_json_value,
    );
}

#[derive(Copy, Clone)]
enum BoundSide {
    Min,
    Max,
}

impl BoundSide {
    fn inclusive_key(self) -> &'static str {
        match self {
            Self::Min => "minimum",
            Self::Max => "maximum",
        }
    }

    fn exclusive_key(self) -> &'static str {
        match self {
            Self::Min => "exclusiveMinimum",
            Self::Max => "exclusiveMaximum",
        }
    }
}

fn insert_bound<T>(
    map: &mut Map<String, Value>,
    side: BoundSide,
    value: Option<&T>,
    exclusive: bool,
    draft: Draft,
    to_value: impl Fn(&T) -> Value,
) {
    let Some(value) = value else { return };
    if matches!(draft, Draft::Draft4) {
        map.insert(side.inclusive_key().into(), to_value(value));
        if exclusive {
            map.insert(side.exclusive_key().into(), Value::Bool(true));
        }
    } else {
        let key = if exclusive {
            side.exclusive_key()
        } else {
            side.inclusive_key()
        };
        map.insert(key.into(), to_value(value));
    }
}
