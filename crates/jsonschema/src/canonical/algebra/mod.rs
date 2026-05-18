//! Schema algebra: the operations that compute a *new* value-set from existing canonical schemas.
//!
//! Unlike `rewrite` (value-set-preserving) and `oracle` (read-only), the algebra builds a different set:
//!
//! - [`intersect`] - `A ∩ B`, validating iff both inputs do.
//! - [`negate`] - the complement `¬A`.
//!
//! `union`/`subtract` derive from these as combinators on
//! [`CanonicalSchema`](crate::canonical::CanonicalSchema). Per-type leaf reasoning goes through the
//! [`leaves`](crate::canonical::leaves) domains via the `Leaf` contract.

pub(crate) mod intersect;
pub(crate) mod negate;
