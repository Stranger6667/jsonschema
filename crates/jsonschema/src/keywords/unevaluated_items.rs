//! `unevaluatedItems`. Compilation builds an `ItemsValidators` tree; reference cycles share one
//! `Arc<OnceLock>` cell.
use referencing::{Draft, Vocabulary};
use serde_json::{Map, Value};
use std::{
    fmt,
    sync::{Arc, OnceLock},
};

use crate::{
    compiler,
    evaluation::{ChildList, ErrorDescription},
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    validator::{EvaluationResult, Validate, ValidationContext},
    Array, Json, Node, SerdeJson, ValidationError,
};

use super::{
    unevaluated::{self, Checked, Marking},
    CompilationResult,
};

/// The cell reference cycles share while their target compiles.
pub(crate) type PendingItemsValidators<F = SerdeJson> = Arc<OnceLock<ItemsValidators<F>>>;

/// Holds compiled validators for items evaluation in unevaluatedItems.
pub(crate) struct ItemsValidators<F: Json = SerdeJson> {
    unevaluated: Option<SchemaNode<F>>,
    contains: Option<SchemaNode<F>>,
    ref_: Option<Checked<F, PendingItemsValidators<F>>>,
    dynamic_ref: Option<Checked<F, PendingItemsValidators<F>>>,
    recursive_ref: Option<Checked<F, PendingItemsValidators<F>>>,
    /// Drafts up to 2019-09: `items` marks this many leading items.
    items_limit: Option<usize>,
    /// Items schema present - for Draft 2020-12+ "items" keyword
    /// If true, marks ALL items as evaluated
    items_all: bool,
    prefix_items: Option<usize>,
    conditional: Option<Box<ConditionalValidators<F>>>,
    all_of: Option<Vec<(SchemaNode<F>, ItemsValidators<F>)>>,
    any_of: Option<Vec<(SchemaNode<F>, ItemsValidators<F>)>>,
    one_of: Option<Vec<(SchemaNode<F>, ItemsValidators<F>)>>,
}

// Manual impls: derives would require `F: Clone` / `F: Debug` even though `F` is a marker type.
impl<F: Json> Clone for ItemsValidators<F> {
    fn clone(&self) -> Self {
        ItemsValidators {
            unevaluated: self.unevaluated.clone(),
            contains: self.contains.clone(),
            ref_: self.ref_.clone(),
            dynamic_ref: self.dynamic_ref.clone(),
            recursive_ref: self.recursive_ref.clone(),
            items_limit: self.items_limit,
            items_all: self.items_all,
            prefix_items: self.prefix_items,
            conditional: self.conditional.clone(),
            all_of: self.all_of.clone(),
            any_of: self.any_of.clone(),
            one_of: self.one_of.clone(),
        }
    }
}

impl<F: Json> fmt::Debug for ItemsValidators<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ItemsValidators").finish_non_exhaustive()
    }
}

/// Conditional validators from "if/then/else" keywords
struct ConditionalValidators<F: Json = SerdeJson> {
    condition: SchemaNode<F>,
    /// `None` for a boolean `if`, which evaluates nothing.
    if_: Option<ItemsValidators<F>>,
    then_: Option<Checked<F, ItemsValidators<F>>>,
    else_: Option<Checked<F, ItemsValidators<F>>>,
}

impl<F: Json> Clone for ConditionalValidators<F> {
    fn clone(&self) -> Self {
        ConditionalValidators {
            condition: self.condition.clone(),
            if_: self.if_.clone(),
            then_: self.then_.clone(),
            else_: self.else_.clone(),
        }
    }
}

impl<F: Json> fmt::Debug for ConditionalValidators<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConditionalValidators")
            .finish_non_exhaustive()
    }
}

impl<F: Json> ItemsValidators<F> {
    /// Core implementation for marking evaluated indexes.
    ///
    /// When `include_unevaluated` is `true` (used by `is_valid`/`validate`), also marks
    /// items validated by `unevaluatedItems` itself — needed so nested schemas can propagate
    /// evaluations upward. When `false` (used by `evaluate`), those items are left unmarked
    /// so `evaluate_instance()` is called on them to collect annotations.
    fn mark_evaluated_indexes_impl(
        &self,
        instance: &F::Node<'_>,
        indexes: &mut Vec<bool>,
        ctx: &mut ValidationContext,
        include_unevaluated: bool,
        marking: Marking,
    ) {
        // Break cycles from self-referential `$dynamicRef`/`$recursiveRef` under
        // `unevaluatedItems`.
        let validators_id = std::ptr::from_ref::<ItemsValidators<F>>(self) as usize;
        let identity = instance.identity();
        if ctx.enter_marking(validators_id, identity) {
            return;
        }
        self.mark_evaluated_indexes_inner(instance, indexes, ctx, include_unevaluated, marking);
        if identity.is_some() {
            ctx.exit_marking();
        }
    }

    fn mark_evaluated_indexes_inner(
        &self,
        instance: &F::Node<'_>,
        indexes: &mut Vec<bool>,
        ctx: &mut ValidationContext,
        include_unevaluated: bool,
        marking: Marking,
    ) {
        // Early return optimization: if items marks ALL items, no need to check anything else
        if self.items_all {
            // Draft 2020-12+: items keyword marks ALL items as evaluated
            for idx in indexes.iter_mut() {
                *idx = true;
            }
            return;
        }

        for reference in [&self.ref_, &self.recursive_ref, &self.dynamic_ref]
            .into_iter()
            .flatten()
        {
            if reference.counts(instance, ctx, marking) {
                initialized(&reference.validators).mark_evaluated_indexes(
                    instance,
                    indexes,
                    ctx,
                    Marking::Verdict,
                );
            }
        }

        if let Some(limit) = self.items_limit {
            for idx in indexes.iter_mut().take(limit) {
                *idx = true;
            }
        }

        if let Some(limit) = self.prefix_items {
            for idx in indexes.iter_mut().take(limit) {
                *idx = true;
            }
        }

        if indexes.iter().all(|&evaluated| evaluated) {
            return;
        }

        if let Some(array) = instance.as_array() {
            for (item, is_evaluated) in array.elements().zip(indexes.iter_mut()) {
                if *is_evaluated {
                    continue;
                }
                if let Some(validator) = &self.contains {
                    if validator.is_valid(&item, ctx) {
                        *is_evaluated = true;
                        continue;
                    }
                }
                // unevaluatedItems itself can mark items.
                // Skipped when called from evaluate() so evaluate_instance() can collect annotations.
                if include_unevaluated {
                    if let Some(validator) = &self.unevaluated {
                        if validator.is_valid(&item, ctx) {
                            *is_evaluated = true;
                        }
                    }
                }
            }
        }

        if let Some(conditional) = &self.conditional {
            conditional.mark_evaluated_indexes(instance, indexes, ctx, marking);
        }

        // Handle allOf - each schema that validates successfully marks items
        if let Some(all_of) = &self.all_of {
            for (validator, item_validators) in all_of {
                if validator.is_valid(instance, ctx) {
                    item_validators.mark_evaluated_indexes(
                        instance,
                        indexes,
                        ctx,
                        Marking::Verdict,
                    );
                }
            }
        }

        // Handle anyOf - each schema that validates successfully marks items
        if let Some(any_of) = &self.any_of {
            for (validator, item_validators) in any_of {
                if validator.is_valid(instance, ctx) {
                    item_validators.mark_evaluated_indexes(
                        instance,
                        indexes,
                        ctx,
                        Marking::Verdict,
                    );
                }
            }
        }

        // Handle oneOf - only mark if exactly one schema validates
        // Short-circuit: stop checking after finding 2 matches
        if let Some(one_of) = &self.one_of {
            let mut match_count = 0;
            let mut matched_validators = None;
            for (node, validators) in one_of {
                if node.is_valid(instance, ctx) {
                    match_count += 1;
                    if match_count > 1 {
                        break; // More than one match, don't mark any indexes
                    }
                    matched_validators = Some(validators);
                }
            }
            if match_count == 1 {
                if let Some(validators) = matched_validators {
                    validators.mark_evaluated_indexes(instance, indexes, ctx, Marking::Verdict);
                }
            }
        }
    }

    /// Mark all items evaluated by this schema (including by `unevaluatedItems` itself).
    fn mark_evaluated_indexes(
        &self,
        instance: &F::Node<'_>,
        indexes: &mut Vec<bool>,
        ctx: &mut ValidationContext,
        marking: Marking,
    ) {
        self.mark_evaluated_indexes_impl(instance, indexes, ctx, true, marking);
    }

    /// Mark items evaluated by all keywords *except* `unevaluatedItems` itself.
    ///
    /// Used in `evaluate()` so that items that would be covered by `unevaluatedItems`
    /// are still visited by `evaluate_instance()`, allowing their annotations to be collected.
    fn mark_evaluated_indexes_by_other_keywords(
        &self,
        instance: &F::Node<'_>,
        indexes: &mut Vec<bool>,
        ctx: &mut ValidationContext,
    ) {
        self.mark_evaluated_indexes_impl(instance, indexes, ctx, false, Marking::Report);
    }
}

impl<F: Json> ConditionalValidators<F> {
    fn mark_evaluated_indexes(
        &self,
        instance: &F::Node<'_>,
        indexes: &mut Vec<bool>,
        ctx: &mut ValidationContext,
        marking: Marking,
    ) {
        let branch = if self.condition.is_valid(instance, ctx) {
            if let Some(if_) = &self.if_ {
                if_.mark_evaluated_indexes(instance, indexes, ctx, Marking::Verdict);
            }
            &self.then_
        } else {
            &self.else_
        };
        if let Some(branch) = branch {
            if branch.counts(instance, ctx, marking) {
                branch
                    .validators
                    .mark_evaluated_indexes(instance, indexes, ctx, Marking::Verdict);
            }
        }
    }
}

/// Compile all items validators for a schema.
///
/// Recursively builds the `ItemsValidators` tree by examining all keywords that
/// can evaluate items. Handles circular references via pending nodes cached
/// by schema pointer.
fn compile_items_validators<'a, F: Json>(
    ctx: &compiler::Context<'_, F>,
    parent: &'a Map<String, Value>,
    root: bool,
) -> Result<ItemsValidators<F>, ValidationError<'a>> {
    let pending = compile_pending_items_validators(ctx, parent, root)?;
    // Only a reference cycle through this node keeps another handle to the cell
    Ok(match Arc::try_unwrap(pending) {
        Ok(cell) => cell
            .into_inner()
            .expect("pending node is initialized before it is returned"),
        Err(shared) => initialized(&shared).clone(),
    })
}

/// The same compilation, handing back the cell cyclic references share.
/// `root` is the schema with the keyword, the only one whose subschemas are checked: below it, a
/// subschema that validates implies every one it applies in place.
fn compile_pending_items_validators<'a, F: Json>(
    ctx: &compiler::Context<'_, F>,
    parent: &'a Map<String, Value>,
    root: bool,
) -> Result<PendingItemsValidators<F>, ValidationError<'a>> {
    // Create a pending node and cache it before compiling to handle circular refs
    let pending = Arc::new(OnceLock::new());
    let shadowed = ctx.cache_pending_items_validators_for_schema(parent, pending.clone());

    let applicator = ctx.has_vocabulary(&Vocabulary::Applicator);

    let unevaluated = compile_unevaluated(ctx, parent)?;
    // `contains` annotates the items it matches from 2020-12 on; earlier drafts evaluate none.
    let contains = if applicator && matches!(ctx.draft(), Draft::Draft202012 | Draft::Unknown) {
        compile_contains(ctx, parent)?
    } else {
        None
    };
    let ref_ = compile_ref(ctx, parent, root)?;
    let dynamic_ref = compile_dynamic_ref(ctx, parent, root)?;
    let recursive_ref = compile_recursive_ref(ctx, parent, root)?;

    // Determine items behavior based on draft
    let (items_limit, items_all) = if applicator {
        compile_items(ctx, parent)?
    } else {
        (None, false)
    };
    let prefix_items = if applicator {
        compile_prefix_items(ctx, parent)?
    } else {
        None
    };

    let conditional = if applicator {
        compile_conditional(ctx, parent, root)?
    } else {
        None
    };
    let all_of = if applicator {
        compile_all_of(ctx, parent)?
    } else {
        None
    };
    let any_of = if applicator {
        compile_any_of(ctx, parent)?
    } else {
        None
    };
    let one_of = if applicator {
        compile_one_of(ctx, parent)?
    } else {
        None
    };

    let validators = ItemsValidators {
        unevaluated,
        contains,
        ref_,
        dynamic_ref,
        recursive_ref,
        items_limit,
        items_all,
        prefix_items,
        conditional,
        all_of,
        any_of,
        one_of,
    };

    pending
        .set(validators)
        .expect("pending node should not be initialized yet");
    ctx.restore_pending_items_validators_for_schema(parent, shadowed);

    Ok(pending)
}

/// Every cell is initialized before compilation returns, so validation never sees an empty one.
fn initialized<F: Json>(pending: &PendingItemsValidators<F>) -> &ItemsValidators<F> {
    pending
        .get()
        .expect("pending node is initialized before validation")
}

fn compile_unevaluated<'a, F: Json>(
    ctx: &compiler::Context<'_, F>,
    parent: &'a Map<String, Value>,
) -> Result<Option<SchemaNode<F>>, ValidationError<'a>> {
    if let Some(subschema) = parent.get("unevaluatedItems") {
        let unevaluated_ctx = ctx.new_at_location("unevaluatedItems");
        Ok(Some(
            compiler::compile(&unevaluated_ctx, unevaluated_ctx.as_resource_ref(subschema))
                .map_err(ValidationError::to_owned)?,
        ))
    } else {
        Ok(None)
    }
}

fn compile_contains<'a, F: Json>(
    ctx: &compiler::Context<'_, F>,
    parent: &'a Map<String, Value>,
) -> Result<Option<SchemaNode<F>>, ValidationError<'a>> {
    if let Some(subschema) = parent.get("contains") {
        let contains_ctx = ctx.new_at_location("contains");
        Ok(Some(
            compiler::compile(&contains_ctx, contains_ctx.as_resource_ref(subschema))
                .map_err(ValidationError::to_owned)?,
        ))
    } else {
        Ok(None)
    }
}

fn compile_ref<'a, F: Json>(
    ctx: &compiler::Context<'_, F>,
    parent: &'a Map<String, Value>,
    root: bool,
) -> Result<Option<Checked<F, PendingItemsValidators<F>>>, ValidationError<'a>> {
    let Some(Value::String(reference)) = parent.get("$ref") else {
        return Ok(None);
    };

    let resolved = ctx.lookup(reference).map_err(ValidationError::from)?;

    let (contents, resolver, draft) = resolved.into_inner();
    if let Value::Object(subschema) = &contents {
        let vocabularies = resolver.find_vocabularies(draft, contents);
        let ref_ctx =
            ctx.with_resolver_and_draft(resolver, draft, vocabularies, ctx.location().clone())?;

        // Circular reference: the target is already being compiled - return its pending node.
        let check = if root {
            unevaluated::reference(ctx, parent, "$ref").map_err(ValidationError::to_owned)?
        } else {
            None
        };
        if let Some(pending) = ref_ctx.get_pending_items_validators_for_schema(subschema) {
            return Ok(Some(Checked::new(check, pending)));
        }

        Ok(Some(Checked::new(
            check,
            compile_pending_items_validators(&ref_ctx, subschema, false)
                .map_err(ValidationError::to_owned)?,
        )))
    } else {
        Ok(None)
    }
}

fn compile_dynamic_ref<'a, F: Json>(
    ctx: &compiler::Context<'_, F>,
    parent: &Map<String, Value>,
    root: bool,
) -> Result<Option<Checked<F, PendingItemsValidators<F>>>, ValidationError<'a>> {
    let Some(Value::String(reference)) = parent.get("$dynamicRef") else {
        return Ok(None);
    };

    let resolved = ctx
        .lookup_dynamic_ref(reference)
        .map_err(ValidationError::from)?;

    let (contents, resolver, draft) = resolved.into_inner();
    if let Value::Object(subschema) = &contents {
        let vocabularies = resolver.find_vocabularies(draft, contents);
        let ref_ctx =
            ctx.with_resolver_and_draft(resolver, draft, vocabularies, ctx.location().clone())?;

        // Circular reference: the target is already being compiled - return its pending node.
        let check = if root {
            unevaluated::reference(ctx, parent, "$dynamicRef").map_err(ValidationError::to_owned)?
        } else {
            None
        };
        if let Some(pending) = ref_ctx.get_pending_items_validators_for_schema(subschema) {
            return Ok(Some(Checked::new(check, pending)));
        }

        Ok(Some(Checked::new(
            check,
            compile_pending_items_validators(&ref_ctx, subschema, false)
                .map_err(ValidationError::to_owned)?,
        )))
    } else {
        Ok(None)
    }
}

fn compile_recursive_ref<'a, F: Json>(
    ctx: &compiler::Context<'_, F>,
    parent: &Map<String, Value>,
    root: bool,
) -> Result<Option<Checked<F, PendingItemsValidators<F>>>, ValidationError<'a>> {
    if !parent.contains_key("$recursiveRef") {
        return Ok(None);
    }

    // For $recursiveRef, we need to resolve the reference and check if it's already being compiled
    let resolved = ctx
        .lookup_recursive_reference()
        .map_err(ValidationError::from)?;

    // Create context for the resolved reference and check its cache key
    let (contents, resolver, draft) = resolved.into_inner();
    if let Value::Object(subschema) = &contents {
        let vocabularies = resolver.find_vocabularies(draft, contents);
        let ref_ctx =
            ctx.with_resolver_and_draft(resolver, draft, vocabularies, ctx.location().clone())?;

        // Check if we're already compiling this schema (circular reference)
        let check = if root {
            unevaluated::reference(ctx, parent, "$recursiveRef")
                .map_err(ValidationError::to_owned)?
        } else {
            None
        };
        if let Some(pending) = ref_ctx.get_pending_items_validators_for_schema(subschema) {
            return Ok(Some(Checked::new(check, pending)));
        }

        // Not circular, compile normally
        Ok(Some(Checked::new(
            check,
            compile_pending_items_validators(&ref_ctx, subschema, false)
                .map_err(ValidationError::to_owned)?,
        )))
    } else {
        Ok(None)
    }
}

fn compile_items<'a, F: Json>(
    ctx: &compiler::Context<'_, F>,
    parent: &'a Map<String, Value>,
) -> Result<(Option<usize>, bool), ValidationError<'a>> {
    if let Some(subschema) = parent.get("items") {
        if ctx.draft() == Draft::Draft201909
            || ctx.draft() == Draft::Draft7
            || ctx.draft() == Draft::Draft6
            || ctx.draft() == Draft::Draft4
        {
            let limit = match subschema {
                Value::Array(items) => {
                    if parent.contains_key("additionalItems") {
                        usize::MAX
                    } else {
                        items.len()
                    }
                }
                // Schema-form `items` applies to every element.
                Value::Object(_) | Value::Bool(_) => usize::MAX,
                // A non-schema `items` is ignored and evaluates nothing, as is any
                // `additionalItems` beside it.
                _ => return Ok((None, false)),
            };
            Ok((Some(limit), false))
        } else {
            // Draft 2020-12+: items is always a schema that applies to all items
            Ok((None, true))
        }
    } else {
        Ok((None, false))
    }
}

fn compile_prefix_items<'a, F: Json>(
    ctx: &compiler::Context<'_, F>,
    parent: &'a Map<String, Value>,
) -> Result<Option<usize>, ValidationError<'a>> {
    // `prefixItems` arrived in 2020-12; an earlier draft reads it as an unknown keyword, and an
    // unknown keyword evaluates nothing.
    if !ctx.draft().is_known_keyword("prefixItems") {
        return Ok(None);
    }
    if let Some(Some(items)) = parent.get("prefixItems").map(Value::as_array) {
        Ok(Some(items.len()))
    } else {
        Ok(None)
    }
}

/// Compile the item validators for a `then`/`else` branch, entering its subresource so a nested
/// `$id` shifts the base URI for anything resolved inside it.
fn compile_branch<'a, F: Json>(
    ctx: &compiler::Context<'_, F>,
    parent: &'a Map<String, Value>,
    keyword: &'static str,
    root: bool,
) -> Result<Option<Checked<F, ItemsValidators<F>>>, ValidationError<'a>> {
    let Some(value) = parent.get(keyword) else {
        return Ok(None);
    };
    let Value::Object(schema) = value else {
        return Ok(None);
    };
    let branch_ctx = ctx.new_at_location(keyword);
    let inner_ctx = branch_ctx
        .in_subresource(branch_ctx.as_resource_ref(value))
        .map_err(ValidationError::from)?;
    let check = if root {
        Some(unevaluated::subschema(&branch_ctx, value).map_err(ValidationError::to_owned)?)
    } else {
        None
    };
    Ok(Some(Checked::new(
        check,
        compile_items_validators(&inner_ctx, schema, false).map_err(ValidationError::to_owned)?,
    )))
}

fn compile_conditional<'a, F: Json>(
    ctx: &compiler::Context<'_, F>,
    parent: &'a Map<String, Value>,
    root: bool,
) -> Result<Option<Box<ConditionalValidators<F>>>, ValidationError<'a>> {
    let Some(subschema @ (Value::Object(_) | Value::Bool(_))) = parent.get("if") else {
        return Ok(None);
    };
    let if_ctx = ctx.new_at_location("if");
    let if_resource = if_ctx.as_resource_ref(subschema);
    let condition = compiler::compile(&if_ctx, if_resource).map_err(ValidationError::to_owned)?;
    let if_ = if let Value::Object(if_parent) = subschema {
        let if_inner_ctx = if_ctx
            .in_subresource(if_resource)
            .map_err(ValidationError::from)?;
        Some(
            compile_items_validators(&if_inner_ctx, if_parent, false)
                .map_err(ValidationError::to_owned)?,
        )
    } else {
        None
    };

    Ok(Some(Box::new(ConditionalValidators {
        condition,
        if_,
        then_: compile_branch(ctx, parent, "then", root)?,
        else_: compile_branch(ctx, parent, "else", root)?,
    })))
}

type CompiledItemsSubschemas<F> = Vec<(SchemaNode<F>, ItemsValidators<F>)>;

fn compile_all_of<'a, F: Json>(
    ctx: &compiler::Context<'_, F>,
    parent: &'a Map<String, Value>,
) -> Result<Option<CompiledItemsSubschemas<F>>, ValidationError<'a>> {
    if let Some(Some(subschemas)) = parent.get("allOf").map(Value::as_array) {
        let all_of_ctx = ctx.new_at_location("allOf");
        let mut result = Vec::with_capacity(subschemas.len());

        for (idx, subschema) in subschemas.iter().enumerate() {
            if let Value::Object(parent) = subschema {
                let subschema_ctx = all_of_ctx.new_at_location(idx);
                let resource = subschema_ctx.as_resource_ref(subschema);
                let node = compiler::compile(&subschema_ctx, resource)
                    .map_err(ValidationError::to_owned)?;
                let inner_ctx = subschema_ctx
                    .in_subresource(resource)
                    .map_err(ValidationError::from)?;
                result.push((
                    node,
                    compile_items_validators(&inner_ctx, parent, false)
                        .map_err(ValidationError::to_owned)?,
                ));
            }
        }

        Ok(Some(result))
    } else {
        Ok(None)
    }
}

fn compile_any_of<'a, F: Json>(
    ctx: &compiler::Context<'_, F>,
    parent: &'a Map<String, Value>,
) -> Result<Option<CompiledItemsSubschemas<F>>, ValidationError<'a>> {
    if let Some(Some(subschemas)) = parent.get("anyOf").map(Value::as_array) {
        let any_of_ctx = ctx.new_at_location("anyOf");
        let mut result = Vec::with_capacity(subschemas.len());

        for (idx, subschema) in subschemas.iter().enumerate() {
            if let Value::Object(parent) = subschema {
                let subschema_ctx = any_of_ctx.new_at_location(idx);
                let resource = subschema_ctx.as_resource_ref(subschema);
                let node = compiler::compile(&subschema_ctx, resource)
                    .map_err(ValidationError::to_owned)?;
                let inner_ctx = subschema_ctx
                    .in_subresource(resource)
                    .map_err(ValidationError::from)?;
                result.push((
                    node,
                    compile_items_validators(&inner_ctx, parent, false)
                        .map_err(ValidationError::to_owned)?,
                ));
            }
        }

        Ok(Some(result))
    } else {
        Ok(None)
    }
}

fn compile_one_of<'a, F: Json>(
    ctx: &compiler::Context<'_, F>,
    parent: &'a Map<String, Value>,
) -> Result<Option<CompiledItemsSubschemas<F>>, ValidationError<'a>> {
    if let Some(Some(subschemas)) = parent.get("oneOf").map(Value::as_array) {
        let one_of_ctx = ctx.new_at_location("oneOf");
        let mut result = Vec::with_capacity(subschemas.len());

        for (idx, subschema) in subschemas.iter().enumerate() {
            if let Value::Object(parent) = subschema {
                let subschema_ctx = one_of_ctx.new_at_location(idx);
                let resource = subschema_ctx.as_resource_ref(subschema);
                let node = compiler::compile(&subschema_ctx, resource)
                    .map_err(ValidationError::to_owned)?;
                let inner_ctx = subschema_ctx
                    .in_subresource(resource)
                    .map_err(ValidationError::from)?;
                result.push((
                    node,
                    compile_items_validators(&inner_ctx, parent, false)
                        .map_err(ValidationError::to_owned)?,
                ));
            }
        }

        Ok(Some(result))
    } else {
        Ok(None)
    }
}

/// Validator for the `unevaluatedItems` keyword.
pub(crate) struct UnevaluatedItemsValidator<F: Json = SerdeJson> {
    location: Location,
    validators: ItemsValidators<F>,
}

impl UnevaluatedItemsValidator {
    pub(crate) fn compile<'a, F: Json>(
        ctx: &'a compiler::Context<F>,
        parent: &'a Map<String, Value>,
    ) -> CompilationResult<'a, F> {
        let validators =
            compile_items_validators(ctx, parent, true).map_err(ValidationError::to_owned)?;

        Ok(Box::new(UnevaluatedItemsValidator {
            location: ctx.location().join("unevaluatedItems"),
            validators,
        }))
    }
}

impl<F: Json> Validate<F> for UnevaluatedItemsValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            let mut indexes = vec![false; array.len()];
            self.validators
                .mark_evaluated_indexes(instance, &mut indexes, ctx, Marking::Verdict);

            for (item, is_evaluated) in array.elements().zip(indexes) {
                if !is_evaluated {
                    if let Some(validator) = &self.validators.unevaluated {
                        if !validator.is_valid(&item, ctx) {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(array) = instance.as_array() {
            let mut indexes = vec![false; array.len()];
            self.validators
                .mark_evaluated_indexes(instance, &mut indexes, ctx, Marking::Report);
            let mut unevaluated = vec![];

            for (item, is_evaluated) in array.elements().zip(indexes) {
                if !is_evaluated {
                    if let Some(validator) = &self.validators.unevaluated {
                        if !validator.is_valid(&item, ctx) {
                            unevaluated.push(item.to_value().to_string());
                        }
                    }
                }
            }

            if !unevaluated.is_empty() {
                return Err(ValidationError::unevaluated_items(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance.lazy_value(),
                    unevaluated,
                ));
            }
        }
        Ok(())
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Some(array) = instance.as_array() {
            let mut indexes = vec![false; array.len()];
            self.validators
                .mark_evaluated_indexes_by_other_keywords(instance, &mut indexes, ctx);
            let mut children = ChildList::default();
            let mut unevaluated = Vec::new();
            let mut invalid = false;

            for (idx, (item, is_evaluated)) in array.elements().zip(indexes.iter()).enumerate() {
                if *is_evaluated {
                    continue;
                }
                if let Some(validator) = &self.validators.unevaluated {
                    let child =
                        validator.evaluate_instance_below(&item, &location.push(idx), tracker, ctx);
                    if !child.valid {
                        invalid = true;
                        unevaluated.push(item.to_value().to_string());
                    }
                    children.push(ctx.arena(), child);
                }
            }

            let mut errors = Vec::new();
            if !unevaluated.is_empty() {
                errors.push(ErrorDescription::from_validation_error(
                    &ValidationError::unevaluated_items(
                        self.location.clone(),
                        crate::paths::capture_evaluation_path(tracker, &self.location),
                        location.into(),
                        instance.lazy_value(),
                        unevaluated,
                    ),
                ));
            }

            if invalid {
                EvaluationResult::Invalid {
                    errors,
                    children,
                    annotations: None,
                }
            } else {
                EvaluationResult::Valid {
                    annotations: None,
                    children,
                }
            }
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

pub(crate) fn compile<'a, F: Json>(
    ctx: &'a compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    match schema.as_bool() {
        Some(true) => None,
        _ => Some(UnevaluatedItemsValidator::compile(ctx, parent)),
    }
}

#[cfg(test)]
mod tests {
    use crate::{error::ValidationErrorKind, tests_util};
    use referencing::{Draft, Retrieve, Uri};
    use serde_json::{json, Value};
    use test_case::test_case;

    // A non-schema `items` is ignored, as is any `additionalItems` beside it, so neither
    // evaluates a single element. Only a registry resource can carry such a value, since
    // top-level builds are meta-validated.
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "items": 42, "unevaluatedItems": false}); "invalid items alone")]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "items": 42, "additionalItems": false, "unevaluatedItems": false}); "beside additionalItems")]
    fn ignored_items_evaluates_nothing(resource: &Value) {
        let registry = crate::Registry::new()
            .add("https://example.com/tail", resource)
            .expect("resource accepted")
            .prepare()
            .expect("registry build failed");
        let validator = crate::options()
            .with_registry(&registry)
            .build(&json!({"$ref": "https://example.com/tail"}))
            .expect("schema compiles");
        tests_util::is_valid_with(&validator, &json!([]));
        tests_util::is_not_valid_with(&validator, &json!([1]));
    }

    fn errors(schema: &Value, instance: &Value) -> Vec<(String, String, String)> {
        let validator = crate::validator_for(schema).expect("schema compiles");
        validator
            .iter_errors(instance)
            .map(|error| {
                (
                    error.instance_path().as_str().to_owned(),
                    error.evaluation_path().as_str().to_owned(),
                    error.to_string(),
                )
            })
            .collect()
    }

    /// `is_valid`, `iter_errors`, `evaluate()` validity and `evaluate()` errors, each error as
    /// a (schema location, message) pair.
    type Outcome = (bool, Vec<(String, String)>, bool, Vec<(String, String)>);

    fn errors_and_evaluation(schema: &Value, instance: &Value) -> Outcome {
        let validator = crate::validator_for(schema).expect("schema compiles");
        let errors = validator
            .iter_errors(instance)
            .map(|error| {
                (
                    error.evaluation_path().as_str().to_owned(),
                    error.to_string(),
                )
            })
            .collect();
        let evaluation = validator.evaluate(instance);
        let evaluation_errors = evaluation
            .iter_errors()
            .map(|entry| (entry.schema_location.to_owned(), entry.error.to_string()))
            .collect();
        (
            validator.is_valid(instance),
            errors,
            evaluation.flag().valid,
            evaluation_errors,
        )
    }

    // The reference beside the keyword evaluates the items of the node it points to
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$defs": {"R": {"items": [{"type": "integer"}, {"$recursiveRef": "#", "unevaluatedItems": false}]}}, "$ref": "#/$defs/R"}), &json!([1, [2]]), &[]; "2019-09 prefixed item")]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$defs": {"R": {"items": [{"type": "integer"}, {"$recursiveRef": "#", "unevaluatedItems": false}]}}, "$ref": "#/$defs/R"}), &json!([1, [2, [3]]]), &[]; "2019-09 nested prefixed item")]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$defs": {"R": {"items": [{"type": "integer"}, {"$recursiveRef": "#", "unevaluatedItems": false}]}}, "$ref": "#/$defs/R"}), &json!([1, [2, 3, 4]]), &[("/1", "/$ref/items/1/unevaluatedItems", "Unevaluated items are not allowed ('4' was unexpected)")]; "2019-09 item past the prefix")]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2019-09/schema", "$defs": {"R": {"items": [{"type": "integer"}, {"$recursiveRef": "#", "unevaluatedItems": false}]}}, "$ref": "#/$defs/R"}), &json!([1, [2, [3, 4, 5]]]), &[("/1", "/$ref/items/1/unevaluatedItems", "Unevaluated items are not allowed ('2', '[3,4,5]' were unexpected)"), ("/1/1", "/$ref/items/1/$recursiveRef/$ref/items/1/unevaluatedItems", "Unevaluated items are not allowed ('5' was unexpected)")]; "2019-09 nested item past the prefix")]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$defs": {"R": {"prefixItems": [{"type": "integer"}, {"$dynamicRef": "#", "unevaluatedItems": false}]}}, "$ref": "#/$defs/R"}), &json!([1, [2]]), &[]; "2020-12 prefixed item")]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema", "$defs": {"R": {"prefixItems": [{"type": "integer"}, {"$dynamicRef": "#", "unevaluatedItems": false}]}}, "$ref": "#/$defs/R"}), &json!([1, [2, 3, 4]]), &[("/1", "/$ref/prefixItems/1/unevaluatedItems", "Unevaluated items are not allowed ('4' was unexpected)")]; "2020-12 item past the prefix")]
    fn reference_to_root_evaluates_the_root_items(
        schema: &Value,
        instance: &Value,
        expected: &[(&str, &str, &str)],
    ) {
        let expected: Vec<_> = expected
            .iter()
            .map(|(instance_path, evaluation_path, message)| {
                (
                    (*instance_path).to_owned(),
                    (*evaluation_path).to_owned(),
                    (*message).to_owned(),
                )
            })
            .collect();
        assert_eq!(errors(schema, instance), expected);
    }

    #[test_case(Draft::Draft201909, &json!([]), true; "2019-09 empty array")]
    #[test_case(Draft::Draft201909, &json!([1]), false; "2019-09 leaves the first item unevaluated")]
    #[test_case(Draft::Draft202012, &json!([1]), true; "2020-12 evaluates the prefixed index")]
    #[test_case(Draft::Draft202012, &json!([1, 2]), false; "2020-12 leaves the index past the prefix unevaluated")]
    fn prefix_items_evaluate_only_where_the_draft_defines_them(
        draft: Draft,
        instance: &Value,
        expected: bool,
    ) {
        let validator = crate::options()
            .with_draft(draft)
            .build(&json!({"prefixItems": [{"type": "integer"}], "unevaluatedItems": false}))
            .expect("schema compiles");
        assert_eq!(validator.is_valid(instance), expected);
    }

    #[test]
    fn dynamic_ref_cycle_does_not_overflow() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/root",
            "$dynamicAnchor": "node",
            "type": "array",
            "$dynamicRef": "#node",
            "unevaluatedItems": false
        });

        let validator = crate::options().build(&schema).expect("schema compiles");

        assert!(validator.is_valid(&json!([])));
    }

    // A `$ref` cycle back to the node evaluates exactly what the node evaluates
    #[test_case(&json!({"$ref": "#"}); "self reference")]
    #[test_case(&json!({"$id": "https://example.com/root.json", "$ref": "https://example.com/root.json"}); "self through id")]
    #[test_case(&json!({"allOf": [{"$ref": "#"}]}); "self through allOf")]
    #[test_case(&json!({"if": {"$ref": "#"}}); "self through if")]
    #[test_case(&json!({"$defs": {"a": {"$ref": "#/$defs/b"}, "b": {"$ref": "#/$defs/a"}}, "$ref": "#/$defs/a"}); "mutually recursive definitions")]
    #[test_case(&json!({"$defs": {"a": {"$ref": "#/$defs/a"}}, "$ref": "#/$defs/a"}); "self recursive definition")]
    fn ref_cycle_evaluates_what_the_node_evaluates(applicator: &Value) {
        let mut schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "prefixItems": [true],
            "unevaluatedItems": false
        });
        schema
            .as_object_mut()
            .expect("object schema")
            .extend(applicator.as_object().expect("object applicator").clone());

        let validator = crate::validator_for(&schema).expect("schema compiles");

        assert!(validator.is_valid(&json!([1])));
        assert!(!validator.is_valid(&json!([1, 2])));
    }

    // The keyword inside a recursive definition sees the definition's own prefix
    #[test]
    fn ref_cycle_inside_definition_evaluates_the_definition() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$defs": {
                "node": {
                    "prefixItems": [{"$ref": "#/$defs/node"}],
                    "allOf": [{"$ref": "#/$defs/node"}],
                    "unevaluatedItems": false
                }
            },
            "$ref": "#/$defs/node"
        });

        let validator = crate::validator_for(&schema).expect("schema compiles");

        assert!(validator.is_valid(&json!([[[]]])));
        assert!(!validator.is_valid(&json!([[[], 1]])));
    }

    #[test]
    fn prefix_items_do_not_evaluate_without_applicator_vocabulary() {
        let meta = json!({
            "$id": "json-schema:///meta/no-applicator-items",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$vocabulary": {
                "https://json-schema.org/draft/2020-12/vocab/core": true,
                "https://json-schema.org/draft/2020-12/vocab/validation": true,
                "https://json-schema.org/draft/2020-12/vocab/unevaluated": true,
                "https://json-schema.org/draft/2020-12/vocab/format-annotation": true
            }
        });
        let registry = crate::Registry::new()
            .add("json-schema:///meta/no-applicator-items", &meta)
            .expect("resource accepted")
            .prepare()
            .expect("registry build failed");
        let schema = json!({
            "$schema": "json-schema:///meta/no-applicator-items",
            "prefixItems": [{"type": "integer"}],
            "unevaluatedItems": false
        });
        let validator = crate::options()
            .with_registry(&registry)
            .build(&schema)
            .expect("schema compiles");
        assert!(validator.is_valid(&json!([])));
        assert!(!validator.is_valid(&json!([1])));
    }

    #[test]
    fn dynamic_ref_cycle_via_all_of_does_not_overflow() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/root",
            "$dynamicAnchor": "node",
            "type": "array",
            "allOf": [{ "$dynamicRef": "#node" }],
            "unevaluatedItems": false
        });

        let validator = crate::options().build(&schema).expect("schema compiles");

        assert!(validator.is_valid(&json!([])));
    }

    #[test]
    fn test_unevaluated_items_with_recursion() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "allOf": [
                {
                    "$ref": "#/$defs/array_1"
                }
            ],
            "unevaluatedItems": false,
            "$defs": {
                "array_1": {
                    "type": "array",
                    "prefixItems": [
                        {
                            "type": "string"
                        },
                        {
                            "allOf": [
                                {
                                    "$ref": "#/$defs/array_2"
                                }
                            ],
                            "type": "array",
                            "unevaluatedItems": false
                        }
                    ]
                },
                "array_2": {
                    "type": "array",
                    "prefixItems": [
                        {
                            "type": "number"
                        },
                        {
                            "allOf": [
                                {
                                    "$ref": "#/$defs/array_1"
                                }
                            ],
                            "type": "array",
                            "unevaluatedItems": false
                        }
                    ]
                }
            }
        });

        let validator = crate::validator_for(&schema).expect("Schema should compile");

        // This instance should fail validation because the nested array has an unevaluated item
        let instance = json!([
            "string",
            [
                42,
                [
                    "string",
                    [
                        42,
                        "unexpected" // This item should cause validation to fail
                    ]
                ]
            ]
        ]);

        assert!(!validator.is_valid(&instance));
        assert!(validator.validate(&instance).is_err());

        // This instance should pass validation as all items are evaluated
        let valid_instance = json!(["string", [42, ["string", [42]]]]);

        assert!(validator.is_valid(&valid_instance));
        assert!(validator.validate(&valid_instance).is_ok());
    }

    #[test_case(&json!({"allOf": [{"$id": "https://example.com/nested/branch.json", "$ref": "target.json"}]}); "allOf")]
    #[test_case(&json!({"anyOf": [{"$id": "https://example.com/nested/branch.json", "$ref": "target.json"}]}); "anyOf")]
    #[test_case(&json!({"oneOf": [{"$id": "https://example.com/nested/branch.json", "$ref": "target.json"}]}); "oneOf")]
    #[test_case(&json!({"if": {"$id": "https://example.com/nested/branch.json", "$ref": "target.json"}}); "if branch")]
    #[test_case(&json!({"if": {}, "then": {"$id": "https://example.com/nested/branch.json", "$ref": "target.json"}}); "then branch")]
    #[test_case(&json!({"if": {"not": {}}, "else": {"$id": "https://example.com/nested/branch.json", "$ref": "target.json"}}); "else branch")]
    fn subresource_id_resolves_relative_reference_against_its_own_base(applicator: &Value) {
        let mut schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/root.json",
            "$defs": {
                "reachable_through_the_branch_id": {
                    "$id": "https://example.com/nested/target.json",
                    "prefixItems": [{"type": "string"}]
                },
                "reachable_through_the_root_id": {
                    "$id": "https://example.com/target.json",
                    "prefixItems": [{"type": "string"}, {"type": "string"}]
                }
            },
            "unevaluatedItems": false
        });
        schema
            .as_object_mut()
            .expect("object schema")
            .extend(applicator.as_object().expect("object applicator").clone());

        let validator = crate::validator_for(&schema).expect("schema compiles");

        assert!(validator.is_valid(&json!(["x"])));
        assert!(!validator.is_valid(&json!(["x", "y"])));
    }

    #[test_case(&json!({"allOf": [{"$id": "https://example.com/nested/branch.json", "$ref": "target.json"}]}); "allOf")]
    #[test_case(&json!({"anyOf": [{"$id": "https://example.com/nested/branch.json", "$ref": "target.json"}]}); "anyOf")]
    #[test_case(&json!({"oneOf": [{"$id": "https://example.com/nested/branch.json", "$ref": "target.json"}]}); "oneOf")]
    #[test_case(&json!({"if": {"$id": "https://example.com/nested/branch.json", "$ref": "target.json"}}); "if branch")]
    #[test_case(&json!({"if": {}, "then": {"$id": "https://example.com/nested/branch.json", "$ref": "target.json"}}); "then branch")]
    #[test_case(&json!({"if": {"not": {}}, "else": {"$id": "https://example.com/nested/branch.json", "$ref": "target.json"}}); "else branch")]
    fn subresource_id_keeps_the_schema_compilable(applicator: &Value) {
        let mut schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/root.json",
            "$defs": {
                "target": {
                    "$id": "https://example.com/nested/target.json",
                    "prefixItems": [{"type": "string"}]
                }
            },
            "unevaluatedItems": false
        });
        schema
            .as_object_mut()
            .expect("object schema")
            .extend(applicator.as_object().expect("object applicator").clone());

        crate::validator_for(&schema).expect("schema compiles");
    }

    #[test]
    fn reference_target_resolves_its_own_relative_reference() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/root.json",
            "$defs": {
                "outer": {
                    "$id": "https://example.com/nested/outer.json",
                    "$ref": "inner.json"
                },
                "reachable_through_the_outer_id": {
                    "$id": "https://example.com/nested/inner.json",
                    "prefixItems": [{"type": "string"}]
                },
                "reachable_through_the_root_id": {
                    "$id": "https://example.com/inner.json",
                    "prefixItems": [{"type": "string"}, {"type": "string"}]
                }
            },
            "$ref": "https://example.com/nested/outer.json",
            "unevaluatedItems": false
        });

        let validator = crate::validator_for(&schema).expect("schema compiles");

        assert!(validator.is_valid(&json!(["x"])));
        assert!(!validator.is_valid(&json!(["x", "y"])));
    }

    const FIRST_UNEXPECTED: &[(&str, &str)] = &[(
        "/unevaluatedItems",
        "Unevaluated items are not allowed ('1' was unexpected)",
    )];

    // Boolean and empty subschemas evaluate nothing; the branches they select still do
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"if": true, "then": {"items": [{}]}}), &[]; "2019-09 if true with then")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"if": false, "else": {"items": [{}]}}), &[]; "2019-09 if false with else")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"if": true, "else": {"items": [{}]}}), FIRST_UNEXPECTED; "2019-09 if true with else only")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"if": false, "then": {"items": [{}]}}), FIRST_UNEXPECTED; "2019-09 if false with then only")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"if": true, "then": true}), FIRST_UNEXPECTED; "2019-09 if true with then true")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"if": true, "then": {"items": true}}), &[]; "2019-09 if true with then items true")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"if": {}, "then": {"items": [{}]}}), &[]; "2019-09 if empty with then")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"if": true, "then": {"if": false, "else": {"items": [{}]}}}), &[]; "2019-09 nested boolean if")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"allOf": [true]}), FIRST_UNEXPECTED; "2019-09 allOf true")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"allOf": [true, {"items": [{}]}]}), &[]; "2019-09 allOf true and object")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"anyOf": [true, {"items": [{}]}]}), &[]; "2019-09 anyOf true and object")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"oneOf": [false, {"items": [{}]}]}), &[]; "2019-09 oneOf false and object")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"not": false}), FIRST_UNEXPECTED; "2019-09 not false")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"$defs": {"t": true}, "$ref": "#/$defs/t"}), FIRST_UNEXPECTED; "2019-09 ref to true")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"items": true}), &[]; "2019-09 items true")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"items": [true]}), &[]; "2019-09 items true")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"if": true, "then": {"prefixItems": [{}]}}), &[]; "2020-12 if true with then")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"if": false, "else": {"prefixItems": [{}]}}), &[]; "2020-12 if false with else")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"if": true, "else": {"prefixItems": [{}]}}), FIRST_UNEXPECTED; "2020-12 if true with else only")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"if": false, "then": {"prefixItems": [{}]}}), FIRST_UNEXPECTED; "2020-12 if false with then only")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"if": true, "then": true}), FIRST_UNEXPECTED; "2020-12 if true with then true")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"if": true, "then": {"items": true}}), &[]; "2020-12 if true with then items true")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"if": {}, "then": {"prefixItems": [{}]}}), &[]; "2020-12 if empty with then")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"if": true, "then": {"if": false, "else": {"prefixItems": [{}]}}}), &[]; "2020-12 nested boolean if")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"allOf": [true]}), FIRST_UNEXPECTED; "2020-12 allOf true")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"allOf": [true, {"prefixItems": [{}]}]}), &[]; "2020-12 allOf true and object")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"anyOf": [true, {"prefixItems": [{}]}]}), &[]; "2020-12 anyOf true and object")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"oneOf": [false, {"prefixItems": [{}]}]}), &[]; "2020-12 oneOf false and object")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"not": false}), FIRST_UNEXPECTED; "2020-12 not false")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"$defs": {"t": true}, "$ref": "#/$defs/t"}), FIRST_UNEXPECTED; "2020-12 ref to true")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"items": true}), &[]; "2020-12 items true")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"prefixItems": [true]}), &[]; "2020-12 prefixItems true")]
    fn boolean_subschemas(draft: &str, applicator: &Value, expected: &[(&str, &str)]) {
        assert_first_item_outcome(draft, applicator, expected);
    }

    // `contains` evaluates the items it matches from 2020-12 on, wherever it applies in place
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"contains": {"type": "integer"}}), FIRST_UNEXPECTED; "2019-09 contains")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"allOf": [{"contains": {"type": "integer"}}]}), FIRST_UNEXPECTED; "2019-09 contains in allOf")]
    #[test_case("https://json-schema.org/draft/2019-09/schema", &json!({"$defs": {"c": {"contains": {"type": "integer"}}}, "$ref": "#/$defs/c"}), FIRST_UNEXPECTED; "2019-09 contains in a reference")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"contains": {"type": "integer"}}), &[]; "2020-12 contains")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"allOf": [{"contains": {"type": "integer"}}]}), &[]; "2020-12 contains in allOf")]
    #[test_case("https://json-schema.org/draft/2020-12/schema", &json!({"$defs": {"c": {"contains": {"type": "integer"}}}, "$ref": "#/$defs/c"}), &[]; "2020-12 contains in a reference")]
    fn contains_evaluates_from_2020_12(draft: &str, applicator: &Value, expected: &[(&str, &str)]) {
        assert_first_item_outcome(draft, applicator, expected);
    }

    struct CustomMetaSchema;

    impl Retrieve for CustomMetaSchema {
        fn retrieve(
            &self,
            uri: &Uri<String>,
        ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
            match uri.as_str() {
                "json-schema:///meta/custom" => Ok(json!({
                    "$id": "json-schema:///meta/custom",
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "$vocabulary": {
                        "https://json-schema.org/draft/2020-12/vocab/core": true,
                        "https://json-schema.org/draft/2020-12/vocab/applicator": true,
                        "https://json-schema.org/draft/2020-12/vocab/validation": true,
                        "https://json-schema.org/draft/2020-12/vocab/unevaluated": true
                    }
                })),
                other => Err(format!("unexpected retrieval of {other}").into()),
            }
        }
    }

    // A `$schema` naming a meta-schema outside the registry leaves the draft unknown, which
    // reads `contains` as 2020-12 does
    #[test_case(&json!([1]), true; "matched item")]
    #[test_case(&json!([1, "x"]), false; "unmatched item")]
    fn contains_evaluates_under_a_custom_meta_schema(instance: &Value, expected: bool) {
        let validator = crate::options()
            .with_retriever(CustomMetaSchema)
            .build(&json!({
                "$schema": "json-schema:///meta/custom",
                "contains": {"type": "integer"},
                "unevaluatedItems": false
            }))
            .expect("schema compiles");
        assert_eq!(validator.is_valid(instance), expected);
    }

    // The outcome of `applicator` beside `unevaluatedItems: false` on `[1]`
    fn assert_first_item_outcome(draft: &str, applicator: &Value, expected: &[(&str, &str)]) {
        let expected: Vec<_> = expected
            .iter()
            .map(|(location, message)| ((*location).to_owned(), (*message).to_owned()))
            .collect();
        // `evaluate()` also lists the `false` subschema rejecting the unevaluated value
        let mut evaluation_expected = expected.clone();
        if !expected.is_empty() {
            evaluation_expected.push((
                "/unevaluatedItems".to_owned(),
                "False schema does not allow 1".to_owned(),
            ));
        }
        let mut schema = json!({"$schema": draft, "unevaluatedItems": false});
        schema
            .as_object_mut()
            .expect("object schema")
            .extend(applicator.as_object().expect("object applicator").clone());
        assert_eq!(
            errors_and_evaluation(&schema, &json!([1])),
            (
                expected.is_empty(),
                expected.clone(),
                expected.is_empty(),
                evaluation_expected
            )
        );
    }

    #[test]
    fn malformed() {
        crate::tests_util::assert_compile_error(
            &json!({"unevaluatedItems": 5}),
            "5 is not of types \"boolean\", \"object\"",
            "/unevaluatedItems",
        );
    }

    // A cycle back to the root through a nested resource validates as it does without that resource
    #[test_case("$ref", &json!([]), true; "ref empty")]
    #[test_case("$ref", &json!([1]), true; "ref integer item")]
    #[test_case("$ref", &json!(["a"]), false; "ref string item")]
    #[test_case("$dynamicRef", &json!([]), true; "dynamic ref empty")]
    #[test_case("$dynamicRef", &json!([1]), true; "dynamic ref integer item")]
    #[test_case("$dynamicRef", &json!(["a"]), false; "dynamic ref string item")]
    fn ref_cycle_across_resources(reference: &str, instance: &Value, expected: bool) {
        let schema = |branch_id: Option<&str>| {
            let mut branch = json!({reference: "https://a.com", "unevaluatedItems": false});
            if let Some(id) = branch_id {
                branch["$id"] = json!(id);
            }
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$id": "https://a.com",
                "anyOf": [branch],
                "unevaluatedItems": {"type": "integer"}
            })
        };
        let without_resource = crate::validator_for(&schema(None)).expect("schema compiles");

        let validator =
            crate::validator_for(&schema(Some("https://b.com"))).expect("schema compiles");

        assert_eq!(validator.is_valid(instance), expected);
        assert_eq!(validator.validate(instance).is_ok(), expected);
        assert_eq!(without_resource.is_valid(instance), expected);
    }

    /// Items `unevaluatedItems` reports: through `iter_errors`, and as failing `evaluate()` units.
    fn unevaluated(schema: &Value, instance: &Value) -> (Vec<String>, Vec<String>) {
        let validator = crate::validator_for(schema).expect("schema compiles");
        let reported = validator
            .iter_errors(instance)
            .find_map(|error| match error.kind() {
                ValidationErrorKind::UnevaluatedItems { unexpected } => Some(unexpected.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let units = validator
            .evaluate(instance)
            .iter_errors()
            .filter(|entry| {
                entry.schema_location == "/unevaluatedItems"
                    && !entry.instance_location.as_str().is_empty()
            })
            .map(|entry| entry.instance_location.as_str().to_owned())
            .collect();
        (reported, units)
    }

    // A subschema that fails evaluates none of the items; keywords beside `unevaluatedItems`
    // evaluate theirs whether they pass or not.
    #[test_case(&json!({"$defs": {"t": {"prefixItems": [{"type": "string"}]}}, "$ref": "#/$defs/t", "unevaluatedItems": false}), &json!([1, 2]), &["1", "2"], &["/0", "/1"]; "reference")]
    #[test_case(&json!({"$defs": {"t": {"prefixItems": [{"type": "string"}]}}, "$ref": "#/$defs/t", "unevaluatedItems": false}), &json!(["x", 2]), &["2"], &["/1"]; "valid reference")]
    #[test_case(&json!({"$defs": {"t": {"$dynamicAnchor": "n", "prefixItems": [{"type": "string"}]}}, "$dynamicRef": "#n", "unevaluatedItems": false}), &json!([1, 2]), &["1", "2"], &["/0", "/1"]; "dynamic reference")]
    #[test_case(&json!({"if": true, "then": {"prefixItems": [{"type": "string"}]}, "unevaluatedItems": false}), &json!([1, 2]), &["1", "2"], &["/0", "/1"]; "then")]
    #[test_case(&json!({"if": false, "else": {"prefixItems": [{"type": "string"}]}, "unevaluatedItems": false}), &json!([1, 2]), &["1", "2"], &["/0", "/1"]; "else")]
    #[test_case(&json!({"$defs": {"t": {"contains": {"const": 1}, "minContains": 2}}, "$ref": "#/$defs/t", "unevaluatedItems": false}), &json!([1, 2]), &["1", "2"], &["/0", "/1"]; "contains in a reference")]
    #[test_case(&json!({"allOf": [{"prefixItems": [{"type": "string"}]}], "unevaluatedItems": false}), &json!([1, 2]), &["1", "2"], &["/0", "/1"]; "allOf branch")]
    #[test_case(&json!({"prefixItems": [{"type": "string"}], "unevaluatedItems": false}), &json!([1, 2]), &["2"], &["/1"]; "prefixItems beside")]
    fn failing_subschema_evaluates_nothing(
        schema: &Value,
        instance: &Value,
        expected: &[&str],
        locations: &[&str],
    ) {
        assert_eq!(
            unevaluated(schema, instance),
            (
                expected.iter().map(|item| (*item).to_owned()).collect(),
                locations
                    .iter()
                    .map(|location| (*location).to_owned())
                    .collect()
            )
        );
    }

    #[test]
    fn failing_recursive_reference_evaluates_nothing() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2019-09/schema",
            "$defs": {"R": {"items": [{"type": "string"}, {"$recursiveRef": "#", "unevaluatedItems": false}]}},
            "$ref": "#/$defs/R"
        });
        assert_eq!(
            errors(&schema, &json!(["a", [1]])),
            vec![
                (
                    "/1".to_owned(),
                    "/$ref/items/1/unevaluatedItems".to_owned(),
                    "Unevaluated items are not allowed ('1' was unexpected)".to_owned()
                ),
                (
                    "/1/0".to_owned(),
                    "/$ref/items/1/$recursiveRef/$ref/items/0/type".to_owned(),
                    r#"1 is not of type "string""#.to_owned()
                ),
            ]
        );
    }
}
