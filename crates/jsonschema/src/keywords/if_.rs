use crate::{
    compiler,
    evaluation::{ChildList, InstancePosition},
    keywords::{
        helpers::{unit_over, SchemaSite},
        CompilationResult,
    },
    node::SchemaNode,
    paths::{LazyLocation, RefTracker},
    validator::{Validate, ValidationContext},
    Json, SerdeJson, ValidationError,
};
use serde_json::{Map, Value};

/// The `if` subschema and the branches it chooses between, each with where its unit sits.
struct Conditional<'a, F: Json> {
    condition: (&'a SchemaNode<F>, &'a SchemaSite),
    then: Option<(&'a SchemaNode<F>, &'a SchemaSite)>,
    otherwise: Option<(&'a SchemaNode<F>, &'a SchemaSite)>,
}

/// `if` reports its own unit, holding its subschema only when it passes; the branch taken
/// reports a unit of its own beside it. Returns whether that branch fails.
fn evaluate_conditional<F: Json>(
    conditional: &Conditional<'_, F>,
    instance: &F::Node<'_>,
    location: &LazyLocation,
    instance_location: InstancePosition,
    tracker: Option<&RefTracker>,
    ctx: &mut ValidationContext,
    children: &mut ChildList,
) -> bool {
    let (condition, condition_site) = conditional.condition;
    let node = condition.evaluate_instance_at(instance, location, instance_location, tracker, ctx);
    let passed = node.valid;
    let held = if passed {
        ChildList::of(ctx.arena(), node)
    } else {
        ChildList::default()
    };
    let unit = unit_over(
        condition_site,
        Vec::new(),
        None,
        held,
        instance_location,
        tracker,
        ctx,
    );
    children.push(ctx.arena(), unit);
    let branch = if passed {
        conditional.then
    } else {
        conditional.otherwise
    };
    let Some((schema, site)) = branch else {
        return false;
    };
    let node = schema.evaluate_instance_at(instance, location, instance_location, tracker, ctx);
    let failed = !node.valid;
    let held = ChildList::of(ctx.arena(), node);
    let unit = unit_over(
        site,
        Vec::new(),
        None,
        held,
        instance_location,
        tracker,
        ctx,
    );
    children.push(ctx.arena(), unit);
    failed
}

pub(crate) struct IfThenValidator<F: Json> {
    schema: SchemaNode<F>,
    then_schema: SchemaNode<F>,
    if_site: SchemaSite,
    then_site: SchemaSite,
}

impl IfThenValidator<SerdeJson> {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
        then_schema: &'a Value,
    ) -> CompilationResult<'a, F> {
        Ok(Box::new(IfThenValidator {
            schema: {
                let ctx = ctx.new_at_location("if");
                compiler::compile(&ctx, ctx.as_resource_ref(schema))?
            },
            then_schema: {
                let ctx = ctx.new_at_location("then");
                compiler::compile(&ctx, ctx.as_resource_ref(then_schema))?
            },
            if_site: SchemaSite::new(ctx, ctx.location().join("if")),
            then_site: SchemaSite::new(ctx, ctx.location().join("then")),
        }))
    }
}

impl<F: Json> Validate<F> for IfThenValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if self.schema.is_valid_child(instance, ctx) {
            self.then_schema.is_valid_child(instance, ctx)
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
        if self.schema.is_valid(instance, ctx) {
            self.then_schema.validate(instance, location, tracker, ctx)
        } else {
            Ok(())
        }
    }

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        if self.schema.is_valid(instance, ctx) {
            self.then_schema
                .collect_errors(instance, location, tracker, ctx, errors);
        }
    }

    fn reports_units(&self) -> bool {
        true
    }

    fn evaluate_units(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: InstancePosition,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        children: &mut ChildList,
    ) -> bool {
        evaluate_conditional(
            &Conditional {
                condition: (&self.schema, &self.if_site),
                then: Some((&self.then_schema, &self.then_site)),
                otherwise: None,
            },
            instance,
            location,
            instance_location,
            tracker,
            ctx,
            children,
        )
    }
}

pub(crate) struct IfElseValidator<F: Json> {
    schema: SchemaNode<F>,
    else_schema: SchemaNode<F>,
    if_site: SchemaSite,
    else_site: SchemaSite,
}

impl IfElseValidator<SerdeJson> {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
        else_schema: &'a Value,
    ) -> CompilationResult<'a, F> {
        Ok(Box::new(IfElseValidator {
            schema: {
                let ctx = ctx.new_at_location("if");
                compiler::compile(&ctx, ctx.as_resource_ref(schema))?
            },
            else_schema: {
                let ctx = ctx.new_at_location("else");
                compiler::compile(&ctx, ctx.as_resource_ref(else_schema))?
            },
            if_site: SchemaSite::new(ctx, ctx.location().join("if")),
            else_site: SchemaSite::new(ctx, ctx.location().join("else")),
        }))
    }
}

impl<F: Json> Validate<F> for IfElseValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if self.schema.is_valid_child(instance, ctx) {
            true
        } else {
            self.else_schema.is_valid_child(instance, ctx)
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if self.schema.is_valid(instance, ctx) {
            Ok(())
        } else {
            self.else_schema.validate(instance, location, tracker, ctx)
        }
    }

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        if !self.schema.is_valid(instance, ctx) {
            self.else_schema
                .collect_errors(instance, location, tracker, ctx, errors);
        }
    }

    fn reports_units(&self) -> bool {
        true
    }

    fn evaluate_units(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: InstancePosition,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        children: &mut ChildList,
    ) -> bool {
        evaluate_conditional(
            &Conditional {
                condition: (&self.schema, &self.if_site),
                then: None,
                otherwise: Some((&self.else_schema, &self.else_site)),
            },
            instance,
            location,
            instance_location,
            tracker,
            ctx,
            children,
        )
    }
}

pub(crate) struct IfThenElseValidator<F: Json> {
    schema: SchemaNode<F>,
    then_schema: SchemaNode<F>,
    else_schema: SchemaNode<F>,
    if_site: SchemaSite,
    then_site: SchemaSite,
    else_site: SchemaSite,
}

impl IfThenElseValidator<SerdeJson> {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
        then_schema: &'a Value,
        else_schema: &'a Value,
    ) -> CompilationResult<'a, F> {
        Ok(Box::new(IfThenElseValidator {
            schema: {
                let ctx = ctx.new_at_location("if");
                compiler::compile(&ctx, ctx.as_resource_ref(schema))?
            },
            then_schema: {
                let ctx = ctx.new_at_location("then");
                compiler::compile(&ctx, ctx.as_resource_ref(then_schema))?
            },
            else_schema: {
                let ctx = ctx.new_at_location("else");
                compiler::compile(&ctx, ctx.as_resource_ref(else_schema))?
            },
            if_site: SchemaSite::new(ctx, ctx.location().join("if")),
            then_site: SchemaSite::new(ctx, ctx.location().join("then")),
            else_site: SchemaSite::new(ctx, ctx.location().join("else")),
        }))
    }
}

impl<F: Json> Validate<F> for IfThenElseValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        if self.schema.is_valid_child(instance, ctx) {
            self.then_schema.is_valid_child(instance, ctx)
        } else {
            self.else_schema.is_valid_child(instance, ctx)
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if self.schema.is_valid(instance, ctx) {
            self.then_schema.validate(instance, location, tracker, ctx)
        } else {
            self.else_schema.validate(instance, location, tracker, ctx)
        }
    }

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        if self.schema.is_valid(instance, ctx) {
            self.then_schema
                .collect_errors(instance, location, tracker, ctx, errors);
        } else {
            self.else_schema
                .collect_errors(instance, location, tracker, ctx, errors);
        }
    }

    fn reports_units(&self) -> bool {
        true
    }

    fn evaluate_units(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: InstancePosition,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        children: &mut ChildList,
    ) -> bool {
        evaluate_conditional(
            &Conditional {
                condition: (&self.schema, &self.if_site),
                then: Some((&self.then_schema, &self.then_site)),
                otherwise: Some((&self.else_schema, &self.else_site)),
            },
            instance,
            location,
            instance_location,
            tracker,
            ctx,
            children,
        )
    }
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    let then = parent.get("then");
    let else_ = parent.get("else");
    match (then, else_) {
        (Some(then_schema), Some(else_schema)) => Some(IfThenElseValidator::compile(
            ctx,
            schema,
            then_schema,
            else_schema,
        )),
        (None, Some(else_schema)) => Some(IfElseValidator::compile(ctx, schema, else_schema)),
        (Some(then_schema), None) => Some(IfThenValidator::compile(ctx, schema, then_schema)),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    // `if` reports its own unit, holding its subschema only when it passes; `then` and `else`
    // report units of their own beside it.
    #[test_case(
        &json!({"if": {"type": "integer"}, "then": {"minimum": 2}, "else": {"type": "string"}}),
        &json!(1),
        &[
            " @ FAIL",
            "  /if @ ok",
            "    /if @ ok",
            "      /if/type @ ok",
            "  /then @ FAIL",
            "    /then @ FAIL",
            "      /then/minimum @ FAIL err=minimum",
        ] ; "then fails"
    )]
    #[test_case(
        &json!({"if": {"type": "integer"}, "then": {"minimum": 2}, "else": {"type": "string"}}),
        &json!(true),
        &[
            " @ FAIL",
            "  /if @ ok",
            "  /else @ FAIL",
            "    /else @ FAIL",
            "      /else/type @ FAIL err=type",
        ] ; "else fails"
    )]
    #[test_case(
        &json!({"if": {"type": "integer"}, "then": {"minimum": 2}, "else": {"type": "string"}}),
        &json!(3),
        &[
            " @ ok",
            "  /if @ ok",
            "    /if @ ok",
            "      /if/type @ ok",
            "  /then @ ok",
            "    /then @ ok",
            "      /then/minimum @ ok",
        ] ; "then passes"
    )]
    #[test_case(
        &json!({"if": {"type": "integer"}, "then": {"minimum": 2}}),
        &json!("x"),
        &[" @ ok", "  /if @ ok"] ; "no else"
    )]
    #[test_case(
        &json!({"if": {"type": "integer"}, "else": {"type": "string"}}),
        &json!(1),
        &[" @ ok", "  /if @ ok", "    /if @ ok", "      /if/type @ ok"] ; "no then"
    )]
    fn evaluate_units(schema: &Value, instance: &Value, expected: &[&str]) {
        assert_eq!(tests_util::evaluation_outline(schema, instance), expected);
    }

    #[test_case(&json!({"if": {"minimum": 0}, "else": {"multipleOf": 2}}), &json!(-1), "/else/multipleOf")]
    #[test_case(&json!({"if": {"minimum": 0}, "then": {"multipleOf": 2}}), &json!(3), "/then/multipleOf")]
    #[test_case(&json!({"if": {"minimum": 0}, "then": {"multipleOf": 2}, "else": {"multipleOf": 2}}), &json!(-1), "/else/multipleOf")]
    #[test_case(&json!({"if": {"minimum": 0}, "then": {"multipleOf": 2}, "else": {"multipleOf": 2}}), &json!(3), "/then/multipleOf")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }

    #[test]
    fn malformed() {
        tests_util::assert_compile_error(
            &json!({"if": 5, "then": {}}),
            "5 is not of types \"boolean\", \"object\"",
            "/if",
        );
    }
}
