use crate::cmp;
use ahash::{AHashSet, AHasher};
use serde_json::Value;
use std::{
    borrow::Borrow,
    hash::{Hash, Hasher},
};

// Based on implementation proposed by Sven Marnach:
// https://stackoverflow.com/questions/60882381/what-is-the-fastest-correct-way-to-detect-that-there-are-no-duplicates-in-a-json
pub(crate) struct HashedValue<'a>(&'a Value);

impl PartialEq for HashedValue<'_> {
    fn eq(&self, other: &Self) -> bool {
        cmp::equal(self.0, other.0)
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
                    number.to_bits().hash(state);
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
pub(crate) const ITEMS_SIZE_THRESHOLD: usize = 15;

// Generic over `Borrow<Value>` so both borrowed `serde_json` slices and the `Cow<Value>` handles
// materialized from other representations run the same duplicate-detection algorithm.
#[inline]
#[must_use]
pub fn is_unique<T: Borrow<Value>>(items: &[T]) -> bool {
    let size = items.len();
    if size <= 1 {
        // Empty arrays and one-element arrays always contain unique elements
        true
    } else if let [first, second] = items {
        !cmp::equal(first.borrow(), second.borrow())
    } else if let [first, second, third] = items {
        !cmp::equal(first.borrow(), second.borrow())
            && !cmp::equal(first.borrow(), third.borrow())
            && !cmp::equal(second.borrow(), third.borrow())
    } else if size <= ITEMS_SIZE_THRESHOLD {
        // If the array size is small enough we can compare all elements pairwise, which will
        // be faster than calculating hashes for each element, even if the algorithm is O(N^2)
        let mut idx = 0_usize;
        while idx < items.len() {
            let mut inner_idx = idx + 1;
            while inner_idx < items.len() {
                if cmp::equal(items[idx].borrow(), items[inner_idx].borrow()) {
                    return false;
                }
                inner_idx += 1;
            }
            idx += 1;
        }
        true
    } else {
        let mut seen = AHashSet::with_capacity(size);
        items
            .iter()
            .map(|item| HashedValue(item.borrow()))
            .all(move |x| seen.insert(x))
    }
}

#[cfg(test)]
mod tests {
    use super::{is_unique, ITEMS_SIZE_THRESHOLD};
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(&[] => true; "empty array")]
    #[test_case(&[json!(1)] => true; "one element array")]
    #[test_case(&[json!(1), json!(2)] => true; "two unique elements")]
    #[test_case(&[json!(1), json!(1)] => false; "two non-unique elements")]
    #[test_case(&[json!(1), json!(2), json!(3)] => true; "three unique elements")]
    #[test_case(&[json!(1), json!(2), json!(1)] => false; "three non-unique elements")]
    #[test_case(&[json!(1), json!(2), json!(3), json!(4), json!(5), json!(6), json!(7), json!(8), json!(9), json!(10), json!(11), json!(12), json!(13), json!(14), json!(15), json!(1.0)] => false; "positive numbers with fractions")]
    #[test_case(&[json!(-1), json!(-2), json!(-3), json!(-4), json!(-5), json!(-6), json!(-7), json!(-8), json!(-9), json!(-10), json!(-11), json!(-12), json!(-13), json!(-14), json!(-15), json!(-1.0)] => false; "negative numbers with fractions")]
    #[test_case(&[json!(1), json!("string"), json!(true), json!(null), json!({"key": "value"}), json!([1, 2, 3])] => true; "mixed types")]
    #[test_case(&[json!({"a": 1, "b": 1}), json!({"a": 1, "b": 2}), json!({"a": 1, "b": 3})] => true; "complex objects unique")]
    #[test_case(&[json!({"a": 1, "b": 2}), json!({"b": 2, "a": 1}), json!({"a": 1, "b": 2})] => false; "complex objects non-unique")]
    fn test_is_unique(items: &[Value]) -> bool {
        is_unique(items)
    }

    #[test_case(ITEMS_SIZE_THRESHOLD => true; "small array unique")]
    #[test_case(ITEMS_SIZE_THRESHOLD + 1 => true; "large array unique")]
    fn test_unique_arrays(size: usize) -> bool {
        let arr = (1..=size).map(|i| json!(i)).collect::<Vec<_>>();
        is_unique(&arr)
    }

    #[test_case(ITEMS_SIZE_THRESHOLD => false; "small array non-unique")]
    #[test_case(ITEMS_SIZE_THRESHOLD + 1 => false; "large array non-unique")]
    fn test_non_unique_arrays(size: usize) -> bool {
        let mut arr = (1..=size).map(|i| json!(i)).collect::<Vec<_>>();
        arr[size - 1] = json!(1);
        is_unique(&arr)
    }
}
