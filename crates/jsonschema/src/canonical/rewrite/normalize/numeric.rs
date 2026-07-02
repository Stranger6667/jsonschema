#![cfg_attr(
    not(feature = "arbitrary-precision"),
    allow(
        clippy::clone_on_copy,
        clippy::cloned_instead_of_copied,
        clippy::map_clone,
        clippy::op_ref,
        clippy::trivially_copy_pass_by_ref
    )
)]

use num_traits::Zero;
#[cfg(not(feature = "arbitrary-precision"))]
use serde_json::{Number, Value};

use crate::canonical::{
    context::{CanonicalizationContext, WalkStage},
    intern::shared,
    ir::{
        BoundFraction, BoundInteger, CanonicalJson, CanonicalKind, IntegerBounds, IntegerLeaf,
        NumberLeaf, Schema, SchemaKindSet, SharedSchema,
    },
    numeric::{
        bounded_integer_grid_leaf, bounded_number_grid_leaf, number_bounds_to_integer,
        number_multiple_of_to_integer, number_not_multiple_of_to_integer,
    },
};

/// Tightens numeric leaves: an integer-valued `multipleOf` promotes `number` to `integer`, bounds snap to the
/// nearest admissible value, and a window holding a single value collapses to `const`.
///
/// ```text
/// BEFORE: {"type": "number", "multipleOf": 1.0}
/// AFTER:  {"type": "integer"}
///
/// BEFORE: {"type": "integer", "minimum": 1, "multipleOf": 2}
/// AFTER:  {"type": "integer", "minimum": 2, "multipleOf": 2}
///
/// BEFORE: {"type": "integer", "minimum": 5, "maximum": 5}
/// AFTER:  {"const": 5}
/// ```
#[must_use]
pub(crate) fn normalize(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
    super::run::<NumericStage>(schema, ctx)
}

pub(crate) struct NumericStage;

impl super::NormalizeStage for NumericStage {
    const WALK: WalkStage = WalkStage::Numeric;
    const MASK: SchemaKindSet =
        SchemaKindSet::from_kinds(&[CanonicalKind::Number, CanonicalKind::Integer]);

    fn rewrite(recursed: SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
        match recursed.as_schema() {
            Schema::Number(leaf) => {
                if let Some(promoted) = try_promote_number(leaf) {
                    promoted
                } else if let Some(collapsed) = try_collapse_number_to_const(leaf) {
                    collapsed
                } else if let Some(canonical) = bounded_number_grid_leaf(leaf) {
                    canonical
                } else if let Some(split) = try_split_number_window_integers(leaf) {
                    split
                } else {
                    recursed
                }
            }
            Schema::Integer(leaf) => {
                let mut node = inclusive_integer_bounds(leaf).unwrap_or(recursed);
                if let Schema::Integer(leaf) = node.as_schema() {
                    if let Some(snapped) = snap_integer_bounds(leaf) {
                        node = snapped;
                    }
                }
                if let Schema::Integer(leaf) = node.as_schema() {
                    if let Some(trimmed) = trim_integer_exclusion_endpoints(leaf) {
                        node = trimmed;
                    }
                }
                if let Schema::Integer(leaf) = node.as_schema() {
                    if let Some(reduced) = reduce_modular_exclusions(leaf) {
                        node = reduced;
                    }
                }
                let Schema::Integer(leaf) = node.as_schema() else {
                    // The integer rewrites above only rebuild the same variant.
                    unreachable!("integer leaf changed variant");
                };
                if let Some(enumerated) = bounded_integer_grid_leaf(leaf, ctx.draft()) {
                    return enumerated;
                }
                node
            }
            _ => recursed,
        }
    }
}

/// Reduce each `not multipleOf m` to the minimal modulus with the same exclusion within a `multipleOf k` leaf:
/// keep the prime powers of `m` exceeding `k`'s (`{int mult2 not mult6}` and `{not mult3}` both give `not mult3`).
/// A reduction to `1` means the leaf is empty; left unchanged for the emptiness detector to own.
fn reduce_modular_exclusions(leaf: &IntegerLeaf) -> Option<SharedSchema> {
    let multiple_of = leaf.multiple_of.as_ref()?;
    if leaf.not_multiple_of.is_empty() {
        return None;
    }
    let modulus = multiple_of.abs().to_i64()?;
    if modulus <= 1 {
        return None;
    }
    let modulus_factors = prime_factorization(modulus);
    let mut changed = false;
    let mut reduced: Vec<BoundInteger> = Vec::with_capacity(leaf.not_multiple_of.len());
    for entry in &leaf.not_multiple_of {
        let Some(exclusion) = entry.abs().to_i64() else {
            reduced.push((entry).owned());
            continue;
        };
        let target = reduced_exclusion_modulus(&modulus_factors, exclusion);
        if target <= 1 || target == exclusion {
            reduced.push((entry).owned());
        } else {
            reduced.push(BoundInteger::from(target));
            changed = true;
        }
    }
    if !changed {
        return None;
    }
    reduced.sort_unstable();
    reduced.dedup();
    Some(shared(Schema::Integer(IntegerLeaf {
        bounds: leaf.bounds.clone(),
        multiple_of: (leaf.multiple_of.as_ref()).map(BoundInteger::owned),
        not_multiple_of: reduced,
    })))
}

/// The minimal `m'` with `lcm(modulus, m') == lcm(modulus, exclusion)`: the product of `exclusion`'s prime
/// powers exceeding `modulus`'s (passed as its factorization). `1` when `exclusion` divides `modulus`.
fn reduced_exclusion_modulus(modulus_factors: &[(i64, u32)], exclusion: i64) -> i64 {
    if exclusion <= 1 {
        return exclusion;
    }
    let modulus_power = |prime: i64| {
        modulus_factors
            .iter()
            .find(|(factor, _)| *factor == prime)
            .map_or(0, |(_, power)| *power)
    };
    let mut remaining = exclusion;
    let mut result = 1_i64;
    let mut prime = 2_i64;
    while prime.saturating_mul(prime) <= remaining {
        if remaining % prime == 0 {
            let mut exclusion_power = 0_u32;
            while remaining % prime == 0 {
                remaining /= prime;
                exclusion_power += 1;
            }
            if exclusion_power > modulus_power(prime) {
                result = result.saturating_mul(prime.saturating_pow(exclusion_power));
            }
        }
        prime += 1;
    }
    if remaining > 1 && modulus_power(remaining) == 0 {
        result = result.saturating_mul(remaining);
    }
    result
}

/// Prime factorization of `value` as `(prime, power)` pairs in ascending order.
fn prime_factorization(mut value: i64) -> Vec<(i64, u32)> {
    let mut factors = Vec::new();
    let mut prime = 2_i64;
    while prime.saturating_mul(prime) <= value {
        if value % prime == 0 {
            let mut power = 0_u32;
            while value % prime == 0 {
                value /= prime;
                power += 1;
            }
            factors.push((prime, power));
        }
        prime += 1;
    }
    if value > 1 {
        factors.push((value, 1));
    }
    factors
}

/// Drop integer exclusion moduli when the `1` entry already excludes every integer, and split a bounded
/// window into fractional-window plus integer-branch - the spelling a negation image cannot recover the modulus from.
///
/// ```text
/// BEFORE: {"type": "number", "minimum": 0, "maximum": 1, "not": {"multipleOf": 2}}
/// AFTER:  {"anyOf": [{"type": "number", "minimum": 0, "maximum": 1, "not": {"multipleOf": 1}},
///                    {"const": 1}]}
/// ```
fn try_split_number_window_integers(leaf: &NumberLeaf) -> Option<SharedSchema> {
    if leaf.multiple_of.is_some() {
        return None;
    }
    let one = BoundFraction::from(1);
    let has_integer_marker = leaf.not_multiple_of.contains(&one);
    let (integer_moduli, kept): (Vec<&BoundFraction>, Vec<&BoundFraction>) = leaf
        .not_multiple_of
        .iter()
        .partition(|entry| entry.denominator_is_one() && **entry != one);
    if integer_moduli.is_empty() {
        return None;
    }
    if has_integer_marker {
        // Multiples of an integer modulus are integers, which the `1` entry already excludes.
        return Some(shared(Schema::Number(NumberLeaf {
            bounds: leaf.bounds.clone(),
            multiple_of: None,
            not_multiple_of: kept.into_iter().cloned().collect(),
        })));
    }
    if leaf.bounds.minimum.is_none() || leaf.bounds.maximum.is_none() {
        return None;
    }
    // An integer multiple of `q = a/b` (lowest terms) is a multiple of `a`, so each exclusion entry
    // yields one integer modulus; the integer leaf then takes its ordinary spelling through the fixpoint.
    let mut integer_moduli: Vec<BoundInteger> = leaf
        .not_multiple_of
        .iter()
        .map(|entry| entry.numerator_or_zero().abs())
        .collect();
    integer_moduli.sort_unstable();
    integer_moduli.dedup();
    let integer_branch = shared(Schema::Integer(IntegerLeaf {
        bounds: number_bounds_to_integer(&leaf.bounds),
        multiple_of: None,
        not_multiple_of: integer_moduli,
    }));
    let mut exclusions: Vec<BoundFraction> = kept.into_iter().cloned().collect();
    exclusions.push(one);
    exclusions.sort_unstable();
    let window = shared(Schema::Number(NumberLeaf {
        bounds: leaf.bounds.clone(),
        multiple_of: None,
        not_multiple_of: exclusions,
    }));
    Some(shared(Schema::AnyOf(vec![window, integer_branch])))
}

/// Promote `Number` to `Integer` when `multipleOf` is integer-valued. The leaf then denotes only integers, so
/// each `not multipleOf q` maps to the integer exclusion on `q`'s numerator (no fractional witness survives).
fn try_promote_number(leaf: &NumberLeaf) -> Option<SharedSchema> {
    let modulus = leaf.multiple_of.as_ref()?;
    if !modulus.denominator_is_one() || modulus.is_zero() {
        return None;
    }
    let bounds = number_bounds_to_integer(&leaf.bounds);
    let multiple_of = number_multiple_of_to_integer(Some(modulus));
    let mut not_multiple_of = Vec::with_capacity(leaf.not_multiple_of.len());
    for entry in &leaf.not_multiple_of {
        not_multiple_of.push(number_not_multiple_of_to_integer(entry)?);
    }
    not_multiple_of.sort_unstable();
    not_multiple_of.dedup();
    Some(shared(Schema::Integer(IntegerLeaf {
        bounds,
        multiple_of,
        not_multiple_of,
    })))
}

/// Integer domains are discrete: exclusive bounds tighten to the inclusive neighbor (`exclusiveMinimum: 0` is
/// `minimum: 1`), giving one canonical form. A bound without a representable neighbor stays exclusive.
fn inclusive_integer_bounds(leaf: &IntegerLeaf) -> Option<SharedSchema> {
    let mut bounds = leaf.bounds.clone();
    let mut changed = false;
    if bounds.exclusive_minimum {
        if let Some(next) = bounds
            .minimum
            .as_ref()
            .and_then(BoundInteger::checked_increment)
        {
            bounds.minimum = Some(next);
            bounds.exclusive_minimum = false;
            changed = true;
        }
    }
    if bounds.exclusive_maximum {
        if let Some(previous) = bounds
            .maximum
            .as_ref()
            .and_then(BoundInteger::checked_decrement)
        {
            bounds.maximum = Some(previous);
            bounds.exclusive_maximum = false;
            changed = true;
        }
    }
    if !changed {
        return None;
    }
    Some(shared(Schema::Integer(IntegerLeaf {
        bounds,
        multiple_of: (leaf.multiple_of.as_ref()).map(BoundInteger::owned),
        not_multiple_of: leaf.not_multiple_of.clone(),
    })))
}

/// Tighten bounds to the nearest admissible multiple.
fn snap_integer_bounds(leaf: &IntegerLeaf) -> Option<SharedSchema> {
    let modulus = leaf.multiple_of.as_ref()?;
    if modulus.is_zero() {
        return None;
    }
    let modulus = modulus.abs();
    let mut bounds = leaf.bounds.clone();
    let mut changed = false;
    if snap_lower(&mut bounds, &modulus) {
        changed = true;
    }
    if snap_upper(&mut bounds, &modulus) {
        changed = true;
    }
    if !changed {
        return None;
    }
    Some(shared(Schema::Integer(IntegerLeaf {
        bounds,
        multiple_of: (leaf.multiple_of.as_ref()).map(BoundInteger::owned),
        not_multiple_of: leaf.not_multiple_of.clone(),
    })))
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "`&BoundInteger` is `&i64` without `arbitrary-precision` but `&BigInt` with it."
)]
fn snap_lower(bounds: &mut IntegerBounds, modulus: &BoundInteger) -> bool {
    let Some(original) = bounds.minimum.as_ref() else {
        return false;
    };
    // `effective_minimum` folds an exclusive bound to its inclusive neighbor; `None` (no representable neighbor) keeps it.
    let Some(effective) = bounds.effective_minimum() else {
        return false;
    };
    // The next aligned multiple may exceed the representable range; keep the bound if so.
    let Some(snapped) = effective.checked_next_multiple_of(modulus) else {
        return false;
    };
    if snapped == *original && !bounds.exclusive_minimum {
        return false;
    }
    bounds.minimum = Some(snapped);
    bounds.exclusive_minimum = false;
    true
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "`&BoundInteger` is `&i64` without `arbitrary-precision` but `&BigInt` with it."
)]
fn snap_upper(bounds: &mut IntegerBounds, modulus: &BoundInteger) -> bool {
    let Some(original) = bounds.maximum.as_ref() else {
        return false;
    };
    // `effective_maximum` folds an exclusive bound to its inclusive neighbor; `None` (no representable neighbor) keeps it.
    let Some(effective) = bounds.effective_maximum() else {
        return false;
    };
    // Euclidean remainder so negatives round toward -inf.
    let remainder = effective.mod_floor(modulus);
    // The previous aligned multiple may underflow the representable range; keep the bound if so.
    let Some(snapped) = effective.checked_sub(&remainder) else {
        return false;
    };
    if snapped == *original && !bounds.exclusive_maximum {
        return false;
    }
    bounds.maximum = Some(snapped);
    bounds.exclusive_maximum = false;
    true
}

/// `Const(value)` when the bounds pin exactly one admissible value; otherwise `None`. Instances are exact
/// decimals here, so only an inclusive `[c, c]` singleton collapses, and the const keeps the exact bound.
#[cfg(feature = "arbitrary-precision")]
fn try_collapse_number_to_const(leaf: &NumberLeaf) -> Option<SharedSchema> {
    if leaf.bounds.exclusive_minimum || leaf.bounds.exclusive_maximum {
        return None;
    }
    let minimum = leaf.bounds.minimum.as_ref()?;
    if minimum != leaf.bounds.maximum.as_ref()? {
        return None;
    }
    if let Some(modulus) = leaf.multiple_of.as_ref() {
        if !fraction_is_multiple_of(minimum, modulus) {
            return None;
        }
    }
    // The single value is excluded if it is a multiple of any `not_multiple_of` entry.
    if leaf
        .not_multiple_of
        .iter()
        .any(|q| !q.numer().is_some_and(Zero::is_zero) && fraction_is_multiple_of(minimum, q))
    {
        return Some(shared(Schema::False));
    }
    Some(shared(Schema::Const(CanonicalJson::from_value(
        &minimum.to_json_value(),
    ))))
}

#[cfg(feature = "arbitrary-precision")]
fn fraction_is_multiple_of(value: &BoundFraction, modulus: &BoundFraction) -> bool {
    let modulus = modulus.abs();
    if modulus.is_zero() {
        return true;
    }
    (value / &modulus).denominator_is_one()
}

/// `Const(value)` when the f64-projected interval contains exactly one representable f64 satisfying `multipleOf`;
/// otherwise `None`. Instances are f64 in this build, so stepping exclusive bounds one ULP inward is exact.
#[cfg(not(feature = "arbitrary-precision"))]
fn try_collapse_number_to_const(leaf: &NumberLeaf) -> Option<SharedSchema> {
    let minimum = projected_lower(leaf.bounds.minimum.as_ref(), leaf.bounds.exclusive_minimum)?;
    let maximum = projected_upper(leaf.bounds.maximum.as_ref(), leaf.bounds.exclusive_maximum)?;
    if minimum.to_bits() != maximum.to_bits() {
        return None;
    }
    if let Some(modulus) = leaf.multiple_of.as_ref() {
        if !f64_is_multiple_of(minimum, modulus) {
            return None;
        }
    }
    // The single value is excluded if it is a multiple of any `not_multiple_of` entry.
    if leaf
        .not_multiple_of
        .iter()
        .any(|q| !q.numer().is_some_and(Zero::is_zero) && f64_is_multiple_of(minimum, q))
    {
        return Some(shared(Schema::False));
    }
    Some(shared(Schema::Const(CanonicalJson::from_value(
        &f64_to_json_value(minimum),
    ))))
}

#[cfg(not(feature = "arbitrary-precision"))]
fn projected_lower(value: Option<&BoundFraction>, exclusive: bool) -> Option<f64> {
    let raw = value?.to_f64()?;
    if !raw.is_finite() {
        return None;
    }
    Some(if exclusive { next_up_f64(raw) } else { raw })
}

#[cfg(not(feature = "arbitrary-precision"))]
fn projected_upper(value: Option<&BoundFraction>, exclusive: bool) -> Option<f64> {
    let raw = value?.to_f64()?;
    if !raw.is_finite() {
        return None;
    }
    Some(if exclusive { next_down_f64(raw) } else { raw })
}

// Hand-rolled `f64::next_up`/`next_down` (stable in 1.86; MSRV is 1.83). Drop once MSRV >= 1.86.
#[cfg(not(feature = "arbitrary-precision"))]
fn next_up_f64(value: f64) -> f64 {
    if value.is_nan() || value == f64::INFINITY {
        return value;
    }
    if value == 0.0 {
        return f64::from_bits(1);
    }
    let bits = value.to_bits();
    if value > 0.0 {
        f64::from_bits(bits + 1)
    } else {
        f64::from_bits(bits - 1)
    }
}

#[cfg(not(feature = "arbitrary-precision"))]
fn next_down_f64(value: f64) -> f64 {
    if value.is_nan() || value == f64::NEG_INFINITY {
        return value;
    }
    if value == 0.0 {
        return -f64::from_bits(1);
    }
    let bits = value.to_bits();
    if value > 0.0 {
        f64::from_bits(bits - 1)
    } else {
        f64::from_bits(bits + 1)
    }
}

#[cfg(not(feature = "arbitrary-precision"))]
fn f64_is_multiple_of(value: f64, modulus: &BoundFraction) -> bool {
    let modulus = modulus.abs();
    if modulus.is_zero() {
        return true;
    }
    let value_fraction = BoundFraction::from(value);
    let ratio = value_fraction / modulus;
    ratio.denominator_is_one()
}

/// Integer-valued f64s serialize as integers so they hash-eq with `{"const": 0}`; the rest use `serde_json`'s
/// shortest round-trip form.
#[cfg(not(feature = "arbitrary-precision"))]
fn f64_to_json_value(value: f64) -> Value {
    if value.is_finite() && value.fract() == 0.0 && i64_can_hold(value) {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "cast is exact: guarded by `i64_can_hold` and zero fractional part"
        )]
        let integer = value as i64;
        return Value::Number(Number::from(integer));
    }
    Number::from_f64(value).map_or(Value::Null, Value::Number)
}

#[cfg(not(feature = "arbitrary-precision"))]
fn i64_can_hold(value: f64) -> bool {
    // f64 -> i64 is exact only inside [-2^53, 2^53].
    let limit = 2_f64.powi(53);
    value >= -limit && value <= limit
}

/// Step between admissible integers when walking an endpoint inward: the `multipleOf` modulus (magnitude),
/// or `1` without one. `None` when the modulus is zero (no well-defined step).
fn exclusion_walk_step(leaf: &IntegerLeaf) -> Option<BoundInteger> {
    match leaf.multiple_of.as_ref() {
        Some(modulus) if modulus.is_zero() => None,
        Some(modulus) => Some(modulus.abs()),
        None => Some(BoundInteger::one()),
    }
}

/// Whether `value` is a multiple of any `not_multiple_of` entry, so the leaf excludes it.
fn is_excluded(value: &BoundInteger, not_multiple_of: &[BoundInteger]) -> bool {
    not_multiple_of
        .iter()
        .any(|excluded| excluded.divides(value))
}

fn trim_single_exclusion_endpoint(leaf: &IntegerLeaf) -> Option<SharedSchema> {
    let step = exclusion_walk_step(leaf)?;
    let excluded = |value: &BoundInteger| is_excluded(value, &leaf.not_multiple_of);
    let mut bounds = leaf.bounds.clone();
    let mut changed = false;
    if let Some(mut minimum) = leaf.bounds.effective_minimum() {
        for _ in 0..8 {
            if !excluded(&minimum) {
                break;
            }
            minimum = minimum.checked_add(&step)?;
            changed = true;
        }
        if excluded(&minimum) {
            return None;
        }
        if changed {
            bounds.minimum = Some(minimum);
            bounds.exclusive_minimum = false;
        }
    } else if let Some(mut maximum) = leaf.bounds.effective_maximum() {
        for _ in 0..8 {
            if !excluded(&maximum) {
                break;
            }
            maximum = maximum.checked_sub(&step)?;
            changed = true;
        }
        if excluded(&maximum) {
            return None;
        }
        if changed {
            bounds.maximum = Some(maximum);
            bounds.exclusive_maximum = false;
        }
    }
    if !changed {
        return None;
    }
    Some(shared(Schema::Integer(IntegerLeaf {
        bounds,
        multiple_of: (leaf.multiple_of.as_ref()).map(BoundInteger::owned),
        not_multiple_of: leaf.not_multiple_of.clone(),
    })))
}

fn trim_integer_exclusion_endpoints(leaf: &IntegerLeaf) -> Option<SharedSchema> {
    if leaf.not_multiple_of.is_empty() {
        return None;
    }
    // A one-sided leaf trims its present endpoint only (the entry-retention pass below needs the full window).
    let (Some(minimum), Some(maximum)) = (
        leaf.bounds.effective_minimum(),
        leaf.bounds.effective_maximum(),
    ) else {
        return trim_single_exclusion_endpoint(leaf);
    };
    let mut minimum = minimum;
    let mut maximum = maximum;
    if minimum > maximum {
        return None;
    }
    let step = exclusion_walk_step(leaf)?;
    let excluded = |value: &BoundInteger| is_excluded(value, &leaf.not_multiple_of);
    let mut changed = false;
    // Cap both walks: every step past an excluded endpoint shrinks the window by one candidate.
    for _ in 0..8 {
        if minimum > maximum || !excluded(&minimum) {
            break;
        }
        minimum = minimum.checked_add(&step)?;
        changed = true;
    }
    for _ in 0..8 {
        if minimum > maximum || !excluded(&maximum) {
            break;
        }
        maximum = maximum.checked_sub(&step)?;
        changed = true;
    }
    if minimum > maximum {
        // The window emptied out; the leaves stage proves emptiness from the crossed bounds.
        return Some(shared(Schema::Integer(IntegerLeaf {
            bounds: IntegerBounds {
                minimum: Some(minimum),
                maximum: Some(maximum),
                exclusive_minimum: false,
                exclusive_maximum: false,
            },
            multiple_of: (leaf.multiple_of.as_ref()).map(BoundInteger::owned),
            not_multiple_of: Vec::new(),
        })));
    }
    if excluded(&minimum) || excluded(&maximum) {
        // An endpoint is still excluded past the walk cap; a partial trim is not a unique spelling.
        return None;
    }
    let retained: Vec<BoundInteger> = leaf
        .not_multiple_of
        .iter()
        .filter(|q| {
            if q.is_zero() {
                return true;
            }
            match minimum.checked_next_multiple_of(&q.abs()) {
                Some(first_multiple) => first_multiple <= maximum,
                // Unrepresentable next multiple: keep the exclusion rather than guess.
                None => true,
            }
        })
        .map(BoundInteger::owned)
        .collect();
    if retained.len() != leaf.not_multiple_of.len() {
        changed = true;
    }
    if !changed {
        return None;
    }
    Some(shared(Schema::Integer(IntegerLeaf {
        bounds: IntegerBounds {
            minimum: Some(minimum),
            maximum: Some(maximum),
            exclusive_minimum: false,
            exclusive_maximum: false,
        },
        multiple_of: (leaf.multiple_of.as_ref()).map(BoundInteger::owned),
        not_multiple_of: retained,
    })))
}
