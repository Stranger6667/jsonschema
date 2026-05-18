//! The three sound decision oracles over canonical schemas.
//!
//! Each oracle returns `Proven`/`Unknown` (never `Refuted`), so an unknown answer propagates
//! conservatively. An oracle never changes its inputs, but answering may run the algebra and the
//! rewrite pipeline internally on scratch nodes (memoized and fuel-bounded).
//!
//! - [`membership`] - `admits`: does a value belong to a schema (plus a finite-domain emptiness proof).
//! - [`coverage`] - `covers`: is one schema a subset of another (per-structure containment checks).
//! - [`prover`] - the coinductive engine those checks run inside (assumptions, memo, fuel).

pub(crate) mod coverage;
pub(crate) mod membership;
pub(crate) mod prover;
