use crate::canonical::{
    cardinality::finite_universe_size,
    context::CanonicalizationContext,
    coverage,
    coverage::covers,
    intersect::intersect_canonical,
    ir::{ArrayLeaf, BoundCardinality, ContainsClause, LengthBounds, Schema, SharedSchema},
    leaves::{Intersection, Leaf, TypedLeaf, Verdict},
    prover::Prover,
};

use super::{intersect_internal, min_option};

impl Leaf for ArrayLeaf {
    /// Length bounds tighten, `prefixItems` merge position-by-position (falling back to the other
    /// side's `items` tail past its end), and `contains` clauses union.
    ///
    /// ```text
    /// BEFORE: {"type": "array", "minItems": 2}  and  {"type": "array", "maxItems": 5}
    /// AFTER:  {"type": "array", "minItems": 2, "maxItems": 5}
    ///
    /// BEFORE: {"type": "array", "prefixItems": [{"type": "integer"}]}  and  {"type": "array", "prefixItems": [{"minimum": 0}]}
    /// AFTER:  {"type": "array", "prefixItems": [{"type": "integer", "minimum": 0}]}
    /// ```
    fn intersect(&self, other: &Self, ctx: &CanonicalizationContext) -> Intersection<Self> {
        let tail = intersect_internal(&self.tail, &other.tail, ctx);
        let prefix = intersect_prefixes(&self.prefix, &self.tail, &other.prefix, &other.tail, ctx);

        let mut contains: Vec<ContainsClause> = self
            .contains
            .iter()
            .chain(other.contains.iter())
            .cloned()
            .collect();
        if !merge_equal_contains(&mut contains) {
            return Intersection::Empty;
        }
        drop_subsumed_contains(&mut contains, ctx);

        let min_contains = contains_min(&contains);
        let mut minimum = if self.length.minimum >= other.length.minimum {
            self.length.minimum.owned()
        } else {
            other.length.minimum.owned()
        };
        if min_contains > minimum {
            minimum = min_contains;
        }
        let maximum = min_option(self.length.maximum.as_ref(), other.length.maximum.as_ref());
        if let Some(maximum) = &maximum {
            if &minimum > maximum {
                return Intersection::Empty;
            }
        }

        let leaf = ArrayLeaf {
            prefix,
            tail,
            length: LengthBounds { minimum, maximum },
            unique_items: self.unique_items || other.unique_items,
            repeated_items: self.repeated_items || other.repeated_items,
            contains,
        };
        match leaf.normalize_repeated_items() {
            Some(leaf) => Intersection::Merged(leaf),
            None => Intersection::Empty,
        }
    }

    fn covers(&self, other: &Self, prover: &Prover<'_>) -> Verdict {
        Verdict::proven_if(coverage::array_leaf_covers(self, other, prover))
    }

    /// `contains` is only decided when nothing else can bound or collide with the witnesses.
    fn inhabited(&self, _formats_asserted: bool) -> Verdict {
        Verdict::proven_if(
            self.contains.is_empty()
                || (self.prefix.is_empty()
                    && matches!(self.tail.as_schema(), Schema::True)
                    && self.length.maximum.is_none()
                    && !self.unique_items
                    && !self.repeated_items
                    && self
                        .contains
                        .iter()
                        .all(|clause| clause.max_contains.is_none())),
        )
    }

    fn is_open(&self) -> bool {
        self.prefix.is_empty()
            && matches!(self.tail.as_schema(), Schema::True)
            && self.length == LengthBounds::default()
            && !self.unique_items
            && !self.repeated_items
            && self.contains.is_empty()
    }

    fn is_empty(&self, ctx: &CanonicalizationContext) -> bool {
        if array_length_bounds_empty(self) {
            return true;
        }
        if array_items_block_min(self) {
            return true;
        }
        if contains_unsatisfiable(&self.contains) {
            return true;
        }
        if unique_finite_universe_overflow(self) {
            return true;
        }
        if items_contains_disjoint(self) {
            return true;
        }
        contains_witness_impossible(self, ctx)
    }
}

impl TypedLeaf for ArrayLeaf {
    fn wrap(self) -> Schema {
        Schema::Array(self)
    }
    fn project(schema: &Schema) -> Option<&Self> {
        match schema {
            Schema::Array(leaf) => Some(leaf),
            _ => None,
        }
    }
}

fn array_length_bounds_empty(leaf: &ArrayLeaf) -> bool {
    leaf.length
        .maximum
        .as_ref()
        .is_some_and(|maximum| &leaf.length.minimum > maximum)
}

fn array_items_block_min(leaf: &ArrayLeaf) -> bool {
    let minimum = &leaf.length.minimum;
    if minimum.is_zero() {
        return false;
    }
    for (index, schema) in leaf.prefix.iter().enumerate() {
        let position = u64::try_from(index).unwrap_or(u64::MAX);
        if position < *minimum && matches!(schema.as_schema(), Schema::False) {
            return true;
        }
    }
    if matches!(leaf.tail.as_schema(), Schema::False) {
        let prefix_len = u64::try_from(leaf.prefix.len()).unwrap_or(u64::MAX);
        if prefix_len < *minimum {
            return true;
        }
    }
    false
}

fn contains_unsatisfiable(contains: &[ContainsClause]) -> bool {
    contains.iter().any(|clause| {
        !clause.min_contains.is_zero() && matches!(clause.schema.as_schema(), Schema::False)
    })
}

fn unique_finite_universe_overflow(leaf: &ArrayLeaf) -> bool {
    if !leaf.unique_items || leaf.length.minimum < 2u64 {
        return false;
    }
    let mut capacity: u64 = 0;
    for slot in &leaf.prefix {
        let Some(size) = finite_universe_size(slot) else {
            return false;
        };
        let Some(next) = capacity.checked_add(size) else {
            return false;
        };
        capacity = next;
    }
    let Some(tail_size) = finite_universe_size(&leaf.tail) else {
        return false;
    };
    let Some(total) = capacity.checked_add(tail_size) else {
        return false;
    };
    total < leaf.length.minimum
}

fn items_contains_disjoint(leaf: &ArrayLeaf) -> bool {
    if leaf.length.minimum.is_zero() && contains_min(&leaf.contains).is_zero() {
        return false;
    }
    let Some(item_type) = leaf.tail.as_schema().pinned_kind() else {
        return false;
    };
    let prefix_kinds: Vec<crate::JsonType> = leaf
        .prefix
        .iter()
        .filter_map(|schema| schema.as_schema().pinned_kind())
        .collect();
    if prefix_kinds.len() != leaf.prefix.len() {
        return false;
    }
    leaf.contains.iter().any(|clause| {
        if clause.min_contains.is_zero() {
            return false;
        }
        let Some(contains_type) = clause.schema.as_schema().pinned_kind() else {
            return false;
        };
        if contains_type.overlaps(item_type) {
            return false;
        }
        prefix_kinds
            .iter()
            .all(|prefix_kind| !contains_type.overlaps(*prefix_kind))
    })
}

fn contains_min(contains: &[ContainsClause]) -> BoundCardinality {
    contains
        .iter()
        .map(|clause| &clause.min_contains)
        .max()
        .map_or_else(|| BoundCardinality::from(0u64), BoundCardinality::owned)
}

fn contains_witness_impossible(leaf: &ArrayLeaf, ctx: &CanonicalizationContext) -> bool {
    leaf.contains.iter().any(|clause| {
        !clause.min_contains.is_zero()
            && intersect_is_empty(&leaf.tail, &clause.schema, ctx)
            && leaf
                .prefix
                .iter()
                .all(|slot| intersect_is_empty(slot, &clause.schema, ctx))
    })
}

fn intersect_is_empty(
    left: &SharedSchema,
    right: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> bool {
    matches!(
        intersect_canonical(left, right, ctx).as_schema(),
        Schema::False
    )
}

/// Longer prefix's tail positions intersect against the shorter side's `tail`.
fn intersect_prefixes(
    left_prefix: &[SharedSchema],
    left_tail: &SharedSchema,
    right_prefix: &[SharedSchema],
    right_tail: &SharedSchema,
    ctx: &CanonicalizationContext,
) -> Vec<SharedSchema> {
    let len = left_prefix.len().max(right_prefix.len());
    let mut out = Vec::with_capacity(len);
    for index in 0..len {
        let lhs = left_prefix.get(index).unwrap_or(left_tail);
        let rhs = right_prefix.get(index).unwrap_or(right_tail);
        out.push(intersect_internal(lhs, rhs, ctx));
    }
    out
}

/// Merges `contains` clauses with identical schemas by intersecting their count windows: `[a1, b1]`
/// and `[a2, b2]` become `[max(a1,a2), min(b1,b2)]`. An empty window makes the leaf unsatisfiable.
///
/// ```text
/// BEFORE: {"allOf": [{"contains": {"type": "integer"}, "maxContains": 0},
///                    {"contains": {"type": "integer"}, "maxContains": 1, "minContains": 1}]}
/// AFTER:  false   // no array holds both exactly 0 and exactly 1 integer
/// ```
fn merge_equal_contains(clauses: &mut Vec<ContainsClause>) -> bool {
    let mut index = 0;
    while index < clauses.len() {
        let mut other = index + 1;
        while other < clauses.len() {
            if clauses[index].schema != clauses[other].schema {
                other += 1;
                continue;
            }
            let merged = clauses.remove(other);
            let min_contains =
                std::mem::take(&mut clauses[index].min_contains).max(merged.min_contains);
            let max_contains = match (clauses[index].max_contains.take(), merged.max_contains) {
                (Some(left), Some(right)) => Some(left.min(right)),
                (Some(bound), None) | (None, Some(bound)) => Some(bound),
                (None, None) => None,
            };
            if let Some(max_contains) = &max_contains {
                if &min_contains > max_contains {
                    return false;
                }
            }
            clauses[index].min_contains = min_contains;
            clauses[index].max_contains = max_contains;
        }
        index += 1;
    }
    true
}

/// Drops a `contains` clause implied by a stricter sibling sharing the same count window: N items
/// matching the narrower schema are also N items matching the wider one.
///
/// ```text
/// BEFORE: {"allOf": [{"contains": {"type": "integer"}},
///                    {"contains": {"type": "integer", "minimum": 0}}]}
/// AFTER:  {"contains": {"type": "integer", "minimum": 0}, "minItems": 1}
/// ```
///
/// Windows must match: with differing counts the wider clause's `maxContains` still bounds the
/// count, so dropping it would widen the admissible range.
fn drop_subsumed_contains(clauses: &mut Vec<ContainsClause>, ctx: &CanonicalizationContext) {
    if clauses.len() <= 1 {
        return;
    }
    let mut keep = vec![true; clauses.len()];
    for (index, wider) in clauses.iter().enumerate() {
        for (other_index, stricter) in clauses.iter().enumerate() {
            if index == other_index || !keep[other_index] {
                continue;
            }
            if is_subsumed_by(wider, stricter, ctx) {
                keep[index] = false;
                break;
            }
        }
    }
    let mut keep_iter = keep.into_iter();
    clauses.retain(|_| {
        keep_iter
            .next()
            .expect("keep has the same length as clauses")
    });
}

/// True when `wider` is redundant given `stricter`: same count window, and `wider` covers `stricter`
/// but not the reverse - the reverse check stops two equivalent clauses from both dropping.
fn is_subsumed_by(
    wider: &ContainsClause,
    stricter: &ContainsClause,
    ctx: &CanonicalizationContext,
) -> bool {
    if wider.min_contains != stricter.min_contains || wider.max_contains != stricter.max_contains {
        return false;
    }
    covers(&wider.schema, &stricter.schema, ctx) && !covers(&stricter.schema, &wider.schema, ctx)
}
