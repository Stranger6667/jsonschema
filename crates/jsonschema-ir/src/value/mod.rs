mod number;

pub use number::Number;

/// An immutable JSON value representation optimized for fast comparison with other JSON instances.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(Number),
    String(Box<str>),
    Array(Box<[JsonValue]>),
    // Sorted key / value pairs
    Object(Box<[(Box<str>, JsonValue)]>),
}

const _: () = const {
    assert!(std::mem::size_of::<JsonValue>() == 24);
};
