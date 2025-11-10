use referencing::Draft;
use serde_json::{Map, Value};

use crate::{
    compiler,
    node::SchemaNode,
    paths::{LazyLocation, Location},
    validator::Validate,
    ValidationError,
};

use super::CompilationResult;

pub(crate) trait ItemsFilter: Send + Sync + Sized + 'static {
    fn new<'a>(
        ctx: &'a compiler::Context<'a>,
        parent: &'a Map<String, Value>,
    ) -> Result<Self, ValidationError<'a>>;
    fn unevaluated(&self) -> Option<&SchemaNode>;

    fn is_valid(&self, instance: &Value) -> bool {
        self.unevaluated()
            .as_ref()
            .is_some_and(|u| u.is_valid(instance))
    }

    fn mark_evaluated_indexes(&self, instance: &Value, indexes: &mut Vec<bool>);
}

pub(crate) struct UnevaluatedItemsValidator<F: ItemsFilter> {
    location: Location,
    filter: F,
}

impl<F: ItemsFilter> UnevaluatedItemsValidator<F> {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &'a compiler::Context,
        parent: &'a Map<String, Value>,
    ) -> CompilationResult<'a> {
        Ok(Box::new(UnevaluatedItemsValidator {
            location: ctx.location().join("unevaluatedItems"),
            filter: F::new(ctx, parent)?,
        }))
    }
}

impl<F: ItemsFilter> Validate for UnevaluatedItemsValidator<F> {
    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Array(items) = instance {
            // NOTE: It could be a packed bitset instead
            let mut indexes = vec![false; items.len()];
            self.filter.mark_evaluated_indexes(instance, &mut indexes);

            for (item, is_evaluated) in items.iter().zip(indexes) {
                if !is_evaluated && !self.filter.is_valid(item) {
                    return false;
                }
            }
        }
        true
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Array(items) = instance {
            // NOTE: It could be a packed bitset instead
            let mut indexes = vec![false; items.len()];
            self.filter.mark_evaluated_indexes(instance, &mut indexes);
            let mut unevaluated = vec![];
            for (item, is_evaluated) in items.iter().zip(indexes) {
                if !is_evaluated && !self.filter.is_valid(item) {
                    unevaluated.push(item.to_string());
                }
            }
            if !unevaluated.is_empty() {
                return Err(ValidationError::unevaluated_items(
                    self.location.clone(),
                    location.into(),
                    instance,
                    unevaluated,
                ));
            }
        }
        Ok(())
    }
}

struct Draft2019ItemsFilter {
    unevaluated: Option<SchemaNode>,
    contains: Option<SchemaNode>,
    ref_: Option<Box<Self>>,
    recursive_ref: Option<Box<Self>>,
    items: Option<usize>,
    conditional: Option<Box<ConditionalFilter<Self>>>,
    all_of: Option<CombinatorFilter<Self>>,
    any_of: Option<CombinatorFilter<Self>>,
    one_of: Option<CombinatorFilter<Self>>,
}

impl ItemsFilter for Draft2019ItemsFilter {
    fn new<'a>(
        ctx: &'a compiler::Context<'_>,
        parent: &'a Map<String, Value>,
    ) -> Result<Self, ValidationError<'a>> {
        let mut ref_ = None;

        if let Some(Value::String(reference)) = parent.get("$ref") {
            let resolved = ctx.lookup(reference)?;
            if let Value::Object(subschema) = resolved.contents() {
                ref_ = Some(Box::new(
                    Self::new(ctx, subschema).map_err(ValidationError::to_owned)?,
                ));
            }
        }
        let mut recursive_ref = None;

        if parent.contains_key("$recursiveRef") {
            let resolved = ctx.lookup_recursive_reference()?;
            if let Value::Object(subschema) = resolved.contents() {
                recursive_ref = Some(Box::new(
                    Self::new(ctx, subschema).map_err(ValidationError::to_owned)?,
                ));
            }
        }

        let mut conditional = None;

        if let Some(subschema) = parent.get("if") {
            if let Value::Object(if_parent) = subschema {
                let if_ctx = ctx.new_at_location("if");
                let mut then_ = None;
                if let Some(Value::Object(subschema)) = parent.get("then") {
                    let then_ctx = ctx.new_at_location("then");
                    then_ =
                        Some(Self::new(&then_ctx, subschema).map_err(ValidationError::to_owned)?);
                }
                let mut else_ = None;
                if let Some(Value::Object(subschema)) = parent.get("else") {
                    let else_ctx = ctx.new_at_location("else");
                    else_ =
                        Some(Self::new(&else_ctx, subschema).map_err(ValidationError::to_owned)?);
                }
                conditional = Some(Box::new(ConditionalFilter {
                    condition: compiler::compile(&if_ctx, if_ctx.as_resource_ref(subschema))
                        .map_err(ValidationError::to_owned)?,
                    if_: Self::new(&if_ctx, if_parent).map_err(ValidationError::to_owned)?,
                    then_,
                    else_,
                }));
            }
        }

        let mut contains = None;
        if let Some(subschema) = parent.get("contains") {
            let contains_ctx = ctx.new_at_location("contains");
            contains = Some(
                compiler::compile(&contains_ctx, contains_ctx.as_resource_ref(subschema))
                    .map_err(ValidationError::to_owned)?,
            );
        }
        let mut unevaluated = None;
        if let Some(subschema) = parent.get("unevaluatedItems") {
            let unevaluated_ctx = ctx.new_at_location("unevaluatedItems");
            unevaluated = Some(
                compiler::compile(&unevaluated_ctx, unevaluated_ctx.as_resource_ref(subschema))
                    .map_err(ValidationError::to_owned)?,
            );
        }
        let mut all_of = None;
        if let Some(Some(subschemas)) = parent.get("allOf").map(Value::as_array) {
            let all_of_ctx = ctx.new_at_location("allOf");
            all_of = Some(
                CombinatorFilter::new(&all_of_ctx, subschemas)
                    .map_err(ValidationError::to_owned)?,
            );
        }
        let mut any_of = None;
        if let Some(Some(subschemas)) = parent.get("anyOf").map(Value::as_array) {
            let any_of_ctx = ctx.new_at_location("anyOf");
            any_of = Some(
                CombinatorFilter::new(&any_of_ctx, subschemas)
                    .map_err(ValidationError::to_owned)?,
            );
        }
        let mut one_of = None;
        if let Some(Some(subschemas)) = parent.get("oneOf").map(Value::as_array) {
            let one_of_ctx = ctx.new_at_location("oneOf");
            one_of = Some(
                CombinatorFilter::new(&one_of_ctx, subschemas)
                    .map_err(ValidationError::to_owned)?,
            );
        }
        let mut items = None;
        if let Some(subschema) = parent.get("items") {
            let limit = if parent.contains_key("additionalItems") || subschema.is_object() {
                usize::MAX
            } else {
                subschema
                    .as_array()
                    .expect("Items are either an object or an array")
                    .len()
            };
            items = Some(limit);
        }

        Ok(Draft2019ItemsFilter {
            unevaluated,
            contains,
            ref_,
            recursive_ref,
            items,
            conditional,
            all_of,
            any_of,
            one_of,
        })
    }
    fn unevaluated(&self) -> Option<&SchemaNode> {
        self.unevaluated.as_ref()
    }
    fn mark_evaluated_indexes(&self, instance: &Value, indexes: &mut Vec<bool>) {
        if let Some(limit) = self.items {
            for idx in indexes.iter_mut().take(limit) {
                *idx = true;
            }
        }

        if let Some(ref_) = &self.ref_ {
            ref_.mark_evaluated_indexes(instance, indexes);
        }

        if let Some(recursive_ref) = &self.recursive_ref {
            recursive_ref.mark_evaluated_indexes(instance, indexes);
        }

        if let Some(conditional) = &self.conditional {
            conditional.mark_evaluated_indexes(instance, indexes);
        }
        if let Value::Array(items) = instance {
            for (item, is_evaluated) in items.iter().zip(indexes.iter_mut()) {
                if *is_evaluated {
                    continue;
                }
                if let Some(validator) = &self.contains {
                    if validator.is_valid(item) {
                        *is_evaluated = true;
                        continue;
                    }
                }
                if let Some(validator) = &self.unevaluated {
                    if validator.is_valid(item) {
                        *is_evaluated = true;
                    }
                }
            }
        }

        if let Some(combinator) = &self.all_of {
            if combinator
                .subschemas
                .iter()
                .all(|(v, _)| v.is_valid(instance))
            {
                combinator.mark_evaluated_indexes(instance, indexes);
            }
        }

        if let Some(combinator) = &self.any_of {
            if combinator
                .subschemas
                .iter()
                .all(|(v, _)| v.is_valid(instance))
            {
                combinator.mark_evaluated_indexes(instance, indexes);
            }
        }

        if let Some(combinator) = &self.one_of {
            if combinator
                .subschemas
                .iter()
                .filter(|(v, _)| v.is_valid(instance))
                .count()
                == 1
            {
                combinator.mark_evaluated_indexes(instance, indexes);
            }
        }
    }
}

struct DefaultItemsFilter {
    unevaluated: Option<SchemaNode>,
    contains: Option<SchemaNode>,
    ref_: Option<Box<Self>>,
    dynamic_ref: Option<Box<Self>>,
    items: bool,
    prefix_items: Option<usize>,
    conditional: Option<Box<ConditionalFilter<Self>>>,
    all_of: Option<CombinatorFilter<Self>>,
    any_of: Option<CombinatorFilter<Self>>,
    one_of: Option<CombinatorFilter<Self>>,
}

impl ItemsFilter for DefaultItemsFilter {
    fn new<'a>(
        ctx: &'a compiler::Context<'a>,
        parent: &'a Map<String, Value>,
    ) -> Result<DefaultItemsFilter, ValidationError<'a>> {
        let mut ref_ = None;

        if let Some(Value::String(reference)) = parent.get("$ref") {
            let resolved = ctx.lookup(reference)?;
            if let Value::Object(subschema) = resolved.contents() {
                ref_ = Some(Box::new(
                    Self::new(ctx, subschema).map_err(ValidationError::to_owned)?,
                ));
            }
        }

        let mut dynamic_ref = None;

        if let Some(Value::String(reference)) = parent.get("$dynamicRef") {
            let resolved = ctx.lookup(reference)?;
            if let Value::Object(subschema) = resolved.contents() {
                dynamic_ref = Some(Box::new(
                    Self::new(ctx, subschema).map_err(ValidationError::to_owned)?,
                ));
            }
        }

        let mut conditional = None;

        if let Some(subschema) = parent.get("if") {
            if let Value::Object(if_parent) = subschema {
                let if_ctx = ctx.new_at_location("if");
                let mut then_ = None;
                if let Some(Value::Object(subschema)) = parent.get("then") {
                    let then_ctx = ctx.new_at_location("then");
                    then_ =
                        Some(Self::new(&then_ctx, subschema).map_err(ValidationError::to_owned)?);
                }
                let mut else_ = None;
                if let Some(Value::Object(subschema)) = parent.get("else") {
                    let else_ctx = ctx.new_at_location("else");
                    else_ =
                        Some(Self::new(&else_ctx, subschema).map_err(ValidationError::to_owned)?);
                }
                conditional = Some(Box::new(ConditionalFilter {
                    condition: compiler::compile(&if_ctx, if_ctx.as_resource_ref(subschema))
                        .map_err(ValidationError::to_owned)?,
                    if_: Self::new(&if_ctx, if_parent).map_err(ValidationError::to_owned)?,
                    then_,
                    else_,
                }));
            }
        }

        let mut prefix_items = None;
        if let Some(Some(items)) = parent.get("prefixItems").map(Value::as_array) {
            prefix_items = Some(items.len());
        }

        let mut contains = None;
        if let Some(subschema) = parent.get("contains") {
            let contains_ctx = ctx.new_at_location("contains");
            contains = Some(
                compiler::compile(&contains_ctx, contains_ctx.as_resource_ref(subschema))
                    .map_err(ValidationError::to_owned)?,
            );
        }
        let mut unevaluated = None;
        if let Some(subschema) = parent.get("unevaluatedItems") {
            let unevaluated_ctx = ctx.new_at_location("unevaluatedItems");
            unevaluated = Some(
                compiler::compile(&unevaluated_ctx, unevaluated_ctx.as_resource_ref(subschema))
                    .map_err(ValidationError::to_owned)?,
            );
        }
        let mut all_of = None;
        if let Some(Some(subschemas)) = parent.get("allOf").map(Value::as_array) {
            let all_of_ctx = ctx.new_at_location("allOf");
            all_of = Some(
                CombinatorFilter::new(&all_of_ctx, subschemas)
                    .map_err(ValidationError::to_owned)?,
            );
        }
        let mut any_of = None;
        if let Some(Some(subschemas)) = parent.get("anyOf").map(Value::as_array) {
            let any_of_ctx = ctx.new_at_location("anyOf");
            any_of = Some(
                CombinatorFilter::new(&any_of_ctx, subschemas)
                    .map_err(ValidationError::to_owned)?,
            );
        }

        let mut one_of = None;
        if let Some(Some(subschemas)) = parent.get("oneOf").map(Value::as_array) {
            let one_of_ctx = ctx.new_at_location("oneOf");
            one_of = Some(
                CombinatorFilter::new(&one_of_ctx, subschemas)
                    .map_err(ValidationError::to_owned)?,
            );
        }

        Ok(DefaultItemsFilter {
            unevaluated,
            contains,
            ref_,
            dynamic_ref,
            items: parent.contains_key("items"),
            prefix_items,
            conditional,
            all_of,
            any_of,
            one_of,
        })
    }
    fn unevaluated(&self) -> Option<&SchemaNode> {
        self.unevaluated.as_ref()
    }

    fn mark_evaluated_indexes(&self, instance: &Value, indexes: &mut Vec<bool>) {
        if self.items {
            for idx in indexes {
                *idx = true;
            }
            return;
        }

        if let Some(ref_) = &self.ref_ {
            ref_.mark_evaluated_indexes(instance, indexes);
        }

        if let Some(dynamic_ref) = &self.dynamic_ref {
            dynamic_ref.mark_evaluated_indexes(instance, indexes);
        }

        if let Some(limit) = self.prefix_items {
            for idx in indexes.iter_mut().take(limit) {
                *idx = true;
            }
        }
        if let Some(conditional) = &self.conditional {
            conditional.mark_evaluated_indexes(instance, indexes);
        }
        if let Value::Array(items) = instance {
            for (item, is_evaluated) in items.iter().zip(indexes.iter_mut()) {
                if *is_evaluated {
                    continue;
                }
                if let Some(validator) = &self.contains {
                    if validator.is_valid(item) {
                        *is_evaluated = true;
                        continue;
                    }
                }
                if let Some(validator) = &self.unevaluated {
                    if validator.is_valid(item) {
                        *is_evaluated = true;
                    }
                }
            }
        }

        if let Some(combinator) = &self.all_of {
            if combinator
                .subschemas
                .iter()
                .all(|(v, _)| v.is_valid(instance))
            {
                combinator.mark_evaluated_indexes(instance, indexes);
            }
        }

        if let Some(combinator) = &self.any_of {
            if combinator
                .subschemas
                .iter()
                .all(|(v, _)| v.is_valid(instance))
            {
                combinator.mark_evaluated_indexes(instance, indexes);
            }
        }

        if let Some(combinator) = &self.one_of {
            if combinator
                .subschemas
                .iter()
                .filter(|(v, _)| v.is_valid(instance))
                .count()
                == 1
            {
                combinator.mark_evaluated_indexes(instance, indexes);
            }
        }
    }
}

struct CombinatorFilter<F> {
    subschemas: Vec<(SchemaNode, F)>,
}

impl<F: ItemsFilter> CombinatorFilter<F> {
    fn mark_evaluated_indexes(&self, instance: &Value, indexes: &mut Vec<bool>) {
        for (_, subschema) in &self.subschemas {
            subschema.mark_evaluated_indexes(instance, indexes);
        }
    }
}

impl<F: ItemsFilter> CombinatorFilter<F> {
    fn new<'a>(
        ctx: &'a compiler::Context<'a>,
        subschemas: &'a [Value],
    ) -> Result<CombinatorFilter<F>, ValidationError<'a>> {
        let mut buffer = Vec::with_capacity(subschemas.len());
        for (idx, subschema) in subschemas.iter().enumerate() {
            if let Value::Object(parent) = subschema {
                let subschema_ctx = ctx.new_at_location(idx);
                buffer.push((
                    compiler::compile(&subschema_ctx, subschema_ctx.as_resource_ref(subschema))
                        .map_err(ValidationError::to_owned)?,
                    F::new(&subschema_ctx, parent).map_err(ValidationError::to_owned)?,
                ));
            }
        }
        Ok(CombinatorFilter { subschemas: buffer })
    }
}

struct ConditionalFilter<F> {
    condition: SchemaNode,
    if_: F,
    then_: Option<F>,
    else_: Option<F>,
}

impl<F: ItemsFilter> ConditionalFilter<F> {
    fn mark_evaluated_indexes(&self, instance: &Value, indexes: &mut Vec<bool>) {
        if self.condition.is_valid(instance) {
            self.if_.mark_evaluated_indexes(instance, indexes);
            if let Some(then_) = &self.then_ {
                then_.mark_evaluated_indexes(instance, indexes);
            }
        } else if let Some(else_) = &self.else_ {
            else_.mark_evaluated_indexes(instance, indexes);
        }
    }
}

pub(crate) fn compile<'a>(
    ctx: &'a compiler::Context,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    match schema.as_bool() {
        Some(true) => None,
        _ => {
            if ctx.draft() == Draft::Draft201909 {
                Some(UnevaluatedItemsValidator::<Draft2019ItemsFilter>::compile(
                    ctx, parent,
                ))
            } else {
                Some(UnevaluatedItemsValidator::<DefaultItemsFilter>::compile(
                    ctx, parent,
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

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
}
