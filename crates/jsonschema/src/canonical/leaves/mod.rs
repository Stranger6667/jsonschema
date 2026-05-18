//! Per-type leaf domains and the contract every domain implements.
//!
//! Each submodule owns one JSON type's leaf. The [`Leaf`] contract (conservative verdicts plus
//! `intersect`/`covers`/`inhabited`) is what the operation layers delegate per-type work down to.

pub(crate) mod numeric;
pub(crate) mod object;

use crate::canonical::{context::CanonicalizationContext, ir::Schema, prover::Prover};

/// Outcome of intersecting two same-kind canonical leaves.
pub(crate) enum Intersection<T> {
    /// The single combined leaf.
    Merged(T),
    /// Provably empty.
    Empty,
    /// Not representable as one leaf; the caller keeps both (as `AllOf`).
    Residual,
}

/// Per-domain schema algebra. Implementations may assume both leaves are canonical;
/// `inhabited` judges only leaf-local facets, child schemas are the caller's responsibility.
pub(crate) trait Leaf: Sized {
    fn intersect(&self, other: &Self, ctx: &CanonicalizationContext) -> Intersection<Self>;
    /// `other ⊆ self`, conservative: `Proven` is a guarantee, `Unknown` decides nothing.
    fn covers(&self, other: &Self, prover: &Prover<'_>) -> Verdict;
    /// Inhabitation of the leaf's own facets. `formats_asserted` gates `format`; children
    /// (prefix/tail/constraints/...) are judged by the caller.
    fn inhabited(&self, formats_asserted: bool) -> Verdict;
    /// `true` when the leaf carries no facet constraints — the unconstrained form of its type.
    fn is_open(&self) -> bool;
    /// `true` when the leaf is provably empty (no value of its type satisfies its facets).
    /// Called on pre-canonical leaves during normalization to collapse them to `False`.
    /// `ctx` is consumed by the array/object impls; the scalar leaves ignore it.
    fn is_empty(&self, ctx: &CanonicalizationContext) -> bool;
}

/// Ties a leaf domain to its [`Schema`] variant so dispatch is written once (`project` in, `wrap`
/// out) instead of re-matching all five variants per call site.
pub(crate) trait TypedLeaf: Leaf {
    fn wrap(self) -> Schema;
    /// Borrow this leaf out of a `Schema`, or `None` for a different variant.
    fn project(schema: &Schema) -> Option<&Self>;
}

/// Conservative verdict: `Proven` is a guarantee; `Unknown` decides nothing.
/// No `Refuted`: sound non-containment needs a decidably inhabited residual, which is the
/// `is_subschema_of` pipeline's judgment, not a leaf relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Proven,
    Unknown,
}

impl Verdict {
    pub(crate) fn proven_if(condition: bool) -> Self {
        if condition {
            Self::Proven
        } else {
            Self::Unknown
        }
    }

    pub(crate) fn is_proven(self) -> bool {
        matches!(self, Self::Proven)
    }
}

/// Sound 3-valued value-membership verdict: `Yes`/`No` are guarantees, `Unknown` decides nothing.
/// Unlike [`Verdict`] this is bidirectional - a `No` is as trustworthy as a `Yes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Membership {
    Yes,
    No,
    Unknown,
}

impl Membership {
    pub(crate) fn from_bool(value: bool) -> Self {
        if value {
            Self::Yes
        } else {
            Self::No
        }
    }

    /// Negate a verdict for `Not`; `Unknown` stays undecided.
    pub(crate) fn negate(self) -> Self {
        match self {
            Self::Yes => Self::No,
            Self::No => Self::Yes,
            Self::Unknown => Self::Unknown,
        }
    }
}
