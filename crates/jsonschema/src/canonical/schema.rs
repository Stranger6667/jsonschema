//! The public canonical schema handle and its algebra.
//!
//! [`CanonicalSchema`] is the result of canonicalization and the receiver for the schema algebra
//! (`intersect`/`union`/`negate`/`subtract`) and the decision queries (`is_satisfiable`/`is_subschema_of`).

use std::{
    cmp::Ordering,
    hash::{Hash, Hasher},
    sync::Arc,
};

use ahash::AHashMap;
use referencing::Draft;
use serde_json::{Map, Value};

use crate::{
    canonical::{
        canonicalize_ir,
        context::CanonicalizationContext,
        coverage,
        definitions::{disambiguate_definitions, reachable_definitions, union_definitions},
        document::raw_schema_from_value,
        emit,
        intern::{self, shared},
        intersect,
        ir::{CanonicalKind, ObjectRequirement, Schema, SharedSchema},
        leaves::Leaf,
        membership, negate,
        oracle::prover,
        walk, DefinitionMap,
    },
    keywords::format::is_known_format,
    options::PatternEngineOptions,
    JsonType,
};

/// Canonical JSON Schema IR handle.
#[derive(Clone, Debug)]
pub struct CanonicalSchema {
    inner: SharedSchema,
    draft: Draft,
    pattern_options: PatternEngineOptions,
    validate_formats: bool,
    /// Transitive-closure map of every reachable reference uri to its canonical target; child handles share the
    /// `Arc` so a `Reference`/`Recursive` leaf at any depth resolves against one map.
    definitions: Arc<DefinitionMap>,
}

// The draft decides the emitted `$schema` header and keyword semantics (Draft 4 integers are
// lexical), and the format-assertion flag decides whether `format` rejects instances, so both are
// part of the schema's identity alongside the IR.
impl PartialEq for CanonicalSchema {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
            && self.draft == other.draft
            && self.validate_formats == other.validate_formats
            && self.definitions == other.definitions
    }
}

impl Eq for CanonicalSchema {}

impl PartialOrd for CanonicalSchema {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CanonicalSchema {
    fn cmp(&self, other: &Self) -> Ordering {
        self.inner
            .cmp(&other.inner)
            .then_with(|| self.draft.cmp(&other.draft))
            .then_with(|| self.validate_formats.cmp(&other.validate_formats))
            .then_with(|| self.definitions.cmp(&other.definitions))
    }
}

impl Hash for CanonicalSchema {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
        self.draft.hash(state);
        self.validate_formats.hash(state);
        self.definitions.hash(state);
    }
}

impl CanonicalSchema {
    #[cfg(test)]
    pub(crate) fn from_inner(inner: SharedSchema, draft: Draft) -> Self {
        Self::from_inner_with_pattern_options(inner, draft, PatternEngineOptions::default())
    }

    #[cfg(test)]
    pub(crate) fn from_inner_with_pattern_options(
        inner: SharedSchema,
        draft: Draft,
        pattern_options: PatternEngineOptions,
    ) -> Self {
        Self::with_definitions(
            inner,
            draft,
            pattern_options,
            crate::compiler::formats_are_assertions_by_default(draft),
            Arc::new(DefinitionMap::new()),
        )
    }

    pub(crate) fn with_definitions(
        inner: SharedSchema,
        draft: Draft,
        pattern_options: PatternEngineOptions,
        validate_formats: bool,
        definitions: Arc<DefinitionMap>,
    ) -> Self {
        Self {
            inner,
            draft,
            pattern_options,
            validate_formats,
            definitions,
        }
    }

    /// Emit this canonical schema back to JSON Schema.
    #[must_use]
    pub fn to_json_schema(&self) -> Value {
        emit::to_json_schema(
            &self.inner,
            self.draft,
            self.validate_formats,
            &self.definitions,
        )
    }

    /// Borrow the internal canonical IR variant.
    #[must_use]
    pub(crate) fn as_schema(&self) -> &Schema {
        self.inner.as_schema()
    }

    /// A canonicalization context carrying this schema's draft, pattern, and format settings.
    pub(crate) fn context(&self) -> CanonicalizationContext {
        CanonicalizationContext::with_pattern_options(self.pattern_options)
            .with_draft(self.draft)
            .with_format_assertions(self.validate_formats)
    }

    /// Wrap a child IR node into a public handle, propagating draft + pattern options and the shared definitions map.
    pub(crate) fn wrap_child(&self, child: &SharedSchema) -> Self {
        Self::with_definitions(
            child.clone(),
            self.draft,
            self.pattern_options,
            self.validate_formats,
            Arc::clone(&self.definitions),
        )
    }

    /// Resolve symbolic references to their canonical targets.
    ///
    /// Transitive closure: every uri reachable through any [`CanonicalView::Reference`]/[`CanonicalView::Recursive`]
    /// maps to a [`CanonicalSchema`] target (itself possibly holding such leaves); an absent uri is dangling.
    ///
    /// [`CanonicalView::Reference`]: crate::canonical::CanonicalView::Reference
    /// [`CanonicalView::Recursive`]: crate::canonical::CanonicalView::Recursive
    #[must_use]
    pub fn definitions(&self) -> impl ExactSizeIterator<Item = (String, CanonicalSchema)> + '_ {
        self.definitions.iter().map(|(uri, body)| {
            (
                emit::strip_synthetic_root(uri).to_string(),
                self.wrap_child(body),
            )
        })
    }

    /// Return a schema that validates iff both inputs validate.
    ///
    /// Emits under the newer operand draft. Uses `self.pattern_options`; format assertions from either operand remain
    /// assertions. Mixed-draft operands whose semantics the newer draft cannot express (Draft 4 lexical integers,
    /// `Raw` fragments) combine as a raw `allOf` keeping each operand under its own dialect.
    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        if self.needs_exact_cross_draft_fallback(other) {
            return self.exact_cross_draft_combination(other, "allOf");
        }
        // Operands are already canonical; `intersect_canonical` canonicalizes the result.
        self.binary_operation(other, |left, right, ctx| {
            intersect::intersect_canonical(&left, &right, ctx)
        })
    }

    /// Return a schema that validates iff either input validates.
    ///
    /// Emits under the newer operand draft. Uses `self.pattern_options`; format assertions from either operand remain
    /// assertions. Mixed-draft operands whose semantics the newer draft cannot express (Draft 4 lexical integers,
    /// `Raw` fragments) combine as a raw `anyOf` keeping each operand under its own dialect.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        if self.needs_exact_cross_draft_fallback(other) {
            return self.exact_cross_draft_combination(other, "anyOf");
        }
        self.binary_operation(other, |left, right, ctx| {
            canonicalize_ir(&intern::shared(Schema::AnyOf(vec![left, right])), ctx)
        })
    }

    /// Whether combining under [`promoted_draft`] would reinterpret an operand: a `Raw` fragment replays its
    /// source keywords verbatim under the result dialect, Draft 4's lexical `type: integer` has no spelling in
    /// newer drafts, and `Unknown` may carry constructs older concrete drafts cannot spell.
    fn needs_exact_cross_draft_fallback(&self, other: &Self) -> bool {
        fn contains_raw(schema: &CanonicalSchema) -> bool {
            !schema.inner.mask.is_disjoint(CanonicalKind::Raw)
                || schema
                    .definitions
                    .values()
                    .any(|body| !body.mask.is_disjoint(CanonicalKind::Raw))
        }
        if self.draft == other.draft {
            return false;
        }
        if contains_raw(self) || contains_raw(other) {
            return true;
        }
        match (self.draft, other.draft) {
            (Draft::Draft4, Draft::Unknown) | (Draft::Unknown, Draft::Draft4) => true,
            (Draft::Draft4, _) => self.contains_lexical_integer(),
            (_, Draft::Draft4) => other.contains_lexical_integer(),
            (Draft::Unknown, concrete) | (concrete, Draft::Unknown) => {
                concrete != effective_draft(Draft::Unknown)
            }
            _ => false,
        }
    }

    /// Whether any node (root or definitions) pins the integer type, whose Draft 4 meaning is lexical.
    fn contains_lexical_integer(&self) -> bool {
        fn scan(schema: &SharedSchema) -> bool {
            match schema.as_schema() {
                Schema::Integer(_) => true,
                Schema::TypedGroup { ty, .. } | Schema::TypeGuard { ty, .. }
                    if *ty == JsonType::Integer =>
                {
                    true
                }
                Schema::MultiType(set) => set.contains(JsonType::Integer),
                other => {
                    let mut found = false;
                    other.for_each_child(|child| found = found || scan(child));
                    found
                }
            }
        }
        scan(&self.inner) || self.definitions.values().any(scan)
    }

    /// Exact mixed-draft combination: each operand becomes an embedded resource carrying its own `$schema`,
    /// so validators evaluate it under its source dialect. The result is opaque (`Raw`) to further algebra.
    fn exact_cross_draft_combination(&self, other: &Self, combinator: &str) -> Self {
        // Metaschema validation reads the whole document under the root dialect, and legacy metas
        // reject modern syntax (draft-04 has no boolean subschemas), so the combined document must
        // use the newest effective dialect of the pair.
        let draft = effective_draft(self.draft).max(effective_draft(other.draft));
        let validate_formats = self.validate_formats || other.validate_formats;
        let mut branch_texts: Vec<String> = Vec::new();
        let mut branch_values: Vec<Value> = Vec::new();
        let mut push_operand = |operand: &CanonicalSchema, slot: &str| {
            let branches =
                match previous_combination_branches(operand, combinator, validate_formats) {
                    Some(branches) => branches,
                    None => vec![dialect_branch(operand, slot, validate_formats)],
                };
            for branch in branches {
                let text = serde_json::to_string(&branch).expect("emitted branch serializes");
                // `allOf`/`anyOf` branches are idempotent; a branch already present adds nothing.
                if !branch_texts.contains(&text) {
                    branch_texts.push(text);
                    branch_values.push(branch);
                }
            }
        };
        push_operand(self, "left");
        push_operand(other, "right");
        let uri = emit::schema_uri(draft).expect("combined dialect is concrete");
        // The root is a resource too: a later same-draft operation embeds this document verbatim,
        // and `$schema` is only valid at resource roots. `max` of two distinct drafts is at least
        // Draft 6 (`effective_draft` never yields Draft 4 for `Unknown`), so `$id` is its keyword.
        let mut hash_parts: Vec<&str> = vec![combinator];
        hash_parts.extend(branch_texts.iter().map(String::as_str));
        let id = format!(
            "urn:jsonschema:cross-draft:root:{:016x}",
            content_hash(&hash_parts)
        );
        let document = serde_json::json!({
            "$schema": uri,
            "$id": id,
            combinator: branch_values,
        });
        raw_schema_from_value(document, draft, self.pattern_options, validate_formats)
    }

    /// Shared skeleton for binary operations: disambiguate colliding definition keys, combine the operands, then
    /// prune the merged definitions down to what the result still references.
    fn binary_operation(
        &self,
        other: &Self,
        combine: impl FnOnce(SharedSchema, SharedSchema, &CanonicalizationContext) -> SharedSchema,
    ) -> Self {
        // `with_draft` resets format assertions to the draft default; re-apply the combined operand setting.
        let ctx = self
            .context()
            .with_draft(promoted_draft(self.draft, other.draft))
            .with_format_assertions(self.validate_formats || other.validate_formats);
        self.binary_operation_in(other, &ctx, combine)
    }

    /// [`Self::binary_operation`] under a caller-provided context; its draft and format settings must match the
    /// promoted operand settings, or memoized results computed under one setting would be read under another.
    fn binary_operation_in(
        &self,
        other: &Self,
        ctx: &CanonicalizationContext,
        combine: impl FnOnce(SharedSchema, SharedSchema, &CanonicalizationContext) -> SharedSchema,
    ) -> Self {
        let draft = promoted_draft(self.draft, other.draft);
        let validate_formats = self.validate_formats || other.validate_formats;
        debug_assert_eq!(ctx.draft(), draft);
        debug_assert_eq!(ctx.validates_formats(), validate_formats);
        // Definition keys are local pointers (`#/$defs/F`), so operands can collide on a key with different bodies;
        // relocate the right side into a fresh keyspace before the union drops one.
        let ((self_inner, self_definitions), (other_inner, other_definitions)) =
            disambiguate_definitions(
                &self.inner,
                &self.definitions,
                &other.inner,
                &other.definitions,
            );
        let strip = |schema: &SharedSchema, draft, operand_formats| {
            strip_unasserted_formats(schema, draft, operand_formats, validate_formats)
        };
        let strip_defs = |defs: &Arc<DefinitionMap>, draft, operand_formats| {
            strip_unasserted_formats_in_definitions(defs, draft, operand_formats, validate_formats)
        };
        let self_inner = strip(&self_inner, self.draft, self.validate_formats);
        let other_inner = strip(&other_inner, other.draft, other.validate_formats);
        let self_definitions = strip_defs(&self_definitions, self.draft, self.validate_formats);
        let other_definitions = strip_defs(&other_definitions, other.draft, other.validate_formats);
        let inner = combine(self_inner, other_inner, ctx);
        let definitions = reachable_definitions(
            &inner,
            &union_definitions(&self_definitions, &other_definitions),
        );
        Self::with_definitions(
            inner,
            draft,
            self.pattern_options,
            validate_formats,
            definitions,
        )
    }

    /// The underlying canonical IR node.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn as_shared(&self) -> &SharedSchema {
        &self.inner
    }

    /// Returns `false` when this schema is canonical `false` or provably admits no instances.
    #[must_use]
    pub fn is_satisfiable(&self) -> bool {
        self.is_satisfiable_in(&self.context())
    }

    /// [`Self::is_satisfiable`] under a caller-provided context matching this schema's settings.
    fn is_satisfiable_in(&self, ctx: &CanonicalizationContext) -> bool {
        if matches!(self.as_schema(), Schema::False) {
            return false;
        }
        !membership::is_provably_empty(self.as_schema(), ctx)
    }

    #[must_use]
    pub fn draft(&self) -> Draft {
        self.draft
    }

    /// Return a schema that validates iff this one does not.
    ///
    /// Uses `self.draft` and `self.pattern_options`.
    #[must_use]
    pub fn negate(&self) -> Self {
        self.negate_in(&self.context())
    }

    /// [`Self::negate`] under a caller-provided context matching this schema's settings.
    fn negate_in(&self, ctx: &CanonicalizationContext) -> Self {
        // `negate` emits a non-canonical tree; re-run the pipeline to restore the canonical-form invariant.
        let inner = canonicalize_ir(&negate::negate_for_draft(&self.inner, ctx), ctx);
        let definitions = reachable_definitions(&inner, &self.definitions);
        Self::with_definitions(
            inner,
            self.draft,
            self.pattern_options,
            self.validate_formats,
            definitions,
        )
    }

    /// Return a schema validating iff `self` validates but `other` does not (`self \ other`).
    ///
    /// Emits under the newer operand draft. Uses `self.pattern_options`; format assertions from either operand remain
    /// assertions.
    #[must_use]
    pub fn subtract(&self, other: &Self) -> Self {
        self.intersect(&other.negate())
    }

    /// Return whether every value satisfying `self` also satisfies `other` (`self ⊆ other`).
    ///
    /// `Some(true)` = proven; `Some(false)` = proven not; `None` = inconclusive.
    #[must_use]
    pub fn is_subschema_of(&self, other: &Self) -> Option<bool> {
        // The context's memos key on node pointers alone, so one context (with its regex, intern, and
        // covers/intersect state) may serve every phase only when the operands agree on all
        // semantics-affecting settings; otherwise each phase builds its own context as the public
        // operations do.
        let shared_settings = self.draft == other.draft
            && self.validate_formats == other.validate_formats
            && self.pattern_options == other.pattern_options;
        let ctx = self.context();
        // Fast structural proof of containment (also decides some reference cases the residual cannot). Sound
        // only when settings match: the prover runs under `self`'s context, so `other`'s asserted facets (e.g.
        // `format`) would be judged under `self`'s laxer semantics. Differing settings use the residual instead.
        if shared_settings {
            let prover = prover::Prover::new(&ctx, &other.definitions, &self.definitions);
            if coverage::covers_with(&prover, &other.inner, &self.inner) {
                return Some(true);
            }
        }
        let residual = if shared_settings {
            let negated = other.negate_in(&ctx);
            self.binary_operation_in(&negated, &ctx, |left, right, ctx| {
                intersect::intersect_canonical(&left, &right, ctx)
            })
        } else {
            self.subtract(other)
        };
        // Residual `False` means `self \ other` is empty, so `self` is a subschema (sound: every empty schema
        // canonicalizes to `False`).
        let satisfiable = if shared_settings {
            residual.is_satisfiable_in(&ctx)
        } else {
            residual.is_satisfiable()
        };
        if !satisfiable {
            return Some(true);
        }
        // Non-`False` residual: commit to non-containment only for a decidably-inhabited shape, where `False`-collapse
        // is a complete emptiness oracle; otherwise (references/recursion) stay inconclusive. The residual carries the
        // combined operands' format-assertion setting, so judge inhabitedness under it, not `self`'s.
        if is_decidably_inhabited(&residual.inner, residual.validate_formats) {
            Some(false)
        } else {
            None
        }
    }
}

/// The concrete draft whose semantics [`Draft::Unknown`] follows.
fn effective_draft(draft: Draft) -> Draft {
    if draft == Draft::Unknown {
        Draft::Draft202012
    } else {
        draft
    }
}

/// FNV-1a over `parts`: emitted ids must be stable across processes, platforms, and dependency
/// versions; hasher crates randomize per process or change output between versions.
fn content_hash(parts: &[&str]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for part in parts {
        for byte in part.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    hash
}

/// Whether a root `$ref` suppresses its sibling keywords under `draft`.
fn legacy_ref_semantics(draft: Draft) -> bool {
    matches!(draft, Draft::Draft4 | Draft::Draft6 | Draft::Draft7)
}

/// The branch list of an operand that is itself an exact combination under the same combinator:
/// associativity lets those branches extend the new document directly, so chained operations do
/// not deepen it.
fn previous_combination_branches(
    schema: &CanonicalSchema,
    combinator: &str,
    result_validate_formats: bool,
) -> Option<Vec<Value>> {
    // Branches were format-stripped under their combination's own setting; a differing result
    // setting re-enters through `dialect_branch`, whose walker re-strips the whole document.
    if schema.validate_formats != result_validate_formats {
        return None;
    }
    if !matches!(schema.as_schema(), Schema::Raw(_)) {
        return None;
    }
    let Value::Object(mut map) = schema.to_json_schema() else {
        return None;
    };
    if !map
        .get("$id")
        .and_then(Value::as_str)
        .is_some_and(|id| id.starts_with("urn:jsonschema:cross-draft:root:"))
    {
        return None;
    }
    match map.remove(combinator) {
        Some(Value::Array(branches)) => Some(branches),
        _ => None,
    }
}

/// One operand of an exact cross-draft combination, as an embedded resource: `$schema` pins the
/// dialect its text is written in, and a content-derived resource id keeps same-document pointers
/// resolving against this root.
fn dialect_branch(schema: &CanonicalSchema, slot: &str, result_validate_formats: bool) -> Value {
    match schema.to_json_schema() {
        Value::Object(map) => {
            let dialect = effective_draft(schema.draft);
            let mut document = Value::Object(map);
            // Mirrors `strip_unasserted_formats` on the algebraic path: once the combined
            // document asserts formats, a `format` survives only if this operand asserted it and
            // its dialect knows it - anything else is an annotation that would turn into a
            // constraint (or a strict-consumer build error, for unknown names).
            if result_validate_formats {
                let operand_asserts = schema.validate_formats;
                for_each_reachable_schema_object(
                    &mut document,
                    dialect,
                    true,
                    &mut |object, object_draft| {
                        let keep = operand_asserts
                            && object
                                .get("format")
                                .and_then(Value::as_str)
                                .is_some_and(|format| is_known_format(object_draft, format));
                        if !keep {
                            object.remove("format");
                        }
                        true
                    },
                );
            }
            let Value::Object(mut map) = document else {
                unreachable!("schema object stays an object")
            };
            // On legacy drafts a root `$ref` suppresses sibling keywords — including the resource
            // id inserted below; on 2019-09+ `$ref` is an ordinary keyword and the document embeds
            // unchanged.
            if legacy_ref_semantics(dialect) && map.contains_key("$ref") {
                map = wrap_legacy_ref_document(map, dialect);
            }
            // A raw-preserved operand keeps its source text, which may lack `$schema`; without it
            // the branch would be read under the combined document's dialect. An unrecognized
            // custom `$schema` must not survive either: nothing can compile it, and the branch is
            // evaluated under the effective dialect.
            let uri = emit::schema_uri(dialect).expect("effective draft is concrete");
            if schema.draft == Draft::Unknown {
                map.insert("$schema".into(), Value::String(uri.to_string()));
            } else {
                map.entry("$schema")
                    .or_insert_with(|| Value::String(uri.to_string()));
            }
            // The id keyword is dialect-relative, and chained operations nest earlier results
            // verbatim, so ids must be unique per nesting level.
            let id_keyword = crate::bundler::id_keyword(dialect);
            if !map.contains_key(id_keyword) {
                let text = serde_json::to_string(&map).expect("emitted schema serializes");
                let id = format!(
                    "urn:jsonschema:cross-draft:{slot}:{:016x}",
                    content_hash(&[&text])
                );
                map.insert(id_keyword.into(), Value::String(id));
            }
            Value::Object(map)
        }
        // Boolean roots are dialect-independent.
        other => other,
    }
}

/// Relocate a legacy document whose root carries `$ref` into `definitions/source`, entered through
/// a pointer from a fresh root: the document's own keywords (and their `$ref`-masking semantics)
/// apply verbatim at the new position, while the fresh root can carry the resource id and
/// `$schema`. Local pointer refs gain the relocation prefix.
fn wrap_legacy_ref_document(mut map: Map<String, Value>, dialect: Draft) -> Map<String, Value> {
    // `$schema` is only valid at resource roots; the wrapper root pins the same dialect.
    map.remove("$schema");
    let mut source = Value::Object(map);
    rewrite_local_pointer_refs(&mut source, dialect, "#/definitions/source");
    let mut wrapper = Map::new();
    wrapper.insert(
        "allOf".into(),
        serde_json::json!([{"$ref": "#/definitions/source"}]),
    );
    wrapper.insert("definitions".into(), serde_json::json!({"source": source}));
    wrapper
}

/// Prefix every same-document pointer `$ref` for a relocation of the whole document under
/// `prefix`. Subtrees under a live nested resource id are position-independent and stay untouched.
fn rewrite_local_pointer_refs(document: &mut Value, draft: Draft, prefix: &str) {
    for_each_reachable_schema_object(document, draft, false, &mut |object, object_draft| {
        if establishes_resource(object, object_draft) {
            return false;
        }
        if let Some(Value::String(reference)) = object.get_mut("$ref") {
            if let Some(pointer) = reference.strip_prefix('#') {
                if pointer.is_empty() {
                    *reference = prefix.to_string();
                } else if pointer.starts_with('/') {
                    *reference = format!("{prefix}{pointer}");
                }
            }
        }
        true
    });
}

/// Whether this object is a nested resource root: an id that is a base URI (not a plain-name
/// anchor, not the empty reference resolving to the enclosing base) and is not voided by a legacy
/// `$ref` sibling.
fn establishes_resource(object: &Map<String, Value>, draft: Draft) -> bool {
    if legacy_ref_semantics(draft) && object.contains_key("$ref") {
        return false;
    }
    object
        .get(crate::bundler::id_keyword(draft))
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty() && !id.starts_with('#'))
}

/// Dialect governing `pointer`'s target and whether the path from the document root crosses a
/// nested resource root, tracked segment by segment (the target's own `$schema` and resource
/// status are the visitor's to judge).
fn pointer_target_scope(document: &Value, pointer: &str, mut draft: Draft) -> (Draft, bool) {
    let mut node = document;
    let mut crossed = false;
    for (position, segment) in pointer.split('/').skip(1).enumerate() {
        if let Value::Object(map) = node {
            if let Some(uri) = map.get("$schema").and_then(Value::as_str) {
                draft = effective_draft(Draft::from_schema_uri(uri));
            }
            // The document root is the relocation subject itself, not a nested boundary.
            if position > 0 && establishes_resource(map, draft) {
                crossed = true;
            }
        }
        let token = segment.replace("~1", "/").replace("~0", "~");
        let child = match node {
            Value::Object(map) => map.get(&token),
            Value::Array(items) => token
                .parse::<usize>()
                .ok()
                .and_then(|index| items.get(index)),
            _ => None,
        };
        match child {
            Some(child) => node = child,
            None => return (draft, crossed),
        }
    }
    (draft, crossed)
}

/// Visit every subschema object reachable in `document` under `draft`: structurally through the
/// dialect's applicator and schema-container keywords, and through same-document pointer `$ref`s
/// (schemas referenced out of containers the dialect does not know). Embedded `$schema` markers
/// switch the dialect for their subtree. `visit` returns whether to descend below the object.
/// Pointer targets behind a nested resource root are followed only when `follow_into_resources`
/// is set - their refs resolve against the nested base, not the document root.
fn for_each_reachable_schema_object(
    document: &mut Value,
    draft: Draft,
    follow_into_resources: bool,
    visit: &mut dyn FnMut(&mut Map<String, Value>, Draft) -> bool,
) {
    let root_draft = draft;
    let mut visited = ahash::AHashSet::new();
    let mut pending: Vec<(String, Draft, bool)> = vec![(String::new(), draft, false)];
    while let Some((path, entry_draft, is_pointer_target)) = pending.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }
        let entry_draft = if is_pointer_target {
            // A pointer jumps over the ancestor chain; recover its dialect (and resource
            // boundaries) by walking the path.
            let (target_draft, crossed) = pointer_target_scope(&*document, &path, root_draft);
            if crossed && !follow_into_resources {
                continue;
            }
            target_draft
        } else {
            entry_draft
        };
        let Some(Value::Object(object)) = document.pointer_mut(&path) else {
            continue;
        };
        let draft = match object.get("$schema").and_then(Value::as_str) {
            Some(uri) => effective_draft(Draft::from_schema_uri(uri)),
            None => entry_draft,
        };
        // The pointer target is captured before `visit`, which may rewrite the `$ref` value.
        // Fragments may be percent-encoded; resolution decodes them before pointer lookup.
        if let Some(pointer) = object
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|reference| reference.strip_prefix('#'))
            .filter(|pointer| pointer.starts_with('/'))
            .and_then(|pointer| {
                percent_encoding::percent_decode_str(pointer)
                    .decode_utf8()
                    .ok()
            })
        {
            pending.push((pointer.into_owned(), draft, true));
        }
        if !visit(object, draft) {
            continue;
        }
        let single_schema = |keyword: &str| {
            matches!(
                keyword,
                "additionalProperties"
                    | "additionalItems"
                    | "items"
                    | "not"
                    | "if"
                    | "then"
                    | "else"
                    | "contains"
                    | "propertyNames"
                    | "unevaluatedItems"
                    | "unevaluatedProperties"
                    | "contentSchema"
            )
        };
        let schema_array = |keyword: &str| {
            matches!(
                keyword,
                "allOf" | "anyOf" | "oneOf" | "prefixItems" | "items"
            )
        };
        let schema_map = |keyword: &str| {
            matches!(
                keyword,
                "properties" | "patternProperties" | "definitions" | "$defs" | "dependentSchemas"
            )
        };
        for (keyword, value) in object.iter() {
            if !draft.is_known_keyword(keyword) {
                continue;
            }
            // Keywords are fixed names free of `~`/`/`; only map keys need RFC 6901 escaping.
            let keyed_child = |key: &str| {
                let mut child = format!("{path}/{keyword}/");
                referencing::write_escaped_str(&mut child, key);
                child
            };
            match value {
                Value::Object(_) if single_schema(keyword) => {
                    pending.push((format!("{path}/{keyword}"), draft, false));
                }
                Value::Array(items) if schema_array(keyword) => {
                    for index in 0..items.len() {
                        pending.push((format!("{path}/{keyword}/{index}"), draft, false));
                    }
                }
                Value::Object(entries) if schema_map(keyword) => {
                    for key in entries.keys() {
                        pending.push((keyed_child(key), draft, false));
                    }
                }
                // Schema-form `dependencies` entries are subschemas; array-form entries are
                // property-name lists.
                Value::Object(entries) if keyword == "dependencies" => {
                    for (key, entry) in entries {
                        if entry.is_object() {
                            pending.push((keyed_child(key), draft, false));
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn promoted_draft(left: Draft, right: Draft) -> Draft {
    // `Draft::Unknown` is the greatest `Ord` variant, so a plain `max` would leak that hidden sentinel out of
    // a result mixing it with a concrete draft. Prefer the concrete operand.
    match (left, right) {
        (Draft::Unknown, other) | (other, Draft::Unknown) => other,
        (left, right) => left.max(right),
    }
}

fn strip_unasserted_formats_in_definitions(
    definitions: &Arc<DefinitionMap>,
    draft: Draft,
    validate_formats: bool,
    result_validate_formats: bool,
) -> Arc<DefinitionMap> {
    if !result_validate_formats || definitions.is_empty() {
        return Arc::clone(definitions);
    }
    let mut changed = false;
    let mut stripped = DefinitionMap::new();
    for (uri, body) in definitions.iter() {
        let next = strip_unasserted_formats(body, draft, validate_formats, result_validate_formats);
        changed |= !Arc::ptr_eq(&next, body);
        stripped.insert(Arc::clone(uri), next);
    }
    if changed {
        Arc::new(stripped)
    } else {
        Arc::clone(definitions)
    }
}

fn strip_unasserted_formats(
    schema: &SharedSchema,
    draft: Draft,
    validate_formats: bool,
    result_validate_formats: bool,
) -> SharedSchema {
    if !result_validate_formats {
        return Arc::clone(schema);
    }
    if schema.mask.is_disjoint(CanonicalKind::String) {
        return Arc::clone(schema);
    }
    let schema = walk::map_children(schema, |child| {
        strip_unasserted_formats(child, draft, validate_formats, result_validate_formats)
    });
    let Schema::String(leaf) = schema.as_schema() else {
        return schema;
    };
    let Some(format) = &leaf.format else {
        return schema;
    };
    if validate_formats && is_known_format(draft, format) {
        return schema;
    }
    let mut leaf = leaf.clone();
    leaf.format = None;
    shared(Schema::String(leaf))
}

/// `true` when a non-`False` canonical form is guaranteed to admit a value.
///
/// Canonicalisation collapses every empty ref-free schema to `False`, so non-`False` means inhabited - except for
/// references, recursion, surviving `allOf`/`oneOf`/`not`/`if`, and leaf facets the pipeline does not decide.
pub(crate) fn is_decidably_inhabited(schema: &SharedSchema, formats_asserted: bool) -> bool {
    is_decidably_inhabited_memo(schema, formats_asserted, &mut AHashMap::new())
}

fn is_decidably_inhabited_memo(
    schema: &SharedSchema,
    formats_asserted: bool,
    cache: &mut AHashMap<*const (), bool>,
) -> bool {
    let key = Arc::as_ptr(schema).cast::<()>();
    if let Some(&cached) = cache.get(&key) {
        return cached;
    }
    let result = match schema.as_schema() {
        Schema::Null
        | Schema::Boolean(_)
        | Schema::MultiType(_)
        | Schema::Const(_)
        | Schema::Enum(_)
        | Schema::True => true,
        Schema::Integer(leaf) => leaf.inhabited(formats_asserted).is_proven(),
        Schema::Number(leaf) => leaf.inhabited(formats_asserted).is_proven(),
        Schema::String(leaf) => leaf.inhabited(formats_asserted).is_proven(),
        // A union admits a value iff some branch provably does.
        Schema::AnyOf(branches) => branches
            .iter()
            .any(|branch| is_decidably_inhabited_memo(branch, formats_asserted, cache)),
        // A type tag only narrows; the reduced body carries the inhabitation.
        Schema::TypedGroup { body, .. } | Schema::TypeGuard { body, .. } => {
            is_decidably_inhabited_memo(body, formats_asserted, cache)
        }
        // Containers stay decidable only while every child schema is: a reference child reopens emptiness
        // (e.g. `minItems` forcing elements through an empty ref).
        Schema::Array(leaf) => {
            leaf.inhabited(formats_asserted).is_proven()
                && leaf
                    .prefix
                    .iter()
                    .all(|child| is_decidably_inhabited_memo(child, formats_asserted, cache))
                && is_decidably_inhabited_memo(&leaf.tail, formats_asserted, cache)
                && leaf.contains.iter().all(|clause| {
                    is_decidably_inhabited_memo(&clause.schema, formats_asserted, cache)
                })
        }
        Schema::Object(leaf) => {
            leaf.inhabited(formats_asserted).is_proven()
                && leaf
                    .constraints
                    .iter()
                    .all(|constraint| is_decidably_inhabited_memo(&constraint.schema, formats_asserted, cache))
                // An existential needs an inhabited witness schema.
                && leaf.requirements.iter().all(|requirement| match requirement {
                    ObjectRequirement::PatternPropertyRequirement { schema, .. } => {
                        is_decidably_inhabited_memo(schema, formats_asserted, cache)
                    }
                    _ => true,
                })
                && leaf
                    .property_names
                    .as_ref()
                    .is_none_or(|names| is_decidably_inhabited_memo(names, formats_asserted, cache))
        }
        Schema::AllOf(_)
        | Schema::OneOf(_)
        | Schema::Not(_)
        | Schema::IfThenElse(_)
        | Schema::Reference(_)
        | Schema::Recursive(_)
        | Schema::DynamicRef(_)
        | Schema::Raw(_)
        | Schema::False => false,
    };
    cache.insert(key, result);
    result
}

impl AsRef<Schema> for CanonicalSchema {
    fn as_ref(&self) -> &Schema {
        self.inner.as_schema()
    }
}

impl PartialEq<Schema> for CanonicalSchema {
    fn eq(&self, other: &Schema) -> bool {
        self.inner.as_schema() == other
    }
}

impl PartialEq<CanonicalSchema> for Schema {
    fn eq(&self, other: &CanonicalSchema) -> bool {
        self == other.inner.as_schema()
    }
}
