//! A signed integer bound.
// `InnerInteger` is `i64` by default and `BigInt` under arbitrary precision, so borrowing operands
// and cloning them is required for one and redundant for the other.
#![allow(
    clippy::clone_on_copy,
    clippy::op_ref,
    clippy::trivially_copy_pass_by_ref
)]
// `i64` has inherent sign predicates; `BigInt` takes them from `Signed`.
#[cfg(feature = "arbitrary-precision")]
use num_traits::Signed;
use num_traits::{One, Zero};

/// Which way [`BoundInteger::multiple_beyond`] rounds.
#[derive(Clone, Copy)]
pub(crate) enum Round {
    Up,
    Down,
}
#[cfg(feature = "arbitrary-precision")]
type InnerInteger = num_bigint::BigInt;
#[cfg(not(feature = "arbitrary-precision"))]
type InnerInteger = i64;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct BoundInteger(InnerInteger);

impl BoundInteger {
    /// This bound as an exact JSON number.
    pub(crate) fn to_number(&self) -> serde_json::Number {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            serde_json::Number::from(self.0)
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            match num_traits::ToPrimitive::to_i64(&self.0) {
                Some(value) => serde_json::Number::from(value),
                None => serde_json::Number::from_string_unchecked(self.0.to_string()),
            }
        }
    }

    pub(crate) fn zero() -> Self {
        Self(InnerInteger::from(0))
    }

    pub(crate) fn is_one(&self) -> bool {
        self.0.is_one()
    }

    /// Whether `f64` holds this value exactly. The runtime `multipleOf` divides in `f64`, so beyond
    /// this magnitude its verdict and exact integer arithmetic disagree.
    pub(crate) fn is_exact_in_f64(&self) -> bool {
        const LIMIT: i64 = 1 << 53;
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            self.0.unsigned_abs() <= LIMIT.unsigned_abs()
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            self.0.magnitude() <= &num_bigint::BigUint::from(LIMIT.unsigned_abs())
        }
    }

    /// The nearest integer to `number` in `direction`, or `None` when it leaves the representable
    /// range. Used to pull a fractional real bound onto the integers it admits.
    pub(crate) fn round_from_number(number: &serde_json::Number, direction: Round) -> Option<Self> {
        // An integer is already at rest; only a fractional value moves.
        if let Some(whole) = Self::from_number(number) {
            return Some(whole);
        }
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            // `i64::MAX as f64` rounds up to 2^63, so the range is bounded by that power of two
            // instead: it is exact in `f64`, and its negation is `i64::MIN` exactly.
            const LIMIT: f64 = 9_223_372_036_854_775_808.0;
            let value = number.as_f64()?;
            let rounded = match direction {
                Round::Up => value.ceil(),
                Round::Down => value.floor(),
            };
            if !(-LIMIT..LIMIT).contains(&rounded) {
                return None;
            }
            // The range check above leaves no value the cast can truncate.
            #[allow(clippy::cast_possible_truncation)]
            Some(Self(rounded as i64))
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            let value = jsonschema_value::numeric::bignum::try_parse_bigfraction(number)?;
            let rounded = match direction {
                Round::Up => value.ceil(),
                Round::Down => value.floor(),
            };
            // Rounding leaves a denominator of one, so the numerator is the magnitude already.
            let magnitude = num_bigint::BigInt::from(rounded.numer()?.clone());
            Some(Self(
                if matches!(rounded.sign(), Some(fraction::Sign::Minus)) {
                    -magnitude
                } else {
                    magnitude
                },
            ))
        }
    }

    /// Whether `self` divides `value` exactly. `self` is a positive divisor.
    pub(crate) fn divides(&self, value: &Self) -> bool {
        (&value.0 % &self.0).is_zero()
    }

    /// The least common multiple of two positive divisors, or `None` when it is not representable.
    pub(crate) fn checked_lcm(&self, other: &Self) -> Option<Self> {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            self.0
                .checked_div(gcd(&self.0, &other.0))?
                .checked_mul(other.0)
                .map(Self)
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            Some(Self(&self.0 / gcd(&self.0, &other.0) * &other.0))
        }
    }

    /// The multiple of `self` nearest `value` in `direction`, or `None` when it is not representable.
    pub(crate) fn multiple_beyond(&self, value: &Self, direction: Round) -> Option<Self> {
        Some(Self(snap(&self.0, &value.0, direction)?))
    }

    pub(crate) fn is_positive(&self) -> bool {
        self.0.is_positive()
    }

    /// This bound minus one, or `None` when that leaves the representable range.
    pub(crate) fn checked_decrement(self) -> Option<Self> {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            self.0.checked_sub(1).map(Self)
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            Some(Self(self.0 - 1))
        }
    }

    /// A signed integer from a JSON number, reading integer-valued floats; `None` for fractional values
    /// or (default build) magnitudes past `i64`.
    pub(crate) fn from_number(number: &serde_json::Number) -> Option<Self> {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            number
                .as_i64()
                .or_else(|| crate::canonical::json::integer_valued_i64(number.as_f64()?))
                .map(Self)
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            let text = number.as_str();
            let canonical = crate::canonical::json::canonical_number(text);
            let integer = canonical.as_deref().unwrap_or(text);
            let digits = integer.strip_prefix('-').unwrap_or(integer);
            if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
                integer.parse::<num_bigint::BigInt>().ok().map(Self)
            } else {
                None
            }
        }
    }
}

impl super::Discrete for BoundInteger {
    /// This bound plus one, or `None` when that leaves the representable range.
    fn checked_increment(self) -> Option<Self> {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            self.0.checked_add(1).map(Self)
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            Some(Self(self.0 + 1))
        }
    }
}

/// Round `value` to a multiple of the positive `step`, away from zero in `direction`.
fn snap(step: &InnerInteger, value: &InnerInteger, direction: Round) -> Option<InnerInteger> {
    #[cfg(not(feature = "arbitrary-precision"))]
    {
        let rest = value.checked_rem(*step)?;
        if rest.is_zero() {
            return Some(*value);
        }
        // `%` truncates toward zero, so the correction depends on the sign of the remainder.
        match direction {
            Round::Up if rest.is_negative() => value.checked_sub(rest),
            Round::Up => value.checked_sub(rest)?.checked_add(*step),
            Round::Down if rest.is_negative() => value.checked_sub(rest)?.checked_sub(*step),
            Round::Down => value.checked_sub(rest),
        }
    }
    #[cfg(feature = "arbitrary-precision")]
    {
        let rest = value % step;
        if rest.is_zero() {
            return Some(value.clone());
        }
        Some(match direction {
            Round::Up if rest.is_negative() => value - rest,
            Round::Up => value - rest + step,
            Round::Down if rest.is_negative() => value - rest - step,
            Round::Down => value - rest,
        })
    }
}

/// The greatest common divisor of two positive values.
fn gcd(left: &InnerInteger, right: &InnerInteger) -> InnerInteger {
    let mut a = left.clone();
    let mut b = right.clone();
    while !b.is_zero() {
        let rest = &a % &b;
        a = b;
        b = rest;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    #[test_case("1.5", 2, 1; "positive fraction")]
    #[test_case("-1.5", -1, -2; "negative fraction")]
    #[test_case("2", 2, 2; "positive whole")]
    #[test_case("-2", -2, -2; "negative whole")]
    #[test_case("0.0001", 1, 0; "just above zero")]
    #[test_case("-0.5", 0, -1; "just below zero")]
    #[test_case("0", 0, 0; "zero")]
    fn rounds_toward_the_named_direction(text: &str, up: i64, down: i64) {
        let number: serde_json::Number = text.parse().expect("number");
        let expected = |value: i64| BoundInteger::from_number(&value.into()).expect("whole");
        assert_eq!(
            BoundInteger::round_from_number(&number, Round::Up),
            Some(expected(up))
        );
        assert_eq!(
            BoundInteger::round_from_number(&number, Round::Down),
            Some(expected(down))
        );
    }
}
