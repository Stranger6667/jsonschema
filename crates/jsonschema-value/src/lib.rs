//! JSON value representations and semantics shared by the validator and its bindings.

pub mod ext;
pub mod types;

#[cfg(feature = "serde_json")]
mod serde_json;

#[cfg(feature = "serde_json")]
pub use serde_json::SerdeJson;

use std::borrow::Cow;

use ::serde_json::Value;

use crate::types::JsonType;

/// Ties together the concrete node types for one JSON representation.
pub trait Json: Sized + Send + Sync + 'static {
    type Node<'a>: JsonNode<'a, Self>;

    /// A property-name key prepared once at schema compile time for repeated object lookups.
    type PreparedKey: Send + Sync;

    fn prepare_key(key: &str) -> Self::PreparedKey;
}

/// One JSON value in some concrete representation; `Clone` must be cheap (borrow, refcount bump, or machine
/// word copy).
pub trait JsonNode<'a, F: Json>: Clone {
    type Object: JsonObjectAccess<'a, F, Node = Self>;
    type Array: JsonArrayAccess<'a, F, Node = Self>;

    fn as_object(&self) -> Option<Self::Object>;
    fn as_array(&self) -> Option<Self::Array>;
    fn as_string(&self) -> Option<Cow<'a, str>>;
    /// Borrowed for `serde_json`, cheaply owned elsewhere; `Number` keeps the existing numeric machinery
    /// applicable to every representation.
    fn as_number(&self) -> Option<Cow<'a, ::serde_json::Number>>;
    fn as_boolean(&self) -> Option<bool>;
    fn is_null(&self) -> bool;

    /// Whether this node is a JSON number, answered without materializing the numeric value.
    /// Representations where `as_number` must construct or format (e.g. Python floats under
    /// arbitrary precision) override this to skip that cost. Must agree with `as_number().is_some()`.
    fn is_number(&self) -> bool {
        self.as_number().is_some()
    }

    /// Sugar over [`JsonNode::json_type`], the single source of type classification.
    fn is_string(&self) -> bool {
        self.json_type() == JsonType::String
    }

    /// The JSON type of this node; numbers always report [`JsonType::Number`] (integer distinction is a
    /// numeric property, not a type tag).
    fn json_type(&self) -> JsonType;

    /// String length in Unicode code points, without extracting the bytes where the representation allows.
    fn string_length(&self) -> Option<u64>;

    /// Deep equality against a schema constant (`const`/`enum`); numbers compare mathematically.
    fn equals_value(&self, expected: &Value) -> bool;

    /// The node as a `serde_json::Value`; borrowed for `serde_json`, materialized elsewhere. Intended for
    /// cold paths (error construction, annotations).
    fn to_value(&self) -> Cow<'a, Value>;

    /// Identity for the `is_valid` result cache; `None` disables caching for this node.
    fn cache_key(&self) -> Option<usize>;

    /// Cache identity restricted to containers, where identities are stable for the whole validation call.
    fn container_cache_key(&self) -> Option<usize> {
        if matches!(self.json_type(), JsonType::Object | JsonType::Array) {
            self.cache_key()
        } else {
            None
        }
    }
}

pub trait JsonObjectAccess<'a, F: Json> {
    type Node: JsonNode<'a, F>;
    /// Member name handle; a plain `&str` where the representation can borrow, owned elsewhere.
    type MemberName: AsRef<str> + Into<Cow<'a, str>>;
    type MembersIter: Iterator<Item = (Self::MemberName, Self::Node)>;

    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn get(&self, key: &F::PreparedKey) -> Option<Self::Node>;
    fn members(&self) -> Self::MembersIter;
}

// `len` is an element count used for validation bounds, not a container emptiness probe; no caller
// needs `is_empty`.
#[allow(clippy::len_without_is_empty)]
pub trait JsonArrayAccess<'a, F: Json> {
    type Node: JsonNode<'a, F>;
    type ElementsIter: Iterator<Item = Self::Node>;

    fn len(&self) -> usize;
    fn elements(&self) -> Self::ElementsIter;

    /// The backing `serde_json::Value` slice when the representation stores elements contiguously as
    /// `Value`, enabling zero-copy algorithms such as `uniqueItems`; `None` otherwise.
    fn as_value_slice(&self) -> Option<&'a [Value]>;
}
