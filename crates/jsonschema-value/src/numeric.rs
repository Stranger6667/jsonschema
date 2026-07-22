#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::must_use_candidate
)]

use fraction::{BigFraction, One, Zero};
#[cfg(feature = "arbitrary-precision")]
use std::cmp::Ordering;

macro_rules! define_num_cmp {
    ($($trait_fn:ident => $fn_name:ident, $op:tt, $infinity_positive:literal, $ord_pat:pat),* $(,)?) => {
        $(
            pub fn $fn_name<N, T>(value: &N, limit: T) -> bool
            where
                N: crate::JsonNumber,
                T: Copy + num_traits::ToPrimitive,
                u64: num_cmp::NumCmp<T>,
                i64: num_cmp::NumCmp<T>,
                f64: num_cmp::NumCmp<T>,
            {
                if let Some(v) = value.as_u64() {
                    num_cmp::NumCmp::$trait_fn(v, limit)
                } else if let Some(v) = value.as_i64() {
                    num_cmp::NumCmp::$trait_fn(v, limit)
                } else if let Some(v) = value.as_f64() {
                    // Integers outside u64/i64 lose precision in `as_f64` and can round exactly
                    // onto the limit (e.g. -9223372036854775809 -> -2^63); compare them exactly.
                    #[cfg(feature = "arbitrary-precision")]
                    if v <= i64::MIN as f64 || v >= u64::MAX as f64 {
                        if let Some(big_value) = bignum::try_parse_bigint(&value.to_number()) {
                            if let Some(ordering) = bignum::compare_bigint_to_limit(&big_value, limit) {
                                return matches!(ordering, $ord_pat);
                            }
                        }
                    }
                    num_cmp::NumCmp::$trait_fn(v, limit)
                } else {
                    #[cfg(feature = "arbitrary-precision")]
                    {
                        if let Some(big_value) = bignum::try_parse_bigfraction(&value.to_number()) {
                            if let Some(limit_f64) = num_traits::ToPrimitive::to_f64(&limit) {
                                let limit_frac = BigFraction::from(limit_f64);
                                return big_value $op limit_frac;
                            }
                        }
                        // Treat unparsable numbers as infinity based on sign
                        let is_negative = value.as_str().starts_with('-');
                        if $infinity_positive {
                            !is_negative
                        } else {
                            is_negative
                        }
                    }
                    #[cfg(not(feature = "arbitrary-precision"))]
                    {
                        unreachable!("Always Some without `arbitrary-precision`")
                    }
                }
            }
        )*
    };
}

define_num_cmp!(
    num_ge => ge, >=, true, Ordering::Greater | Ordering::Equal,   // +infinity passes >=, >
    num_le => le, <=, false, Ordering::Less | Ordering::Equal,  // -infinity passes <=, <
    num_gt => gt, >, true, Ordering::Greater,
    num_lt => lt, <, false, Ordering::Less,
);

#[cfg(feature = "macros")]
pub fn eq<N, T>(value: &N, limit: T) -> bool
where
    N: crate::JsonNumber,
    T: Copy + num_traits::ToPrimitive,
    u64: num_cmp::NumCmp<T>,
    i64: num_cmp::NumCmp<T>,
    f64: num_cmp::NumCmp<T>,
{
    if let Some(v) = value.as_u64() {
        num_cmp::NumCmp::num_eq(v, limit)
    } else if let Some(v) = value.as_i64() {
        num_cmp::NumCmp::num_eq(v, limit)
    } else if let Some(v) = value.as_f64() {
        num_cmp::NumCmp::num_eq(v, limit)
    } else {
        #[cfg(feature = "arbitrary-precision")]
        {
            if let Some(big_value) = bignum::try_parse_bigfraction(&value.to_number()) {
                if let Some(limit_f64) = num_traits::ToPrimitive::to_f64(&limit) {
                    return big_value == BigFraction::from(limit_f64);
                }
            }
            false
        }
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            unreachable!("Always Some without `arbitrary-precision`")
        }
    }
}

pub fn is_multiple_of_float<N: crate::JsonNumber>(value: &N, multiple: f64) -> bool {
    if let Some(value_f64) = value.as_f64() {
        // Zero is a multiple of any non-zero number
        // This check must come first to avoid division-related edge cases
        if value_f64.is_zero() {
            return true;
        }
        if value_f64.abs() < multiple {
            return false;
        }
        // From the JSON Schema spec
        //
        // > A numeric instance is valid only if division by this keyword's value results in an integer.
        //
        // For fractions, integers have denominator equal to one.
        //
        // Ref: https://json-schema.org/draft/2020-12/json-schema-validation#section-6.2.1
        (BigFraction::from(value_f64) / BigFraction::from(multiple))
            .denom()
            .is_none_or(One::is_one)
    } else {
        // This branch is only possible for large floats in scientific notation, we don't really
        // support it
        false
    }
}

/// The maximum integer that can be exactly represented in f64.
/// Beyond this value, f64 loses precision and arithmetic operations become unreliable.
const MAX_SAFE_INTEGER: u64 = 1u64 << 53;

pub fn is_multiple_of_integer<N: crate::JsonNumber>(value: &N, multiple: f64) -> bool {
    // Integer instances use integer modulo directly: it is exact and avoids the slower float
    // `fract()` + `%`. The divisor guard keeps it exact - divisors above 2^53 may already have
    // lost precision when converted to f64 during schema compilation, and `multiple > 0.0` avoids
    // a divide-by-zero panic on the integer modulo. Non-integer or huge instances fall through to
    // the f64 path below.
    let divisor_ok =
        multiple > 0.0 && multiple <= MAX_SAFE_INTEGER as f64 && multiple.fract() == 0.0;
    if divisor_ok {
        if let Some(v) = value.as_u64() {
            return (v % (multiple as u64)) == 0;
        }
        if let Some(v) = value.as_i64() {
            return (v % (multiple as i64)) == 0;
        }
    }

    if let Some(value_f64) = value.as_f64() {
        // As the divisor has its fractional part as zero, then any value with a non-zero
        // fractional part can't be a multiple of this divisor, therefore it is short-circuited
        value_f64.fract() == 0. && (value_f64 % multiple) == 0.
    } else {
        // Number doesn't fit in f64 - must be huge with arbitrary_precision
        #[cfg(feature = "arbitrary-precision")]
        {
            // Try parsing as BigInt first for large integers
            if let Some(big_value) = bignum::try_parse_bigint(&value.to_number()) {
                use num_bigint::BigInt;
                // Convert the multiple to BigInt.
                // Note: For large divisors beyond i64/u64 range, the schema compilation
                // should have created a MultipleOfBigIntValidator instead, which stores
                // the divisor as BigInt directly. This path handles the case where the
                // instance is huge but the divisor fits in f64.
                // Since we know multiple is an integer (checked before calling this function),
                // we can safely convert via i64 for divisors in the i64 range.
                // For divisors beyond i64 but representable in f64, precision may be lost,
                // but that's inherent to f64 representation.
                let multiple_int = BigInt::from(multiple as i64);
                return bignum::is_multiple_of_bigint(&big_value, &multiple_int);
            }
            // Not an integer - can't be a multiple of an integer divisor
            false
        }
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            unreachable!("Always Some without `arbitrary-precision`")
        }
    }
}

#[cfg(feature = "arbitrary-precision")]
pub mod bignum {
    use fraction::BigFraction;
    use num_bigint::BigInt;
    use num_traits::{ToPrimitive, Zero};
    use serde_json::Number;
    use std::str::FromStr;

    /// Guardrail for how many decimal shifts we are willing to perform when normalizing
    /// a JSON number written in scientific notation.
    ///
    /// Schema authors (and instances) are untrusted input: a literal like `"1e1000000000"`
    /// would otherwise force us to append billions of zeros just to materialize the number,
    /// opening the door to denial-of-service attacks. Limiting the exponent adjustment to
    /// one million digits keeps conversions deterministic while still covering realistic
    /// use-cases (`10^1_000_000` is already astronomically large for JSON Schema).
    const MAX_EXPONENT_ADJUSTMENT: u32 = 1_000_000;

    #[derive(Debug, Clone)]
    struct DecimalComponents {
        negative: bool,
        digits: String,
        fraction_digits: usize,
        exponent: i64,
    }

    impl DecimalComponents {
        fn parse(num_str: &str) -> Option<Self> {
            let bytes = num_str.as_bytes();
            if bytes.is_empty() {
                return None;
            }

            let mut idx = 0;
            let negative = if bytes[idx] == b'-' {
                idx += 1;
                true
            } else {
                false
            };

            if idx >= bytes.len() {
                return None;
            }

            let mut digits = String::with_capacity(bytes.len());
            let int_start = idx;
            while idx < bytes.len() && bytes[idx].is_ascii_digit() {
                idx += 1;
            }
            if int_start == idx {
                return None;
            }
            digits.push_str(&num_str[int_start..idx]);

            let mut fraction_digits = 0usize;
            if idx < bytes.len() && bytes[idx] == b'.' {
                idx += 1;
                let frac_start = idx;
                while idx < bytes.len() && bytes[idx].is_ascii_digit() {
                    idx += 1;
                }
                if frac_start == idx {
                    return None;
                }
                digits.push_str(&num_str[frac_start..idx]);
                fraction_digits = idx - frac_start;
            }

            let mut exponent: i64 = 0;
            if idx < bytes.len() && (bytes[idx] == b'e' || bytes[idx] == b'E') {
                idx += 1;
                if idx >= bytes.len() {
                    return None;
                }
                let mut exp_sign: i64 = 1;
                if bytes[idx] == b'+' {
                    idx += 1;
                } else if bytes[idx] == b'-' {
                    exp_sign = -1;
                    idx += 1;
                }
                let exp_start = idx;
                while idx < bytes.len() && bytes[idx].is_ascii_digit() {
                    idx += 1;
                }
                if exp_start == idx {
                    return None;
                }
                let exp_value = num_str[exp_start..idx].parse::<i64>().ok()?;
                exponent = exp_value.checked_mul(exp_sign)?;
            }

            if idx != bytes.len() {
                return None;
            }

            Some(Self {
                negative,
                digits,
                fraction_digits,
                exponent,
            })
        }

        #[inline]
        fn decimal_shift(&self) -> i64 {
            self.exponent - self.fraction_digits as i64
        }
    }

    fn digits_are_zero(s: &str) -> bool {
        s.bytes().all(|b| b == b'0')
    }

    fn trailing_zero_count(s: &str) -> usize {
        s.as_bytes()
            .iter()
            .rev()
            .take_while(|b| **b == b'0')
            .count()
    }

    fn append_zeros(target: &mut String, count: usize) -> Option<()> {
        let new_len = target.len().checked_add(count)?;
        target.reserve(count);
        target.extend(std::iter::repeat_n('0', count));
        debug_assert_eq!(target.len(), new_len);
        Some(())
    }

    fn pow10_bigint(exp: usize) -> Option<BigInt> {
        if exp == 0 {
            return Some(BigInt::from(1));
        }
        let exp_u32 = u32::try_from(exp).ok()?;
        Some(BigInt::from(10).pow(exp_u32))
    }

    fn shift_exceeds_limit(shift: i64) -> bool {
        if shift <= 0 {
            return false;
        }
        shift as u64 > u64::from(MAX_EXPONENT_ADJUSTMENT)
    }

    fn exponent_reduction_exceeds_limit(exponent: i64) -> bool {
        if exponent >= 0 {
            return false;
        }
        match exponent.checked_abs() {
            Some(abs) => abs as u64 > u64::from(MAX_EXPONENT_ADJUSTMENT),
            None => true,
        }
    }

    /// Try to parse a Number as `BigInt` if it's outside i64 range or for compile-time
    /// schema values that need exact representation
    pub fn try_parse_bigint(num: &Number) -> Option<BigInt> {
        use super::MAX_SAFE_INTEGER;

        let num_str = num.as_str();

        // Parse as BigInt if it's beyond 2^53 (where f64 loses precision).
        // Values beyond 2^53 need BigInt for accurate arithmetic even if they fit in i64/u64.
        // Note: If as_i64() fails but as_u64() succeeds, the value is in [2^63, 2^64-1],
        // which is always > 2^53, so no additional check needed for u64.
        if let Some(v) = num.as_i64() {
            if v.unsigned_abs() <= MAX_SAFE_INTEGER {
                return None;
            }
        }

        let has_fraction_or_exponent = num_str.bytes().any(|b| b == b'.' || b == b'e' || b == b'E');
        if !has_fraction_or_exponent {
            return BigInt::from_str(num_str).ok();
        }

        let mut components = DecimalComponents::parse(num_str)?;
        let mut shift = components.decimal_shift();

        if shift < 0 {
            let needed = (-shift) as usize;
            if digits_are_zero(&components.digits) {
                components.digits.clear();
                components.digits.push('0');
                shift = 0;
            } else {
                if exponent_reduction_exceeds_limit(components.exponent) {
                    return None;
                }
                let zeros = trailing_zero_count(&components.digits);
                if zeros < needed {
                    return None;
                }
                let new_len = components.digits.len() - needed;
                components.digits.truncate(new_len);
                shift = 0;
            }
        }

        if shift > 0 {
            if shift_exceeds_limit(shift) {
                return None;
            }
            append_zeros(&mut components.digits, shift as usize)?;
        }

        let digits_trimmed = components.digits.trim_start_matches('0');
        let digits_ref = if digits_trimmed.is_empty() {
            "0"
        } else {
            digits_trimmed
        };
        let mut value = BigInt::from_str(digits_ref).ok()?;
        if components.negative && !value.is_zero() {
            value = -value;
        }
        Some(value)
    }

    /// Try to parse a Number as `BigFraction` for arbitrary precision decimal support
    ///
    /// Returns Some for numbers requiring exact decimal precision:
    /// - Decimals with a decimal point (e.g., `0.1`, `123.456`)
    /// - Scientific notation decimals that can't be represented exactly as f64
    ///
    /// Returns None for:
    /// - Integers that fit in i64 (handled by standard numeric path)
    /// - Large integers including u64 beyond `i64::MAX` (handled by `try_parse_bigint`)
    pub fn try_parse_bigfraction(num: &Number) -> Option<BigFraction> {
        // Skip integers that fit in i64 - they don't need BigFraction
        if num.as_i64().is_some() {
            return None;
        }

        let num_str = num.as_str();

        // Check for decimal point and exponent in a single pass
        let mut has_decimal_point = false;
        let mut has_exponent = false;
        for b in num_str.bytes() {
            if b == b'.' {
                has_decimal_point = true;
            } else if b == b'e' || b == b'E' {
                has_exponent = true;
                break;
            }
        }

        if !has_decimal_point && !has_exponent {
            return None;
        }

        if !has_exponent {
            return BigFraction::from_str(num_str).ok();
        }

        let components = DecimalComponents::parse(num_str)?;
        let shift = components.decimal_shift();

        // A number with exponent that still resolves to an integer is handled by BigInt.
        if shift >= 0 {
            return None;
        }

        if exponent_reduction_exceeds_limit(components.exponent) {
            return None;
        }

        let denom_power = (-shift) as usize;
        let denominator = pow10_bigint(denom_power)?;
        let mut numerator = BigInt::from_str(&components.digits).ok()?;
        if components.negative && !numerator.is_zero() {
            numerator = -numerator;
        }
        Some(BigFraction::from(numerator) / BigFraction::from(denominator))
    }

    /// Exact ordering of a big-integer instance against a numeric limit.
    ///
    /// Integer-representable limits compare via `BigInt`; infinite limits (schema numbers
    /// beyond the exponent cap) order every finite instance. `None` means the limit has no
    /// exact integer form and the caller should fall back to `f64` comparison.
    pub(crate) fn compare_bigint_to_limit<T>(big: &BigInt, limit: T) -> Option<std::cmp::Ordering>
    where
        T: Copy + ToPrimitive,
    {
        use std::cmp::Ordering;

        let limit_f64 = limit.to_f64()?;
        if limit_f64.fract() == 0.0 {
            // `to_i64`/`to_u64` are exact for u64/i64 limits and for integer-valued f64 limits.
            if let Some(limit_int) = limit.to_i64() {
                return Some(big.cmp(&BigInt::from(limit_int)));
            }
            if let Some(limit_int) = limit.to_u64() {
                return Some(big.cmp(&BigInt::from(limit_int)));
            }
        }
        if limit_f64 == f64::INFINITY {
            return Some(Ordering::Less);
        }
        if limit_f64 == f64::NEG_INFINITY {
            return Some(Ordering::Greater);
        }
        None
    }

    macro_rules! define_bigint_cmp {
        ($($fn_name:ident, $prim_type:ty, $to_prim:ident, $op:tt, $overflow_sign:expr);* $(;)?) => {
            $(
                pub fn $fn_name(bigint: &BigInt, value: $prim_type) -> bool {
                    if let Some(converted) = bigint.$to_prim() {
                        converted $op value
                    } else {
                        bigint.sign() == $overflow_sign
                    }
                }
            )*
        };
    }

    define_bigint_cmp!(
        bigint_ge_u64, u64, to_u64, >=, num_bigint::Sign::Plus;
        bigint_le_u64, u64, to_u64, <=, num_bigint::Sign::Minus;
        bigint_gt_u64, u64, to_u64, >, num_bigint::Sign::Plus;
        bigint_lt_u64, u64, to_u64, <, num_bigint::Sign::Minus;
        bigint_ge_i64, i64, to_i64, >=, num_bigint::Sign::Plus;
        bigint_le_i64, i64, to_i64, <=, num_bigint::Sign::Minus;
        bigint_gt_i64, i64, to_i64, >, num_bigint::Sign::Plus;
        bigint_lt_i64, i64, to_i64, <, num_bigint::Sign::Minus;
        bigint_ge_f64, f64, to_f64, >=, num_bigint::Sign::Plus;
        bigint_le_f64, f64, to_f64, <=, num_bigint::Sign::Minus;
        bigint_gt_f64, f64, to_f64, >, num_bigint::Sign::Plus;
        bigint_lt_f64, f64, to_f64, <, num_bigint::Sign::Minus;
    );

    // Generate reverse comparison functions (primitive op BigType -> BigType op primitive)
    macro_rules! define_reverse_cmp {
        ($($rev_ge:ident, $rev_le:ident, $rev_gt:ident, $rev_lt:ident, $prim_type:ty, $big_type:ty, $fwd_ge:ident, $fwd_le:ident, $fwd_gt:ident, $fwd_lt:ident);* $(;)?) => {
            $(
                pub fn $rev_ge(value: $prim_type, big: &$big_type) -> bool {
                    $fwd_le(big, value)
                }

                pub fn $rev_le(value: $prim_type, big: &$big_type) -> bool {
                    $fwd_ge(big, value)
                }

                pub fn $rev_gt(value: $prim_type, big: &$big_type) -> bool {
                    $fwd_lt(big, value)
                }

                pub fn $rev_lt(value: $prim_type, big: &$big_type) -> bool {
                    $fwd_gt(big, value)
                }
            )*
        };
    }

    define_reverse_cmp!(
        u64_ge_bigint, u64_le_bigint, u64_gt_bigint, u64_lt_bigint, u64, BigInt, bigint_ge_u64, bigint_le_u64, bigint_gt_u64, bigint_lt_u64;
        i64_ge_bigint, i64_le_bigint, i64_gt_bigint, i64_lt_bigint, i64, BigInt, bigint_ge_i64, bigint_le_i64, bigint_gt_i64, bigint_lt_i64;
        f64_ge_bigint, f64_le_bigint, f64_gt_bigint, f64_lt_bigint, f64, BigInt, bigint_ge_f64, bigint_le_f64, bigint_gt_f64, bigint_lt_f64;
    );

    /// Check if a Number (as `BigInt`) is a multiple of another `BigInt`
    pub fn is_multiple_of_bigint(value: &BigInt, multiple: &BigInt) -> bool {
        // Zero is a multiple of any non-zero number
        // Mathematically: 0 = k * multiple for k = 0
        if value.is_zero() {
            return true;
        }

        // Note: multiple.is_zero() case is not handled here because JSON Schema
        // validation rejects schemas with "multipleOf: 0" during compilation
        // (exclusiveMinimum constraint requires multipleOf > 0).
        // The modulo operation below would panic if multiple is zero, but this
        // is prevented by schema validation.

        (value % multiple).is_zero()
    }

    // BigFraction comparison functions
    macro_rules! define_bigfraction_cmp {
        ($($fn_name:ident, $prim_type:ty, $op:tt);* $(;)?) => {
            $(
                pub fn $fn_name(bigfrac: &BigFraction, value: $prim_type) -> bool {
                    let value_frac = BigFraction::from(value);
                    *bigfrac $op value_frac
                }
            )*
        };
    }

    define_bigfraction_cmp!(
        bigfrac_ge_u64, u64, >=;
        bigfrac_le_u64, u64, <=;
        bigfrac_gt_u64, u64, >;
        bigfrac_lt_u64, u64, <;
        bigfrac_ge_i64, i64, >=;
        bigfrac_le_i64, i64, <=;
        bigfrac_gt_i64, i64, >;
        bigfrac_lt_i64, i64, <;
        bigfrac_ge_f64, f64, >=;
        bigfrac_le_f64, f64, <=;
        bigfrac_gt_f64, f64, >;
        bigfrac_lt_f64, f64, <;
    );

    define_reverse_cmp!(
        u64_ge_bigfrac, u64_le_bigfrac, u64_gt_bigfrac, u64_lt_bigfrac, u64, BigFraction, bigfrac_ge_u64, bigfrac_le_u64, bigfrac_gt_u64, bigfrac_lt_u64;
        i64_ge_bigfrac, i64_le_bigfrac, i64_gt_bigfrac, i64_lt_bigfrac, i64, BigFraction, bigfrac_ge_i64, bigfrac_le_i64, bigfrac_gt_i64, bigfrac_lt_i64;
        f64_ge_bigfrac, f64_le_bigfrac, f64_gt_bigfrac, f64_lt_bigfrac, f64, BigFraction, bigfrac_ge_f64, bigfrac_le_f64, bigfrac_gt_f64, bigfrac_lt_f64;
    );

    /// Check if a `BigFraction` is a multiple of another value
    pub fn is_multiple_of_bigfrac(value: &BigFraction, multiple: &BigFraction) -> bool {
        // Zero is a multiple of any non-zero number
        if value.is_zero() {
            return true;
        }
        // Division by zero is undefined, so return false
        if multiple.is_zero() {
            return false;
        }
        // A number is a multiple of another if division results in an integer
        // (denominator of the result is 1)
        (value / multiple).denom().is_none_or(fraction::One::is_one)
    }
}

#[cfg(all(test, feature = "arbitrary-precision"))]
mod tests {
    use super::bignum;
    use fraction::BigFraction;
    use num_bigint::BigInt;
    use serde_json::{Number, Value};
    use std::cmp::Ordering;
    use test_case::test_case;

    fn number_from_str(raw: &str) -> Number {
        match serde_json::from_str::<Value>(raw).expect("valid JSON number") {
            Value::Number(num) => num,
            _ => unreachable!(),
        }
    }

    #[test_case("18446744073709551616", u64::MAX, Ordering::Greater; "above u64 limit")]
    fn compare_bigint_to_u64_limit(big: &str, limit: u64, expected: Ordering) {
        let big = BigInt::parse_bytes(big.as_bytes(), 10).unwrap();
        assert_eq!(bignum::compare_bigint_to_limit(&big, limit), Some(expected));
    }

    #[test_case("-18446744073709551616", i64::MIN, Ordering::Less; "below i64 limit")]
    fn compare_bigint_to_i64_limit(big: &str, limit: i64, expected: Ordering) {
        let big = BigInt::parse_bytes(big.as_bytes(), 10).unwrap();
        assert_eq!(bignum::compare_bigint_to_limit(&big, limit), Some(expected));
    }

    // Infinity limits come from schema numbers beyond the exponent cap (e.g. `1e2000000`);
    // limits without an exact integer form defer to the caller's f64 comparison.
    #[test_case(f64::INFINITY, Some(Ordering::Less); "infinity limit")]
    #[test_case(f64::NEG_INFINITY, Some(Ordering::Greater); "negative infinity limit")]
    #[test_case(0.5, None; "no exact integer form")]
    fn compare_bigint_to_f64_limit(limit: f64, expected: Option<Ordering>) {
        let big = BigInt::parse_bytes(b"18446744073709551616", 10).unwrap();
        assert_eq!(bignum::compare_bigint_to_limit(&big, limit), expected);
    }

    #[test]
    fn bigint_parses_scientific_integer() {
        let num = number_from_str("1e19");
        let parsed = bignum::try_parse_bigint(&num).expect("parsed bigint");
        assert_eq!(
            parsed,
            BigInt::parse_bytes(b"10000000000000000000", 10).unwrap()
        );
    }

    #[test]
    fn bigint_rejects_non_integer_scientific() {
        let num = number_from_str("1.25e1");
        assert!(bignum::try_parse_bigint(&num).is_none());
    }

    #[test]
    fn bigfraction_parses_scientific_decimal() {
        let num = number_from_str("1.5e-5");
        let parsed = bignum::try_parse_bigfraction(&num).expect("parsed bigfraction");
        let expected =
            BigFraction::from(BigInt::from(3)) / BigFraction::from(BigInt::from(200_000));
        assert_eq!(parsed, expected);
    }

    #[test]
    fn bigfraction_skips_scientific_integer() {
        let num = number_from_str("3e4");
        assert!(bignum::try_parse_bigfraction(&num).is_none());
    }
}
