//! [`BoundInteger`]: a signed integer bound or `multipleOf` modulus, abstracting the
//! `arbitrary-precision` `i64`/`BigInt` fork.

#![cfg_attr(
    not(feature = "arbitrary-precision"),
    allow(
        clippy::clone_on_copy,
        clippy::trivially_copy_pass_by_ref,
        clippy::wrong_self_convention
    )
)]
#![cfg_attr(feature = "arbitrary-precision", allow(clippy::cmp_owned))]

use std::fmt;
#[cfg(feature = "arbitrary-precision")]
pub(super) type InnerInteger = num_bigint::BigInt;
#[cfg(not(feature = "arbitrary-precision"))]
pub(super) type InnerInteger = i64;

/// A signed integer bound or `multipleOf` modulus. Newtype over `i64` (default) or `BigInt`
/// (`arbitrary-precision`); arithmetic delegates to the inner type, everything else is an inherent method.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(not(feature = "arbitrary-precision"), derive(Copy))]
pub(crate) struct BoundInteger(InnerInteger);

macro_rules! forward_integer_binops {
    ($($trait:ident :: $method:ident),* $(,)?) => {$(
        impl core::ops::$trait for BoundInteger {
            type Output = BoundInteger;
            fn $method(self, rhs: BoundInteger) -> BoundInteger {
                BoundInteger(core::ops::$trait::$method(self.0, rhs.0))
            }
        }
        impl core::ops::$trait<&BoundInteger> for BoundInteger {
            type Output = BoundInteger;
            fn $method(self, rhs: &BoundInteger) -> BoundInteger {
                BoundInteger(core::ops::$trait::$method(self.0, &rhs.0))
            }
        }
        impl core::ops::$trait<BoundInteger> for &BoundInteger {
            type Output = BoundInteger;
            fn $method(self, rhs: BoundInteger) -> BoundInteger {
                BoundInteger(core::ops::$trait::$method(&self.0, rhs.0))
            }
        }
        impl core::ops::$trait<&BoundInteger> for &BoundInteger {
            type Output = BoundInteger;
            fn $method(self, rhs: &BoundInteger) -> BoundInteger {
                BoundInteger(core::ops::$trait::$method(&self.0, &rhs.0))
            }
        }
    )*};
}
forward_integer_binops!(Add::add, Sub::sub, Mul::mul, Div::div, Rem::rem);

impl core::ops::Neg for BoundInteger {
    type Output = BoundInteger;
    fn neg(self) -> BoundInteger {
        BoundInteger(-self.0)
    }
}

impl core::ops::Neg for &BoundInteger {
    type Output = BoundInteger;
    fn neg(self) -> BoundInteger {
        BoundInteger(-&self.0)
    }
}

impl From<i64> for BoundInteger {
    fn from(value: i64) -> Self {
        Self(InnerInteger::from(value))
    }
}
#[cfg(not(feature = "arbitrary-precision"))]
impl From<BoundInteger> for serde_json::Number {
    fn from(value: BoundInteger) -> Self {
        serde_json::Number::from(value.0)
    }
}

impl core::ops::AddAssign for BoundInteger {
    fn add_assign(&mut self, other: Self) {
        self.0 += other.0;
    }
}

impl core::ops::SubAssign for BoundInteger {
    fn sub_assign(&mut self, other: Self) {
        self.0 -= other.0;
    }
}

impl fmt::Display for BoundInteger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl num_traits::Zero for BoundInteger {
    fn zero() -> Self {
        Self(num_traits::Zero::zero())
    }
    fn is_zero(&self) -> bool {
        num_traits::Zero::is_zero(&self.0)
    }
}

impl num_traits::One for BoundInteger {
    fn one() -> Self {
        Self(num_traits::One::one())
    }
    fn is_one(&self) -> bool {
        num_traits::One::is_one(&self.0)
    }
}

impl num_traits::Num for BoundInteger {
    type FromStrRadixErr = <InnerInteger as num_traits::Num>::FromStrRadixErr;
    fn from_str_radix(s: &str, radix: u32) -> Result<Self, Self::FromStrRadixErr> {
        <InnerInteger as num_traits::Num>::from_str_radix(s, radix).map(Self)
    }
}

impl num_traits::Signed for BoundInteger {
    fn abs(&self) -> Self {
        Self(num_traits::Signed::abs(&self.0))
    }
    fn abs_sub(&self, other: &Self) -> Self {
        Self(num_traits::Signed::abs_sub(&self.0, &other.0))
    }
    fn signum(&self) -> Self {
        Self(num_traits::Signed::signum(&self.0))
    }
    fn is_positive(&self) -> bool {
        num_traits::Signed::is_positive(&self.0)
    }
    fn is_negative(&self) -> bool {
        num_traits::Signed::is_negative(&self.0)
    }
}

impl num_traits::ToPrimitive for BoundInteger {
    fn to_i64(&self) -> Option<i64> {
        num_traits::ToPrimitive::to_i64(&self.0)
    }
    fn to_u64(&self) -> Option<u64> {
        num_traits::ToPrimitive::to_u64(&self.0)
    }
    fn to_i128(&self) -> Option<i128> {
        num_traits::ToPrimitive::to_i128(&self.0)
    }
    fn to_u128(&self) -> Option<u128> {
        num_traits::ToPrimitive::to_u128(&self.0)
    }
    fn to_usize(&self) -> Option<usize> {
        num_traits::ToPrimitive::to_usize(&self.0)
    }
}

impl num_integer::Integer for BoundInteger {
    fn div_floor(&self, other: &Self) -> Self {
        Self(num_integer::Integer::div_floor(&self.0, &other.0))
    }
    fn mod_floor(&self, other: &Self) -> Self {
        Self(num_integer::Integer::mod_floor(&self.0, &other.0))
    }
    fn gcd(&self, other: &Self) -> Self {
        Self(num_integer::Integer::gcd(&self.0, &other.0))
    }
    fn lcm(&self, other: &Self) -> Self {
        Self(num_integer::Integer::lcm(&self.0, &other.0))
    }
    fn is_multiple_of(&self, other: &Self) -> bool {
        num_integer::Integer::is_multiple_of(&self.0, &other.0)
    }
    fn is_even(&self) -> bool {
        num_integer::Integer::is_even(&self.0)
    }
    fn is_odd(&self) -> bool {
        num_integer::Integer::is_odd(&self.0)
    }
    fn div_rem(&self, other: &Self) -> (Self, Self) {
        let (quotient, remainder) = num_integer::Integer::div_rem(&self.0, &other.0);
        (Self(quotient), Self(remainder))
    }
}

impl From<i32> for BoundInteger {
    fn from(value: i32) -> Self {
        Self(InnerInteger::from(value))
    }
}

impl From<u32> for BoundInteger {
    fn from(value: u32) -> Self {
        Self(InnerInteger::from(value))
    }
}

impl From<u64> for BoundInteger {
    fn from(value: u64) -> Self {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            Self(i64::try_from(value).unwrap_or(i64::MAX))
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            Self(num_bigint::BigInt::from(value))
        }
    }
}

#[cfg(feature = "arbitrary-precision")]
impl From<num_bigint::BigInt> for BoundInteger {
    fn from(value: num_bigint::BigInt) -> Self {
        Self(value)
    }
}

#[cfg(feature = "arbitrary-precision")]
impl From<num_bigint::BigUint> for BoundInteger {
    fn from(value: num_bigint::BigUint) -> Self {
        Self(num_bigint::BigInt::from(value))
    }
}

impl std::str::FromStr for BoundInteger {
    type Err = <InnerInteger as std::str::FromStr>::Err;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        InnerInteger::from_str(s).map(Self)
    }
}

/// `i128` into the widest lossless `serde_json::Number`; `None` past the `i64`/`u64` range.
#[cfg(feature = "arbitrary-precision")]
pub(super) fn number_from_i128(value: i128) -> Option<serde_json::Number> {
    if let Ok(small) = i64::try_from(value) {
        return Some(serde_json::Number::from(small));
    }
    if let Ok(small) = u64::try_from(value) {
        return Some(serde_json::Number::from(small));
    }
    None
}

impl BoundInteger {
    /// This bound as a JSON number, exactly.
    pub(crate) fn to_number(&self) -> serde_json::Number {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            serde_json::Number::from(self.0)
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            match num_traits::ToPrimitive::to_i128(&self.0).and_then(number_from_i128) {
                Some(small) => small,
                None => serde_json::Number::from_string_unchecked(self.0.to_string()),
            }
        }
    }

    /// Own a borrowed value: a copy in the default build, a clone under `arbitrary-precision`.
    pub(crate) fn owned(&self) -> Self {
        self.clone()
    }

    /// Consume into the inner integer; used by sibling newtypes that bridge to it (e.g. `BoundFraction`).
    pub(super) fn into_inner(self) -> InnerInteger {
        self.0
    }

    /// `BigInt`-radix rendering, used only by the `arbitrary-precision` decimal emitter.
    #[cfg(feature = "arbitrary-precision")]
    pub(crate) fn to_str_radix(&self, radix: u32) -> String {
        self.0.to_str_radix(radix)
    }

    // Inherent mirrors of `num_traits`/`num_integer` so callers need no trait imports; the trait impls
    // below cover generic bounds and fully-qualified calls.
    pub(crate) fn zero() -> Self {
        Self(num_traits::Zero::zero())
    }
    pub(crate) fn one() -> Self {
        Self(num_traits::One::one())
    }
    pub(crate) fn is_zero(&self) -> bool {
        num_traits::Zero::is_zero(&self.0)
    }
    pub(crate) fn is_one(&self) -> bool {
        num_traits::One::is_one(&self.0)
    }
    pub(crate) fn abs(&self) -> Self {
        Self(num_traits::Signed::abs(&self.0))
    }
    pub(crate) fn to_i64(&self) -> Option<i64> {
        num_traits::ToPrimitive::to_i64(&self.0)
    }
    pub(crate) fn to_u64(&self) -> Option<u64> {
        num_traits::ToPrimitive::to_u64(&self.0)
    }
    pub(crate) fn gcd(&self, other: &Self) -> Self {
        Self(num_integer::Integer::gcd(&self.0, &other.0))
    }
    // The default-build LCM goes through `gcd` + `checked_mul`; only `arbitrary-precision` uses this.
    #[cfg(feature = "arbitrary-precision")]
    pub(crate) fn lcm(&self, other: &Self) -> Self {
        Self(num_integer::Integer::lcm(&self.0, &other.0))
    }
    pub(crate) fn mod_floor(&self, other: &Self) -> Self {
        Self(num_integer::Integer::mod_floor(&self.0, &other.0))
    }

    pub(crate) fn checked_add(&self, other: &Self) -> Option<Self> {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            self.0.checked_add(other.0).map(Self)
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            Some(Self(&self.0 + &other.0))
        }
    }
    pub(crate) fn checked_sub(&self, other: &Self) -> Option<Self> {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            self.0.checked_sub(other.0).map(Self)
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            Some(Self(&self.0 - &other.0))
        }
    }
    // Only the default-build LCM path needs an overflow-checked multiply; under `arbitrary-precision`
    // multiplication never overflows.
    #[cfg(not(feature = "arbitrary-precision"))]
    pub(crate) fn checked_mul(&self, other: &Self) -> Option<Self> {
        self.0.checked_mul(other.0).map(Self)
    }
    /// The next integer, or `None` when it is not representable in the default build.
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
    /// The previous integer, or `None` when it is not representable in the default build.
    pub(crate) fn checked_decrement(&self) -> Option<Self> {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            self.0.checked_sub(1).map(Self)
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            Some(Self(&self.0 - 1))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BoundInteger;
    use num_integer::Integer;
    use num_traits::{Num, One, Signed, ToPrimitive, Zero};
    use test_case::test_case;

    fn integer(value: i64) -> BoundInteger {
        BoundInteger::from(value)
    }

    #[test_case(7 => integer(-7) ; "positive")]
    #[test_case(-7 => integer(7) ; "negative")]
    #[test_case(0 => integer(0) ; "zero")]
    fn neg_owned(value: i64) -> BoundInteger {
        -integer(value)
    }

    #[test_case(7 => integer(-7) ; "positive")]
    #[test_case(-7 => integer(7) ; "negative")]
    fn neg_borrowed(value: i64) -> BoundInteger {
        -&integer(value)
    }

    #[test_case(10, 5 => integer(15) ; "ten plus five")]
    #[test_case(0, 0 => integer(0) ; "zero plus zero")]
    fn add_assign(start: i64, delta: i64) -> BoundInteger {
        let mut value = integer(start);
        value += integer(delta);
        value
    }

    #[test_case(15, 8 => integer(7) ; "stays positive")]
    #[test_case(3, 5 => integer(-2) ; "goes negative")]
    fn sub_assign(start: i64, delta: i64) -> BoundInteger {
        let mut value = integer(start);
        value -= integer(delta);
        value
    }

    #[test_case(0 => "0")]
    #[test_case(42 => "42")]
    #[test_case(-13 => "-13")]
    fn display(value: i64) -> String {
        integer(value).to_string()
    }

    #[test_case(0 => true ; "zero")]
    #[test_case(1 => false ; "non-zero")]
    fn trait_is_zero(value: i64) -> bool {
        Zero::is_zero(&integer(value))
    }

    #[test_case(1 => true ; "one")]
    #[test_case(2 => false ; "non-one")]
    fn trait_is_one(value: i64) -> bool {
        One::is_one(&integer(value))
    }

    #[test]
    fn zero_one_constructors() {
        assert_eq!(<BoundInteger as Zero>::zero(), integer(0));
        assert_eq!(<BoundInteger as One>::one(), integer(1));
        // Inherent mirrors used by production.
        assert_eq!(BoundInteger::zero(), integer(0));
        assert!(integer(0).is_zero());
    }

    #[test_case("ff", 16 => Some(integer(255)))]
    #[test_case("101", 2 => Some(integer(5)))]
    #[test_case("zz", 16 => None)]
    fn num_from_str_radix(text: &str, radix: u32) -> Option<BoundInteger> {
        <BoundInteger as Num>::from_str_radix(text, radix).ok()
    }

    #[test_case(-7 => integer(-1) ; "negative")]
    #[test_case(0 => integer(0) ; "zero")]
    #[test_case(9 => integer(1) ; "positive")]
    fn signed_signum(value: i64) -> BoundInteger {
        Signed::signum(&integer(value))
    }

    #[test_case(3 => true ; "positive")]
    #[test_case(-3 => false ; "negative")]
    #[test_case(0 => false ; "zero")]
    fn signed_is_positive(value: i64) -> bool {
        Signed::is_positive(&integer(value))
    }

    #[test_case(-3 => true ; "negative")]
    #[test_case(3 => false ; "positive")]
    fn signed_is_negative(value: i64) -> bool {
        Signed::is_negative(&integer(value))
    }

    // `abs_sub(a, b)` is `max(a - b, 0)`.
    #[test_case(5, 3 => integer(2) ; "positive difference")]
    #[test_case(3, 5 => integer(0) ; "clamped to zero")]
    fn signed_abs_sub(left: i64, right: i64) -> BoundInteger {
        Signed::abs_sub(&integer(left), &integer(right))
    }

    #[test_case(5 => Some(5) ; "in range")]
    #[test_case(-1 => None ; "negative")]
    fn to_u64(value: i64) -> Option<u64> {
        ToPrimitive::to_u64(&integer(value))
    }

    #[test_case(-9 => Some(-9i128) ; "negative i128")]
    fn to_i128(value: i64) -> Option<i128> {
        ToPrimitive::to_i128(&integer(value))
    }

    #[test_case(7 => Some(7u128) ; "non-negative")]
    #[test_case(-1 => None ; "negative")]
    fn to_u128(value: i64) -> Option<u128> {
        ToPrimitive::to_u128(&integer(value))
    }

    #[test_case(7 => Some(7usize) ; "non-negative")]
    fn to_usize(value: i64) -> Option<usize> {
        ToPrimitive::to_usize(&integer(value))
    }

    #[test]
    fn to_u64_inherent_mirror() {
        assert_eq!(integer(5).to_u64(), Some(5));
    }

    #[test_case(12, 18 => integer(6) ; "gcd 12 18")]
    fn integer_gcd(left: i64, right: i64) -> BoundInteger {
        Integer::gcd(&integer(left), &integer(right))
    }

    #[test_case(4, 6 => integer(12) ; "lcm 4 6")]
    fn integer_lcm(left: i64, right: i64) -> BoundInteger {
        Integer::lcm(&integer(left), &integer(right))
    }

    #[test_case(12, 4 => true ; "divisible")]
    #[test_case(13, 4 => false ; "not divisible")]
    fn integer_is_multiple_of(left: i64, right: i64) -> bool {
        Integer::is_multiple_of(&integer(left), &integer(right))
    }

    #[test_case(4 => true ; "even")]
    #[test_case(3 => false ; "odd")]
    fn integer_is_even(value: i64) -> bool {
        Integer::is_even(&integer(value))
    }

    #[test_case(3 => true ; "odd")]
    #[test_case(4 => false ; "even")]
    fn integer_is_odd(value: i64) -> bool {
        Integer::is_odd(&integer(value))
    }

    #[test_case(17, 5 => (integer(3), integer(2)) ; "17 div 5")]
    fn integer_div_rem(left: i64, right: i64) -> (BoundInteger, BoundInteger) {
        Integer::div_rem(&integer(left), &integer(right))
    }

    #[test_case(5u64 => integer(5) ; "from u64")]
    fn from_u64(value: u64) -> BoundInteger {
        BoundInteger::from(value)
    }

    #[test_case(5u32 => integer(5) ; "from u32")]
    fn from_u32(value: u32) -> BoundInteger {
        BoundInteger::from(value)
    }

    #[test_case(5i32 => integer(5) ; "from i32")]
    fn from_i32(value: i32) -> BoundInteger {
        BoundInteger::from(value)
    }

    // Deliberate refs: exercises the reference-operand impls.
    #[allow(clippy::op_ref)]
    #[test]
    fn binop_reference_combinations() {
        let (left, right) = (integer(6), integer(4));
        assert_eq!(&left + &right, integer(10));
        assert_eq!(&left - &right, integer(2));
        assert_eq!(left.clone() + &right, integer(10));
        assert_eq!(&left * right.clone(), integer(24));
    }

    #[test_case("123" => Some(integer(123)))]
    #[test_case("-7" => Some(integer(-7)))]
    #[test_case("nope" => None)]
    fn from_str(text: &str) -> Option<BoundInteger> {
        text.parse::<BoundInteger>().ok()
    }

    #[test_case(5 => Some(5i64) ; "positive")]
    #[test_case(-9 => Some(-9i64) ; "negative")]
    fn to_i64(value: i64) -> Option<i64> {
        ToPrimitive::to_i64(&integer(value))
    }

    #[test]
    fn to_i64_inherent_mirror() {
        assert_eq!(integer(5).to_i64(), Some(5));
    }

    // `num_integer::div_rem` is truncated division; the remainder keeps the dividend's sign.
    #[test_case(-17, 5 => (integer(-3), integer(-2)) ; "negative dividend")]
    #[test_case(17, -5 => (integer(-3), integer(2)) ; "negative divisor")]
    fn integer_div_rem_negative(left: i64, right: i64) -> (BoundInteger, BoundInteger) {
        Integer::div_rem(&integer(left), &integer(right))
    }

    // Inherent `mod_floor` is Euclidean (floored), so it diverges from `%` for negative operands.
    #[test_case(17, 5 => integer(2) ; "positive")]
    #[test_case(-17, 5 => integer(3) ; "floored toward negative infinity")]
    fn mod_floor(left: i64, right: i64) -> BoundInteger {
        integer(left).mod_floor(&integer(right))
    }

    // Default build saturates u64 -> i64; arbitrary-precision keeps the exact value.
    #[cfg(not(feature = "arbitrary-precision"))]
    #[test_case(u64::MAX => integer(i64::MAX) ; "saturates to i64::MAX")]
    fn from_u64_saturates(value: u64) -> BoundInteger {
        BoundInteger::from(value)
    }

    // The fallible `checked_*` steps return `None` only in the default build, at the i64 range edges.
    #[cfg(not(feature = "arbitrary-precision"))]
    #[test]
    fn checked_steps_stop_at_range_edges() {
        assert_eq!(integer(i64::MAX).checked_increment(), None);
        assert_eq!(integer(i64::MIN).checked_decrement(), None);
    }

    // `lcm` is an inherent method only under arbitrary-precision; the default build derives it elsewhere.
    #[cfg(feature = "arbitrary-precision")]
    #[test]
    fn inherent_lcm() {
        assert_eq!(integer(4).lcm(&integer(6)), integer(12));
    }
}
