//! Uniform child-rewriting walker shared by every canonicalize stage.
//!
//! Preserves Arc identity when no child changes - the fixed-point loop relies on this contract.

use std::sync::Arc;

use crate::canonical::{
    context::{CanonicalizationContext, WalkStage},
    intern::shared,
    ir::{
        ArrayLeaf, BoundCardinality, ContainsClause, IfThenElse, ObjectConstraint, ObjectLeaf,
        ObjectRequirement, OneOf, Schema, SharedSchema,
    },
};

/// Bottom-up hash-consing: children first, then the node itself, memoized per subtree.
pub(crate) fn intern_tree(schema: &SharedSchema, ctx: &CanonicalizationContext) -> SharedSchema {
    ctx.with_walk_memo(WalkStage::Intern, schema, || {
        ctx.intern(map_children(schema, |child| intern_tree(child, ctx)))
    })
}

/// Whether an optional child changed: present-and-repointed, or appeared/disappeared.
fn opt_changed(new: Option<&SharedSchema>, prev: Option<&SharedSchema>) -> bool {
    match (new, prev) {
        (Some(next), Some(prev)) => !Arc::ptr_eq(next, prev),
        (None, None) => false,
        _ => true,
    }
}

/// Apply `transform` to every direct child, preserving Arc identity when nothing changed.
pub(crate) fn map_children<F>(schema: &SharedSchema, transform: F) -> SharedSchema
where
    F: Fn(&SharedSchema) -> SharedSchema,
{
    match schema.as_schema() {
        Schema::Null
        | Schema::Boolean(_)
        | Schema::Integer(_)
        | Schema::Number(_)
        | Schema::String(_)
        | Schema::Const(_)
        | Schema::Enum(_)
        | Schema::True
        | Schema::False
        | Schema::Reference(_)
        | Schema::Recursive(_)
        | Schema::DynamicRef(_)
        | Schema::Raw(_)
        | Schema::MultiType(_) => Arc::clone(schema),
        Schema::AllOf(branches) => match map_branches(branches, &transform) {
            None => Arc::clone(schema),
            Some(next) => shared(Schema::AllOf(next)),
        },
        Schema::AnyOf(branches) => match map_branches(branches, &transform) {
            None => Arc::clone(schema),
            Some(next) => shared(Schema::AnyOf(next)),
        },
        Schema::OneOf(OneOf(branches)) => match map_branches(branches, &transform) {
            None => Arc::clone(schema),
            Some(next) => shared(Schema::OneOf(OneOf(next))),
        },
        Schema::Not(inner) => {
            let next = transform(inner);
            if Arc::ptr_eq(&next, inner) {
                Arc::clone(schema)
            } else {
                shared(Schema::Not(next))
            }
        }
        Schema::IfThenElse(IfThenElse {
            condition,
            then_branch,
            else_branch,
        }) => {
            let new_condition = transform(condition);
            let new_then = then_branch.as_ref().map(&transform);
            let new_else = else_branch.as_ref().map(&transform);
            let condition_changed = !Arc::ptr_eq(&new_condition, condition);
            let then_changed = opt_changed(new_then.as_ref(), then_branch.as_ref());
            let else_changed = opt_changed(new_else.as_ref(), else_branch.as_ref());
            if !condition_changed && !then_changed && !else_changed {
                Arc::clone(schema)
            } else {
                shared(Schema::IfThenElse(IfThenElse {
                    condition: new_condition,
                    then_branch: new_then,
                    else_branch: new_else,
                }))
            }
        }
        Schema::TypedGroup { ty, body } => {
            let next = transform(body);
            if Arc::ptr_eq(&next, body) {
                Arc::clone(schema)
            } else {
                shared(Schema::TypedGroup {
                    ty: *ty,
                    body: next,
                })
            }
        }
        Schema::TypeGuard { ty, body } => {
            let next = transform(body);
            if Arc::ptr_eq(&next, body) {
                Arc::clone(schema)
            } else {
                shared(Schema::TypeGuard {
                    ty: *ty,
                    body: next,
                })
            }
        }
        Schema::Array(leaf) => map_array_leaf(schema, leaf, &transform),
        Schema::Object(leaf) => map_object_leaf(schema, leaf, &transform),
    }
}

/// Walk `items`, transforming each item's schema child via `transform`; returns `None` when every child keeps its
/// Arc identity. Items without a schema child (`child_of` returns `None`) are cloned through unchanged.
fn map_lazy<T, F, R, C>(items: &[T], child_of: C, rebuild: R, transform: &F) -> Option<Vec<T>>
where
    T: Clone,
    F: Fn(&SharedSchema) -> SharedSchema,
    R: Fn(SharedSchema, &T) -> T,
    C: Fn(&T) -> Option<&SharedSchema>,
{
    let mut new: Option<Vec<T>> = None;
    for (index, item) in items.iter().enumerate() {
        match (child_of(item), new.as_mut()) {
            (Some(child), Some(buffer)) => {
                buffer.push(rebuild(transform(child), item));
            }
            (None, Some(buffer)) => buffer.push(item.clone()),
            (Some(child), None) => {
                let next = transform(child);
                if Arc::ptr_eq(&next, child) {
                    continue;
                }
                let mut buffer = Vec::with_capacity(items.len());
                buffer.extend(items[..index].iter().cloned());
                buffer.push(rebuild(next, item));
                new = Some(buffer);
            }
            (None, None) => {}
        }
    }
    new
}

fn map_branches<F>(branches: &[SharedSchema], transform: &F) -> Option<Vec<SharedSchema>>
where
    F: Fn(&SharedSchema) -> SharedSchema,
{
    map_lazy(
        branches,
        |branch| Some(branch),
        |new_child, _| new_child,
        transform,
    )
}

fn map_array_leaf<F>(schema: &SharedSchema, leaf: &ArrayLeaf, transform: &F) -> SharedSchema
where
    F: Fn(&SharedSchema) -> SharedSchema,
{
    let new_prefix = map_branches(&leaf.prefix, transform);
    let new_tail = transform(&leaf.tail);
    let new_contains = map_contains(&leaf.contains, transform);

    let tail_changed = !Arc::ptr_eq(&new_tail, &leaf.tail);
    if new_prefix.is_none() && !tail_changed && new_contains.is_none() {
        return Arc::clone(schema);
    }
    shared(Schema::Array(ArrayLeaf {
        prefix: new_prefix.unwrap_or_else(|| leaf.prefix.clone()),
        tail: new_tail,
        length: leaf.length.clone(),
        unique_items: leaf.unique_items,
        repeated_items: leaf.repeated_items,
        contains: new_contains.unwrap_or_else(|| leaf.contains.clone()),
    }))
}

fn map_contains<F>(contains: &[ContainsClause], transform: &F) -> Option<Vec<ContainsClause>>
where
    F: Fn(&SharedSchema) -> SharedSchema,
{
    map_lazy(
        contains,
        |clause| Some(&clause.schema),
        |schema, clause| ContainsClause {
            schema,
            min_contains: clause.min_contains.owned(),
            max_contains: (clause.max_contains.as_ref()).map(BoundCardinality::owned),
        },
        transform,
    )
}

fn map_object_leaf<F>(schema: &SharedSchema, leaf: &ObjectLeaf, transform: &F) -> SharedSchema
where
    F: Fn(&SharedSchema) -> SharedSchema,
{
    let new_requirements = map_requirements(&leaf.requirements, transform);
    let new_constraints = map_constraints(&leaf.constraints, transform);
    let new_property_names = leaf.property_names.as_ref().map(transform);
    let property_names_changed =
        opt_changed(new_property_names.as_ref(), leaf.property_names.as_ref());

    if new_requirements.is_none() && new_constraints.is_none() && !property_names_changed {
        return Arc::clone(schema);
    }
    shared(Schema::Object(ObjectLeaf {
        requirements: new_requirements.unwrap_or_else(|| leaf.requirements.clone()),
        constraints: new_constraints.unwrap_or_else(|| leaf.constraints.clone()),
        property_names: new_property_names,
    }))
}

fn map_requirements<F>(
    requirements: &[ObjectRequirement],
    transform: &F,
) -> Option<Vec<ObjectRequirement>>
where
    F: Fn(&SharedSchema) -> SharedSchema,
{
    map_lazy(
        requirements,
        requirement_child,
        rebuild_requirement,
        transform,
    )
}

fn requirement_child(requirement: &ObjectRequirement) -> Option<&SharedSchema> {
    match requirement {
        ObjectRequirement::DependentSchemaRequirement { schema, .. }
        | ObjectRequirement::PatternPropertyRequirement { schema, .. } => Some(schema),
        _ => None,
    }
}

fn rebuild_requirement(new_child: SharedSchema, source: &ObjectRequirement) -> ObjectRequirement {
    match source {
        ObjectRequirement::DependentSchemaRequirement { property, .. } => {
            ObjectRequirement::DependentSchemaRequirement {
                property: Arc::clone(property),
                schema: new_child,
            }
        }
        ObjectRequirement::PatternPropertyRequirement { matcher, .. } => {
            ObjectRequirement::PatternPropertyRequirement {
                matcher: matcher.clone(),
                schema: new_child,
            }
        }
        _ => unreachable!("rebuild_requirement called on a variant without a schema child"),
    }
}

fn map_constraints<F>(
    constraints: &[ObjectConstraint],
    transform: &F,
) -> Option<Vec<ObjectConstraint>>
where
    F: Fn(&SharedSchema) -> SharedSchema,
{
    map_lazy(
        constraints,
        |constraint| Some(&constraint.schema),
        |schema, constraint| ObjectConstraint {
            matcher: constraint.matcher.clone(),
            schema,
        },
        transform,
    )
}
