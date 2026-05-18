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

/// Fraction magnitude to `BoundInteger`, exactly; `None` past `i64` in the default build.
macro_rules! magnitude_to_integer {
    ($value:expr) => {{
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            i64::try_from(*$value)
                .ok()
                .map($crate::canonical::ir::BoundInteger::from)
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            ::std::option::Option::Some($crate::canonical::ir::BoundInteger::from(
                ::std::clone::Clone::clone($value),
            ))
        }
    }};
}

pub(crate) use magnitude_to_integer;
