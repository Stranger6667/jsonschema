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
        }
    };
}

impl_json_number!(serde_json::Number);
impl_json_number!(&serde_json::Number);
