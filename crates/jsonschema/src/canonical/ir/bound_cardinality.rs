//! [`BoundCardinality`]: a non-negative count bound (array/string/object size), abstracting the
//! `arbitrary-precision` `u64`/`BigInt` fork.

#![cfg_attr(
    not(feature = "arbitrary-precision"),
    allow(
        clippy::clone_on_copy,
        clippy::trivially_copy_pass_by_ref,
        clippy::wrong_self_convention
    )
)]
#![cfg_attr(feature = "arbitrary-precision", allow(clippy::cmp_owned))]

use std::{cmp::Ordering, fmt};
#[cfg(feature = "arbitrary-precision")]
type InnerCardinality = num_bigint::BigInt;
#[cfg(not(feature = "arbitrary-precision"))]
type InnerCardinality = u64;

/// A non-negative count bound (array/string/object size). Newtype over `u64` (default) or `BigInt`
/// (`arbitrary-precision`) so the feature fork lives in this type's methods, not at every call site.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(not(feature = "arbitrary-precision"), derive(Copy))]
pub(crate) struct BoundCardinality(InnerCardinality);

impl BoundCardinality {
    /// This count as a JSON number, exactly.
    pub(crate) fn to_number(&self) -> serde_json::Number {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            serde_json::Number::from(self.0)
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            match num_traits::ToPrimitive::to_i128(&self.0)
                .and_then(super::bound_integer::number_from_i128)
            {
                Some(small) => small,
                None => serde_json::Number::from_string_unchecked(self.0.to_string()),
            }
        }
    }

    /// Own a borrowed value: a copy in the default build, a clone under `arbitrary-precision`.
    pub(crate) fn owned(&self) -> Self {
        self.clone()
    }
    pub(crate) fn to_u64(&self) -> Option<u64> {
        num_traits::ToPrimitive::to_u64(&self.0)
    }

    pub(crate) fn to_usize(&self) -> Option<usize> {
        num_traits::ToPrimitive::to_usize(&self.0)
    }

    pub(crate) fn is_zero(&self) -> bool {
        num_traits::Zero::is_zero(&self.0)
    }

    /// The next count, or `None` when it is not representable in the default build.
    pub(crate) fn checked_increment(&self) -> Option<Self> {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            self.0.checked_add(1).map(Self)
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            Some(Self(&self.0 + 1))
        }
    }

    /// The previous count, or `None` at zero / when not representable.
    pub(crate) fn checked_decrement(&self) -> Option<Self> {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            self.0.checked_sub(1).map(Self)
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            if num_traits::Zero::is_zero(&self.0) {
                None
            } else {
                Some(Self(&self.0 - 1))
            }
        }
    }
}

impl From<u8> for BoundCardinality {
    fn from(value: u8) -> Self {
        Self(InnerCardinality::from(value))
    }
}

impl From<u64> for BoundCardinality {
    fn from(value: u64) -> Self {
        Self(InnerCardinality::from(value))
    }
}

impl PartialEq<u64> for BoundCardinality {
    fn eq(&self, other: &u64) -> bool {
        self.to_u64() == Some(*other)
    }
}

impl PartialOrd<u64> for BoundCardinality {
    // No `BoundCardinality::from(u64)` round-trip: that allocates a `BigInt` per comparison under
    // `arbitrary-precision`.
    fn partial_cmp(&self, other: &u64) -> Option<Ordering> {
        Some(match self.to_u64() {
            Some(value) => value.cmp(other),
            // Outside `u64`: a negative value sorts below every count, an oversized one above.
            #[cfg(feature = "arbitrary-precision")]
            None if num_traits::Signed::is_negative(&self.0) => Ordering::Less,
            None => Ordering::Greater,
        })
    }
}

impl PartialEq<BoundCardinality> for u64 {
    fn eq(&self, other: &BoundCardinality) -> bool {
        other == self
    }
}

impl PartialOrd<BoundCardinality> for u64 {
    fn partial_cmp(&self, other: &BoundCardinality) -> Option<Ordering> {
        other.partial_cmp(self).map(Ordering::reverse)
    }
}

impl From<usize> for BoundCardinality {
    fn from(value: usize) -> Self {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            Self(u64::try_from(value).unwrap_or(u64::MAX))
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            Self(InnerCardinality::from(value))
        }
    }
}

#[cfg(feature = "arbitrary-precision")]
impl From<num_bigint::BigUint> for BoundCardinality {
    fn from(value: num_bigint::BigUint) -> Self {
        Self(num_bigint::BigInt::from(value))
    }
}

impl std::ops::Add for BoundCardinality {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl std::ops::Sub for BoundCardinality {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        // Counts are non-negative; underflow would wrap in the default build and go negative under
        // `arbitrary-precision`.
        debug_assert!(self.0 >= other.0, "cardinality subtraction underflow");
        Self(self.0 - other.0)
    }
}

impl std::ops::AddAssign for BoundCardinality {
    fn add_assign(&mut self, other: Self) {
        self.0 += other.0;
    }
}

impl std::ops::Mul for BoundCardinality {
    type Output = Self;
    fn mul(self, other: Self) -> Self {
        Self(self.0 * other.0)
    }
}

impl fmt::Display for BoundCardinality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl num_traits::Zero for BoundCardinality {
    fn zero() -> Self {
        Self(<InnerCardinality as num_traits::Zero>::zero())
    }
    fn is_zero(&self) -> bool {
        num_traits::Zero::is_zero(&self.0)
    }
}

impl num_traits::One for BoundCardinality {
    fn one() -> Self {
        Self(<InnerCardinality as num_traits::One>::one())
    }
    fn is_one(&self) -> bool {
        num_traits::One::is_one(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::BoundCardinality;
    use num_traits::{One, Zero};
    use std::cmp::Ordering;
    use test_case::test_case;

    fn cardinality(value: u64) -> BoundCardinality {
        BoundCardinality::from(value)
    }

    #[test_case(5, 5 => Ordering::Equal ; "equal")]
    #[test_case(3, 4 => Ordering::Less ; "less")]
    #[test_case(7, 6 => Ordering::Greater ; "greater")]
    fn compares_with_u64(left: u64, right: u64) -> Ordering {
        cardinality(left).partial_cmp(&right).unwrap()
    }

    #[test_case(5, 5 => Ordering::Equal ; "equal")]
    #[test_case(4, 3 => Ordering::Greater ; "greater")]
    fn u64_compares_with_cardinality(left: u64, right: u64) -> Ordering {
        left.partial_cmp(&cardinality(right)).unwrap()
    }

    #[test_case(5, 5 => true ; "equal")]
    #[test_case(5, 6 => false ; "not equal")]
    fn cardinality_equals_u64(left: u64, right: u64) -> bool {
        cardinality(left) == right
    }

    #[test_case(5, 5 => true ; "equal")]
    #[test_case(6, 5 => false ; "not equal")]
    fn u64_equals_cardinality(left: u64, right: u64) -> bool {
        left == cardinality(right)
    }

    #[test_case(3, 4 => cardinality(7) ; "add")]
    fn addition(left: u64, right: u64) -> BoundCardinality {
        cardinality(left) + cardinality(right)
    }

    #[test_case(9, 4 => cardinality(5) ; "subtract")]
    fn subtraction(left: u64, right: u64) -> BoundCardinality {
        cardinality(left) - cardinality(right)
    }

    // Counts are non-negative; an underflowing subtraction is a caller bug and must not produce a
    // wrapped (default) or negative (arbitrary-precision) cardinality.
    #[test]
    #[should_panic(expected = "cardinality subtraction underflow")]
    fn subtraction_underflow_panics_in_debug() {
        let _ = cardinality(1) - cardinality(2);
    }

    #[test_case(3, 4 => cardinality(12) ; "multiply")]
    fn multiplication(left: u64, right: u64) -> BoundCardinality {
        cardinality(left) * cardinality(right)
    }

    #[test_case(2, 5 => cardinality(7) ; "add assign")]
    fn add_assign(start: u64, delta: u64) -> BoundCardinality {
        let mut value = cardinality(start);
        value += cardinality(delta);
        value
    }

    #[test_case(0 => true ; "zero")]
    #[test_case(3 => false ; "non-zero")]
    fn trait_is_zero(value: u64) -> bool {
        Zero::is_zero(&cardinality(value))
    }

    #[test_case(1 => true ; "one")]
    #[test_case(2 => false ; "non-one")]
    fn trait_is_one(value: u64) -> bool {
        One::is_one(&cardinality(value))
    }

    #[test]
    fn zero_one_constructors() {
        assert_eq!(<BoundCardinality as Zero>::zero(), cardinality(0));
        assert_eq!(<BoundCardinality as One>::one(), cardinality(1));
    }

    #[test_case(0 => "0" ; "zero")]
    #[test_case(42 => "42" ; "forty-two")]
    fn display(value: u64) -> String {
        cardinality(value).to_string()
    }

    #[test_case(5 => Some(cardinality(6)) ; "increment")]
    fn checked_increment(value: u64) -> Option<BoundCardinality> {
        cardinality(value).checked_increment()
    }

    #[test_case(5 => Some(cardinality(4)) ; "above zero")]
    #[test_case(0 => None ; "at zero")]
    fn checked_decrement(value: u64) -> Option<BoundCardinality> {
        cardinality(value).checked_decrement()
    }

    #[test_case(7 => Some(7usize) ; "seven")]
    fn to_usize(value: u64) -> Option<usize> {
        cardinality(value).to_usize()
    }

    #[test_case(7usize => cardinality(7) ; "from usize")]
    fn from_usize(value: usize) -> BoundCardinality {
        BoundCardinality::from(value)
    }

    // A count above `i64::MAX` is representable as `u64`/`BigInt`.
    #[test]
    fn to_u64_above_i64_range() {
        assert_eq!(BoundCardinality::from(u64::MAX).to_u64(), Some(u64::MAX));
    }

    // `checked_increment` is fallible only in the default build, at the u64 ceiling.
    #[cfg(not(feature = "arbitrary-precision"))]
    #[test]
    fn checked_increment_stops_at_max() {
        assert_eq!(BoundCardinality::from(u64::MAX).checked_increment(), None);
    }
}
