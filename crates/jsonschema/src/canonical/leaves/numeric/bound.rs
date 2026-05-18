#![cfg_attr(
    not(feature = "arbitrary-precision"),
    allow(clippy::trivially_copy_pass_by_ref, clippy::wrong_self_convention)
)]

use std::cmp::Ordering;

use num_traits::One;

use crate::canonical::ir::{
    BoundFraction, BoundInteger, CanonicalJson, IntegerBounds, NumberBounds,
};

use super::magnitude_to_integer;

macro_rules! div_floor_parts {
    ($numerator:expr, $denominator:expr) => {{
        let (quotient, remainder) =
            $crate::canonical::ir::BoundInteger::div_rem($numerator, $denominator);
        if remainder.is_negative() {
            quotient - $crate::canonical::ir::BoundInteger::one()
        } else {
            quotient
        }
    }};
}

macro_rules! div_ceil_parts {
    ($numerator:expr, $denominator:expr) => {{
        let (quotient, remainder) =
            $crate::canonical::ir::BoundInteger::div_rem($numerator, $denominator);
        if remainder.is_zero() || remainder.is_negative() {
            quotient
        } else {
            quotient + $crate::canonical::ir::BoundInteger::one()
        }
    }};
}

impl IntegerBounds {
    /// Least admissible integer: minimum stepped past an exclusive bound; `None` if unbounded below or overflowing.
    #[must_use]
    pub(crate) fn effective_minimum(&self) -> Option<BoundInteger> {
        let value = self.minimum.as_ref()?;
        if self.exclusive_minimum {
            value.checked_increment()
        } else {
            Some(value.owned())
        }
    }

    /// Greatest admissible integer: maximum stepped past an exclusive bound; `None` if unbounded above or overflowing.
    #[must_use]
    pub(crate) fn effective_maximum(&self) -> Option<BoundInteger> {
        let value = self.maximum.as_ref()?;
        if self.exclusive_maximum {
            value.checked_decrement()
        } else {
            Some(value.owned())
        }
    }
}

impl BoundInteger {
    /// Least common multiple, returning `None` when it overflows `i64` in the default build.
    ///
    /// `multipleOf` operands are positive, so the result is non-negative; never overflows under `arbitrary-precision`.
    #[must_use]
    pub(crate) fn checked_lcm(&self, other: &BoundInteger) -> Option<BoundInteger> {
        if self.is_zero() || other.is_zero() {
            return Some(BoundInteger::zero());
        }
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            let gcd = self.gcd(other);
            (self / gcd).checked_mul(other).map(|value| value.abs())
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            Some(self.lcm(other))
        }
    }

    /// Whether `self` divides `value`: the remainder is zero. A zero divisor divides nothing.
    #[must_use]
    pub(crate) fn divides(&self, value: &BoundInteger) -> bool {
        !self.is_zero() && value.mod_floor(self).is_zero()
    }

    /// Smallest multiple of positive `modulus` at or above `self`; `None` when it is not
    /// representable in the default build.
    #[must_use]
    pub(crate) fn checked_next_multiple_of(&self, modulus: &BoundInteger) -> Option<BoundInteger> {
        let remainder = self.mod_floor(modulus);
        if remainder.is_zero() {
            Some(self.owned())
        } else {
            self.checked_add(&(modulus - remainder))
        }
    }
}

impl BoundFraction {
    /// Least common multiple of two `multipleOf` fractions: `lcm(p1, p2) / gcd(q1, q2)` in lowest terms.
    /// `None` when the numerator LCM overflows the bound type.
    #[must_use]
    pub(crate) fn checked_lcm(&self, other: &BoundFraction) -> Option<BoundFraction> {
        let left_numerator = magnitude_to_integer!(self.numer()?)?;
        let left_denominator = magnitude_to_integer!(self.denom()?)?;
        let right_numerator = magnitude_to_integer!(other.numer()?)?;
        let right_denominator = magnitude_to_integer!(other.denom()?)?;
        if left_numerator.is_zero() || right_numerator.is_zero() {
            return Some(BoundFraction::from(BoundInteger::zero()));
        }
        let numerator = left_numerator.checked_lcm(&right_numerator)?;
        // `multipleOf` denominators are positive, so their gcd is too; no zero-divisor guard needed.
        let denominator = left_denominator.gcd(&right_denominator);
        let result = BoundFraction::from(numerator) / BoundFraction::from(denominator);
        // A merged modulus that cannot be emitted exactly would round through `f64` and enforce a
        // different grid than the leaf holds; declining keeps the strict `AllOf`.
        #[cfg(not(feature = "arbitrary-precision"))]
        if !result.emits_exactly() {
            return None;
        }
        Some(result)
    }

    /// The exact numerator on the bound carrier; `None` past `i64` in the default build.
    #[must_use]
    pub(crate) fn integer_numerator(&self) -> Option<BoundInteger> {
        magnitude_to_integer!(self.numer()?)
    }

    #[must_use]
    pub(crate) fn denominator_is_one(&self) -> bool {
        self.denom().is_some_and(One::is_one)
    }

    /// Greatest integer at or below `self`; `None` when the fraction parts overflow the carrier.
    #[must_use]
    pub(crate) fn floor(&self) -> Option<BoundInteger> {
        let (signed_numerator, denominator) = signed_fraction_parts(self)?;
        Some(div_floor_parts!(&signed_numerator, &denominator))
    }

    /// Least integer at or above `self`; `None` when the fraction parts overflow the carrier.
    #[must_use]
    pub(crate) fn ceil(&self) -> Option<BoundInteger> {
        let (signed_numerator, denominator) = signed_fraction_parts(self)?;
        Some(div_ceil_parts!(&signed_numerator, &denominator))
    }

    /// Smallest integer at or above `self`; `None` on overflow.
    #[must_use]
    pub(crate) fn ceil_integer(&self) -> Option<BoundInteger> {
        let floor = self.floor()?;
        if BoundFraction::from(floor.owned()).cmp(self) == Ordering::Equal {
            Some(floor)
        } else {
            floor.checked_increment()
        }
    }

    #[must_use]
    pub(crate) fn floor_div(&self, modulus: &BoundFraction) -> Option<BoundInteger> {
        let ratio = self.checked_div(modulus)?;
        let (signed_numerator, denominator) = signed_fraction_parts(&ratio)?;
        Some(div_floor_parts!(&signed_numerator, &denominator))
    }

    #[must_use]
    pub(crate) fn ceil_div(&self, modulus: &BoundFraction) -> Option<BoundInteger> {
        let ratio = self.checked_div(modulus)?;
        let (signed_numerator, denominator) = signed_fraction_parts(&ratio)?;
        Some(div_ceil_parts!(&signed_numerator, &denominator))
    }

    /// The exact integer value, or `None` when the fraction is not a whole number or (in the default
    /// build) does not fit `i64`. Value-preserving: never rounds or saturates.
    #[must_use]
    pub(crate) fn to_integer(&self) -> Option<BoundInteger> {
        if !self.denominator_is_one() {
            return None;
        }
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            let magnitude = *self.numer()?;
            // `|i64::MIN| == 2^63` has no positive `i64` form; map it directly.
            if self.is_sign_negative() && magnitude == i64::MAX as u64 + 1 {
                return Some(BoundInteger::from(i64::MIN));
            }
            let numerator = BoundInteger::from(i64::try_from(magnitude).ok()?);
            Some(apply_fraction_sign(self, numerator))
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            let numerator = magnitude_to_integer!(self.numer()?)?;
            Some(apply_fraction_sign(self, numerator))
        }
    }

    #[must_use]
    pub(crate) fn to_canonical_json(&self) -> Option<CanonicalJson> {
        if self.denominator_is_one() {
            let value = self.to_integer()?;
            let value = value.to_i64()?;
            return Some(CanonicalJson::from_value(&serde_json::Value::Number(
                serde_json::Number::from(value),
            )));
        }
        let number = self.to_json_number()?;
        Some(CanonicalJson::from_value(&serde_json::Value::Number(
            number,
        )))
    }

    /// Whether `self` divides `value`: their ratio is a whole number. A zero divisor divides nothing.
    #[must_use]
    /// Whether `value` is a multiple of `self`; `None` when the ratio overflows the default
    /// build's magnitude carrier and divisibility cannot be decided.
    pub(crate) fn divides(&self, value: &BoundFraction) -> Option<bool> {
        if self.is_zero() {
            return Some(false);
        }
        Some(value.checked_div(self)?.denominator_is_one())
    }
}

/// Fractional values round inward (lower -> ceil, upper -> floor); fractional exclusive endpoints behave as inclusive
/// after rounding inward. `None` when a fractional bound's parts overflow the carrier.
pub(crate) fn number_bounds_to_integer(bounds: &NumberBounds) -> Option<IntegerBounds> {
    let (minimum, exclusive_minimum) = lift_bound(
        bounds.minimum.as_ref(),
        bounds.exclusive_minimum,
        BoundFraction::ceil,
    )?;
    let (maximum, exclusive_maximum) = lift_bound(
        bounds.maximum.as_ref(),
        bounds.exclusive_maximum,
        BoundFraction::floor,
    )?;
    Some(IntegerBounds {
        minimum,
        maximum,
        exclusive_minimum,
        exclusive_maximum,
    })
}

/// Fractional `v` rounds inward via `round` (`ceil` for lower, `floor` for upper) and loses
/// exclusivity; integer `v` preserves its exclusivity flag. `None` when rounding overflows.
fn lift_bound(
    value: Option<&BoundFraction>,
    exclusive: bool,
    round: fn(&BoundFraction) -> Option<BoundInteger>,
) -> Option<(Option<BoundInteger>, bool)> {
    let Some(value) = value else {
        return Some((None, false));
    };
    if let Some(integer) = value.to_integer() {
        return Some((Some(integer), exclusive));
    }
    Some((Some(round(value)?), false))
}

/// Integer projection of a number-carrier `multipleOf`: `m = p/q` in lowest terms constrains
/// integers to multiples of `p`; `p = 1` constrains nothing.
pub(crate) enum IntegerMultipleOf {
    Unconstrained,
    Multiple(BoundInteger),
}

impl IntegerMultipleOf {
    pub(crate) fn into_modulus(self) -> Option<BoundInteger> {
        match self {
            Self::Unconstrained => None,
            Self::Multiple(value) => Some(value),
        }
    }
}

/// For `m = p/q` in lowest terms the smallest integer multiple is `p`. `None` when `p` overflows
/// the carrier.
pub(crate) fn number_multiple_of_to_integer(
    multiple_of: Option<&BoundFraction>,
) -> Option<IntegerMultipleOf> {
    let Some(value) = multiple_of else {
        return Some(IntegerMultipleOf::Unconstrained);
    };
    let numerator = value.integer_numerator()?;
    if numerator.abs().is_one() {
        return Some(IntegerMultipleOf::Unconstrained);
    }
    Some(IntegerMultipleOf::Multiple(numerator))
}

/// For `not multipleOf: p/q`, integer witnesses excluded by the number constraint are multiples of `p`.
pub(crate) fn number_not_multiple_of_to_integer(value: &BoundFraction) -> Option<BoundInteger> {
    let numerator = value.integer_numerator()?;
    Some(numerator.abs())
}

fn apply_fraction_sign(value: &BoundFraction, numerator: BoundInteger) -> BoundInteger {
    if value.is_sign_negative() && !numerator.is_zero() {
        -numerator
    } else {
        numerator
    }
}

fn signed_fraction_parts(value: &BoundFraction) -> Option<(BoundInteger, BoundInteger)> {
    let numerator = magnitude_to_integer!(value.numer()?)?;
    let denominator = magnitude_to_integer!(value.denom()?)?;
    if denominator.is_zero() {
        return None;
    }
    Some((apply_fraction_sign(value, numerator), denominator))
}
