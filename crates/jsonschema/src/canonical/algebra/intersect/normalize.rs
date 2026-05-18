use std::sync::Arc;

use ahash::AHashSet;

use crate::canonical::ir::{BoundInteger, IntegerLeaf, NumberLeaf, StringLeaf};

/// Sort+dedup `not_multiple_of`, drop entries implied by a kept one (`p | q`), and return `None`
/// (caller emits `Schema::False`) on contradiction: any `abs(q) == 1`, or `q` divides `multiple_of`.
///
/// ```text
/// BEFORE: {"type": "integer", "not": {"multipleOf": 2}}  and  {"type": "integer", "not": {"multipleOf": 4}}
/// AFTER:  {"type": "integer", "not": {"multipleOf": 2}}  // multiples of 4 are already excluded as multiples of 2
///
/// BEFORE: {"type": "integer", "multipleOf": 6}  and  {"type": "integer", "not": {"multipleOf": 2}}
/// AFTER:  false                                          // every multiple of 6 is even
/// ```
pub(crate) fn normalize_integer_not_multiple_of(mut leaf: IntegerLeaf) -> Option<IntegerLeaf> {
    let kept = dedup_not_multiple_of(
        &mut leaf.not_multiple_of,
        leaf.multiple_of.as_ref(),
        BoundInteger::is_zero,
        |divisor, value| value.mod_floor(divisor).is_zero(),
    )?;
    leaf.not_multiple_of = kept;
    // `q == 1` excludes every integer, so the leaf is empty. (No number analogue: `0.5` is not a multiple of `1`.)
    if leaf.not_multiple_of.iter().any(|q| q.abs().is_one()) {
        return None;
    }
    Some(leaf)
}

/// Number variant: uses fraction divisibility -- `m` is a multiple of `q` iff `m / q` is an integer.
///
/// ```text
/// BEFORE: {"type": "number", "multipleOf": 1.5}  and  {"type": "number", "not": {"multipleOf": 0.5}}
/// AFTER:  false                                  // 1.5 / 0.5 = 3, so every multiple of 1.5 is a multiple of 0.5
/// ```
pub(crate) fn normalize_number_not_multiple_of(mut leaf: NumberLeaf) -> Option<NumberLeaf> {
    leaf.not_multiple_of = dedup_not_multiple_of(
        &mut leaf.not_multiple_of,
        leaf.multiple_of.as_ref(),
        |q| q.numer().is_some_and(num_traits::Zero::is_zero),
        |divisor, value| (value / divisor).denominator_is_one(),
    )?;
    Some(leaf)
}

/// Sort+dedup `not_multiple_of`, drop entries implied by a kept one (`divides(kept, entry)`), return
/// `None` when `q` divides `multiple_of`. `divides` tests divisibility; `is_zero` guards a zero modulus.
fn dedup_not_multiple_of<T: Ord>(
    entries: &mut Vec<T>,
    multiple_of: Option<&T>,
    is_zero: impl Fn(&T) -> bool,
    divides: impl Fn(&T, &T) -> bool,
) -> Option<Vec<T>> {
    entries.sort_unstable();
    entries.dedup();
    let mut kept: Vec<T> = Vec::with_capacity(entries.len());
    for entry in std::mem::take(entries) {
        if !kept
            .iter()
            .any(|other| !is_zero(other) && divides(other, &entry))
        {
            kept.push(entry);
        }
    }
    if let Some(m) = multiple_of {
        if kept.iter().any(|q| !is_zero(q) && divides(q, m)) {
            return None;
        }
    }
    Some(kept)
}

/// Sort+dedup `not_patterns`; return `None` (=> caller emits `Schema::False`) when a pattern appears in both `patterns`
/// and `not_patterns` (syntactic equality -- sound, incomplete; full regex emptiness is not implemented).
///
/// ```text
/// BEFORE: {"type": "string", "pattern": "^a"}  and  {"type": "string", "not": {"pattern": "^a"}}
/// AFTER:  false                                 // a string can't both match and not match `^a`
/// ```
pub(crate) fn normalize_string_not_patterns(mut leaf: StringLeaf) -> Option<StringLeaf> {
    leaf.not_patterns.sort_unstable();
    leaf.not_patterns.dedup();
    let patterns: AHashSet<&Arc<str>> = leaf.patterns.iter().collect();
    if leaf.not_patterns.iter().any(|r| patterns.contains(r)) {
        return None;
    }
    Some(leaf)
}
