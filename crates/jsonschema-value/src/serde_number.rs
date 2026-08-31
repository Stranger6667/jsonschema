//! `JsonNumber` for `serde_json`'s own number type, which every representation reports through.

use std::borrow::Cow;

use crate::JsonNumber;

macro_rules! impl_json_number {
    ($ty:ty) => {
        impl JsonNumber for $ty {
            fn as_u64(&self) -> Option<u64> {
                serde_json::Number::as_u64(self)
            }
            fn as_i64(&self) -> Option<i64> {
                serde_json::Number::as_i64(self)
            }
            fn as_f64(&self) -> Option<f64> {
                serde_json::Number::as_f64(self)
            }
            #[cfg(feature = "arbitrary-precision")]
            fn as_str(&self) -> Cow<'_, str> {
                Cow::Borrowed(serde_json::Number::as_str(self))
            }
            #[cfg(not(feature = "arbitrary-precision"))]
            fn as_str(&self) -> Cow<'_, str> {
                Cow::Owned(self.to_string())
            }
            fn to_number(&self) -> Cow<'_, serde_json::Number> {
                Cow::Borrowed(self)
            }

            // Here `as_str` re-renders the `f64`, so a plain integer past its exact range comes
            // back indistinguishable from an exponent literal. Every `f64` that large is whole,
            // and metaschemas write bounds up there, so exponent form is read as an integer.
            #[cfg(not(feature = "arbitrary-precision"))]
            fn is_written_as_integer(&self) -> bool {
                if serde_json::Number::as_u64(self).is_some()
                    || serde_json::Number::as_i64(self).is_some()
                {
                    return true;
                }
                let text = self.to_string();
                if text.contains(['e', 'E']) {
                    // 2^63; every `f64` past it is a whole number.
                    return serde_json::Number::as_f64(self)
                        .is_some_and(|value| value.abs() >= 9_223_372_036_854_775_808.0);
                }
                !text.contains('.')
            }
        }
    };
}

impl_json_number!(serde_json::Number);
impl_json_number!(&serde_json::Number);
