//! A signed integer bound.
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

    /// This bound plus one, or `None` when that leaves the representable range.
    pub(crate) fn checked_increment(self) -> Option<Self> {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            self.0.checked_add(1).map(Self)
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            Some(Self(self.0 + 1))
        }
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
