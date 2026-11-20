//! What `unevaluatedProperties` and `unevaluatedItems` share: a subschema that fails evaluates
//! nothing.
use serde_json::{Map, Value};

use crate::{
    compiler,
    node::SchemaNode,
    validator::{Validate, ValidationContext},
    Json, ValidationError,
};

use super::{ref_, BuiltinKeyword};

/// Which answer the marks feed. A failing `allOf` branch, `$ref` or taken `then`/`else` evaluates
/// nothing but fails the schema, so the true/false answer may count it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Marking {
    /// The true/false answer: every subschema counts.
    Verdict,
    /// Members as errors and `evaluate()` report them: a subschema counts only where it validates.
    Report,
}

/// Members a subschema evaluates, and what must validate for them to count when reporting.
pub(crate) struct Checked<F: Json, V> {
    /// `None` where nothing compiles to check, as for a reference to the schema it sits in.
    check: Option<SchemaNode<F>>,
    pub(crate) validators: V,
}

impl<F: Json, V: Clone> Clone for Checked<F, V> {
    fn clone(&self) -> Self {
        Checked {
            check: self.check.clone(),
            validators: self.validators.clone(),
        }
    }
}

impl<F: Json, V> Checked<F, V> {
    pub(crate) fn new(check: Option<SchemaNode<F>>, validators: V) -> Self {
        Checked { check, validators }
    }

    pub(crate) fn check(&self) -> Option<&SchemaNode<F>> {
        self.check.as_ref()
    }

    /// Whether the members count for `instance`.
    #[inline]
    pub(crate) fn counts(
        &self,
        instance: &F::Node<'_>,
        ctx: &mut ValidationContext,
        marking: Marking,
    ) -> bool {
        marking == Marking::Verdict
            || self
                .check
                .as_ref()
                .is_none_or(|node| node.is_valid(instance, ctx))
    }
}

/// The reference keyword `keyword` of `parent` alone, which validates its target.
pub(crate) fn reference<'a, F: Json>(
    ctx: &compiler::Context<'_, F>,
    parent: &'a Map<String, Value>,
    keyword: &str,
) -> Result<Option<SchemaNode<F>>, ValidationError<'a>> {
    let Some(value) = parent.get(keyword) else {
        return Ok(None);
    };
    let (builtin, compiled) = match keyword {
        "$recursiveRef" => (
            BuiltinKeyword::RecursiveRef,
            ref_::compile_recursive_ref(ctx, parent, value),
        ),
        "$dynamicRef" => (
            BuiltinKeyword::DynamicRef,
            ref_::compile_dynamic_ref(ctx, parent, value),
        ),
        _ => (BuiltinKeyword::Ref, ref_::compile_ref(ctx, parent, value)),
    };
    let Some(validator) = compiled.transpose()? else {
        return Ok(None);
    };
    Ok(Some(SchemaNode::from_keywords(
        ctx,
        vec![(builtin.into(), validator)],
        None,
        None,
    )))
}

/// The subschema `value` at `ctx`.
pub(crate) fn subschema<'a, F: Json>(
    ctx: &compiler::Context<'_, F>,
    value: &'a Value,
) -> Result<SchemaNode<F>, ValidationError<'a>> {
    compiler::compile(ctx, ctx.as_resource_ref(value))
}
