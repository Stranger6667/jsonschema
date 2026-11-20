// Draw arithmetic is approximate: `draw_valid_instance` checks every value against the validator,
// so a saturating or lossy cast only shifts which values get drawn, and float equality compares
// the written bounds.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::float_cmp
)]
// `next_up`/`next_down` postdate the MSRV; the MSRV job builds the library alone, never tests.
#![allow(clippy::incompatible_msrv)]

mod container;
mod fraction;
mod pool;
mod scalar;

use hegel::{generators as gs, generators::Generator, TestCase};
use jsonschema::{
    canonical::{CanonicalSchema, CanonicalView},
    JsonType,
};
use serde_json::{Number, Value};

// Each test target including this module uses its own subset of these.
#[allow(unused_imports)]
pub(crate) use pool::{
    aliased_number, arbitrary_instance, arbitrary_scalar, draw_keys, finite_float, small_int,
    wide_number,
};

// A node whose minimum size exceeds this draws nothing.
pub(crate) const MAX_SIZE: u64 = 8;

// Draws `draw_valid_instance` tries before giving up on the node.
pub(crate) const MAX_ATTEMPTS: usize = 8;

// A size floor no draw can reach reads as `u64::MAX`, which `MAX_SIZE` then declines.
pub(crate) fn size_floor(bound: Option<&Number>) -> u64 {
    bound.map_or(0, |bound| bound.as_u64().unwrap_or(u64::MAX))
}

// A size ceiling past `u64` bounds nothing this generator builds.
pub(crate) fn size_ceiling(bound: Option<&Number>) -> u64 {
    bound.map_or(u64::MAX, |bound| bound.as_u64().unwrap_or(u64::MAX))
}

fn value_has_type(value: &Value, ty: JsonType) -> bool {
    match ty {
        JsonType::Null => value.is_null(),
        JsonType::Boolean => value.is_boolean(),
        JsonType::Integer => value.is_i64() || value.is_u64(),
        JsonType::Number => value.is_number(),
        JsonType::String => value.is_string(),
        JsonType::Array => value.is_array(),
        JsonType::Object => value.is_object(),
    }
}

/// One position in the recursive draw: the engine handle, the document root `#` resolves to,
/// and the remaining recursion depth.
#[derive(Clone, Copy)]
pub(crate) struct Sampler<'a> {
    pub(crate) tc: &'a TestCase,
    pub(crate) root: &'a CanonicalSchema,
    pub(crate) depth: u8,
}

impl Sampler<'_> {
    /// Draw a child of the current position: one level deeper, declining when depth runs out.
    pub(crate) fn descend(&self, schema: &CanonicalSchema) -> Option<Value> {
        Sampler {
            depth: self.depth.saturating_sub(1),
            ..*self
        }
        .draw(schema)
    }

    // A value aimed at what the node accepts, or `None` where no exact construction exists.
    // Engine-divergent patterns, `not` and `oneOf` exclusivity are left to the validator check in
    // `draw_valid_instance`; no draw aims outside the node.
    pub(crate) fn draw(&self, schema: &CanonicalSchema) -> Option<Value> {
        if self.depth == 0 {
            return None;
        }
        match schema.view() {
            CanonicalView::False => None,
            // For `Raw` or `not`, the validator check decides what the candidate is worth.
            CanonicalView::True | CanonicalView::Raw(_) | CanonicalView::Not(_) => {
                Some(self.tc.draw(arbitrary_instance()))
            }
            CanonicalView::Const(value) => Some(value),
            CanonicalView::Enum(values) => {
                let index = self.tc.draw(
                    gs::integers::<usize>()
                        .min_value(0)
                        .max_value(values.len() - 1),
                );
                values.into_iter().nth(index)
            }
            CanonicalView::MultiType(set) => {
                let types: Vec<JsonType> = set.iter().collect();
                let ty = self.tc.draw(gs::sampled_from(types).print_as_debug());
                Some(pool::draw_unconstrained(self.tc, ty))
            }
            CanonicalView::TypedGroup(group) => {
                let value = self.descend(&group.body)?;
                value_has_type(&value, group.ty).then_some(value)
            }
            CanonicalView::String(view) => scalar::draw_string(self.tc, &view),
            CanonicalView::Integer(view) => scalar::draw_integer(self.tc, &view),
            CanonicalView::Number(view) => scalar::draw_number(self.tc, &view),
            CanonicalView::Array(view) => container::draw_array(self, view),
            CanonicalView::Object(view) => container::draw_object(self, schema.draft(), &view),
            CanonicalView::AllOf(branches) => {
                // Following each pointer one hop often lets `intersect` fold the `allOf`
                // into one node.
                let mut resolved = Vec::new();
                for branch in &branches {
                    match branch.view() {
                        CanonicalView::Reference(uri) => {
                            let target = if uri == "#" {
                                self.root.clone()
                            } else {
                                branch.definition(&uri)?
                            };
                            resolved.push(target);
                        }
                        _ => resolved.push(branch.clone()),
                    }
                }
                let mut folded = resolved.first()?.clone();
                let mut fold_failed = false;
                for other in &resolved[1..] {
                    if let Ok(next) = folded.intersect(other) {
                        folded = next;
                    } else {
                        fold_failed = true;
                        break;
                    }
                }
                if !fold_failed && !matches!(folded.view(), CanonicalView::AllOf(_)) {
                    return self.descend(&folded);
                }
                let index = self.tc.draw(
                    gs::integers::<usize>()
                        .min_value(0)
                        .max_value(branches.len() - 1),
                );
                self.descend(&branches[index])
            }
            CanonicalView::AnyOf(branches) | CanonicalView::OneOf(branches) => {
                let index = self.tc.draw(
                    gs::integers::<usize>()
                        .min_value(0)
                        .max_value(branches.len() - 1),
                );
                self.descend(&branches[index])
            }
            CanonicalView::Reference(uri) => {
                let target = if uri == "#" {
                    Some(self.root.clone())
                } else {
                    schema.definition(&uri)
                };
                self.descend(&target?)
            }
        }
    }
}

// Returns only values `validator` accepts: draws aim at the schema where they can, and the
// validator rejects the rest of a draw's attempts.
pub(crate) fn draw_valid_instance(
    tc: &TestCase,
    canonical: &CanonicalSchema,
    validator: &jsonschema::Validator,
) -> Option<Value> {
    let sampler = Sampler {
        tc,
        root: canonical,
        depth: 5,
    };
    for _ in 0..MAX_ATTEMPTS {
        let Some(candidate) = sampler.draw(canonical) else {
            continue;
        };
        if validator.is_valid(&candidate) {
            return Some(candidate);
        }
    }
    None
}
