//! [`BoundFraction`]: an exact rational bound or `multipleOf` modulus, abstracting the
//! `arbitrary-precision` `Fraction`/`BigFraction` fork.

#![cfg_attr(
    not(feature = "arbitrary-precision"),
    allow(
        clippy::clone_on_copy,
        // Magnitudes are non-`Copy` `BigUint` under `arbitrary-precision`.
        clippy::cloned_instead_of_copied,
        clippy::trivially_copy_pass_by_ref,
        clippy::wrong_self_convention
    )
)]
#![cfg_attr(feature = "arbitrary-precision", allow(clippy::cmp_owned))]

#[cfg(feature = "arbitrary-precision")]
use std::str::FromStr;

use super::BoundInteger;
#[cfg(feature = "arbitrary-precision")]
type InnerFraction = fraction::BigFraction;
#[cfg(not(feature = "arbitrary-precision"))]
type InnerFraction = fraction::Fraction;

/// The numerator/denominator magnitude carried by [`BoundFraction`] (`u64` default, `BigUint` under
/// `arbitrary-precision`).
#[cfg(feature = "arbitrary-precision")]
pub(crate) type FractionMagnitude = num_bigint::BigUint;
#[cfg(not(feature = "arbitrary-precision"))]
pub(crate) type FractionMagnitude = u64;

/// An exact rational bound or `multipleOf` modulus. Newtype over `fraction::Fraction` (default) or
/// `BigFraction` (`arbitrary-precision`); carries `NaN`/`Infinity` states the parser screens out.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(not(feature = "arbitrary-precision"), derive(Copy))]
pub(crate) struct BoundFraction(InnerFraction);

// The inner `GenericFraction` only implements owned `Add<Self>`/etc, so reference operands clone the
// inner first (a copy in the default build, a clone under `arbitrary-precision`).
macro_rules! forward_fraction_binops {
    ($($trait:ident :: $method:ident),* $(,)?) => {$(
        impl core::ops::$trait for BoundFraction {
            type Output = BoundFraction;
            fn $method(self, rhs: BoundFraction) -> BoundFraction {
                BoundFraction(core::ops::$trait::$method(self.0, rhs.0))
            }
        }
        impl core::ops::$trait<&BoundFraction> for BoundFraction {
            type Output = BoundFraction;
            fn $method(self, rhs: &BoundFraction) -> BoundFraction {
                BoundFraction(core::ops::$trait::$method(self.0, rhs.0.clone()))
            }
        }
        impl core::ops::$trait<BoundFraction> for &BoundFraction {
            type Output = BoundFraction;
            fn $method(self, rhs: BoundFraction) -> BoundFraction {
                BoundFraction(core::ops::$trait::$method(self.0.clone(), rhs.0))
            }
        }
        impl core::ops::$trait<&BoundFraction> for &BoundFraction {
            type Output = BoundFraction;
            fn $method(self, rhs: &BoundFraction) -> BoundFraction {
                BoundFraction(core::ops::$trait::$method(self.0.clone(), rhs.0.clone()))
            }
        }
    )*};
}
forward_fraction_binops!(Add::add, Sub::sub, Mul::mul, Div::div);

impl BoundFraction {
    /// Own a borrowed value: a copy in the default build, a clone under `arbitrary-precision`.
    pub(crate) fn owned(&self) -> Self {
        self.clone()
    }
    /// Division that survives magnitude overflow in the default build; `None` means the exact
    /// ratio is unrepresentable and the caller must stay conservative.
    pub(crate) fn checked_div(&self, rhs: &Self) -> Option<Self> {
        num_traits::CheckedDiv::checked_div(&self.0, &rhs.0).map(Self)
    }
    /// Multiplication that survives magnitude overflow in the default build.
    pub(crate) fn checked_mul(&self, rhs: &Self) -> Option<Self> {
        num_traits::CheckedMul::checked_mul(&self.0, &rhs.0).map(Self)
    }
    pub(crate) fn numer(&self) -> Option<&FractionMagnitude> {
        self.0.numer()
    }
    pub(crate) fn denom(&self) -> Option<&FractionMagnitude> {
        self.0.denom()
    }
    pub(crate) fn is_nan(&self) -> bool {
        self.0.is_nan()
    }
    pub(crate) fn is_infinite(&self) -> bool {
        self.0.is_infinite()
    }
    pub(crate) fn is_sign_negative(&self) -> bool {
        self.0.is_sign_negative()
    }
    pub(crate) fn abs(&self) -> Self {
        Self(num_traits::Signed::abs(&self.0))
    }
    pub(crate) fn is_zero(&self) -> bool {
        num_traits::Zero::is_zero(&self.0)
    }
    pub(crate) fn one() -> Self {
        Self(<InnerFraction as num_traits::One>::one())
    }
    /// Round-to-nearest IEEE-754 projection. Stored fractions are always finite (the parser screens
    /// `NaN`/`Infinity`), so the inner `ToPrimitive` conversion is sufficient.
    pub(crate) fn to_f64(&self) -> Option<f64> {
        num_traits::ToPrimitive::to_f64(&self.0)
    }

    /// This fraction as a JSON value. Under `arbitrary-precision` an exact decimal when the fraction
    /// terminates; otherwise a whole-number integer or the `f64` projection.
    pub(crate) fn to_json_value(&self) -> serde_json::Value {
        #[cfg(feature = "arbitrary-precision")]
        {
            if let Some(text) = self.to_decimal_text() {
                if serde_json::Number::from_str(&text).is_ok() {
                    return serde_json::Value::Number(serde_json::Number::from_string_unchecked(
                        text,
                    ));
                }
            }
        }
        // Emit whole-number fractions as exact JSON integers so `minimum: 5` serializes as `5` not `5.0`
        // and near-`2^63` values do not round through `f64`.
        if let Some(integer) = self.to_integer().and_then(|integer| integer.to_i64()) {
            return serde_json::Value::Number(serde_json::Number::from(integer));
        }
        let approx = self.to_f64().unwrap_or(f64::NAN);
        serde_json::Number::from_f64(approx)
            .map_or(serde_json::Value::Null, serde_json::Value::Number)
    }

    /// Whether emission preserves this value exactly: a whole `i64` (exact integer path) or a
    /// value that round-trips through the `f64` projection.
    #[cfg(not(feature = "arbitrary-precision"))]
    pub(crate) fn emits_exactly(&self) -> bool {
        if self
            .to_integer()
            .and_then(|integer| integer.to_i64())
            .is_some()
        {
            return true;
        }
        self.to_f64()
            .is_some_and(|value| Self::from(value).cmp(self) == std::cmp::Ordering::Equal)
    }

    /// This fraction as a JSON number. `None` only for the non-finite projection (never occurs for a stored bound).
    pub(crate) fn to_json_number(&self) -> Option<serde_json::Number> {
        match self.to_json_value() {
            serde_json::Value::Number(number) => Some(number),
            _ => None,
        }
    }

    /// The exact terminating decimal rendering, or `None` when the reduced denominator has a prime
    /// factor other than 2 or 5 (the decimal would not terminate) or the text fails to round-trip.
    #[cfg(feature = "arbitrary-precision")]
    fn to_decimal_text(&self) -> Option<String> {
        fn trim_trailing_zeros(text: &str) -> &str {
            text.trim_end_matches('0')
        }
        let numerator_unsigned = self.numer()?.clone();
        let denominator_unsigned = self.denom()?.clone();
        if num_traits::Zero::is_zero(&denominator_unsigned) {
            return None;
        }
        let sign_negative =
            self.is_sign_negative() && !num_traits::Zero::is_zero(&numerator_unsigned);

        // Strip 2-factors and 5-factors; larger count is the decimal shift `k` such that denominator * 10^k is a
        // power of 10.
        let mut denominator = BoundInteger::from(denominator_unsigned);
        let two = BoundInteger::from(2u32);
        let five = BoundInteger::from(5u32);
        let mut twos: u32 = 0;
        while (&denominator % &two).is_zero() {
            denominator = &denominator / &two;
            twos += 1;
        }
        let mut fives: u32 = 0;
        while (&denominator % &five).is_zero() {
            denominator = &denominator / &five;
            fives += 1;
        }
        if !num_traits::One::is_one(&denominator) {
            return None;
        }
        let shift = twos.max(fives);
        let extra_twos = shift - twos;
        let extra_fives = shift - fives;

        let numerator = BoundInteger::from(numerator_unsigned)
            * num_traits::pow(two, extra_twos as usize)
            * num_traits::pow(five, extra_fives as usize);

        let digits = numerator.to_str_radix(10);
        let shift_usize = shift as usize;
        let mut text = if shift_usize == 0 {
            digits
        } else if digits.len() > shift_usize {
            let split = digits.len() - shift_usize;
            let (integer_part, fraction_part) = digits.split_at(split);
            let trimmed_fraction = trim_trailing_zeros(fraction_part);
            if trimmed_fraction.is_empty() {
                integer_part.to_string()
            } else {
                format!("{integer_part}.{trimmed_fraction}")
            }
        } else {
            let padding = shift_usize - digits.len();
            let mut buffer = String::with_capacity(2 + padding + digits.len());
            buffer.push_str("0.");
            for _ in 0..padding {
                buffer.push('0');
            }
            buffer.push_str(&digits);
            let trimmed = trim_trailing_zeros(&buffer[2..]);
            if trimmed.is_empty() {
                "0".to_string()
            } else {
                format!("0.{trimmed}")
            }
        };
        if sign_negative && text != "0" {
            text.insert(0, '-');
        }
        // Fail closed if the rendered text doesn't round-trip.
        let parsed = BoundFraction::from_str(&text).ok()?;
        if parsed.cmp(self) != std::cmp::Ordering::Equal {
            return None;
        }
        Some(text)
    }
}

impl From<InnerFraction> for BoundFraction {
    fn from(value: InnerFraction) -> Self {
        Self(value)
    }
}

impl From<i32> for BoundFraction {
    fn from(value: i32) -> Self {
        Self(InnerFraction::from(value))
    }
}

impl From<i64> for BoundFraction {
    fn from(value: i64) -> Self {
        Self(InnerFraction::from(value))
    }
}

impl From<u64> for BoundFraction {
    fn from(value: u64) -> Self {
        Self(InnerFraction::from(value))
    }
}

impl From<usize> for BoundFraction {
    fn from(value: usize) -> Self {
        Self(InnerFraction::from(value))
    }
}

impl From<f64> for BoundFraction {
    fn from(value: f64) -> Self {
        Self(InnerFraction::from(value))
    }
}

impl std::str::FromStr for BoundFraction {
    type Err = <InnerFraction as std::str::FromStr>::Err;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        InnerFraction::from_str(s).map(Self)
    }
}

impl From<BoundInteger> for BoundFraction {
    fn from(value: BoundInteger) -> Self {
        Self(InnerFraction::from(value.into_inner()))
    }
}

#[cfg(test)]
mod tests {
    use super::{BoundFraction, BoundInteger, FractionMagnitude, InnerFraction};
    use test_case::test_case;

    fn fraction(value: i64) -> BoundFraction {
        BoundFraction::from(value)
    }

    // 3/2, reduced from 6/4.
    fn three_halves() -> BoundFraction {
        fraction(6) / fraction(4)
    }

    // Deliberate refs: exercises the reference-operand impls.
    #[allow(clippy::op_ref)]
    #[test]
    fn binop_reference_combinations() {
        let (left, right) = (fraction(6), fraction(4));
        assert_eq!(left.clone() + right.clone(), fraction(10));
        assert_eq!(&left + right.clone(), fraction(10));
        assert_eq!(left.clone() + &right, fraction(10));
        assert_eq!(&left + &right, fraction(10));
        assert_eq!(&left - &right, fraction(2));
        assert_eq!(&left * &right, fraction(24));
        assert_eq!(&left / &right, three_halves());
    }

    #[test_case(3 => false ; "positive")]
    #[test_case(-3 => true ; "negative")]
    fn is_sign_negative(value: i64) -> bool {
        fraction(value).is_sign_negative()
    }

    #[test]
    fn finite_value_is_neither_nan_nor_infinite() {
        let value = three_halves();
        assert!(!value.is_nan());
        assert!(!value.is_infinite());
    }

    #[test_case(-3 => fraction(3) ; "negative")]
    #[test_case(3 => fraction(3) ; "positive")]
    fn abs(value: i64) -> BoundFraction {
        fraction(value).abs()
    }

    #[test]
    fn one_and_numerator_denominator() {
        assert_eq!(BoundFraction::one(), fraction(1));
        let value = three_halves();
        assert_eq!(value.numer().cloned(), Some(FractionMagnitude::from(3u64)));
        assert_eq!(value.denom().cloned(), Some(FractionMagnitude::from(2u64)));
    }

    #[test_case(7 => fraction(7) ; "seven")]
    fn from_bound_integer(value: i64) -> BoundFraction {
        BoundFraction::from(BoundInteger::from(value))
    }

    #[test]
    fn numeric_conversions() {
        assert_eq!(BoundFraction::from(3i32), fraction(3));
        assert_eq!(BoundFraction::from(3u64), fraction(3));
        assert_eq!(BoundFraction::from(3usize), fraction(3));
        // 1.5 == 3/2.
        assert_eq!(BoundFraction::from(1.5f64), three_halves());
        assert_eq!(BoundFraction::from(InnerFraction::from(4i32)), fraction(4));
    }

    #[cfg(not(feature = "arbitrary-precision"))]
    #[test_case(6, 4 => Some(1.5) ; "three halves")]
    fn projects_to_f64(numerator: i64, denominator: i64) -> Option<f64> {
        (fraction(numerator) / fraction(denominator)).to_f64()
    }
}
