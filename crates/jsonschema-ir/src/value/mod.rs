mod number;

use std::collections::BTreeMap;

pub use number::Number;

/// An immutable JSON value representation optimized for fast comparison with other JSON instances.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(Number),
    String(Box<str>),
    Array(Box<[JsonValue]>),
    Object(BTreeMap<Box<str>, JsonValue>),
}

const _: () = const {
    assert!(std::mem::size_of::<JsonValue>() == 32);
};
