//! Containment-query state: definitions environment, coinductive assumptions, per-query memo.

use std::{
    cell::RefCell,
    sync::{Arc, LazyLock},
};

use ahash::{AHashMap, AHashSet};

use crate::canonical::{
    context::CanonicalizationContext,
    definition_entry,
    ir::{CanonicalKind, SchemaKindSet, SchemaNode, SharedSchema},
    DefinitionMap,
};

/// Mask bits whose presence makes structural equality env-dependent: equal ref nodes only denote
/// equal value sets when both sides resolve names the same way.
const REF_LIKE: SchemaKindSet = SchemaKindSet::from_kinds(&[
    CanonicalKind::Reference,
    CanonicalKind::Recursive,
    CanonicalKind::DynamicRef,
    CanonicalKind::Raw,
]);

static EMPTY_DEFINITIONS: LazyLock<DefinitionMap> = LazyLock::new(DefinitionMap::new);

pub(crate) struct Prover<'a> {
    ctx: &'a CanonicalizationContext,
    big_definitions: &'a DefinitionMap,
    small_definitions: &'a DefinitionMap,
    /// Shared keys resolve identically, so structural equality stays trustworthy on ref subtrees.
    envs_compatible: bool,
    /// In-flight `(big, small)` pairs; a revisit is the coinductive hypothesis and proves the pair.
    /// Keys stay alive via `assumption_keys`, so addresses cannot be reused mid-query.
    assumptions: RefCell<AHashSet<(*const SchemaNode, *const SchemaNode)>>,
    assumption_keys: RefCell<Vec<(SharedSchema, SharedSchema)>>,
    /// Per-query memo for env-carrying provers (results may depend on the env / assumptions).
    local_memo: RefCell<AHashMap<(SharedSchema, SharedSchema), bool>>,
}

impl<'a> Prover<'a> {
    /// Env-less prover for pipeline callers: never resolves refs, never assumes.
    pub(crate) fn without_definitions(ctx: &'a CanonicalizationContext) -> Self {
        Self::new(ctx, &EMPTY_DEFINITIONS, &EMPTY_DEFINITIONS)
    }

    pub(crate) fn new(
        ctx: &'a CanonicalizationContext,
        big_definitions: &'a DefinitionMap,
        small_definitions: &'a DefinitionMap,
    ) -> Self {
        let envs_compatible = std::ptr::eq(big_definitions, small_definitions)
            || (big_definitions.len() == small_definitions.len()
                && big_definitions.iter().all(|(key, body)| {
                    small_definitions
                        .get(key)
                        .is_some_and(|other| Arc::ptr_eq(body, other) || body == other)
                }));
        Self {
            ctx,
            big_definitions,
            small_definitions,
            envs_compatible,
            assumptions: RefCell::new(AHashSet::new()),
            assumption_keys: RefCell::new(Vec::new()),
            local_memo: RefCell::new(AHashMap::new()),
        }
    }

    pub(crate) fn ctx(&self) -> &'a CanonicalizationContext {
        self.ctx
    }

    pub(crate) fn has_definitions(&self) -> bool {
        !self.big_definitions.is_empty() || !self.small_definitions.is_empty()
    }

    /// Structural equality that refuses to vouch for ref-carrying subtrees under incompatible envs.
    pub(crate) fn nodes_equal(&self, left: &SharedSchema, right: &SharedSchema) -> bool {
        (Arc::ptr_eq(left, right) || left == right)
            && (self.envs_compatible || left.mask.is_disjoint(REF_LIKE))
    }

    pub(crate) fn resolve_big(&self, key: &str) -> Option<&'a SharedSchema> {
        definition_entry(self.big_definitions, key).map(|(_, body)| body)
    }

    pub(crate) fn resolve_small(&self, key: &str) -> Option<&'a SharedSchema> {
        definition_entry(self.small_definitions, key).map(|(_, body)| body)
    }

    pub(crate) fn assumption_holds(&self, big: &SharedSchema, small: &SharedSchema) -> bool {
        let assumptions = self.assumptions.borrow();
        !assumptions.is_empty() && assumptions.contains(&(Arc::as_ptr(big), Arc::as_ptr(small)))
    }

    /// Record the pair for the rest of the query; always `true` so resolution arms can chain it.
    pub(crate) fn assume(&self, big: &SharedSchema, small: &SharedSchema) -> bool {
        if self
            .assumptions
            .borrow_mut()
            .insert((Arc::as_ptr(big), Arc::as_ptr(small)))
        {
            self.assumption_keys
                .borrow_mut()
                .push((Arc::clone(big), Arc::clone(small)));
        }
        true
    }

    pub(crate) fn local_memo_get(&self, big: &SharedSchema, small: &SharedSchema) -> Option<bool> {
        self.local_memo
            .borrow()
            .get(&(Arc::clone(big), Arc::clone(small)))
            .copied()
    }

    pub(crate) fn local_memo_insert(&self, big: &SharedSchema, small: &SharedSchema, value: bool) {
        self.local_memo
            .borrow_mut()
            .insert((Arc::clone(big), Arc::clone(small)), value);
    }
}
