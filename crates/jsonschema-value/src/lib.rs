//! JSON value representations and semantics shared by the validator and its bindings.

pub mod cmp;
#[cfg(feature = "conformance")]
pub mod conformance;
pub mod numeric;
// The bound checks take a `serde_json::Number`, which only that feature makes a `JsonNumber`.
#[cfg(feature = "serde_json")]
pub mod numeric_check;
pub mod types;
pub mod unique;

#[cfg(feature = "pyo3")]
mod pyo3;
#[cfg(feature = "serde_json")]
mod serde_json;

#[cfg(feature = "pyo3")]
pub use pyo3::{probe_root, take_pending_error, PendingErrorScope, Pyo3};
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

/// What tells one node from another within a validation call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeIdentity {
    address: usize,
    tag: u32,
}

impl NodeIdentity {
    /// For representations where a live node's address is its own.
    #[must_use]
    pub fn new(address: usize) -> Self {
        Self { address, tag: 0 }
    }

    /// For representations where nodes share an address, such as an arena addressed by index.
    #[must_use]
    pub fn tagged(address: usize, tag: u32) -> Self {
        Self { address, tag }
    }
}

/// A JSON number, readable without constructing a [`::serde_json::Number`].
pub trait JsonNumber {
    fn as_u64(&self) -> Option<u64>;
    fn as_i64(&self) -> Option<i64>;
    fn as_f64(&self) -> Option<f64>;

    /// Decimal digits; the only form that holds values outside the primitives.
    fn as_str(&self) -> Cow<'_, str>;

    /// For cold paths: error construction and annotations.
    fn to_number(&self) -> Cow<'_, ::serde_json::Number>;

    fn is_integer(&self) -> bool {
        crate::types::number_is_integer(&self.to_number())
    }
}

/// One JSON value; `Clone` must be cheap.
pub trait Node<'a, F: Json>: Clone {
    type Object: Object<'a, F, Node = Self>;
    type Array: Array<'a, F, Node = Self>;
    type Number: JsonNumber;

    fn as_object(&self) -> Option<Self::Object>;
    fn as_array(&self) -> Option<Self::Array>;
    fn as_string(&self) -> Option<Cow<'a, str>>;

    fn as_number(&self) -> Option<Self::Number>;
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
    /// Nodes alive at once must never share one, and two handles on a node must report the same
    /// one, or a collision reports a cycle that is not there. A container's must never pass to a
    /// later node: [`Node::container_identity`] keys a cache outliving it. `None` opts out,
    /// leaving recursion bounded only by the stack.
    fn identity(&self) -> Option<NodeIdentity>;

    fn container_identity(&self) -> Option<NodeIdentity> {
        if matches!(self.json_type(), JsonType::Object | JsonType::Array) {
            self.identity()
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
