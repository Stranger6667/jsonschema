mod bound;
mod finite;
mod leaf_algebra;
mod modular;

use crate::canonical::ir::SharedSchema;

pub(crate) use bound::{
    number_bounds_to_integer, number_multiple_of_to_integer, number_not_multiple_of_to_integer,
};
pub(crate) use finite::{bounded_integer_grid_leaf, bounded_number_grid_leaf};
pub(crate) use leaf_algebra::{negate_in_kind, numeric_leaf_covers, NumericBounds, NumericLeaf};

pub(crate) fn simplify_intersection_branches(branches: &mut Vec<SharedSchema>) -> bool {
    modular::absorb_not_multiple_of_siblings(branches)
}

/// Fraction magnitude to `BoundInteger`. Saturates past `i64::MAX` in the default build (via
/// `From<u64>`); callers relying on exactness are parse-gated to `i64`-representable magnitudes.
macro_rules! magnitude_to_integer {
    ($value:expr) => {{
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            $crate::canonical::ir::BoundInteger::from(*$value)
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            $crate::canonical::ir::BoundInteger::from(::std::clone::Clone::clone($value))
        }
    }};
}

pub(crate) use magnitude_to_integer;
