use crate::{
    compiler::{self, DeferredAbsoluteLocation},
    evaluation::{
        format_keyword_location, Annotations, ChildList, ErrorDescription, EvaluationNode,
    },
    keywords::{BoxedValidator, CompilationResult},
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
    Array, Draft, Json, Node, SerdeJson, ValidationError,
};
use referencing::{Uri, Vocabulary};
use serde_json::{Map, Value};
use std::{
    marker::PhantomData,
    sync::{Arc, OnceLock},
};

pub(crate) struct ItemsArrayValidator<F: Json = SerdeJson> {
    items: Vec<SchemaNode<F>>,
}
impl ItemsArrayValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schemas: &'a [Value],
    ) -> CompilationResult<'a, F> {
        let kctx = ctx.new_at_location("items");
        let mut items = Vec::with_capacity(schemas.len());
        for (idx, item) in schemas.iter().enumerate() {
            let ictx = kctx.new_at_location(idx);
            let validators = compiler::compile(&ictx, ictx.as_resource_ref(item))?;
            items.push(validators);
        }
        Ok(Box::new(ItemsArrayValidator { items }))
    }
}
impl<F: Json> Validate<F> for ItemsArrayValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            for (item, node) in array.elements().zip(self.items.iter()) {
                if !node.is_valid(&item, ctx) {
                    return false;
                }
            }
            true
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(array) = instance.as_array() {
            for (idx, (item, node)) in array.elements().zip(self.items.iter()).enumerate() {
                node.validate(&item, &location.push(idx), tracker, ctx)?;
            }
        }
        Ok(())
    }

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        let Some(array) = instance.as_array() else {
            return;
        };
        for (idx, (item, node)) in array.elements().zip(self.items.iter()).enumerate() {
            node.collect_errors(&item, &location.push(idx), tracker, ctx, errors);
        }
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Some(array) = instance.as_array() {
            let mut children = ChildList::default();
            for (idx, (item, node)) in array.elements().zip(self.items.iter()).enumerate() {
                let child = node.evaluate_instance_below(&item, &location.push(idx), tracker, ctx);
                children.push(&mut ctx.arena, child);
            }
            EvaluationResult::from_children(children)
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

/// Evaluates `node` against every array element, as schema-form `items` does.
fn evaluate_each_item<F: Json>(
    node: &SchemaNode<F>,
    instance: &F::Node<'_>,
    location: &LazyLocation,
    tracker: Option<&RefTracker>,
    ctx: &mut ValidationContext,
) -> EvaluationResult {
    let Some(array) = instance.as_array() else {
        return EvaluationResult::valid_empty();
    };
    let mut children = ChildList::default();
    for (idx, item) in array.elements().enumerate() {
        let child = node.evaluate_instance_below(&item, &location.push(idx), tracker, ctx);
        children.push(&mut ctx.arena, child);
    }
    let mut result = EvaluationResult::from_children(children);
    result.annotate(Annotations::new(serde_json::json!(array.len() != 0)));
    result
}

pub(crate) struct ItemsObjectValidator<F: Json = SerdeJson> {
    node: SchemaNode<F>,
}

impl ItemsObjectValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
    ) -> CompilationResult<'a, F> {
        let ctx = ctx.new_at_location("items");
        let node = compiler::compile(&ctx, ctx.as_resource_ref(schema))?;
        Ok(Box::new(ItemsObjectValidator { node }))
    }
}
impl<F: Json> Validate<F> for ItemsObjectValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            array.elements().all(|item| self.node.is_valid(&item, ctx))
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(array) = instance.as_array() {
            for (idx, item) in array.elements().enumerate() {
                self.node
                    .validate(&item, &location.push(idx), tracker, ctx)?;
            }
        }
        Ok(())
    }

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        let Some(array) = instance.as_array() else {
            return;
        };
        for (idx, item) in array.elements().enumerate() {
            self.node
                .collect_errors(&item, &location.push(idx), tracker, ctx, errors);
        }
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        evaluate_each_item(&self.node, instance, location, tracker, ctx)
    }
}

pub(crate) struct ItemsObjectSkipPrefixValidator<F: Json = SerdeJson> {
    node: SchemaNode<F>,
    skip_prefix: usize,
}

impl ItemsObjectSkipPrefixValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        schema: &'a Value,
        skip_prefix: usize,
        ctx: &compiler::Context<F>,
    ) -> CompilationResult<'a, F> {
        let ctx = ctx.new_at_location("items");
        let node = compiler::compile(&ctx, ctx.as_resource_ref(schema))?;
        Ok(Box::new(ItemsObjectSkipPrefixValidator {
            node,
            skip_prefix,
        }))
    }
}

impl<F: Json> Validate<F> for ItemsObjectSkipPrefixValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            array
                .elements()
                .skip(self.skip_prefix)
                .all(|item| self.node.is_valid(&item, ctx))
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if let Some(array) = instance.as_array() {
            for (idx, item) in array.elements().skip(self.skip_prefix).enumerate() {
                self.node
                    .validate(&item, &location.push(idx + self.skip_prefix), tracker, ctx)?;
            }
        }
        Ok(())
    }

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        let Some(array) = instance.as_array() else {
            return;
        };
        for (idx, item) in array.elements().skip(self.skip_prefix).enumerate() {
            self.node.collect_errors(
                &item,
                &location.push(idx + self.skip_prefix),
                tracker,
                ctx,
                errors,
            );
        }
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Some(array) = instance.as_array() {
            let mut children = ChildList::default();
            for (idx, item) in array.elements().enumerate().skip(self.skip_prefix) {
                let child =
                    self.node
                        .evaluate_instance_below(&item, &location.push(idx), tracker, ctx);
                children.push(&mut ctx.arena, child);
            }
            let schema_was_applied = array.len() > self.skip_prefix;
            let mut result = EvaluationResult::from_children(children);
            result.annotate(Annotations::new(serde_json::json!(schema_was_applied)));
            result
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

/// Element type checked inline by `ItemsTypeValidator`.
pub(crate) trait ItemType: Send + Sync + 'static {
    fn matches<F: Json>(item: &F::Node<'_>) -> bool;
    /// The validator the draft's `type` keyword compiles to for this type.
    fn compile_type<F: Json>(location: Location) -> CompilationResult<'static, F>;
}

pub(crate) struct NumberItems;
pub(crate) struct StringItems;
pub(crate) struct BooleanItems;
pub(crate) struct IntegerItems;
/// Draft 4 integers exclude numbers written with a fractional part, such as `1.0`.
pub(crate) struct IntegerItemsDraft4;

impl ItemType for NumberItems {
    #[inline]
    fn matches<F: Json>(item: &F::Node<'_>) -> bool {
        item.is_number()
    }

    fn compile_type<F: Json>(location: Location) -> CompilationResult<'static, F> {
        super::type_::NumberTypeValidator::compile(location)
    }
}

impl ItemType for StringItems {
    #[inline]
    fn matches<F: Json>(item: &F::Node<'_>) -> bool {
        item.is_string()
    }

    fn compile_type<F: Json>(location: Location) -> CompilationResult<'static, F> {
        super::type_::StringTypeValidator::compile(location)
    }
}

impl ItemType for BooleanItems {
    #[inline]
    fn matches<F: Json>(item: &F::Node<'_>) -> bool {
        item.as_boolean().is_some()
    }

    fn compile_type<F: Json>(location: Location) -> CompilationResult<'static, F> {
        super::type_::BooleanTypeValidator::compile(location)
    }
}

impl ItemType for IntegerItems {
    #[inline]
    fn matches<F: Json>(item: &F::Node<'_>) -> bool {
        item.as_number()
            .is_some_and(|n| super::type_::is_integer(&n))
    }

    fn compile_type<F: Json>(location: Location) -> CompilationResult<'static, F> {
        super::type_::IntegerTypeValidator::compile(location)
    }
}

impl ItemType for IntegerItemsDraft4 {
    #[inline]
    fn matches<F: Json>(item: &F::Node<'_>) -> bool {
        item.as_number()
            .is_some_and(|n| super::legacy::type_draft_4::is_integer(&n))
    }

    fn compile_type<F: Json>(location: Location) -> CompilationResult<'static, F> {
        super::legacy::type_draft_4::IntegerTypeValidator::compile(location)
    }
}

/// A schema position with the locations its output reports, rendered on first use.
struct SchemaSite {
    location: Location,
    base: Option<DeferredAbsoluteLocation>,
    absolute_location: OnceLock<Option<Arc<Uri<String>>>>,
    schema_location: OnceLock<Arc<str>>,
}

impl SchemaSite {
    fn new<F: Json>(ctx: &compiler::Context<F>, location: Location) -> Self {
        SchemaSite {
            location,
            base: ctx.deferred_absolute_location(),
            absolute_location: OnceLock::new(),
            schema_location: OnceLock::new(),
        }
    }

    fn absolute_location(&self) -> Option<&Arc<Uri<String>>> {
        self.absolute_location
            .get_or_init(|| self.base.as_ref().map(|base| base.resolve(&self.location)))
            .as_ref()
    }

    fn schema_location(&self) -> Arc<str> {
        Arc::clone(
            self.schema_location
                .get_or_init(|| format_keyword_location(&self.location, self.absolute_location())),
        )
    }
}

/// `items: {"type": <T>}` with the type check inlined. Output matches the generic `items`: each
/// element gets the subschema node and its `type` keyword node, and failures come from the same
/// `type` validator the generic compiler builds.
pub(crate) struct ItemsTypeValidator<T: ItemType, F: Json = SerdeJson> {
    items: SchemaSite,
    type_: SchemaSite,
    type_validator: BoxedValidator<F>,
    item_type: PhantomData<T>,
}

impl<T: ItemType> ItemsTypeValidator<T> {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(ctx: &compiler::Context<F>) -> CompilationResult<'a, F> {
        // The subschema has no `$id`, so it resolves against the same base as `ctx`.
        let items = ctx.location().join("items");
        let type_ = items.join("type");
        Ok(Box::new(ItemsTypeValidator::<T, F> {
            type_validator: T::compile_type(type_.clone())?,
            items: SchemaSite::new(ctx, items),
            type_: SchemaSite::new(ctx, type_),
            item_type: PhantomData,
        }))
    }
}

impl<T: ItemType, F: Json> ItemsTypeValidator<T, F> {
    /// The node `SchemaNode::evaluate_instance_below` builds for the subschema at one element.
    fn evaluate_item(
        &self,
        item: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationNode {
        let instance_location = match ctx.instance_location() {
            Some(parent) => parent.join_pointer_segment(location.segment()),
            None => location.into(),
        };
        let keyword_location = crate::paths::evaluation_path(tracker, &self.items.location, ctx);
        // A matching element passes `type` with no annotations, so its keyword node needs no call.
        let type_result = if T::matches::<F>(item) {
            EvaluationResult::valid_empty()
        } else {
            let previous = ctx.enter_instance_location(instance_location.clone());
            let result = self.type_validator.evaluate_with_location(
                item,
                location,
                &instance_location,
                tracker,
                ctx,
            );
            ctx.leave_instance_location(previous);
            result
        };
        let type_node = match type_result {
            EvaluationResult::Valid {
                annotations,
                children,
            } => EvaluationNode::valid(
                crate::paths::evaluation_path(tracker, &self.type_.location, ctx),
                self.type_.absolute_location().cloned(),
                self.type_.schema_location(),
                instance_location.clone(),
                annotations,
                children,
            ),
            EvaluationResult::Invalid {
                errors,
                children,
                annotations,
            } => EvaluationNode::invalid(
                crate::paths::evaluation_path(tracker, &self.type_.location, ctx),
                self.type_.absolute_location().cloned(),
                self.type_.schema_location(),
                instance_location.clone(),
                annotations,
                errors,
                children,
            ),
        };
        let valid = type_node.valid;
        let children = ChildList::of(&mut ctx.arena, type_node);
        if valid {
            EvaluationNode::valid(
                keyword_location,
                self.items.absolute_location().cloned(),
                self.items.schema_location(),
                instance_location,
                None,
                children,
            )
        } else {
            EvaluationNode::invalid(
                keyword_location,
                self.items.absolute_location().cloned(),
                self.items.schema_location(),
                instance_location,
                None,
                Vec::new(),
                children,
            )
        }
    }
}

impl<T: ItemType, F: Json> Validate<F> for ItemsTypeValidator<T, F> {
    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, _ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            array.elements().all(|item| T::matches::<F>(&item))
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        let Some(array) = instance.as_array() else {
            return Ok(());
        };
        // The index is only needed for an error, so valid arrays skip counting it.
        if array.elements().all(|item| T::matches::<F>(&item)) {
            return Ok(());
        }
        for (idx, item) in array.elements().enumerate() {
            if !T::matches::<F>(&item) {
                self.type_validator
                    .validate(&item, &location.push(idx), tracker, ctx)
                    .map_err(|error| {
                        error
                            .with_absolute_keyword_location(self.type_.absolute_location().cloned())
                    })?;
            }
        }
        Ok(())
    }

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        let Some(array) = instance.as_array() else {
            return;
        };
        for (idx, item) in array.elements().enumerate() {
            if !T::matches::<F>(&item) {
                let start = errors.len();
                self.type_validator.collect_errors(
                    &item,
                    &location.push(idx),
                    tracker,
                    ctx,
                    errors,
                );
                if let Some(uri) = self.type_.absolute_location() {
                    for error in &mut errors[start..] {
                        error.set_absolute_keyword_location(uri);
                    }
                }
            }
        }
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let Some(array) = instance.as_array() else {
            return EvaluationResult::valid_empty();
        };
        let mut children = ChildList::default();
        for (idx, item) in array.elements().enumerate() {
            let child = self.evaluate_item(&item, &location.push(idx), tracker, ctx);
            children.push(&mut ctx.arena, child);
        }
        let mut result = EvaluationResult::from_children(children);
        result.annotate(Annotations::new(serde_json::json!(array.len() != 0)));
        result
    }
}

struct CountConstraint {
    limit: u64,
    site: SchemaSite,
}

/// Element validation for the fused validator. Single-type variants keep the specialized `items`
/// validator so `is_valid` avoids `SchemaNode` dispatch on the per-element hot path.
enum FusedItems<F: Json> {
    Number(BoxedValidator<F>),
    String(BoxedValidator<F>),
    Boolean(BoxedValidator<F>),
    IntegerDraft4(BoxedValidator<F>),
    IntegerDraft7(BoxedValidator<F>),
    Generic(SchemaNode<F>),
}

impl<F: Json> FusedItems<F> {
    fn compile<'a>(
        ctx: &compiler::Context<F>,
        items: &'a Value,
    ) -> Result<Self, ValidationError<'a>> {
        Ok(match simple_item_type(ctx, items) {
            Some(SimpleItemType::Number) => {
                FusedItems::Number(ItemsTypeValidator::<NumberItems>::compile(ctx)?)
            }
            Some(SimpleItemType::String) => {
                FusedItems::String(ItemsTypeValidator::<StringItems>::compile(ctx)?)
            }
            Some(SimpleItemType::Boolean) => {
                FusedItems::Boolean(ItemsTypeValidator::<BooleanItems>::compile(ctx)?)
            }
            Some(SimpleItemType::IntegerDraft4) => {
                FusedItems::IntegerDraft4(ItemsTypeValidator::<IntegerItemsDraft4>::compile(ctx)?)
            }
            Some(SimpleItemType::Integer) => {
                FusedItems::IntegerDraft7(ItemsTypeValidator::<IntegerItems>::compile(ctx)?)
            }
            None => {
                let ctx = ctx.new_at_location("items");
                FusedItems::Generic(compiler::compile(&ctx, ctx.as_resource_ref(items))?)
            }
        })
    }
}

/// Fused `type: "array"` + optional `minItems`/`maxItems` + schema-form `items`: one `as_array`,
/// one length check, and one element pass instead of three validators re-reading the same node.
pub(crate) struct ArrayShapeValidator<F: Json = SerdeJson> {
    items: FusedItems<F>,
    /// `minItems`, or 0 without one.
    min_items: u64,
    /// `maxItems`, or `u64::MAX` without one.
    max_items: u64,
    /// Shared with the schema node, which reports these keywords as their own output units.
    absorbed: Arc<AbsorbedKeywords>,
}

/// `type`, `minItems` and `maxItems` beside a fused `items`, with the locations their errors and
/// output units report.
pub(crate) struct AbsorbedKeywords {
    type_: SchemaSite,
    min_items: Option<CountConstraint>,
    max_items: Option<CountConstraint>,
}

impl ArrayShapeValidator {
    /// Also returns the absorbed keywords, which the schema node reports as their own units.
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        parent: &'a Map<String, Value>,
        items: &'a Value,
    ) -> Result<(BoxedValidator<F>, Arc<AbsorbedKeywords>), ValidationError<'a>> {
        let items = FusedItems::compile(ctx, items)?;
        let constraint = |key: &str| -> Option<CountConstraint> {
            Some(CountConstraint {
                limit: accepts_item_count(ctx, parent.get(key)?)?,
                site: SchemaSite::new(ctx, ctx.location().join(key)),
            })
        };
        let absorbed = Arc::new(AbsorbedKeywords {
            type_: SchemaSite::new(ctx, ctx.location().join("type")),
            min_items: constraint("minItems"),
            max_items: constraint("maxItems"),
        });
        let validator = Box::new(ArrayShapeValidator {
            items,
            min_items: absorbed.min_items.as_ref().map_or(0, |c| c.limit),
            max_items: absorbed.max_items.as_ref().map_or(u64::MAX, |c| c.limit),
            absorbed: Arc::clone(&absorbed),
        });
        Ok((validator, absorbed))
    }
}

impl AbsorbedKeywords {
    #[cold]
    #[inline(never)]
    fn type_error<'i, F: Json>(
        &self,
        instance: &F::Node<'i>,
        instance_location: impl Into<Location>,
        tracker: Option<&RefTracker>,
    ) -> ValidationError<'i> {
        let site = &self.type_;
        ValidationError::single_type_error(
            site.location.clone(),
            crate::paths::capture_evaluation_path(tracker, &site.location),
            instance_location.into(),
            instance.lazy_value(),
            JsonType::Array,
        )
        .with_absolute_keyword_location(site.absolute_location().cloned())
    }

    #[cold]
    #[inline(never)]
    fn min_items_error<'i, F: Json>(
        &self,
        instance: &F::Node<'i>,
        instance_location: impl Into<Location>,
        tracker: Option<&RefTracker>,
    ) -> ValidationError<'i> {
        let constraint = self.min_items.as_ref().expect("Fails only with its limit");
        let site = &constraint.site;
        ValidationError::min_items(
            site.location.clone(),
            crate::paths::capture_evaluation_path(tracker, &site.location),
            instance_location.into(),
            instance.lazy_value(),
            constraint.limit,
        )
        .with_absolute_keyword_location(site.absolute_location().cloned())
    }

    #[cold]
    #[inline(never)]
    fn max_items_error<'i, F: Json>(
        &self,
        instance: &F::Node<'i>,
        instance_location: impl Into<Location>,
        tracker: Option<&RefTracker>,
    ) -> ValidationError<'i> {
        let constraint = self.max_items.as_ref().expect("Fails only with its limit");
        let site = &constraint.site;
        ValidationError::max_items(
            site.location.clone(),
            crate::paths::capture_evaluation_path(tracker, &site.location),
            instance_location.into(),
            instance.lazy_value(),
            constraint.limit,
        )
        .with_absolute_keyword_location(site.absolute_location().cloned())
    }

    /// The `type` unit, as the standalone `type` validator would report it.
    pub(crate) fn evaluate_type<F: Json>(
        &self,
        instance: &F::Node<'_>,
        is_array: bool,
        instance_location: &Location,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationNode {
        let error = (!is_array).then(|| {
            ErrorDescription::new(
                "type",
                format!(r#"{} is not of type "array""#, instance.to_value()),
            )
        });
        unit(&self.type_, error, instance_location, tracker, ctx)
    }

    /// The `minItems` and `maxItems` units, as their standalone validators would report them.
    /// `count` is `None` for a non-array instance.
    pub(crate) fn evaluate_counts<F: Json>(
        &self,
        instance: &F::Node<'_>,
        count: Option<u64>,
        instance_location: &Location,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        children: &mut ChildList,
    ) {
        if let Some(constraint) = &self.min_items {
            let error = count
                .is_some_and(|count| count < constraint.limit)
                .then(|| {
                    ErrorDescription::from_validation_error(&self.min_items_error::<F>(
                        instance,
                        instance_location.clone(),
                        tracker,
                    ))
                });
            let node = unit(&constraint.site, error, instance_location, tracker, ctx);
            children.push(&mut ctx.arena, node);
        }
        if let Some(constraint) = &self.max_items {
            let error = count
                .is_some_and(|count| count > constraint.limit)
                .then(|| {
                    ErrorDescription::from_validation_error(&self.max_items_error::<F>(
                        instance,
                        instance_location.clone(),
                        tracker,
                    ))
                });
            let node = unit(&constraint.site, error, instance_location, tracker, ctx);
            children.push(&mut ctx.arena, node);
        }
    }
}

/// A leaf keyword's output unit: valid and empty, or failing with `error`.
fn unit(
    site: &SchemaSite,
    error: Option<ErrorDescription>,
    instance_location: &Location,
    tracker: Option<&RefTracker>,
    ctx: &mut ValidationContext,
) -> EvaluationNode {
    let evaluation_path = crate::paths::evaluation_path(tracker, &site.location, ctx);
    match error {
        None => EvaluationNode::valid(
            evaluation_path,
            site.absolute_location().cloned(),
            site.schema_location(),
            instance_location.clone(),
            None,
            ChildList::default(),
        ),
        Some(error) => EvaluationNode::invalid(
            evaluation_path,
            site.absolute_location().cloned(),
            site.schema_location(),
            instance_location.clone(),
            None,
            vec![error],
            ChildList::default(),
        ),
    }
}

impl<F: Json> Validate<F> for ArrayShapeValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        let Some(array) = instance.as_array() else {
            return false;
        };
        let count = array.len() as u64;
        if count < self.min_items {
            return false;
        }
        if count > self.max_items {
            return false;
        }
        match &self.items {
            FusedItems::Number(_) => array
                .elements()
                .all(|item| NumberItems::matches::<F>(&item)),
            FusedItems::String(_) => array
                .elements()
                .all(|item| StringItems::matches::<F>(&item)),
            FusedItems::Boolean(_) => array
                .elements()
                .all(|item| BooleanItems::matches::<F>(&item)),
            FusedItems::IntegerDraft7(_) => array
                .elements()
                .all(|item| IntegerItems::matches::<F>(&item)),
            FusedItems::IntegerDraft4(_) => array
                .elements()
                .all(|item| IntegerItemsDraft4::matches::<F>(&item)),
            FusedItems::Generic(node) => array.elements().all(|item| node.is_valid(&item, ctx)),
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        let Some(array) = instance.as_array() else {
            return Err(self.absorbed.type_error::<F>(instance, location, tracker));
        };
        let count = array.len() as u64;
        if count < self.min_items {
            return Err(self
                .absorbed
                .min_items_error::<F>(instance, location, tracker));
        }
        if count > self.max_items {
            return Err(self
                .absorbed
                .max_items_error::<F>(instance, location, tracker));
        }
        match &self.items {
            FusedItems::Generic(node) => {
                for (idx, item) in array.elements().enumerate() {
                    node.validate(&item, &location.push(idx), tracker, ctx)?;
                }
            }
            FusedItems::Number(cold)
            | FusedItems::String(cold)
            | FusedItems::Boolean(cold)
            | FusedItems::IntegerDraft4(cold)
            | FusedItems::IntegerDraft7(cold) => {
                cold.validate(instance, location, tracker, ctx)?;
            }
        }
        Ok(())
    }

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        let Some(array) = instance.as_array() else {
            errors.push(self.absorbed.type_error::<F>(instance, location, tracker));
            return;
        };
        let count = array.len() as u64;
        if count < self.min_items {
            errors.push(
                self.absorbed
                    .min_items_error::<F>(instance, location, tracker),
            );
        }
        if count > self.max_items {
            errors.push(
                self.absorbed
                    .max_items_error::<F>(instance, location, tracker),
            );
        }
        match &self.items {
            FusedItems::Generic(node) => {
                for (idx, item) in array.elements().enumerate() {
                    node.collect_errors(&item, &location.push(idx), tracker, ctx, errors);
                }
            }
            FusedItems::Number(cold)
            | FusedItems::String(cold)
            | FusedItems::Boolean(cold)
            | FusedItems::IntegerDraft4(cold)
            | FusedItems::IntegerDraft7(cold) => {
                cold.collect_errors(instance, location, tracker, ctx, errors);
            }
        }
    }

    /// Evaluates `items` alone: the schema node reports the [`AbsorbedKeywords`] as their own
    /// units, as they would be without the fusion.
    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        match &self.items {
            FusedItems::Generic(node) => evaluate_each_item(node, instance, location, tracker, ctx),
            FusedItems::Number(cold)
            | FusedItems::String(cold)
            | FusedItems::Boolean(cold)
            | FusedItems::IntegerDraft4(cold)
            | FusedItems::IntegerDraft7(cold) => cold.evaluate(instance, location, tracker, ctx),
        }
    }
}

/// Parses a length keyword exactly as `MinItemsValidator`/`MaxItemsValidator` would accept it.
#[allow(clippy::float_cmp)]
fn accepts_item_count<F: Json>(ctx: &compiler::Context<F>, schema: &Value) -> Option<u64> {
    crate::keywords::helpers::size_limit(ctx, schema)
}

/// Whether `{type: "array", (minItems|maxItems)?, items: {schema}}` can be fused into a single
/// `ArrayShapeValidator`. Any positional or extra element keyword blocks the fusion.
pub(crate) fn array_shape_fusion<F: Json>(
    ctx: &compiler::Context<F>,
    parent: &Map<String, Value>,
) -> bool {
    // Runs once per object schema, so the checks most schemas fail come first.
    if !matches!(
        parent.get("items"),
        Some(Value::Object(_) | Value::Bool(false))
    ) {
        return false;
    }
    match parent.get("type") {
        Some(Value::String(ty)) if ty.as_str() == "array" => {}
        _ => return false,
    }
    // `items` compiles only under the applicator vocabulary, `type` and the bounds under validation.
    if !ctx.has_vocabulary(&Vocabulary::Validation) || !ctx.has_vocabulary(&Vocabulary::Applicator)
    {
        return false;
    }
    for key in [
        "prefixItems",
        "additionalItems",
        "contains",
        "unevaluatedItems",
        "uniqueItems",
    ] {
        if parent.contains_key(key) {
            return false;
        }
    }
    for key in ["type", "items"] {
        if ctx.is_keyword_overridden(key) {
            return false;
        }
    }
    for key in ["minItems", "maxItems"] {
        if let Some(value) = parent.get(key) {
            if ctx.is_keyword_overridden(key) {
                return false;
            }
            if accepts_item_count(ctx, value).is_none() {
                return false;
            }
        }
    }
    true
}

/// Check if schema is a simple `{"type": "<type>"}` pattern and return the type.
fn get_simple_type_schema(schema: &Value) -> Option<&str> {
    let obj = schema.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    obj.get("type")?.as_str()
}

#[derive(Clone, Copy)]
enum SimpleItemType {
    Number,
    String,
    Boolean,
    Integer,
    IntegerDraft4,
}

/// Item schemas whose only keyword is a `type` that `ItemsTypeValidator` can check inline.
/// The inline check asserts `type`, so it needs the validation vocabulary and the built-in keyword.
fn simple_item_type<F: Json>(ctx: &compiler::Context<F>, schema: &Value) -> Option<SimpleItemType> {
    if !ctx.has_vocabulary(&Vocabulary::Validation) || ctx.is_keyword_overridden("type") {
        return None;
    }
    match get_simple_type_schema(schema)? {
        "number" => Some(SimpleItemType::Number),
        "string" => Some(SimpleItemType::String),
        "boolean" => Some(SimpleItemType::Boolean),
        "integer" if ctx.draft() == Draft::Draft4 => Some(SimpleItemType::IntegerDraft4),
        "integer" => Some(SimpleItemType::Integer),
        _ => None,
    }
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    match schema {
        Value::Array(items) => Some(ItemsArrayValidator::compile(ctx, items)),
        Value::Object(_) | Value::Bool(false) => {
            // `prefixItems` arrived in 2020-12; an earlier draft reads it as an unknown keyword,
            // leaving no prefix for schema-form `items` to skip.
            if ctx.draft().is_known_keyword("prefixItems") {
                if let Some(Value::Array(prefix_items)) = parent.get("prefixItems") {
                    return Some(ItemsObjectSkipPrefixValidator::compile(
                        schema,
                        prefix_items.len(),
                        ctx,
                    ));
                }
            }
            if let Some(item_type) = simple_item_type(ctx, schema) {
                return Some(match item_type {
                    SimpleItemType::Number => ItemsTypeValidator::<NumberItems>::compile(ctx),
                    SimpleItemType::String => ItemsTypeValidator::<StringItems>::compile(ctx),
                    SimpleItemType::Boolean => ItemsTypeValidator::<BooleanItems>::compile(ctx),
                    SimpleItemType::Integer => ItemsTypeValidator::<IntegerItems>::compile(ctx),
                    SimpleItemType::IntegerDraft4 => {
                        ItemsTypeValidator::<IntegerItemsDraft4>::compile(ctx)
                    }
                });
            }
            Some(ItemsObjectValidator::compile(ctx, schema))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use referencing::Draft;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(Draft::Draft201909, &json!([]), true; "2019-09 empty array")]
    #[test_case(Draft::Draft201909, &json!([1]), false; "2019-09 items covers the whole array")]
    #[test_case(Draft::Draft202012, &json!([1]), true; "2020-12 items skips the prefix")]
    #[test_case(Draft::Draft202012, &json!([1, 2]), false; "2020-12 items covers past the prefix")]
    fn items_skips_a_prefix_only_where_the_draft_defines_prefix_items(
        draft: Draft,
        instance: &Value,
        expected: bool,
    ) {
        let validator = crate::options()
            .with_draft(draft)
            .build(&json!({"prefixItems": [{"type": "integer"}], "items": false}))
            .expect("schema compiles");
        assert_eq!(validator.is_valid(instance), expected);
    }

    #[test_case(&json!({"items": false}), &json!([1]), "/items")]
    #[test_case(&json!({"items": {"type": "string"}}), &json!([1]), "/items/type")]
    #[test_case(&json!({"prefixItems": [{"type": "string"}]}), &json!([1]), "/prefixItems/0/type")]
    fn schema_location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }

    #[test_case(&json!({"items": {"type": "string"}}), &json!([1]), "/0"; "string first")]
    #[test_case(&json!({"items": {"type": "string"}}), &json!(["a", 1]), "/1"; "string second")]
    #[test_case(&json!({"items": {"type": "number"}}), &json!(["x"]), "/0"; "number first")]
    #[test_case(&json!({"items": {"type": "integer"}}), &json!([1.5]), "/0"; "integer first")]
    #[test_case(&json!({"items": {"type": "boolean"}}), &json!([1]), "/0"; "boolean first")]
    fn instance_location(schema: &Value, instance: &Value, expected: &str) {
        let validator = crate::validator_for(schema).unwrap();
        let error = validator.iter_errors(instance).next().unwrap();
        assert_eq!(error.instance_path().as_str(), expected);
    }

    fn with_id(draft: Draft, mut schema: Value) -> Value {
        let key = if draft == Draft::Draft4 { "id" } else { "$id" };
        schema[key] = json!("https://example.com/root");
        schema
    }

    fn error_report(
        schema: &Value,
        draft: Draft,
        instance: &Value,
    ) -> Vec<(String, String, String, String)> {
        let validator = crate::options()
            .with_draft(draft)
            .build(schema)
            .expect("schema compiles");
        validator
            .iter_errors(instance)
            .map(|error| {
                (
                    error.schema_path().as_str().to_string(),
                    error
                        .absolute_keyword_location()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                    error.instance_path().as_str().to_string(),
                    error.to_string(),
                )
            })
            .collect()
    }

    fn first_error_report(
        schema: &Value,
        draft: Draft,
        instance: &Value,
    ) -> Option<(String, String, String, String, String)> {
        let validator = crate::options()
            .with_draft(draft)
            .build(schema)
            .expect("schema compiles");
        validator.validate(instance).err().map(|error| {
            (
                error.schema_path().as_str().to_string(),
                error.evaluation_path().as_str().to_string(),
                error
                    .absolute_keyword_location()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                error.instance_path().as_str().to_string(),
                error.to_string(),
            )
        })
    }

    fn output_report(schema: &Value, draft: Draft, instance: &Value) -> (Value, Value) {
        let validator = crate::options()
            .with_draft(draft)
            .build(schema)
            .expect("schema compiles");
        let evaluation = validator.evaluate(instance);
        (
            serde_json::to_value(evaluation.list()).expect("list output"),
            serde_json::to_value(evaluation.hierarchical()).expect("hierarchical output"),
        )
    }

    // `"type": [T]` is equivalent to `"type": T` but is not a single-type item schema, so it
    // compiles through the generic `items` path.
    #[test_case(Draft::Draft4, "number", &json!([1, "x", 2, null]))]
    #[test_case(Draft::Draft4, "string", &json!(["a", 1, "b", 2]))]
    #[test_case(Draft::Draft4, "boolean", &json!([true, 1, false, "x"]))]
    #[test_case(Draft::Draft4, "integer", &json!([1, 1.0, 2, 1.5]))]
    #[test_case(Draft::Draft6, "integer", &json!([1, 1.0, 2, 1.5]))]
    #[test_case(Draft::Draft7, "number", &json!([1, "x", 2, null]))]
    #[test_case(Draft::Draft7, "string", &json!(["a", 1, "b", 2]))]
    #[test_case(Draft::Draft7, "boolean", &json!([true, 1, false, "x"]))]
    #[test_case(Draft::Draft7, "integer", &json!([1, 1.0, 2, 1.5]))]
    #[test_case(Draft::Draft201909, "string", &json!(["a", 1, "b", 2]))]
    #[test_case(Draft::Draft201909, "integer", &json!([1, 1.0, 2, 1.5]))]
    #[test_case(Draft::Draft202012, "number", &json!([1, "x", 2, null]))]
    #[test_case(Draft::Draft202012, "string", &json!(["a", 1, "b", 2]))]
    #[test_case(Draft::Draft202012, "boolean", &json!([true, 1, false, "x"]))]
    #[test_case(Draft::Draft202012, "integer", &json!([1, 1.0, 2, 1.5]))]
    fn typed_items_report_like_generic_items(draft: Draft, ty: &str, instance: &Value) {
        let valid = Value::Array(
            instance
                .as_array()
                .expect("array instance")
                .iter()
                .step_by(2)
                .cloned()
                .collect(),
        );
        for (fast, generic) in [
            (
                json!({"items": {"type": ty}}),
                json!({"items": {"type": [ty]}}),
            ),
            (
                json!({"type": "array", "items": {"type": ty}}),
                json!({"type": "array", "items": {"type": [ty]}}),
            ),
            (
                json!({"type": "array", "minItems": 1, "maxItems": 9, "items": {"type": ty}}),
                json!({"type": "array", "minItems": 1, "maxItems": 9, "items": {"type": [ty]}}),
            ),
            // Reached through `$ref`, so evaluation paths differ from schema paths.
            (
                json!({"allOf": [{"$ref": "#/definitions/a"}], "definitions": {"a": {"items": {"type": ty}}}}),
                json!({"allOf": [{"$ref": "#/definitions/a"}], "definitions": {"a": {"items": {"type": [ty]}}}}),
            ),
            (
                json!({"allOf": [{"$ref": "#/definitions/a"}], "definitions": {"a": {"type": "array", "items": {"type": ty}}}}),
                json!({"allOf": [{"$ref": "#/definitions/a"}], "definitions": {"a": {"type": "array", "items": {"type": [ty]}}}}),
            ),
        ]
        .into_iter()
        .flat_map(|(fast, generic)| {
            [
                (with_id(draft, fast.clone()), with_id(draft, generic.clone())),
                (fast, generic),
            ]
        }) {
            for instance in [instance, &valid, &json!([]), &json!({})] {
                assert_eq!(
                    error_report(&fast, draft, instance),
                    error_report(&generic, draft, instance),
                    "{fast} errors for {instance}"
                );
                assert_eq!(
                    first_error_report(&fast, draft, instance),
                    first_error_report(&generic, draft, instance),
                    "{fast} first error for {instance}"
                );
                assert_eq!(
                    output_report(&fast, draft, instance),
                    output_report(&generic, draft, instance),
                    "{fast} output for {instance}"
                );
            }
        }
    }

    #[test]
    fn typed_items_error_locations() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/root",
            "items": {"type": "string"}
        });
        assert_eq!(
            error_report(&schema, Draft::Draft202012, &json!([1])),
            [(
                "/items/type".to_string(),
                "https://example.com/root#/items/type".to_string(),
                "/0".to_string(),
                r#"1 is not of type "string""#.to_string(),
            )]
        );
        let (list, _) = output_report(&schema, Draft::Draft202012, &json!([1]));
        assert_eq!(
            list,
            json!({
                "valid": false,
                "details": [
                    {"valid": false, "evaluationPath": "", "schemaLocation": "https://example.com/root#", "instanceLocation": ""},
                    {"valid": false, "evaluationPath": "/items", "schemaLocation": "https://example.com/root#/items", "instanceLocation": "", "droppedAnnotations": true},
                    {"valid": false, "evaluationPath": "/items", "schemaLocation": "https://example.com/root#/items", "instanceLocation": "/0"},
                    {"valid": false, "evaluationPath": "/items/type", "schemaLocation": "https://example.com/root#/items/type", "instanceLocation": "/0", "errors": {"type": r#"1 is not of type "string""#}}
                ]
            })
        );
    }

    /// Drops the output units of `uniqueItems`, which the unfused schema carries only to block
    /// the fusion.
    fn without_unique_items(output: &mut Value) {
        match output {
            Value::Object(map) => {
                if let Some(Value::Array(details)) = map.get_mut("details") {
                    details.retain(|unit| {
                        !unit["evaluationPath"]
                            .as_str()
                            .is_some_and(|path| path.ends_with("/uniqueItems"))
                    });
                }
                map.values_mut().for_each(without_unique_items);
            }
            Value::Array(items) => items.iter_mut().for_each(without_unique_items),
            _ => {}
        }
    }

    #[test_case(Draft::Draft4)]
    #[test_case(Draft::Draft7)]
    #[test_case(Draft::Draft201909)]
    #[test_case(Draft::Draft202012)]
    fn array_shape_reports_like_unfused(draft: Draft) {
        let instances = [
            json!([]),
            json!([1]),
            json!([1, 2]),
            json!([1, 2, 3]),
            json!(["x"]),
            json!([-1, "x", 2]),
            json!("s"),
            json!({}),
        ];
        // Draft 4 has no boolean schemas.
        let never = if draft == Draft::Draft4 {
            json!({"not": {}})
        } else {
            json!(false)
        };
        for items in [
            json!({"type": "integer"}),
            json!({"type": "integer", "minimum": 0}),
            never,
        ] {
            for bounds in [
                json!({}),
                json!({"minItems": 2}),
                json!({"maxItems": 2}),
                json!({"minItems": 1, "maxItems": 2}),
            ] {
                let mut fused = json!({"type": "array", "items": items});
                fused
                    .as_object_mut()
                    .expect("object schema")
                    .extend(bounds.as_object().expect("object bounds").clone());
                let fused = with_id(draft, fused);
                let mut unfused = fused.clone();
                unfused["uniqueItems"] = json!(false);
                for instance in &instances {
                    assert_eq!(
                        error_report(&fused, draft, instance),
                        error_report(&unfused, draft, instance),
                        "{fused} errors for {instance}"
                    );
                    let (mut list, mut hierarchical) = output_report(&unfused, draft, instance);
                    without_unique_items(&mut list);
                    without_unique_items(&mut hierarchical);
                    assert_eq!(
                        output_report(&fused, draft, instance),
                        (list, hierarchical),
                        "{fused} output for {instance}"
                    );
                }
            }
        }
    }

    // Siblings sorting before and after the bounds, reached through `$ref` and a nested resource.
    #[test_case(Draft::Draft7, "definitions")]
    #[test_case(Draft::Draft202012, "$defs")]
    fn array_shape_reports_like_unfused_beside_siblings_and_refs(draft: Draft, defs: &str) {
        let fused = json!({
            "$id": "https://example.com/root",
            defs: {
                "shape": {
                    "type": "array",
                    "minItems": 2,
                    "maxItems": 3,
                    "items": {"type": "integer"},
                    "const": [1, 2],
                    "minProperties": 0,
                    "required": ["x"]
                },
                "nested": {
                    "$id": "nested",
                    "properties": {"p": {"$ref": format!("https://example.com/root#/{defs}/shape")}}
                }
            },
            "properties": {
                "x": {"$ref": format!("#/{defs}/shape")},
                "y": {"$ref": "nested"},
                "z": {"anyOf": [{"$ref": format!("#/{defs}/shape")}, {"type": "string"}]}
            }
        });
        let mut unfused = fused.clone();
        unfused[defs]["shape"]["uniqueItems"] = json!(false);
        for instance in [
            json!({"x": [1], "y": {"p": "s"}, "z": [1, 2, 3, 4, "q"]}),
            json!({"x": [1, 2], "y": {"p": [1, 2]}, "z": "ok"}),
            json!({"x": {}, "z": []}),
        ] {
            let (mut list, mut hierarchical) = output_report(&unfused, draft, &instance);
            without_unique_items(&mut list);
            without_unique_items(&mut hierarchical);
            assert_eq!(
                output_report(&fused, draft, &instance),
                (list, hierarchical),
                "output for {instance}"
            );
        }
    }

    // Fused `type:array` + optional min/maxItems + schema `items` (ArrayShapeValidator)
    #[test_case(&json!({"type": "array", "items": {"type": "number"}}), &json!([1, 2, 3]), true; "all valid")]
    #[test_case(&json!({"type": "array", "items": {"type": "number"}}), &json!([1, "x"]), false; "bad element")]
    #[test_case(&json!({"type": "array", "items": {"type": "number"}}), &json!("nope"), false; "non-array")]
    #[test_case(&json!({"type": "array", "items": {"type": "number"}}), &json!([]), true; "empty array")]
    #[test_case(&json!({"type": "array", "minItems": 2, "items": {"type": "number"}}), &json!([1]), false; "too short")]
    #[test_case(&json!({"type": "array", "minItems": 2, "items": {"type": "number"}}), &json!([1, 2]), true; "min satisfied")]
    #[test_case(&json!({"type": "array", "maxItems": 2, "items": {"type": "number"}}), &json!([1, 2, 3]), false; "too long")]
    #[test_case(&json!({"type": "array", "minItems": 1, "maxItems": 3, "items": {"type": "number"}}), &json!([1, 2]), true; "within bounds")]
    #[test_case(&json!({"type": "array", "items": {"type": "array", "items": {"type": "number"}}}), &json!([[1], [2, 3]]), true; "nested valid")]
    #[test_case(&json!({"type": "array", "items": {"type": "array", "items": {"type": "number"}}}), &json!([[1], ["x"]]), false; "nested bad element")]
    #[test_case(&json!({"type": "array", "items": {"type": "string"}}), &json!(["a", "b"]), true; "string valid")]
    #[test_case(&json!({"type": "array", "items": {"type": "string"}}), &json!(["a", 1]), false; "string bad element")]
    #[test_case(&json!({"type": "array", "items": {"type": "boolean"}}), &json!([true, false]), true; "boolean valid")]
    #[test_case(&json!({"type": "array", "items": {"type": "boolean"}}), &json!([true, 1]), false; "boolean bad element")]
    #[test_case(&json!({"type": "array", "items": {"type": "integer"}}), &json!([1, 2]), true; "integer valid")]
    #[test_case(&json!({"type": "array", "items": {"type": "integer"}}), &json!([1, 1.5]), false; "integer bad element")]
    #[test_case(&json!({"type": "array", "minItems": 2.0, "items": {"type": "number"}}), &json!([1]), false; "float minItems too short")]
    #[test_case(&json!({"type": "array", "minItems": 2.0, "items": {"type": "number"}}), &json!([1, 2]), true; "float minItems satisfied")]
    #[test_case(&json!({"type": "array", "items": {"type": "null"}}), &json!([null, null]), true; "null items valid")]
    #[test_case(&json!({"type": "array", "items": {"type": "null"}}), &json!([null, 1]), false; "null items bad element")]
    fn array_shape_is_valid(schema: &Value, instance: &Value, expected: bool) {
        let validator = crate::validator_for(schema).unwrap();
        assert_eq!(validator.is_valid(instance), expected);
        // `is_valid`, `validate`, `iter_errors`, and `evaluate` must agree on the verdict.
        assert_eq!(validator.validate(instance).is_ok(), expected);
        assert_eq!(validator.iter_errors(instance).next().is_none(), expected);
        assert_eq!(validator.evaluate(instance).flag().valid, expected);
    }

    // Draft 4 integer element semantics (1.0 is not an integer) route through the fused validator.
    #[test_case(&json!([1, 2]), true; "d4 integers valid")]
    #[test_case(&json!([1, 1.0]), false; "d4 float not integer")]
    #[test_case(&json!([1, "x"]), false; "d4 non-integer element")]
    fn array_shape_integer_draft4(instance: &Value, expected: bool) {
        let schema = json!({"type": "array", "items": {"type": "integer"}});
        if expected {
            tests_util::is_valid_with_draft4(&schema, instance);
        } else {
            tests_util::is_not_valid_with_draft4(&schema, instance);
        }
    }

    // Absorbed keywords keep their own schema location in errors.
    #[test_case(&json!({"type": "array", "items": {"type": "number"}}), &json!("x"), "/type"; "type location")]
    #[test_case(&json!({"type": "array", "minItems": 2, "items": {"type": "number"}}), &json!([1]), "/minItems"; "min location")]
    #[test_case(&json!({"type": "array", "maxItems": 1, "items": {"type": "number"}}), &json!([1, 2]), "/maxItems"; "max location")]
    #[test_case(&json!({"type": "array", "items": {"type": "number"}}), &json!([1, "x"]), "/items/type"; "element location")]
    fn array_shape_schema_location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }

    // A non-integer length keyword blocks fusion so the standalone validator still reports it.
    #[test]
    fn array_shape_invalid_length_keeps_error() {
        let schema = json!({"type": "array", "minItems": 1.5, "items": {"type": "number"}});
        assert!(crate::validator_for(&schema).is_err());
    }

    #[test]
    fn array_shape_yields_to_custom_length_keyword() {
        struct Accept;

        impl<'i> crate::Keyword<'i> for Accept {
            fn validate(&self, _: &'i Value) -> Result<(), crate::ValidationError<'i>> {
                Ok(())
            }

            fn is_valid(&self, _: &'i Value) -> bool {
                true
            }
        }

        let schema = json!({"type": "array", "minItems": 3, "items": {"type": "number"}});
        let validator = crate::options()
            .with_keyword("minItems", |_, _, _| Ok(Box::new(Accept)))
            .build(&schema)
            .unwrap();

        // The custom keyword owns `minItems`; the built-in bound must not also reject.
        assert!(validator.is_valid(&json!([1])));
        assert_eq!(validator.iter_errors(&json!([1])).count(), 0);
        // `type` and `items` still apply.
        assert!(!validator.is_valid(&json!([1, "x"])));
    }

    #[test]
    fn simple_type_items_respects_disabled_validation_vocabulary() {
        let meta = json!({
            "$id": "json-schema:///meta/no-validation",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$vocabulary": {
                "https://json-schema.org/draft/2020-12/vocab/core": true,
                "https://json-schema.org/draft/2020-12/vocab/applicator": true,
                "https://json-schema.org/draft/2020-12/vocab/validation": false
            }
        });
        let registry = crate::Registry::new()
            .add("json-schema:///meta/no-validation", &meta)
            .unwrap()
            .prepare()
            .unwrap();
        let schema = json!({
            "$schema": "json-schema:///meta/no-validation",
            "type": "array",
            "items": {"type": "integer"}
        });
        let validator = crate::options()
            .with_registry(&registry)
            .build(&schema)
            .unwrap();
        assert!(validator.is_valid(&json!([1, "x"])));
    }

    #[test]
    fn array_shape_respects_disabled_applicator_vocabulary() {
        let meta = json!({
            "$id": "json-schema:///meta/no-applicator",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$vocabulary": {
                "https://json-schema.org/draft/2020-12/vocab/core": true,
                "https://json-schema.org/draft/2020-12/vocab/validation": true
            }
        });
        let registry = crate::Registry::new()
            .add("json-schema:///meta/no-applicator", &meta)
            .unwrap()
            .prepare()
            .unwrap();
        let schema = json!({
            "$schema": "json-schema:///meta/no-applicator",
            "type": "array",
            "minItems": 2,
            "items": {"type": "integer"}
        });
        let validator = crate::options()
            .with_registry(&registry)
            .build(&schema)
            .unwrap();
        // `items` is inert, while `type` and `minItems` still apply.
        let verdicts: Vec<_> = [json!("x"), json!([1]), json!(["a", "b"])]
            .iter()
            .map(|instance| {
                (
                    validator.is_valid(instance),
                    validator.evaluate(instance).flag().valid,
                )
            })
            .collect();
        assert_eq!(verdicts, [(false, false), (false, false), (true, true)]);
    }

    fn parse_json(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    // Specialized string type validator tests
    #[test_case(r#"{"items": {"type": "string"}}"#, r#"["a", "b", "c"]"#, true; "all strings valid")]
    #[test_case(r#"{"items": {"type": "string"}}"#, r#"["a", 1, "c"]"#, false; "mixed with number invalid")]
    #[test_case(r#"{"items": {"type": "string"}}"#, r"[]", true; "empty array valid")]
    #[test_case(r#"{"items": {"type": "string"}}"#, r#"[""]"#, true; "empty string valid")]
    #[test_case(r#"{"items": {"type": "string"}}"#, r"[null]", false; "null invalid")]
    #[test_case(r#"{"items": {"type": "string"}}"#, r"[true]", false; "boolean invalid")]
    fn items_string_type(schema_json: &str, instance_json: &str, expected: bool) {
        let schema = parse_json(schema_json);
        let instance = parse_json(instance_json);
        if expected {
            tests_util::is_valid(&schema, &instance);
        } else {
            tests_util::is_not_valid(&schema, &instance);
        }
    }

    // Specialized number type validator tests
    #[test_case(r#"{"items": {"type": "number"}}"#, r"[1, 2.5, -3]", true; "all numbers valid")]
    #[test_case(r#"{"items": {"type": "number"}}"#, r#"[1, "2", 3]"#, false; "mixed with string invalid")]
    #[test_case(r#"{"items": {"type": "number"}}"#, r"[]", true; "empty array valid")]
    #[test_case(r#"{"items": {"type": "number"}}"#, r"[0]", true; "zero valid")]
    #[test_case(r#"{"items": {"type": "number"}}"#, r"[1.0]", true; "float valid")]
    #[test_case(r#"{"items": {"type": "number"}}"#, r"[null]", false; "null invalid")]
    #[test_case(r#"{"items": {"type": "number"}}"#, r"[9223372036854775807]", true; "i64 max valid")]
    #[test_case(r#"{"items": {"type": "number"}}"#, r"[-9223372036854775808]", true; "i64 min valid")]
    #[test_case(r#"{"items": {"type": "number"}}"#, r"[18446744073709551615]", true; "u64 max valid")]
    fn items_number_type(schema_json: &str, instance_json: &str, expected: bool) {
        let schema = parse_json(schema_json);
        let instance = parse_json(instance_json);
        if expected {
            tests_util::is_valid(&schema, &instance);
        } else {
            tests_util::is_not_valid(&schema, &instance);
        }
    }

    // Specialized boolean type validator tests
    #[test_case(r#"{"items": {"type": "boolean"}}"#, r"[true, false]", true; "all booleans valid")]
    #[test_case(r#"{"items": {"type": "boolean"}}"#, r"[true, 1]", false; "mixed with number invalid")]
    #[test_case(r#"{"items": {"type": "boolean"}}"#, r"[]", true; "empty array valid")]
    #[test_case(r#"{"items": {"type": "boolean"}}"#, r"[null]", false; "null invalid")]
    #[test_case(r#"{"items": {"type": "boolean"}}"#, r#"["true"]"#, false; "string true invalid")]
    fn items_boolean_type(schema_json: &str, instance_json: &str, expected: bool) {
        let schema = parse_json(schema_json);
        let instance = parse_json(instance_json);
        if expected {
            tests_util::is_valid(&schema, &instance);
        } else {
            tests_util::is_not_valid(&schema, &instance);
        }
    }

    // Specialized integer type validator tests (Draft 7+ semantics: 1.0 is integer)
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1, 2, 3]", true; "d7 all integers valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1, 2.5, 3]", false; "d7 float invalid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[]", true; "d7 empty array valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[0]", true; "d7 zero valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-42]", true; "d7 negative valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1.0]", true; "d7 1.0 is integer")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[42.0]", true; "d7 42.0 is integer")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-42.0]", true; "d7 neg 42.0 is integer")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[null]", false; "d7 null invalid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r#"["1"]"#, false; "d7 string invalid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[9223372036854775807]", true; "d7 i64 max valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-9223372036854775808]", true; "d7 i64 min valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[18446744073709551615]", true; "d7 u64 max valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1e10]", true; "d7 scientific notation integer")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1e-10]", false; "d7 scientific small not integer")]
    fn items_integer_type_draft7(schema_json: &str, instance_json: &str, expected: bool) {
        let schema = parse_json(schema_json);
        let instance = parse_json(instance_json);
        if expected {
            tests_util::is_valid(&schema, &instance);
        } else {
            tests_util::is_not_valid(&schema, &instance);
        }
    }

    // Draft 4 integer semantics: 1.0 is NOT an integer
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1, 2, 3]", true; "d4 all integers valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1, 2.5, 3]", false; "d4 float invalid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[]", true; "d4 empty array valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1.0]", false; "d4 1.0 is NOT integer")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[42.0]", false; "d4 42.0 is NOT integer")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-42.0]", false; "d4 neg 42.0 is NOT integer")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[9223372036854775807]", true; "d4 i64 max valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-9223372036854775808]", true; "d4 i64 min valid")]
    #[test_case(r#"{"items": {"type": "integer"}}"#, r"[18446744073709551615]", true; "d4 u64 max valid")]
    fn items_integer_type_draft4(schema_json: &str, instance_json: &str, expected: bool) {
        let schema = parse_json(schema_json);
        let instance = parse_json(instance_json);
        if expected {
            tests_util::is_valid_with_draft4(&schema, &instance);
        } else {
            tests_util::is_not_valid_with_draft4(&schema, &instance);
        }
    }

    #[cfg(feature = "arbitrary-precision")]
    mod arbitrary_precision {
        use crate::tests_util;
        use serde_json::Value;
        use test_case::test_case;

        fn parse_json(s: &str) -> Value {
            serde_json::from_str(s).unwrap()
        }

        // Draft 7+ with huge integers
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[18446744073709551616]", true; "u64 max plus 1")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[18446744073709551616.0]", true; "u64 max plus 1 with .0")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[99999999999999999999]", true; "huge plain integer")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[99999999999999999999.0]", true; "huge integer with .0")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-18446744073709551616]", true; "negative huge")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-18446744073709551616.0]", true; "negative huge with .0")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[18446744073709551616.5]", false; "huge decimal")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[99999999999999999999.5]", false; "huge float")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1e1000]", true; "huge scientific notation")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1e1000001]", false; "infinity positive")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-1e1000001]", false; "infinity negative")]
        fn items_integer_huge_draft7(schema_json: &str, instance_json: &str, expected: bool) {
            let schema = parse_json(schema_json);
            let instance = parse_json(instance_json);
            if expected {
                tests_util::is_valid(&schema, &instance);
            } else {
                tests_util::is_not_valid(&schema, &instance);
            }
        }

        // Draft 4 with huge integers (stricter: .0 is NOT integer)
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[18446744073709551616]", true; "u64 max plus 1")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[18446744073709551616.0]", false; "u64 max plus 1 with .0 NOT integer")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[99999999999999999999]", true; "huge plain integer")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[99999999999999999999.0]", false; "huge integer with .0 NOT integer")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-18446744073709551616]", true; "negative huge")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[-18446744073709551616.0]", false; "negative huge with .0 NOT integer")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[18446744073709551616.5]", false; "huge decimal")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1e1000]", false; "huge scientific notation is not an integer in draft4")]
        #[test_case(r#"{"items": {"type": "integer"}}"#, r"[1e1000001]", false; "scientific notation past f64 is not an integer in draft4")]
        fn items_integer_huge_draft4(schema_json: &str, instance_json: &str, expected: bool) {
            let schema = parse_json(schema_json);
            let instance = parse_json(instance_json);
            if expected {
                tests_util::is_valid_with_draft4(&schema, &instance);
            } else {
                tests_util::is_not_valid_with_draft4(&schema, &instance);
            }
        }

        // Huge numbers for number type (all should be valid)
        #[test_case(r#"{"items": {"type": "number"}}"#, r"[18446744073709551616]", true; "huge int valid as number")]
        #[test_case(r#"{"items": {"type": "number"}}"#, r"[18446744073709551616.0]", true; "huge .0 valid as number")]
        #[test_case(r#"{"items": {"type": "number"}}"#, r"[18446744073709551616.5]", true; "huge float valid as number")]
        #[test_case(r#"{"items": {"type": "number"}}"#, r"[1e10000]", true; "infinity valid as number")]
        fn items_number_huge(schema_json: &str, instance_json: &str, expected: bool) {
            let schema = parse_json(schema_json);
            let instance = parse_json(instance_json);
            if expected {
                tests_util::is_valid(&schema, &instance);
            } else {
                tests_util::is_not_valid(&schema, &instance);
            }
        }
    }

    #[test]
    fn array_shape_absolute_keyword_locations() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/s.json",
            "type": "array",
            "minItems": 5,
            "maxItems": 1,
            "items": {"type": "string"}
        });
        tests_util::assert_absolute_keyword_locations(
            &schema,
            &json!([1]),
            &[
                ("minItems", "https://example.com/s.json#/minItems"),
                ("type", "https://example.com/s.json#/items/type"),
            ],
        );
        tests_util::assert_absolute_keyword_locations(
            &schema,
            &json!(["a", "b"]),
            &[
                ("minItems", "https://example.com/s.json#/minItems"),
                ("maxItems", "https://example.com/s.json#/maxItems"),
            ],
        );
        tests_util::assert_absolute_keyword_locations(
            &schema,
            &json!(1),
            &[("type", "https://example.com/s.json#/type")],
        );
    }

    #[test_case(
        &json!([1]),
        &[
            ("[1] has less than 2 items", "", "/minItems"),
            ("1 is not of type \"string\"", "/0", "/items/type"),
        ];
        "minItems with items"
    )]
    #[test_case(
        &json!(["a", "b", "c", "d"]),
        &[("[\"a\",\"b\",\"c\",\"d\"] has more than 3 items", "", "/maxItems")];
        "maxItems"
    )]
    #[test_case(&json!(1), &[("1 is not of type \"array\"", "", "/type")]; "type")]
    fn array_shape_error_locations(instance: &Value, expected: &[(&str, &str, &str)]) {
        tests_util::assert_error_locations(
            &json!({"type": "array", "minItems": 2, "maxItems": 3, "items": {"type": "string"}}),
            instance,
            expected,
        );
    }
}
