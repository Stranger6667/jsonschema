use crate::LazyInstance;
use std::borrow::Cow;

use crate::{
    compiler,
    error::ValidationError,
    keywords::{required, CompilationResult},
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
    Json, Node, Object, SerdeJson,
};
use serde_json::{Map, Value};

pub(crate) struct DependenciesValidator<F: Json = SerdeJson> {
    dependencies: Vec<(F::PreparedKey, SchemaNode<F>)>,
}

impl DependenciesValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
    ) -> CompilationResult<'a, F> {
        if let Value::Object(map) = schema {
            let kctx = ctx.new_at_location("dependencies");
            let mut dependencies = Vec::with_capacity(map.len());
            for (key, subschema) in map {
                let ctx = kctx.new_at_location(key.as_str());
                let s =
                    match subschema {
                        Value::Array(_) => {
                            let validators = vec![required::compile_with_path(
                                subschema,
                                kctx.location().clone(),
                            )
                            .expect("The required validator compilation does not return None")?];
                            SchemaNode::from_array(&kctx, validators)
                        }
                        _ => compiler::compile(&ctx, ctx.as_resource_ref(subschema))?,
                    };
                dependencies.push((F::prepare_key(key), s));
            }
            Ok(Box::new(DependenciesValidator { dependencies }))
        } else {
            let location = ctx.location().join("dependencies");
            Err(ValidationError::single_type_error(
                location.clone(),
                location,
                Location::new(),
                LazyInstance::Ready(Cow::Borrowed(schema)),
                JsonType::Object,
            ))
        }
    }
}

impl<F: Json> Validate<F> for DependenciesValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(object) = instance.as_object() {
            for (property, node) in &self.dependencies {
                if object.get(property).is_some() && !node.is_valid(instance, ctx) {
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
        if let Some(object) = instance.as_object() {
            for (property, dependency) in &self.dependencies {
                if object.get(property).is_some() {
                    dependency.validate(instance, location, tracker, ctx)?;
                }
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
        let Some(object) = instance.as_object() else {
            return;
        };
        for (property, node) in &self.dependencies {
            if object.get(property).is_some() {
                node.collect_errors(instance, location, tracker, ctx, errors);
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
        self.evaluate_with_location(instance, location, &location.into(), tracker, ctx)
    }

    fn evaluate_with_location(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: &Location,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Some(object) = instance.as_object() {
            let mut children = Vec::new();
            for (property, dependency) in &self.dependencies {
                if object.get(property).is_some() {
                    children.push(dependency.evaluate_instance_at(
                        instance,
                        location,
                        instance_location,
                        tracker,
                        ctx,
                    ));
                }
            }
            EvaluationResult::from_children(children)
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

pub(crate) struct DependentRequiredValidator<F: Json = SerdeJson> {
    dependencies: Vec<(F::PreparedKey, SchemaNode<F>)>,
}

impl DependentRequiredValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
    ) -> CompilationResult<'a, F> {
        if let Value::Object(map) = schema {
            let kctx = ctx.new_at_location("dependentRequired");
            let mut dependencies = Vec::with_capacity(map.len());
            for (key, subschema) in map {
                let ictx = kctx.new_at_location(key.as_str());
                if let Value::Array(dependency_array) = subschema {
                    if !crate::unique::is_unique(dependency_array) {
                        let location = ictx.location().clone();
                        return Err(ValidationError::unique_items(
                            location.clone(),
                            location,
                            Location::new(),
                            LazyInstance::Ready(Cow::Borrowed(subschema)),
                        ));
                    }
                    let validators =
                        vec![
                            required::compile_with_path(subschema, kctx.location().clone())
                                .expect(
                                    "The required validator compilation does not return None",
                                )?,
                        ];
                    dependencies.push((
                        F::prepare_key(key),
                        SchemaNode::from_array(&kctx, validators),
                    ));
                } else {
                    let location = ictx.location().clone();
                    return Err(ValidationError::single_type_error(
                        location.clone(),
                        location,
                        Location::new(),
                        LazyInstance::Ready(Cow::Borrowed(subschema)),
                        JsonType::Array,
                    ));
                }
            }
            Ok(Box::new(DependentRequiredValidator { dependencies }))
        } else {
            let location = ctx.location().join("dependentRequired");
            Err(ValidationError::single_type_error(
                location.clone(),
                location,
                Location::new(),
                LazyInstance::Ready(Cow::Borrowed(schema)),
                JsonType::Object,
            ))
        }
    }
}
impl<F: Json> Validate<F> for DependentRequiredValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(object) = instance.as_object() {
            for (property, node) in &self.dependencies {
                if object.get(property).is_some() && !node.is_valid(instance, ctx) {
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
        if let Some(object) = instance.as_object() {
            for (property, dependency) in &self.dependencies {
                if object.get(property).is_some() {
                    dependency.validate(instance, location, tracker, ctx)?;
                }
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
        let Some(object) = instance.as_object() else {
            return;
        };
        for (property, node) in &self.dependencies {
            if object.get(property).is_some() {
                node.collect_errors(instance, location, tracker, ctx, errors);
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
        self.evaluate_with_location(instance, location, &location.into(), tracker, ctx)
    }

    fn evaluate_with_location(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: &Location,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Some(object) = instance.as_object() {
            let mut children = Vec::new();
            for (property, dependency) in &self.dependencies {
                if object.get(property).is_some() {
                    children.push(dependency.evaluate_instance_at(
                        instance,
                        location,
                        instance_location,
                        tracker,
                        ctx,
                    ));
                }
            }
            EvaluationResult::from_children(children)
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

pub(crate) struct DependentSchemasValidator<F: Json = SerdeJson> {
    dependencies: Vec<(F::PreparedKey, SchemaNode<F>)>,
}
impl DependentSchemasValidator {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
    ) -> CompilationResult<'a, F> {
        if let Value::Object(map) = schema {
            let ctx = ctx.new_at_location("dependentSchemas");
            let mut dependencies = Vec::with_capacity(map.len());
            for (key, subschema) in map {
                let ctx = ctx.new_at_location(key.as_str());
                let schema_nodes = compiler::compile(&ctx, ctx.as_resource_ref(subschema))?;
                dependencies.push((F::prepare_key(key), schema_nodes));
            }
            Ok(Box::new(DependentSchemasValidator { dependencies }))
        } else {
            let location = ctx.location().join("dependentSchemas");
            Err(ValidationError::single_type_error(
                location.clone(),
                location,
                Location::new(),
                LazyInstance::Ready(Cow::Borrowed(schema)),
                JsonType::Object,
            ))
        }
    }
}
impl<F: Json> Validate<F> for DependentSchemasValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if let Some(object) = instance.as_object() {
            for (property, node) in &self.dependencies {
                if object.get(property).is_some() && !node.is_valid(instance, ctx) {
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
        if let Some(object) = instance.as_object() {
            for (property, dependency) in &self.dependencies {
                if object.get(property).is_some() {
                    dependency.validate(instance, location, tracker, ctx)?;
                }
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
        let Some(object) = instance.as_object() else {
            return;
        };
        for (property, node) in &self.dependencies {
            if object.get(property).is_some() {
                node.collect_errors(instance, location, tracker, ctx, errors);
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
        self.evaluate_with_location(instance, location, &location.into(), tracker, ctx)
    }

    fn evaluate_with_location(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: &Location,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if let Some(object) = instance.as_object() {
            let mut children = Vec::new();
            for (property, dependency) in &self.dependencies {
                if object.get(property).is_some() {
                    children.push(dependency.evaluate_instance_at(
                        instance,
                        location,
                        instance_location,
                        tracker,
                        ctx,
                    ));
                }
            }
            EvaluationResult::from_children(children)
        } else {
            EvaluationResult::valid_empty()
        }
    }
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    Some(DependenciesValidator::compile(ctx, schema))
}
#[inline]
pub(crate) fn compile_dependent_required<'a, F: Json>(
    ctx: &compiler::Context<F>,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    Some(DependentRequiredValidator::compile(ctx, schema))
}
#[inline]
pub(crate) fn compile_dependent_schemas<'a, F: Json>(
    ctx: &compiler::Context<F>,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    Some(DependentSchemasValidator::compile(ctx, schema))
}
#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(&json!({"dependencies": {"bar": ["foo"]}}), &json!({"bar": 1}), "/dependencies")]
    #[test_case(&json!({"dependencies": {"bar": {"type": "string"}}}), &json!({"bar": 1}), "/dependencies/bar/type")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }
}
