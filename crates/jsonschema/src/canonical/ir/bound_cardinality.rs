//! A non-negative count bound on a string, array, or object size.
#[cfg(feature = "arbitrary-precision")]
type InnerCardinality = num_bigint::BigInt;
#[cfg(not(feature = "arbitrary-precision"))]
type InnerCardinality = u64;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) struct BoundCardinality(InnerCardinality);

impl BoundCardinality {
    /// This count as an exact JSON number.
    pub(crate) fn to_number(&self) -> serde_json::Number {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            serde_json::Number::from(self.0)
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            match num_traits::ToPrimitive::to_u64(&self.0) {
                Some(value) => serde_json::Number::from(value),
                None => serde_json::Number::from_string_unchecked(self.0.to_string()),
            }
        }
    }

    /// A non-negative integer count from a JSON number; `None` past `u64` in the default build.
    pub(crate) fn from_number(number: &serde_json::Number) -> Option<Self> {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            number
                .as_u64()
                .or_else(|| integer_valued_u64(number.as_f64()?))
                .map(Self)
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            let text = number.as_str();
            let canonical = crate::canonical::json::canonical_number(text);
            let integer = canonical.as_deref().unwrap_or(text);
            if integer.bytes().all(|byte| byte.is_ascii_digit()) {
                integer.parse::<num_bigint::BigInt>().ok().map(Self)
            } else {
                None
            }
        }
    }

    pub(crate) fn is_zero(&self) -> bool {
        #[cfg(not(feature = "arbitrary-precision"))]
        {
            self.0 == 0
        }
        #[cfg(feature = "arbitrary-precision")]
        {
            num_traits::Zero::is_zero(&self.0)
        }
    }
}

/// An integer-valued, non-negative, in-`u64`-range `f64` as `u64`, matching the length validators.
#[cfg(not(feature = "arbitrary-precision"))]
#[expect(
    clippy::float_cmp,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "guarded conversion; `u64::MAX` is an inclusive ceiling"
)]
fn integer_valued_u64(float: f64) -> Option<u64> {
    (float.trunc() == float && (0.0..=u64::MAX as f64).contains(&float)).then_some(float as u64)
}

impl From<u64> for BoundCardinality {
    fn from(value: u64) -> Self {
        Self(InnerCardinality::from(value))
    }
}
