use std::{
    cmp::Ordering,
    collections::BTreeMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

use referencing::Draft;
use serde_json::Value;

use crate::{
    canonical::{
        emit,
        ir::{Schema, SchemaKind},
    },
    options::PatternEngineOptions,
};

pub(crate) type DefinitionMap = BTreeMap<Arc<str>, Schema>;

/// Canonical JSON Schema IR handle.
#[derive(Clone, Debug)]
pub struct CanonicalSchema {
    inner: Schema,
    draft: Draft,
    pattern_options: PatternEngineOptions,
    validate_formats: bool,
    /// Shared `$ref` resolution table; every child handle shares this `Arc`.
    definitions: Arc<DefinitionMap>,
}

// Draft and format-assertion policy are part of a schema's identity, not just its IR.
impl PartialEq for CanonicalSchema {
    fn eq(&self, other: &Self) -> bool {
        self.validate_formats == other.validate_formats
            && self.draft == other.draft
            && self.inner == other.inner
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
    pub(crate) fn new(
        inner: Schema,
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
        emit::to_json_schema(&self.inner, self.draft)
    }

    /// Return `false` when this schema provably admits no instances.
    ///
    /// Conservative: `true` means "not provably empty", not a satisfiability proof.
    #[must_use]
    pub fn is_satisfiable(&self) -> bool {
        // TODO(canonical): not modeled yet - only `False` is provably empty; unsatisfiable
        // combinations reduce to `False` as more constructs become modeled.
        !matches!(self.schema_kind(), SchemaKind::False)
    }

    /// Borrow the internal canonical IR kind.
    #[must_use]
    pub(crate) fn schema_kind(&self) -> &SchemaKind {
        self.inner.kind()
    }

    #[must_use]
    pub fn draft(&self) -> Draft {
        self.draft
    }

    /// Wrap a child IR node in a handle sharing this schema's draft, options, and definitions.
    pub(crate) fn wrap_child(&self, child: &Schema) -> Self {
        Self::new(
            child.clone(),
            self.draft,
            self.pattern_options,
            self.validate_formats,
            Arc::clone(&self.definitions),
        )
    }

    /// Every reference uri reachable from this schema, mapped to its canonical target.
    #[must_use]
    pub fn definitions(&self) -> impl ExactSizeIterator<Item = (String, CanonicalSchema)> + '_ {
        self.definitions
            .iter()
            .map(|(uri, body)| (uri.to_string(), self.wrap_child(body)))
    }
}
