use crate::{
    compiler,
    error::ValidationError,
    evaluation::{Annotations, ChildList, ErrorDescription},
    keywords::CompilationResult,
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    validator::{EvaluationResult, Validate, ValidationContext},
    Array, Draft, Json, Node, SerdeJson,
};
use referencing::{Uri, Vocabulary};
use serde_json::{Map, Value};
use std::sync::Arc;

use super::helpers::map_get_u64;

/// How many items a `contains` failure needed.
#[derive(Clone, Copy)]
enum Bound {
    /// At least one, without `minContains`.
    Any,
    Min(u64),
    Max(u64),
}

/// The keyword a `contains` failure is reported at.
struct Site {
    location: Location,
    absolute_location: Option<Arc<Uri<String>>>,
}

impl Site {
    fn new<F: Json>(ctx: &compiler::Context<F>, keyword: &str) -> Self {
        let location = ctx.location().join(keyword);
        Site {
            absolute_location: ctx.absolute_location(&location),
            location,
        }
    }

    #[cold]
    #[inline(never)]
    fn error<'i, F: Json>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        bound: Bound,
    ) -> ValidationError<'i> {
        let schema_path = self.location.clone();
        let evaluation_path = crate::paths::capture_evaluation_path(tracker, &self.location);
        let instance_path = location.into();
        let instance = instance.lazy_value();
        match bound {
            Bound::Any => {
                ValidationError::contains(schema_path, evaluation_path, instance_path, instance)
            }
            Bound::Min(limit) => ValidationError::min_contains(
                schema_path,
                evaluation_path,
                instance_path,
                instance,
                limit,
            ),
            Bound::Max(limit) => ValidationError::max_contains(
                schema_path,
                evaluation_path,
                instance_path,
                instance,
                limit,
            ),
        }
        .with_absolute_keyword_location(self.absolute_location.clone())
    }
}

pub(crate) struct ContainsValidator<F: Json = SerdeJson> {
    node: SchemaNode<F>,
}

impl ContainsValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
    ) -> CompilationResult<'a, F> {
        let ctx = ctx.new_at_location("contains");
        Ok(Box::new(ContainsValidator {
            node: compiler::compile(&ctx, ctx.as_resource_ref(schema))?,
        }))
    }
}

impl<F: Json> Validate<F> for ContainsValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            array
                .elements()
                .any(|item| self.node.is_valid_child(&item, ctx))
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
            if array.elements().any(|item| self.node.is_valid(&item, ctx)) {
                return Ok(());
            }
            let loc = self.node.location();
            Err(ValidationError::contains(
                loc.clone(),
                crate::paths::capture_evaluation_path(tracker, loc),
                location.into(),
                instance.lazy_value(),
            ))
        } else {
            Ok(())
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
            let mut results = ChildList::default();
            let mut indices = Vec::with_capacity(array.len());
            for (idx, item) in array.elements().enumerate() {
                let path = location.push(idx);
                let result = self
                    .node
                    .evaluate_instance_below(&item, &path, tracker, ctx);
                if result.valid {
                    indices.push(idx);
                    results.push(ctx.arena(), result);
                }
            }
            if indices.is_empty() {
                let loc = self.node.location();
                let eval_path = crate::paths::capture_evaluation_path(tracker, loc);
                EvaluationResult::Invalid {
                    errors: vec![ErrorDescription::from_validation_error(
                        &ValidationError::contains(
                            loc.clone(),
                            eval_path,
                            location.into(),
                            instance.lazy_value(),
                        ),
                    )],
                    children: ChildList::default(),
                    annotations: None,
                }
            } else {
                EvaluationResult::Valid {
                    annotations: Some(Annotations::new(Value::from(indices))),
                    children: results,
                }
            }
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

/// The matching items' units and their indices, as plain `contains` reports them, failing with
/// `validator`'s error when the count of matches is out of its bounds.
fn evaluate_bounded<F: Json, V: Validate<F>>(
    validator: &V,
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
    let mut indices = Vec::new();
    for (idx, item) in array.elements().enumerate() {
        let child = node.evaluate_instance_below(&item, &location.push(idx), tracker, ctx);
        if child.valid {
            indices.push(idx);
            children.push(ctx.arena(), child);
        }
    }
    let annotations = (!indices.is_empty()).then(|| Annotations::new(Value::from(indices)));
    match validator.validate(instance, location, tracker, ctx) {
        Ok(()) => EvaluationResult::Valid {
            annotations,
            children,
        },
        Err(error) => EvaluationResult::Invalid {
            errors: vec![ErrorDescription::from_validation_error(&error)],
            children,
            annotations,
        },
    }
}

/// `minContains` validation. Used only if there is no `maxContains` present.
///
/// Docs: <https://json-schema.org/draft/2019-09/json-schema-validation.html#rfc.section.6.4.5>
pub(crate) struct MinContainsValidator<F: Json = SerdeJson> {
    node: SchemaNode<F>,
    min_contains: u64,
    min: Site,
}

impl MinContainsValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
        min_contains: u64,
    ) -> CompilationResult<'a, F> {
        let min = Site::new(ctx, "minContains");
        let ctx = ctx.new_at_location("contains");
        Ok(Box::new(MinContainsValidator {
            node: compiler::compile(&ctx, ctx.as_resource_ref(schema))?,
            min_contains,
            min,
        }))
    }
}

impl<F: Json> Validate<F> for MinContainsValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            let mut matches = 0;
            for item in array.elements() {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(&item, ctx))
                {
                    matches += 1;
                    if matches >= self.min_contains {
                        return true;
                    }
                }
            }
            self.min_contains == 0
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
            let mut matches = 0;
            for item in array.elements() {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(&item, ctx))
                {
                    matches += 1;
                    if matches >= self.min_contains {
                        return Ok(());
                    }
                }
            }
            if self.min_contains > 0 {
                Err(self
                    .min
                    .error::<F>(instance, location, tracker, Bound::Min(self.min_contains)))
            } else {
                Ok(())
            }
        } else {
            Ok(())
        }
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        evaluate_bounded(self, &self.node, instance, location, tracker, ctx)
    }
}

/// `maxContains` validation. Used only if there is no `minContains` present.
///
/// Docs: <https://json-schema.org/draft/2019-09/json-schema-validation.html#rfc.section.6.4.4>
pub(crate) struct MaxContainsValidator<F: Json = SerdeJson> {
    node: SchemaNode<F>,
    max_contains: u64,
    max: Site,
}

impl MaxContainsValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
        max_contains: u64,
    ) -> CompilationResult<'a, F> {
        let max = Site::new(ctx, "maxContains");
        let ctx = ctx.new_at_location("contains");
        Ok(Box::new(MaxContainsValidator {
            node: compiler::compile(&ctx, ctx.as_resource_ref(schema))?,
            max_contains,
            max,
        }))
    }
}

impl<F: Json> Validate<F> for MaxContainsValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            let mut matches = 0;
            for item in array.elements() {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(&item, ctx))
                {
                    matches += 1;
                    if matches > self.max_contains {
                        return false;
                    }
                }
            }
            matches != 0
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
            let mut matches = 0;
            for item in array.elements() {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(&item, ctx))
                {
                    matches += 1;
                    if matches > self.max_contains {
                        return Err(self.max.error::<F>(
                            instance,
                            location,
                            tracker,
                            Bound::Max(self.max_contains),
                        ));
                    }
                }
            }
            if matches > 0 {
                Ok(())
            } else {
                Err(self.max.error::<F>(instance, location, tracker, Bound::Any))
            }
        } else {
            Ok(())
        }
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        evaluate_bounded(self, &self.node, instance, location, tracker, ctx)
    }
}

/// `maxContains` & `minContains` validation combined.
///
/// Docs:
///   `maxContains` - <https://json-schema.org/draft/2019-09/json-schema-validation.html#rfc.section.6.4.4>
///   `minContains` - <https://json-schema.org/draft/2019-09/json-schema-validation.html#rfc.section.6.4.5>
pub(crate) struct MinMaxContainsValidator<F: Json = SerdeJson> {
    node: SchemaNode<F>,
    min_contains: u64,
    max_contains: u64,
    // Both bounds report against their own keyword, which the shared subschema location cannot name.
    min: Site,
    max: Site,
}

impl MinMaxContainsValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
        min_contains: u64,
        max_contains: u64,
    ) -> CompilationResult<'a, F> {
        let min = Site::new(ctx, "minContains");
        let max = Site::new(ctx, "maxContains");
        let ctx = ctx.new_at_location("contains");
        Ok(Box::new(MinMaxContainsValidator {
            node: compiler::compile(&ctx, ctx.as_resource_ref(schema))?,
            min_contains,
            max_contains,
            min,
            max,
        }))
    }
}

impl<F: Json> Validate<F> for MinMaxContainsValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(array) = instance.as_array() {
            let mut matches = 0;
            for item in array.elements() {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(&item, ctx))
                {
                    matches += 1;
                    if matches > self.max_contains {
                        return false;
                    }
                }
            }
            matches <= self.max_contains && matches >= self.min_contains
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
            let mut matches = 0;
            for item in array.elements() {
                if self
                    .node
                    .validators()
                    .all(|validator| validator.is_valid(&item, ctx))
                {
                    matches += 1;
                    if matches > self.max_contains {
                        return Err(self.max.error::<F>(
                            instance,
                            location,
                            tracker,
                            Bound::Max(self.max_contains),
                        ));
                    }
                }
            }
            if matches < self.min_contains {
                Err(self
                    .min
                    .error::<F>(instance, location, tracker, Bound::Min(self.min_contains)))
            } else {
                Ok(())
            }
        } else {
            Ok(())
        }
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        evaluate_bounded(self, &self.node, instance, location, tracker, ctx)
    }
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    match ctx.draft() {
        Draft::Draft4 | Draft::Draft6 | Draft::Draft7 => {
            Some(ContainsValidator::compile(ctx, schema))
        }
        Draft::Draft201909 | Draft::Draft202012 => compile_contains(ctx, parent, schema),
        _ => None,
    }
}

#[inline]
fn compile_contains<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    // `minContains` and `maxContains` belong to the validation vocabulary
    if !ctx.has_vocabulary(&Vocabulary::Validation) {
        return Some(ContainsValidator::compile(ctx, schema));
    }
    let min_contains = match map_get_u64(parent, ctx, "minContains").transpose() {
        Ok(n) => n,
        Err(err) => return Some(Err(err)),
    };
    let max_contains = match map_get_u64(parent, ctx, "maxContains").transpose() {
        Ok(n) => n,
        Err(err) => return Some(Err(err)),
    };

    match (min_contains, max_contains) {
        (Some(min), Some(max)) => Some(MinMaxContainsValidator::compile(ctx, schema, min, max)),
        (Some(min), None) => Some(MinContainsValidator::compile(ctx, schema, min)),
        (None, Some(max)) => Some(MaxContainsValidator::compile(ctx, schema, max)),
        (None, None) => Some(ContainsValidator::compile(ctx, schema)),
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    // `contains` annotates arrays only, with or without bounds.
    #[test_case(&json!({"contains": {"type": "integer"}}) ; "plain")]
    #[test_case(&json!({"contains": {"type": "integer"}, "minContains": 2}) ; "minContains")]
    #[test_case(&json!({"contains": {"type": "integer"}, "maxContains": 1}) ; "maxContains")]
    fn evaluate_non_array(schema: &Value) {
        assert_eq!(
            tests_util::evaluation_outline(schema, &json!(1)),
            [" @ ok", "  /contains @ ok"]
        );
    }

    // With `minContains` or `maxContains`, `contains` still reports the matching items and their
    // indices, as without them.
    #[test_case(
        &json!({"contains": {"type": "integer"}, "minContains": 1, "maxContains": 2}),
        &json!(["a", 1]),
        &[
            " @ ok",
            "  /contains @ ok ann=[1]",
            "    /contains @/1 ok",
            "      /contains/type @/1 ok",
        ] ; "both bounds met"
    )]
    #[test_case(
        &json!({"contains": {"type": "integer"}, "minContains": 2}),
        &json!(["a", 1]),
        &[
            " @ FAIL",
            "  /contains @ FAIL dropped=[1] err=minContains",
            "    /contains @/1 ok",
            "      /contains/type @/1 ok",
        ] ; "too few"
    )]
    #[test_case(
        &json!({"contains": {"type": "integer"}, "maxContains": 1}),
        &json!([1, 2]),
        &[
            " @ FAIL",
            "  /contains @ FAIL dropped=[0,1] err=maxContains",
            "    /contains @/0 ok",
            "      /contains/type @/0 ok",
            "    /contains @/1 ok",
            "      /contains/type @/1 ok",
        ] ; "too many"
    )]
    #[test_case(
        &json!({"contains": {"type": "integer"}, "minContains": 0}),
        &json!(["a"]),
        &[" @ ok", "  /contains @ ok"] ; "none needed, none matching"
    )]
    fn evaluate_with_bounds(schema: &Value, instance: &Value, expected: &[&str]) {
        assert_eq!(tests_util::evaluation_outline(schema, instance), expected);
    }

    #[test]
    fn location() {
        tests_util::assert_schema_location(
            &json!({"contains": {"const": 2}}),
            &json!([]),
            "/contains",
        );
    }

    #[test]
    fn subschema_does_not_replace_a_sibling_keyword() {
        let schema = json!({
            "items": {"type": "object"},
            "contains": {"items": {"type": "null"}},
            "minContains": 0,
            "maxContains": 3
        });
        tests_util::is_valid(&schema, &json!([{}]));
        tests_util::is_not_valid(&schema, &json!([null]));
    }

    // `minContains` and `maxContains` are inert, so `contains` asks for one match.
    #[test_case(&json!({}); "no bounds")]
    #[test_case(&json!({"minContains": 2}); "min")]
    #[test_case(&json!({"maxContains": 1}); "max")]
    #[test_case(&json!({"minContains": 2, "maxContains": 1}); "min and max")]
    #[test_case(&json!({"minContains": 0}); "min zero")]
    fn contains_bounds_without_validation_vocabulary(bounds: &Value) {
        let mut schema = json!({"contains": {"items": false}});
        schema
            .as_object_mut()
            .expect("object schema")
            .extend(bounds.as_object().expect("object bounds").clone());
        let instances = [json!([]), json!([[]]), json!([[], []]), json!([[1], []])];
        assert_eq!(
            tests_util::outcomes_with_only_vocabulary("applicator", &schema, &instances),
            [
                (
                    false,
                    vec![(
                        "/contains".to_string(),
                        "None of [] are valid under the given schema".to_string()
                    )]
                ),
                (true, Vec::new()),
                (true, Vec::new()),
                (true, Vec::new()),
            ]
        );
    }

    #[test_case(&json!({}), &json!([2]), "/contains", "None of [2] are valid under the given schema", "contains"; "contains")]
    #[test_case(&json!({"minContains": 2}), &json!([1]), "/minContains", "[1] has less than 2 items valid under the given schema", "minContains"; "minContains")]
    #[test_case(&json!({"maxContains": 1}), &json!([1, 1]), "/maxContains", "[1,1] has more than 1 item valid under the given schema", "maxContains"; "maxContains")]
    #[test_case(&json!({"maxContains": 1}), &json!([2]), "/maxContains", "None of [2] are valid under the given schema", "contains"; "maxContains without a match")]
    #[test_case(&json!({"minContains": 2, "maxContains": 3}), &json!([1]), "/minContains", "[1] has less than 2 items valid under the given schema", "minContains"; "min of both")]
    #[test_case(
        &json!({"minContains": 2, "maxContains": 3}),
        &json!([1, 1, 1, 1]),
        "/maxContains",
        "[1,1,1,1] has more than 3 items valid under the given schema",
        "maxContains";
        "max of both"
    )]
    fn errors(bounds: &Value, instance: &Value, schema_path: &str, message: &str, keyword: &str) {
        let mut schema = bounds.clone();
        schema["contains"] = json!({"const": 1});
        tests_util::assert_error_locations(&schema, instance, &[(message, "", schema_path)]);
        let error = crate::validator_for(&schema)
            .expect("valid schema")
            .validate(instance)
            .expect_err("invalid instance");
        assert_eq!(error.kind().keyword(), keyword);
    }

    #[test]
    fn malformed() {
        tests_util::assert_compile_error(
            &json!({"contains": 5}),
            "5 is not of types \"boolean\", \"object\"",
            "/contains",
        );
    }
}
