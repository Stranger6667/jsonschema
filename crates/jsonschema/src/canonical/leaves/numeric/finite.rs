use std::cmp::Ordering;

use referencing::Draft;

use crate::{
    canonical::{
        const_enum::intern_value_set,
        context::keeps_draft4_integer_guard,
        intern::shared,
        ir::{
            BoundFraction, BoundInteger, CanonicalJson, IntegerBounds, IntegerLeaf, NumberLeaf,
            Schema, SharedSchema,
        },
        membership::DOMAIN_CAP,
    },
    JsonType,
};

/// The canonical spelling of a *finite* set of numbers: a value set, except an all-integer consecutive
/// run, which stays the range leaf `{int min..max}`.
#[derive(Debug, PartialEq, Eq, Clone)]
enum FiniteForm {
    /// One value -> `const`.
    Single,
    /// Consecutive integers -> `{int min..max}`.
    Window { min: i64, max: i64 },
    /// Anything else -> sorted `enum`.
    Enum,
}

/// Classify a sorted, distinct, non-empty finite set into its [`FiniteForm`]. Pure; carrier-agnostic.
fn classify_finite(sorted: &[BoundFraction]) -> FiniteForm {
    match sorted.len() {
        0 => return FiniteForm::Enum,
        1 => return FiniteForm::Single,
        _ => {}
    }
    if let Some(integers) = sorted
        .iter()
        .map(fraction_to_i64)
        .collect::<Option<Vec<i64>>>()
    {
        if integers
            .windows(2)
            .all(|pair| pair[1].checked_sub(pair[0]) == Some(1))
        {
            return FiniteForm::Window {
                min: integers[0],
                max: integers[integers.len() - 1],
            };
        }
    }
    FiniteForm::Enum
}

/// `BoundFraction` to `i64` when it is an integer in range, else `None`.
fn fraction_to_i64(value: &BoundFraction) -> Option<i64> {
    value.to_integer().and_then(|integer| integer.to_i64())
}

/// Emit the canonical leaf (or `enum`) for a sorted, distinct finite set: a consecutive integer run of three or
/// more keeps the range leaf, everything else is a value set (`False` when empty). `None` if a value resists canonical JSON.
fn emit_finite_core(sorted: &[BoundFraction]) -> Option<SharedSchema> {
    match classify_finite(sorted) {
        FiniteForm::Window { min, max } if sorted.len() >= 3 => {
            Some(integer_leaf(min, max, None, Vec::new()))
        }
        _ => emit_value_set(sorted),
    }
}

/// Emit a finite set as the canonical branch(es): one expressible leaf shape when the whole set is exactly one,
/// otherwise one value set -- never a window/scatter split. The empty set yields no branches.
pub(crate) fn emit_finite(sorted: &[BoundFraction]) -> Option<Vec<SharedSchema>> {
    emit_finite_core(sorted).map(|schema| match schema.as_schema() {
        Schema::False => Vec::new(),
        _ => vec![schema],
    })
}

/// A finite set as `const`/`enum`; `False` when empty. `None` if a value resists canonical JSON.
fn emit_value_set(values: &[BoundFraction]) -> Option<SharedSchema> {
    if values.is_empty() {
        return Some(shared(Schema::False));
    }
    let jsons: Option<Vec<CanonicalJson>> = values
        .iter()
        .map(BoundFraction::to_canonical_json)
        .collect();
    Some(intern_value_set(jsons?))
}

/// A bounded integer leaf `{int min..max [multipleOf] [not multipleOf ...]}`.
fn integer_leaf(
    min: i64,
    max: i64,
    multiple_of: Option<i64>,
    not_multiple_of: Vec<i64>,
) -> SharedSchema {
    shared(Schema::Integer(IntegerLeaf {
        bounds: IntegerBounds {
            minimum: Some(BoundInteger::from(min)),
            maximum: Some(BoundInteger::from(max)),
            exclusive_minimum: false,
            exclusive_maximum: false,
        },
        multiple_of: multiple_of.map(BoundInteger::from),
        not_multiple_of: not_multiple_of
            .into_iter()
            .map(BoundInteger::from)
            .collect(),
    }))
}

/// Collapse an enumerated finite set to its single canonical branch (`False` when empty). `None` when a member
/// resists canonical JSON. `emit_finite` yields at most one branch, so the result is that branch or `False`.
fn emit_finite_single(members: &[BoundFraction]) -> Option<SharedSchema> {
    Some(
        emit_finite(members)?
            .pop()
            .unwrap_or_else(|| shared(Schema::False)),
    )
}

/// The multiples of `step` a bounded numeric leaf admits, from `first_factor` (least multiple `>=` lower bound) while
/// `<= maximum`, minus exclusion-divided candidates. `None` past [`DOMAIN_CAP`] members, `examined_cap` scans, or factor overflow.
fn enumerate_bounded_multiples(
    first_factor: BoundInteger,
    step: &BoundFraction,
    maximum: &BoundFraction,
    exclusions: &[BoundFraction],
    examined_cap: Option<usize>,
) -> Option<Vec<BoundFraction>> {
    let mut members: Vec<BoundFraction> = Vec::new();
    let mut factor = first_factor;
    let mut examined = 0_usize;
    loop {
        // Overflowing grid arithmetic or an undecidable exclusion declines the enumeration.
        let candidate = BoundFraction::from(factor.owned()).checked_mul(step)?;
        if candidate.cmp(maximum) == Ordering::Greater {
            break;
        }
        if members.len() > DOMAIN_CAP {
            return None;
        }
        if examined_cap.is_some_and(|cap| examined >= cap) {
            return None;
        }
        examined += 1;
        let mut is_excluded = false;
        for exclusion in exclusions {
            if exclusion.divides(&candidate)? {
                is_excluded = true;
                break;
            }
        }
        if !is_excluded {
            members.push(candidate);
        }
        factor = factor.checked_increment()?;
    }
    Some(members)
}

/// The finite members of a bounded `{number ... multipleOf q}` leaf with a fractional `q` (integer `q` denotes
/// integers, on the integer carrier). `None` if unbounded, exclusive-bounded, non-fractional, or grid past the guard.
pub(super) fn bounded_number_grid_members(leaf: &NumberLeaf) -> Option<Vec<BoundFraction>> {
    if leaf.bounds.exclusive_minimum || leaf.bounds.exclusive_maximum {
        return None;
    }
    let minimum = leaf.bounds.minimum.as_ref()?;
    let maximum = leaf.bounds.maximum.as_ref()?;
    let modulus = leaf.multiple_of.as_ref()?;
    if modulus.denominator_is_one() || modulus.is_zero() {
        return None;
    }
    let step = modulus.abs();
    let first_factor = minimum.checked_div(&step)?.ceil_integer()?;
    enumerate_bounded_multiples(
        first_factor,
        &step,
        maximum,
        &leaf.not_multiple_of,
        Some(DOMAIN_CAP),
    )
}

/// The canonical finite respelling of a bounded fractional-`multipleOf` number leaf: its grid members as a value
/// set (`False` when empty). `None` outside the bounded-fractional-grid fragment.
pub(crate) fn bounded_number_grid_leaf(leaf: &NumberLeaf) -> Option<SharedSchema> {
    let members = bounded_number_grid_members(leaf)?;
    emit_finite_single(&members)
}

/// The canonical finite respelling of a bounded integer leaf with a modulus or exclusions. A plain bounded
/// range is already canonical unless it emits to a shorter value set.
pub(crate) fn bounded_integer_grid_leaf(leaf: &IntegerLeaf, draft: Draft) -> Option<SharedSchema> {
    if keeps_draft4_integer_guard(JsonType::Integer, draft) {
        return None;
    }
    let minimum = leaf.bounds.effective_minimum()?;
    let maximum = leaf.bounds.effective_maximum()?;
    if minimum > maximum {
        return None;
    }
    let step_integer = match leaf.multiple_of.as_ref() {
        Some(modulus) if modulus.is_zero() => return None,
        Some(modulus) => modulus.abs(),
        None => BoundInteger::from(1),
    };
    // `div_ceil` is the least factor whose multiple is `>= minimum`. Bounds are snapped before this point, so
    // that is `minimum / step`; using the ceiling keeps the grid correct even if a bound ever resists snapping.
    let first_factor = minimum.div_ceil(&step_integer);
    let step = BoundFraction::from(step_integer.owned());
    let maximum = BoundFraction::from(maximum.owned());
    let exclusions: Vec<BoundFraction> = leaf
        .not_multiple_of
        .iter()
        .map(|modulus| BoundFraction::from((modulus).owned()))
        .collect();
    let members = enumerate_bounded_multiples(first_factor, &step, &maximum, &exclusions, None)?;
    let result = emit_finite_single(&members)?;
    if result.as_schema() == &Schema::Integer(leaf.clone()) {
        return None;
    }
    Some(result)
}
