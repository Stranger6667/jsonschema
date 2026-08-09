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
        algebra,
        context::CanonicalizationContext,
        emit,
        error::OperandMismatch,
        ir::{Schema, SchemaKind, UncheckableFacet, Verdict},
        negate, oracle, parse, CanonicalizationError, ROOT_DEFINITION_KEY,
    },
    options::PatternEngineOptions,
};

pub(crate) type DefinitionMap = BTreeMap<Arc<str>, Schema>;

/// What a handle's pointers name: `#` its root, every other pointer an entry of its map. The two
/// travel together, or a handle read on its own binds `#` to itself.
#[derive(Clone, Debug, Eq)]
struct Document {
    root: Schema,
    definitions: Arc<DefinitionMap>,
}

impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root
            && (Arc::ptr_eq(&self.definitions, &other.definitions)
                || self.definitions == other.definitions)
    }
}

impl Ord for Document {
    fn cmp(&self, other: &Self) -> Ordering {
        self.definitions
            .cmp(&other.definitions)
            .then_with(|| self.root.cmp(&other.root))
    }
}

impl PartialOrd for Document {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Canonical JSON Schema IR handle.
#[derive(Clone, Debug)]
pub struct CanonicalSchema {
    inner: Schema,
    draft: Draft,
    pattern_options: PatternEngineOptions,
    validate_formats: bool,
    /// Shared with every child handle.
    document: Document,
}

// Draft and format-assertion policy are part of a schema's identity, not just its IR.
impl PartialEq for CanonicalSchema {
    fn eq(&self, other: &Self) -> bool {
        self.validate_formats == other.validate_formats
            && self.draft == other.draft
            && self.inner == other.inner
            && self.document == other.document
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
            .then_with(|| self.document.cmp(&other.document))
    }
}

// The definition map is left out so a handle does not hash its whole document; handles differing
// only in their targets collide.
impl Hash for CanonicalSchema {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
        self.draft.hash(state);
        self.validate_formats.hash(state);
    }
}

impl CanonicalSchema {
    /// A handle whose node is the document root, so `#` names that node.
    pub(crate) fn new(
        inner: Schema,
        draft: Draft,
        pattern_options: PatternEngineOptions,
        validate_formats: bool,
        definitions: Arc<DefinitionMap>,
    ) -> Self {
        let document = Document {
            root: inner.clone(),
            definitions,
        };
        Self {
            inner,
            draft,
            pattern_options,
            validate_formats,
            document,
        }
    }

    /// A handle for a node read against `document`, which keeps naming whatever it named there.
    fn within(
        inner: Schema,
        draft: Draft,
        pattern_options: PatternEngineOptions,
        validate_formats: bool,
        document: Document,
    ) -> Self {
        Self {
            inner,
            draft,
            pattern_options,
            validate_formats,
            document,
        }
    }

    /// Emit this canonical schema back to JSON Schema.
    #[must_use]
    pub fn to_json_schema(&self) -> Value {
        if self.inner == self.document.root {
            // Parsing already drops what the document never names, so the whole map belongs here.
            return emit::to_json_schema(&self.inner, self.draft, &self.document.definitions);
        }
        let definitions = emit::reachable_definitions(
            &self.inner,
            &self.document.root,
            &self.document.definitions,
        );
        let value = emit::to_json_schema(&self.inner, self.draft, &definitions);
        emit::rebind_document_root(value, &self.document.root, self.draft)
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

    /// Wrap a child IR node in a handle sharing this schema's draft, options, and document.
    pub(crate) fn wrap_child(&self, child: &Schema) -> Self {
        Self::within(
            child.clone(),
            self.draft,
            self.pattern_options,
            self.validate_formats,
            self.document.clone(),
        )
    }

    /// The reference target registered under `uri`, or the document itself under `#`.
    ///
    /// The target keeps the document it was written in, so a `#` inside it names that document and
    /// not the target standing in for it.
    #[must_use]
    pub fn definition(&self, uri: &str) -> Option<CanonicalSchema> {
        if uri == ROOT_DEFINITION_KEY {
            return Some(self.wrap_child(&self.document.root));
        }
        self.document
            .definitions
            .get(uri)
            .map(|body| self.wrap_child(body))
    }

    /// Every reachable reference target known to this document, keyed by its URI.
    #[must_use]
    pub fn definitions(&self) -> impl ExactSizeIterator<Item = (String, CanonicalSchema)> + '_ {
        self.document
            .definitions
            .iter()
            .map(|(uri, body)| (uri.to_string(), self.wrap_child(body)))
    }

    /// Every value both schemas admit.
    ///
    /// # Errors
    ///
    /// [`CanonicalizationError::IncompatibleOperands`] when the operands cannot be combined, and
    /// [`CanonicalizationError::UnmodeledOperand`] when either side is unmodeled or the canonical
    /// form has no exact spelling for their meet.
    pub fn intersect(&self, other: &Self) -> Result<Self, CanonicalizationError> {
        self.check_operands(other)?;
        let definitions = self.merged_definitions(other)?;
        let context =
            CanonicalizationContext::new(self.draft, self.pattern_options, self.validate_formats);
        let inner = algebra::intersect(self.inner.clone(), other.inner.clone(), &context);
        if context.saw_unspellable_meet() || context.outgrew_distribution() {
            return Err(CanonicalizationError::UnmodeledOperand);
        }
        // Two nodes of one document meet inside it, so `#` keeps its target there. Nodes of
        // different documents meet in neither, and the meet is a root of its own.
        let document = if self.document == other.document {
            self.document.clone()
        } else {
            Document {
                root: inner.clone(),
                definitions,
            }
        };
        Ok(Self::within(
            inner,
            self.draft,
            self.pattern_options,
            self.validate_formats,
            document,
        ))
    }

    /// Whether `other` admits every value this schema admits.
    ///
    /// `None` means undecided, never "not a subset": `Some(false)` is returned only against a
    /// concrete value this schema admits and `other` rejects.
    ///
    /// # Errors
    ///
    /// [`CanonicalizationError::IncompatibleOperands`] when the operands cannot be combined, and
    /// [`CanonicalizationError::UnmodeledOperand`] when either side is unmodeled.
    pub fn is_subset_of(&self, other: &Self) -> Result<Option<bool>, CanonicalizationError> {
        self.check_operands(other)?;
        // References resolve through one map, so distinct maps make the two sides incomparable for
        // the same reason they cannot be intersected.
        self.merged_definitions(other)?;
        let context =
            CanonicalizationContext::new(self.draft, self.pattern_options, self.validate_formats);
        if oracle::covers(&self.inner, &other.inner, &context) == Verdict::Admits {
            return Ok(Some(true));
        }
        let Some(values) = self.schema_kind().finite_values() else {
            return Ok(None);
        };
        let refuted = values.iter().any(|value| {
            algebra::admits_value(
                &other.inner,
                value.as_value(),
                UncheckableFacet::Undecided,
                &context,
            ) == Verdict::Rejects
        });
        Ok(refuted.then_some(false))
    }

    /// Every value this schema rejects, or `None` where the canonical form cannot spell it exactly.
    #[must_use]
    pub fn negate(&self) -> Option<Self> {
        let context =
            CanonicalizationContext::new(self.draft, self.pattern_options, self.validate_formats);
        let inner =
            negate::negate_with_definitions(&self.inner, &self.document.definitions, &context)?;
        // A resolved complement may name fewer targets than the source; carrying the dead ones
        // would emit unreferenced definitions and block combination with other documents.
        let mut definitions = (*self.document.definitions).clone();
        parse::prune_unreachable_definitions(&inner, &mut definitions);
        Some(Self::new(
            inner,
            self.draft,
            self.pattern_options,
            self.validate_formats,
            Arc::new(definitions),
        ))
    }

    fn check_operands(&self, other: &Self) -> Result<(), CanonicalizationError> {
        // An unknown `$schema` canonicalizes to `Raw` under `Draft::Unknown`, so the modeling check
        // comes first or that document reports a draft mismatch instead.
        if matches!(self.schema_kind(), SchemaKind::Raw(_))
            || matches!(other.schema_kind(), SchemaKind::Raw(_))
        {
            return Err(CanonicalizationError::UnmodeledOperand);
        }
        let mismatch = if self.draft != other.draft {
            OperandMismatch::Drafts {
                left: self.draft,
                right: other.draft,
            }
        } else if self.validate_formats != other.validate_formats {
            OperandMismatch::FormatAssertions
        } else if self.pattern_options != other.pattern_options {
            OperandMismatch::PatternEngine
        } else {
            return Ok(());
        };
        Err(CanonicalizationError::IncompatibleOperands(mismatch))
    }

    /// The map the result resolves references through.
    fn merged_definitions(
        &self,
        other: &Self,
    ) -> Result<Arc<DefinitionMap>, CanonicalizationError> {
        if Arc::ptr_eq(&self.document.definitions, &other.document.definitions)
            || other.document.definitions.is_empty()
        {
            return Ok(Arc::clone(&self.document.definitions));
        }
        if self.document.definitions.is_empty() {
            return Ok(Arc::clone(&other.document.definitions));
        }
        Err(CanonicalizationError::IncompatibleOperands(
            OperandMismatch::Definitions,
        ))
    }
}
