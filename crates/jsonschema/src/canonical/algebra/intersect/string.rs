use std::sync::Arc;

use ahash::AHashSet;

use crate::canonical::{
    context::CanonicalizationContext,
    coverage,
    ir::{Schema, StringLeaf},
    leaves::{Intersection, Leaf, Membership, TypedLeaf, Verdict},
    prover::Prover,
};

use super::{max_option, min_option, normalize_string_not_patterns};

impl Leaf for StringLeaf {
    /// Lengths tighten, `pattern`/not-pattern/`contentEncoding` lists union, and a lone `format`
    /// carries through. `Residual` when both sides carry differing, non-disjoint formats.
    ///
    /// ```text
    /// BEFORE: {"type": "string", "minLength": 2}  and  {"type": "string", "maxLength": 8}
    /// AFTER:  {"type": "string", "minLength": 2, "maxLength": 8}
    ///
    /// BEFORE: {"type": "string", "format": "email"}  and  {"type": "string", "maxLength": 40}
    /// AFTER:  {"type": "string", "format": "email", "maxLength": 40}
    /// ```
    fn intersect(&self, other: &Self, ctx: &CanonicalizationContext) -> Intersection<Self> {
        let min_length = max_option(self.min_length.as_ref(), other.min_length.as_ref());
        let max_length = min_option(self.max_length.as_ref(), other.max_length.as_ref());
        if let (Some(minimum), Some(maximum)) = (&min_length, &max_length) {
            if minimum > maximum {
                return Intersection::Empty;
            }
        }

        let format = match (&self.format, &other.format) {
            (Some(left_format), Some(right_format)) => {
                if left_format == right_format {
                    Some(Arc::clone(left_format))
                } else if formats_are_known_disjoint(left_format, right_format, ctx) {
                    return Intersection::Empty;
                } else {
                    return Intersection::Residual;
                }
            }
            (Some(value), None) | (None, Some(value)) => Some(Arc::clone(value)),
            (None, None) => None,
        };

        let patterns = union_sorted(&self.patterns, &other.patterns);
        let not_patterns = union_sorted(&self.not_patterns, &other.not_patterns);
        let content = union_sorted(&self.content, &other.content);

        let leaf = StringLeaf {
            min_length,
            max_length,
            patterns,
            not_patterns,
            format,
            content,
        };
        match normalize_string_not_patterns(leaf) {
            Some(leaf) => Intersection::Merged(leaf),
            None => Intersection::Empty,
        }
    }

    fn covers(&self, other: &Self, prover: &Prover<'_>) -> Verdict {
        if self == other {
            return Verdict::Proven;
        }
        Verdict::proven_if(coverage::string_leaf_covers(self, other, prover.ctx()))
    }

    /// Regex relations, asserted formats, and content-decoding interactions are beyond the
    /// pipeline's emptiness reasoning.
    fn inhabited(&self, formats_asserted: bool) -> Verdict {
        Verdict::proven_if(
            self.patterns.is_empty()
                && self.not_patterns.is_empty()
                && (self.format.is_none() || !formats_asserted)
                && self.content.is_empty(),
        )
    }

    fn is_open(&self) -> bool {
        self.min_length.is_none()
            && self.max_length.is_none()
            && self.patterns.is_empty()
            && self.not_patterns.is_empty()
            && self.format.is_none()
            && self.content.is_empty()
    }

    fn is_empty(&self, _ctx: &CanonicalizationContext) -> bool {
        matches!((&self.min_length, &self.max_length), (Some(minimum), Some(maximum)) if minimum > maximum)
    }
}

impl TypedLeaf for StringLeaf {
    fn wrap(self) -> Schema {
        Schema::String(self)
    }
    fn project(schema: &Schema) -> Option<&Self> {
        match schema {
            Schema::String(leaf) => Some(leaf),
            _ => None,
        }
    }
}

/// Two different `format`s the pipeline can't prove disjoint can't collapse into one string leaf, so the caller keeps
/// them as a residual `AllOf` for the validator to enforce both.
///
/// ```text
/// {"type": "string", "format": "email"}  and  {"type": "string", "format": "uri"}
/// => both kept in an `AllOf`: neither format subsumes the other and disjointness is unknown
/// ```
pub(crate) fn is_unmergeable_format_pair(
    left: &StringLeaf,
    right: &StringLeaf,
    ctx: &CanonicalizationContext,
) -> bool {
    matches!(
        (&left.format, &right.format),
        (Some(left_format), Some(right_format))
            if left_format != right_format && !formats_are_known_disjoint(left_format, right_format, ctx)
    )
}

/// Whether two distinct `format`s can never be satisfied by one string, consulted only when formats are asserted
/// (otherwise `format` is an annotation and never makes a schema empty).
///
/// ```text
/// // with format assertions enabled:
/// BEFORE: {"type": "string", "format": "date"}  and  {"type": "string", "format": "email"}
/// AFTER:  false                                 // no string is both a date and an email address
/// ```
///
/// The "rigid" formats in [`is_rigid_format`] have pairwise-disjoint value sets — proven from the validators in
/// `keywords::format` by a required/forbidden-character argument:
///
/// | format      | every value REQUIRES | every value FORBIDS (relevant chars)        |
/// |-------------|----------------------|---------------------------------------------|
/// | `date`      | `-`, 10 chars        | `@ : . / T P` and all letters               |
/// | `time`      | `:`, trailing `Z/z/+/-` | `@ . / P T`, letters except `Z/z`        |
/// | `date-time` | `T/t`, `:`, `-`      | `@ / P`                                     |
/// | `duration`  | leading `P`          | `@ : . - /` and lowercase letters           |
/// | `email`     | `@`                  |                                             |
/// | `idn-email` | `@`                  |                                             |
/// | `uuid`      | `-`, 36 hex chars    | `@ : . / P T` and non-hex letters           |
/// | `ipv4`      | `.`                  | `@ : - /` and all letters                   |
/// | `ipv6`      | `:`                  | `@ - /`, non-hex letters                    |
///
/// Every distinct pair has one side requiring a char the other forbids, so the intersection is empty. Exception:
/// `email`/`idn-email` overlap (every ASCII email is a valid `idn-email`) and are not reported disjoint.
fn formats_are_known_disjoint(
    left: &Arc<str>,
    right: &Arc<str>,
    ctx: &CanonicalizationContext,
) -> bool {
    if !ctx.validates_formats() {
        return false;
    }
    // `left != right` is guaranteed by the caller.
    if matches!(sorted_pair(left, right), ("email", "idn-email")) {
        return false;
    }
    is_rigid_format(left, ctx) && is_rigid_format(right, ctx)
}

/// A `format` with a fixed value set disjoint from the other rigid formats (see [`formats_are_known_disjoint`]).
/// Rigid only when the active draft asserts it - an unasserted `format` is an annotation that never empties an intersection.
fn is_rigid_format(format: &str, ctx: &CanonicalizationContext) -> bool {
    match format {
        "date" | "date-time" | "time" | "email" | "idn-email" | "ipv4" | "ipv6" | "duration"
        | "uuid" => crate::keywords::format::is_known_format(ctx.draft(), format),
        _ => false,
    }
}

fn sorted_pair<'a>(left: &'a str, right: &'a str) -> (&'a str, &'a str) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

pub(super) fn value_matches_string(
    text: &str,
    leaf: &StringLeaf,
    ctx: &CanonicalizationContext,
) -> Membership {
    let char_count = bytecount::num_chars(text.as_bytes()) as u64;
    if let Some(min) = &leaf.min_length {
        if char_count < *min {
            return Membership::No;
        }
    }
    if let Some(max) = &leaf.max_length {
        if char_count > *max {
            return Membership::No;
        }
    }
    // Extended regex (lookaround, backreferences) defers to the validator via the `AllOf` fallback.
    for pattern in &leaf.patterns {
        let Some(regex) = ctx.compile_regex(pattern) else {
            return Membership::Unknown;
        };
        if !regex.is_match(text) {
            return Membership::No;
        }
    }
    for pattern in &leaf.not_patterns {
        let Some(regex) = ctx.compile_regex(pattern) else {
            return Membership::Unknown;
        };
        if regex.is_match(text) {
            return Membership::No;
        }
    }
    if leaf.format.is_some() || !leaf.content.is_empty() {
        return Membership::Unknown;
    }
    Membership::Yes
}

/// Deduplicated, sorted union of two slices.
fn union_sorted<T: Clone + Eq + std::hash::Hash + Ord>(left: &[T], right: &[T]) -> Vec<T> {
    let mut out: Vec<T> = left
        .iter()
        .chain(right.iter())
        .cloned()
        .collect::<AHashSet<_>>()
        .into_iter()
        .collect();
    out.sort_unstable();
    out
}
