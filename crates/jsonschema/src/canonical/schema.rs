//! The public canonical schema handle and its algebra.
//!
//! [`CanonicalSchema`] is the result of canonicalization and the receiver for the schema algebra
//! (`intersect`/`union`/`negate`/`subtract`) and the decision queries (`is_satisfiable`/`is_subschema_of`).

use std::{
    cmp::Ordering,
    hash::{Hash, Hasher},
    sync::Arc,
};

use referencing::Draft;
use serde_json::Value;

use crate::{
    canonical::{
        canonicalize_ir,
        context::CanonicalizationContext,
        coverage,
        definitions::{disambiguate_definitions, reachable_definitions, union_definitions},
        emit,
        intern::{self, shared},
        intersect,
        ir::{ObjectRequirement, Schema, SharedSchema},
        leaves::Leaf,
        membership, negate,
        oracle::prover,
        walk, DefinitionMap,
    },
    keywords::format::is_known_format,
    options::PatternEngineOptions,
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

impl PartialEq for CanonicalSchema {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner && self.definitions == other.definitions
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
            .then_with(|| self.definitions.cmp(&other.definitions))
    }
}

impl Hash for CanonicalSchema {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
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
    fn context(&self) -> CanonicalizationContext {
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
    /// assertions.
    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        // Operands are already canonical; `intersect_canonical` canonicalizes the result.
        self.binary_operation(other, |left, right, ctx| {
            intersect::intersect_canonical(&left, &right, ctx)
        })
    }

    /// Return a schema that validates iff either input validates.
    ///
    /// Emits under the newer operand draft. Uses `self.pattern_options`; format assertions from either operand remain
    /// assertions.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        self.binary_operation(other, |left, right, ctx| {
            canonicalize_ir(&intern::shared(Schema::AnyOf(vec![left, right])), ctx)
        })
    }

    /// Shared skeleton for binary operations: disambiguate colliding definition keys, combine the operands, then
    /// prune the merged definitions down to what the result still references.
    fn binary_operation(
        &self,
        other: &Self,
        combine: impl FnOnce(SharedSchema, SharedSchema, &CanonicalizationContext) -> SharedSchema,
    ) -> Self {
        let draft = promoted_draft(self.draft, other.draft);
        let validate_formats = self.validate_formats || other.validate_formats;
        // `with_draft` resets format assertions to the draft default; re-apply the combined operand setting.
        let ctx = self
            .context()
            .with_draft(draft)
            .with_format_assertions(validate_formats);
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
        let inner = combine(self_inner, other_inner, &ctx);
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
        if matches!(self.as_schema(), Schema::False) {
            return false;
        }
        let ctx = self.context();
        !membership::is_provably_empty(self.as_schema(), &ctx)
    }

    /// Get the JSON Schema draft version of this canonical schema.
    #[must_use]
    pub fn draft(&self) -> Draft {
        self.draft
    }

    /// Return a schema that validates iff this one does not.
    ///
    /// Uses `self.draft` and `self.pattern_options`.
    #[must_use]
    pub fn negate(&self) -> Self {
        // `negate` emits a non-canonical tree; re-run the pipeline to restore the canonical-form invariant.
        let ctx = self.context();
        let inner = canonicalize_ir(
            &negate::negate_for_draft(&self.inner, self.draft, &ctx),
            &ctx,
        );
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
        // Fast structural proof of containment (also decides some reference cases the residual check cannot).
        let ctx = self.context();
        let prover = prover::Prover::new(&ctx, &other.definitions, &self.definitions);
        if coverage::covers_with(&prover, &other.inner, &self.inner) {
            return Some(true);
        }
        let residual = self.subtract(other);
        // Residual `False` means `self \ other` is empty, so `self` is a subschema (sound: every empty schema
        // canonicalizes to `False`).
        if !residual.is_satisfiable() {
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

fn promoted_draft(left: Draft, right: Draft) -> Draft {
    left.max(right)
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
    let inhabited = |child: &SharedSchema| is_decidably_inhabited(child, formats_asserted);
    match schema.as_schema() {
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
        Schema::AnyOf(branches) => branches.iter().any(inhabited),
        // A type tag only narrows; the reduced body carries the inhabitation.
        Schema::TypedGroup { body, .. } | Schema::TypeGuard { body, .. } => inhabited(body),
        // Containers stay decidable only while every child schema is: a reference child reopens emptiness
        // (e.g. `minItems` forcing elements through an empty ref).
        Schema::Array(leaf) => {
            leaf.inhabited(formats_asserted).is_proven()
                && leaf.prefix.iter().all(inhabited)
                && inhabited(&leaf.tail)
                && leaf.contains.iter().all(|clause| inhabited(&clause.schema))
        }
        Schema::Object(leaf) => {
            leaf.inhabited(formats_asserted).is_proven()
                && leaf
                    .constraints
                    .iter()
                    .all(|constraint| inhabited(&constraint.schema))
                // An existential needs an inhabited witness schema.
                && leaf.requirements.iter().all(|requirement| match requirement {
                    ObjectRequirement::PatternPropertyRequirement { schema, .. } => {
                        inhabited(schema)
                    }
                    _ => true,
                })
                && leaf.property_names.as_ref().is_none_or(inhabited)
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
    }
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
