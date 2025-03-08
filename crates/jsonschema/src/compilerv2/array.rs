use std::hash::{Hash, Hasher};

use ahash::{AHashSet, AHasher};
use serde_json::{Map, Value};

use crate::keywords::helpers::equal;

use super::SchemaCompiler;

#[derive(Debug, Clone)]
pub(crate) struct MinItems {
    limit: usize,
}

impl MinItems {
    pub(crate) fn new(limit: usize) -> Self {
        Self { limit }
    }
    pub(crate) fn execute(&self, arr: &[Value]) -> bool {
        arr.len() >= self.limit
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MaxItems {
    limit: usize,
}

impl MaxItems {
    pub(crate) fn new(limit: usize) -> Self {
        Self { limit }
    }
    pub(crate) fn execute(&self, arr: &[Value]) -> bool {
        arr.len() <= self.limit
    }
}

// Based on implementation proposed by Sven Marnach:
// https://stackoverflow.com/questions/60882381/what-is-the-fastest-correct-way-to-detect-that-there-are-no-duplicates-in-a-json
pub(crate) struct HashedValue<'a>(&'a Value);

impl PartialEq for HashedValue<'_> {
    fn eq(&self, other: &Self) -> bool {
        equal(self.0, other.0)
    }
}

impl Eq for HashedValue<'_> {}

impl Hash for HashedValue<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self.0 {
            Value::Null => state.write_u32(3_221_225_473), // chosen randomly
            Value::Bool(ref item) => item.hash(state),
            Value::Number(ref item) => {
                if let Some(number) = item.as_f64() {
                    number.to_bits().hash(state)
                } else if let Some(number) = item.as_u64() {
                    number.hash(state);
                } else if let Some(number) = item.as_i64() {
                    number.hash(state);
                }
            }
            Value::String(ref item) => item.hash(state),
            Value::Array(ref items) => {
                for item in items {
                    HashedValue(item).hash(state);
                }
            }
            Value::Object(ref items) => {
                let mut hash = 0;
                for (key, value) in items {
                    // We have no way of building a new hasher of type `H`, so we
                    // hardcode using the default hasher of a hash map.
                    let mut item_hasher = AHasher::default();
                    key.hash(&mut item_hasher);
                    HashedValue(value).hash(&mut item_hasher);
                    hash ^= item_hasher.finish();
                }
                state.write_u64(hash);
            }
        }
    }
}

// Empirically calculated threshold after which the validator resorts to hashing.
// Calculated for an array of mixed types, large homogeneous arrays of primitive values might be
// processed faster with different thresholds, but this one gives a good baseline for the common
// case.
const ITEMS_SIZE_THRESHOLD: usize = 15;

#[inline]
pub(crate) fn is_unique(items: &[Value]) -> bool {
    let size = items.len();
    if size <= 1 {
        // Empty arrays and one-element arrays always contain unique elements
        true
    } else if let [first, second] = items {
        !equal(first, second)
    } else if let [first, second, third] = items {
        !equal(first, second) && !equal(first, third) && !equal(second, third)
    } else if size <= ITEMS_SIZE_THRESHOLD {
        // If the array size is small enough we can compare all elements pairwise, which will
        // be faster than calculating hashes for each element, even if the algorithm is O(N^2)
        let mut idx = 0_usize;
        while idx < items.len() {
            let mut inner_idx = idx + 1;
            while inner_idx < items.len() {
                if equal(&items[idx], &items[inner_idx]) {
                    return false;
                }
                inner_idx += 1;
            }
            idx += 1;
        }
        true
    } else {
        let mut seen = AHashSet::with_capacity(size);
        items.iter().map(HashedValue).all(move |x| seen.insert(x))
    }
}

pub(super) fn compile(
    compiler: &mut SchemaCompiler,
    obj: &Map<String, Value>,
    jumps: &mut Vec<usize>,
) {
    if let Some(Value::Number(value)) = obj.get("minItems") {
        if !compiler.compile_integer(value, MinItems::new) {
            todo!("Non-integer minItems");
        }
        jumps.push(compiler.emit_jump_if_invalid());
    }
    if let Some(Value::Number(value)) = obj.get("maxItems") {
        if !compiler.compile_integer(value, MaxItems::new) {
            todo!("Non-integer maxItems");
        }
        jumps.push(compiler.emit_jump_if_invalid());
    }
    if let Some(Value::Bool(true)) = obj.get("uniqueItems") {
        compiler.emit_unique_items();
        jumps.push(compiler.emit_jump_if_invalid());
    }
    if let Some(items) = obj.get("items") {
        match items {
            Value::Object(_) | Value::Bool(false) => {
                let iter_idx = compiler.emit_array_iter();
                compiler.compile_schema(items);
                compiler.emit_array_iter_next(iter_idx);
                compiler.patch_array_iter(iter_idx);
                jumps.push(compiler.emit_jump_if_invalid());
            }
            _ => {}
        }
    }
}
