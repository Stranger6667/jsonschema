//! The fixpoint rewriter: value-set-preserving transforms applied until the schema stops changing.
//!
//! Every pass maps a schema to an *equal* one (same accepted set); `algebra` builds a *new* set and feeds it back
//! through this pipeline, so the two are mutually recursive (bounded by interning and memoization). Two ordered
//! groups:
//!
//! - [`structural`] - pick one representative per equivalence class (fold value sets, desugar `if`/`then`/`else`,
//!   eliminate `not`, partition `allOf` by type).
//! - [`normalize`] - tighten leaves and detect emptiness (snap bounds, cap lengths, close requirements, map
//!   contradictions to `false`).
//!
//! [`walk`] preserves `Arc` identity when nothing changes - the fixpoint loop relies on that to detect convergence.

pub(crate) mod normalize;
pub(crate) mod structural;
pub(crate) mod walk;

use std::sync::Arc;

use crate::canonical::{context::CanonicalizationContext, ir::SharedSchema};

/// Bounds the fixpoint loop. Every pass preserves the accepted value-set, so stopping early yields an exact but
/// possibly non-canonical form instead of hanging on a non-converging pass pair.
const MAX_ITERATIONS: u32 = 256;

/// Run the pipeline to a fixpoint. A pass signals "no change" by returning the same `Arc`, which `Arc::ptr_eq` detects.
pub(crate) fn canonicalize_ir(
    schema: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> SharedSchema {
    // Interning makes the `ptr_eq` fixpoint test exact: a pass rebuilding an equal tree converges
    // instead of looping on fresh allocations.
    let mut current = walk::intern_tree(schema, ctx);
    for _ in 0..MAX_ITERATIONS {
        let next = walk::intern_tree(&canonicalize_one(&current, ctx), ctx);
        if Arc::ptr_eq(&current, &next) {
            return next;
        }
        current = next;
    }
    debug_assert!(
        false,
        "canonicalize_ir did not converge after {MAX_ITERATIONS} iterations"
    );
    current
}

/// One full pass over the schema: [`structural`] stages first (logical shape), then [`normalize`] (tighten leaves,
/// detect emptiness). The call sequence below is the authoritative order; `canonicalize_ir` repeats it to a fixpoint.
fn canonicalize_one(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
    let next = structural::const_enum::canonicalize(schema, ctx);
    let next = structural::one_of::canonicalize(&next, ctx);
    let next = structural::if_then_else::canonicalize(&next, ctx);
    let next = structural::not_elim::canonicalize(&next, ctx);
    let next = structural::type_partition::canonicalize(&next, ctx);
    let next = normalize::numeric::normalize(&next, ctx);
    let next = normalize::array::normalize(&next, ctx);
    let next = normalize::object::normalize(&next, ctx);
    let next = normalize::emptiness::normalize(&next, ctx);
    normalize::collapse(&next, ctx)
}
