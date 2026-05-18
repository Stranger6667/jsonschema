use crate::{
    canonical::ir::{ArrayLeaf, BooleanBounds, BoundInteger, IntegerLeaf, Schema, SharedSchema},
    JsonType, JsonTypeSet,
};

/// Conservative upper bound on distinct JSON values that match `schema`. `None` means arbitrarily many or unsupported,
/// never an artificially small bound. Drives the `uniqueItems` `maxItems` cap (a unique boolean array fits at most 2).
///
/// ```text
/// {"type": "boolean"}  -> Some(2)
/// {"type": "integer"}  -> None
/// ```
#[must_use]
pub(crate) fn finite_universe_size(schema: &SharedSchema) -> Option<u64> {
    match schema.as_schema() {
        Schema::Null
        | Schema::Boolean(BooleanBounds::JustTrue | BooleanBounds::JustFalse)
        | Schema::Const(_) => Some(1),
        Schema::False => Some(0),
        Schema::Boolean(BooleanBounds::Any) => Some(2),
        Schema::Enum(values) => Some(values.len() as u64),
        Schema::Integer(leaf) => integer_universe_size(leaf),
        Schema::Array(leaf) => array_universe_size(leaf),
        Schema::AnyOf(branches) => sum_branch_sizes(branches),
        Schema::TypedGroup { body, .. } => finite_universe_size(body),
        Schema::MultiType(set) => multi_type_cardinality(*set),
        Schema::OneOf(_)
        | Schema::Number(_)
        | Schema::TypeGuard { .. }
        | Schema::True
        | Schema::String(_)
        | Schema::Object(_)
        | Schema::AllOf(_)
        | Schema::Not(_)
        | Schema::IfThenElse(_)
        | Schema::Reference(_)
        | Schema::Recursive(_)
        | Schema::DynamicRef(_)
        | Schema::Raw(_) => None,
    }
}

/// Each type tag's universe: `Null` has 1 inhabitant, `Boolean` has 2, every other type is infinite. The set's
/// cardinality is the sum when all entries are finite, else `None`.
fn multi_type_cardinality(set: JsonTypeSet) -> Option<u64> {
    let mut total: u64 = 0;
    for ty in set {
        let count = match ty {
            JsonType::Null => 1,
            JsonType::Boolean => 2,
            _ => return None,
        };
        total = total.checked_add(count)?;
    }
    Some(total)
}

fn sum_branch_sizes(branches: &[SharedSchema]) -> Option<u64> {
    let mut total: u64 = 0;
    for branch in branches {
        total = total.checked_add(finite_universe_size(branch)?)?;
    }
    Some(total)
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "`&BoundInteger` is `&i64` without `arbitrary-precision` but `&BigInt` with it."
)]
fn integer_span_to_u64(minimum: &BoundInteger, maximum: &BoundInteger) -> Option<u64> {
    // `maximum - minimum + 1`, but a full-range integer leaf (`i64::MIN..=i64::MAX`) would overflow the
    // unchecked operators; an unrepresentable span just means "too many to enumerate" -> `None`.
    let span = maximum.checked_sub(minimum)?.checked_increment()?;
    span.to_u64()
}

fn integer_universe_size(leaf: &IntegerLeaf) -> Option<u64> {
    let minimum = (leaf.bounds.minimum.as_ref()).map(BoundInteger::owned)?;
    let maximum = (leaf.bounds.maximum.as_ref()).map(BoundInteger::owned)?;
    integer_span_to_u64(&minimum, &maximum)
}

/// Sum, over every admissible length `N` in `[minItems, maxItems]`, of the distinct length-`N` arrays the leaf accepts.
/// Per-length count is the product of each position's universe (prefix slot, or tail once past the prefix); `None`
/// unless `maxItems` and every position universe are finite.
///
/// `maxItems` can reach `u64::MAX`, so lengths are summed in closed form: prefix-only lengths give `<= prefix_len + 1`
/// terms, longer lengths the geometric series `P_full * tail_size^(N - prefix_len)` (O(1) or overflows within ~64 terms).
///
/// ```text
/// {"type": "array", "items": {"type": "boolean"}, "minItems": 0, "maxItems": 2}  -> Some(7)
/// // length 0: 1, length 1: 2, length 2: 4
/// ```
fn array_universe_size(leaf: &ArrayLeaf) -> Option<u64> {
    let max_items = leaf.length.maximum.as_ref()?.to_u64()?;
    let min_items = leaf.length.minimum.to_u64()?;
    let prefix_sizes: Vec<u64> = leaf
        .prefix
        .iter()
        .map(finite_universe_size)
        .collect::<Option<_>>()?;
    let tail_size = finite_universe_size(&leaf.tail)?;
    let prefix_len = u64::try_from(leaf.prefix.len()).ok()?;

    // `uniqueItems` only reduces the count, so the uniqueness-ignoring product is a sound upper bound; no
    // pigeonhole zeroing, which would undercount heterogeneous per-position universes.
    let mut total: u64 = 0;

    // Range A - lengths in `[min_items, min(max_items, prefix_len)]`, accumulating the prefix product slot
    // by slot. After the loop `prefix_product` is the full prefix product once every slot is consumed.
    let mut prefix_product: u64 = 1;
    let range_a_end = max_items.min(prefix_len);
    for n in 0..=range_a_end {
        if n >= min_items {
            total = total.checked_add(prefix_product)?;
        }
        if n < prefix_len {
            let slot = prefix_sizes[usize::try_from(n).ok()?];
            prefix_product = prefix_product.checked_mul(slot)?;
        }
    }
    if max_items <= prefix_len {
        return Some(total);
    }
    let p_full = prefix_product;

    // Range B - lengths in `[max(min_items, prefix_len + 1), max_items]`, each `P_full * tail_size^e` with
    // `e = N - prefix_len >= 1`. Sum the geometric series `tail_size^e_lo + ... + tail_size^e_hi`.
    let lo = min_items.max(prefix_len + 1);
    if lo > max_items {
        return Some(total);
    }
    let e_lo = lo - prefix_len;
    let e_hi = max_items - prefix_len;
    let term_count = e_hi - e_lo + 1;
    let series = match tail_size {
        // Every range-B length has at least one tail slot, so a zero tail universe contributes nothing.
        0 => 0,
        // All powers are 1, so the series is just the number of admissible lengths.
        1 => term_count,
        // Each power at least doubles, so overflow (`-> None`) is reached within ~64 terms - never a hang.
        _ => {
            let mut sum: u64 = 0;
            let mut power: u64 = tail_size.checked_pow(u32::try_from(e_lo).ok()?)?;
            let mut remaining = term_count;
            loop {
                sum = sum.checked_add(power)?;
                remaining -= 1;
                if remaining == 0 {
                    break;
                }
                power = power.checked_mul(tail_size)?;
            }
            sum
        }
    };
    total = total.checked_add(p_full.checked_mul(series)?)?;
    Some(total)
}
