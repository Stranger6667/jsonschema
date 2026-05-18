//! `if`/`then`/`else` rewrite to `AnyOf` / `AllOf` / `Not`.
//!
//! `if c then t else e` desugars to `(c and t) or ((not c) and e)`; missing branches are `True`.

use std::sync::Arc;

use crate::canonical::{
    context::{CanonicalizationContext, WalkStage},
    coverage::covers,
    intern::{allof_pair, shared},
    ir::{CanonicalKind, IfThenElse, Schema, SchemaKindSet, SharedSchema},
    walk::map_children,
};

/// Desugars `if`/`then`/`else` into a guarded union. A missing `then`/`else` defaults to `true`, and the whole node
/// drops to `true` when both branches are vacuous (e.g. the `then` branch already covers the condition).
///
/// ```text
/// BEFORE: {"if": {"type": "integer"}, "then": {"minimum": 0},
///          "else": {"type": "string"}}
/// AFTER:  {"anyOf": [
///           {"allOf": [{"type": "integer"}, {"minimum": 0}]},
///           {"allOf": [{"not": {"type": "integer"}}, {"type": "string"}]}
///         ]}
/// ```
#[must_use]
pub(crate) fn canonicalize(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
    super::run::<IfThenElseStage>(schema, ctx)
}

struct IfThenElseStage;

impl super::StructuralStage for IfThenElseStage {
    const WALK: WalkStage = WalkStage::IfThenElse;
    const MASK: SchemaKindSet = SchemaKindSet::of(CanonicalKind::IfThenElse);

    fn rewrite(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
        canonicalize_impl(schema, ctx)
    }
}

fn canonicalize_impl(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
    if let Schema::IfThenElse(IfThenElse {
        condition,
        then_branch,
        else_branch,
    }) = schema.as_schema()
    {
        let condition = canonicalize(condition, ctx);
        let then_branch = then_branch.as_ref().map(|b| canonicalize(b, ctx));
        let else_branch = else_branch.as_ref().map(|b| canonicalize(b, ctx));
        if is_vacuous(&condition, then_branch.as_ref(), else_branch.as_ref(), ctx) {
            return shared(Schema::True);
        }
        let then_branch = then_branch.unwrap_or_else(|| shared(Schema::True));
        let else_branch = else_branch.unwrap_or_else(|| shared(Schema::True));
        let then_clause = allof_pair(&condition, &then_branch);
        let else_clause = allof_pair(&shared(Schema::Not(Arc::clone(&condition))), &else_branch);
        return shared(Schema::AnyOf(vec![then_clause, else_clause]));
    }
    map_children(schema, |child| canonicalize(child, ctx))
}

fn is_vacuous(
    condition: &SharedSchema,
    then_branch: Option<&SharedSchema>,
    else_branch: Option<&SharedSchema>,
    ctx: &CanonicalizationContext,
) -> bool {
    let then_true = then_branch.is_none_or(|value| matches!(value.as_schema(), Schema::True));
    let else_true = else_branch.is_none_or(|value| matches!(value.as_schema(), Schema::True));
    if then_true && else_true {
        return true;
    }
    if let Some(then_branch) = then_branch {
        if else_true && (then_branch == condition || covers(then_branch, condition, ctx)) {
            return true;
        }
    }
    false
}
