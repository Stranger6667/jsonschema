//! JSON value representations and semantics shared by the validator and its bindings.

pub mod cmp;
#[cfg(feature = "conformance")]
pub mod conformance;
pub mod numeric;
#[cfg(feature = "arbitrary-precision")]
pub mod numeric_check;
pub mod types;
pub mod unique;

#[cfg(feature = "serde_json")]
mod serde_json;

#[cfg(feature = "serde_json")]
pub use serde_json::SerdeJson;

use std::borrow::Cow;

use ::serde_json::Value;

use crate::types::JsonType;

/// One JSON representation.
pub trait Json: Sized + Send + Sync + 'static {
    type Node<'a>: Node<'a, Self>;

    /// Property name prepared once at compile time, for repeated object lookups.
    type PreparedKey: Send + Sync;

    fn prepare_key(key: &str) -> Self::PreparedKey;
}

/// One JSON value; `Clone` must be cheap.
pub trait Node<'a, F: Json>: Clone {
    type Object: Object<'a, F, Node = Self>;
    type Array: Array<'a, F, Node = Self>;

    fn as_object(&self) -> Option<Self::Object>;
    fn as_array(&self) -> Option<Self::Array>;
    fn as_string(&self) -> Option<Cow<'a, str>>;
    fn as_number(&self) -> Option<Cow<'a, ::serde_json::Number>>;
    fn as_boolean(&self) -> Option<bool>;
    fn is_null(&self) -> bool;

    /// Must agree with `as_number().is_some()`; override where `as_number` has to construct.
    fn is_number(&self) -> bool {
        self.as_number().is_some()
    }

    fn is_string(&self) -> bool {
        self.json_type() == JsonType::String
    }

    /// Numbers always report [`JsonType::Number`]; integer-ness is a numeric property, not a type.
    fn json_type(&self) -> JsonType;

    /// Length in Unicode code points.
    fn string_length(&self) -> Option<u64> {
        self.as_string().map(|string| string.chars().count() as u64)
    }

    /// Equality against a `const`/`enum` value; numbers compare mathematically.
    fn equals_value(&self, expected: &Value) -> bool {
        crate::cmp::equal(&self.to_value(), expected)
    }

    /// For cold paths only: error construction and annotations.
    fn to_value(&self) -> Cow<'a, Value>;

    /// Identity for `$ref` cycle detection and `is_valid` memoization.
    ///
    /// Two nodes alive at once must never share a key, and a key must not change or be reused
    /// during a call: a collision reports a cycle that is not there and accepts the node unchecked.
    /// `None` opts out of both, leaving recursion bounded only by the stack.
    fn cache_key(&self) -> Option<usize>;

    fn container_cache_key(&self) -> Option<usize> {
        if matches!(self.json_type(), JsonType::Object | JsonType::Array) {
            self.cache_key()
        } else {
            None
        }
    }
}

pub trait Object<'a, F: Json> {
    type Node: Node<'a, F>;
    type MemberName: AsRef<str> + Into<Cow<'a, str>>;
    type MembersIter: Iterator<Item = (Self::MemberName, Self::Node)>;

    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn get(&self, key: &F::PreparedKey) -> Option<Self::Node>;
    fn members(&self) -> Self::MembersIter;
}

// `len` bounds validation; no caller probes emptiness.
#[allow(clippy::len_without_is_empty)]
pub trait Array<'a, F: Json> {
    type Node: Node<'a, F>;
    type ElementsIter: Iterator<Item = Self::Node>;

    fn len(&self) -> usize;
    fn elements(&self) -> Self::ElementsIter;

    /// `uniqueItems`: every element distinct under JSON equality.
    fn is_unique(&self) -> bool {
        let values: Vec<Cow<'a, Value>> =
            self.elements().map(|element| element.to_value()).collect();
        crate::unique::is_unique(&values)
    }
}
