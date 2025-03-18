mod number;

use ahash::AHashMap;
use number::Number;
use strumbra::UniqueString;

/// An immutable JSON value representation optimized for fast comparison with other JSON instances.
#[derive(Clone, PartialEq, Eq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(Number),
    String(UniqueString),
    Array(Box<[JsonValue]>),
    // TODO: Drop box?
    Object(Box<AHashMap<UniqueString, JsonValue>>),
}

const _: () = const {
    assert!(std::mem::size_of::<JsonValue>() <= 24);
};
