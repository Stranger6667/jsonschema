use crate::{evaluation::InstancePosition, LazyInstance};
use std::{borrow::Cow, sync::OnceLock};

use crate::{
    compiler,
    error::ValidationError,
    evaluation::ChildList,
    node::SchemaNode,
    paths::{LazyLocation, Location, RefTracker},
    properties::{KeyHead, NameBuckets, NameLengths, PropertyName, HASHMAP_THRESHOLD},
    types::JsonType,
    validator::{EvaluationResult, Validate, ValidationContext},
    Json, Node, Object, SerdeJson,
};
use serde_json::{Map, Value};

use super::CompilationResult;

pub(crate) struct AllOfValidator<F: Json> {
    schemas: Vec<SchemaNode<F>>,
    /// Filled on first use: `$ref` targets are wired up only after compilation.
    objects: OnceLock<Option<ObjectBranches<F>>>,
}

/// What a schema checks when it checks nothing but an object's members.
pub(crate) struct ObjectPart<'a, F: Json> {
    pub(crate) properties: Vec<(&'a str, &'a SchemaNode<F>)>,
    pub(crate) required: Vec<&'a str>,
    /// `type: object`: anything else fails.
    pub(crate) object_only: bool,
}

/// `allOf` branches that only check object members, merged into one pass over the object.
struct ObjectBranches<F: Json> {
    names: MergedNames<F>,
    lengths: NameLengths,
    /// Every name's `Merged::required` bit together.
    required: u64,
    object_only: bool,
    /// The branches checked one by one, as indices into `AllOfValidator::schemas`.
    others: Vec<usize>,
}

/// Bucketed past `HASHMAP_THRESHOLD`, as `properties` buckets its own names.
enum MergedNames<F: Json> {
    Scan(Vec<(PropertyName, Merged<F>)>),
    Buckets(NameBuckets<Merged<F>>),
}

struct Merged<F: Json> {
    /// Every branch's subschema for this name.
    nodes: Vec<SchemaNode<F>>,
    /// This name's bit when a branch requires it, otherwise zero.
    required: u64,
}

impl<F: Json> ObjectBranches<F> {
    fn new(schemas: &[SchemaNode<F>]) -> Option<Self> {
        let mut names: Vec<(PropertyName, Merged<F>)> = Vec::new();
        let mut required = 0u64;
        let mut object_only = false;
        let mut merged = 0;
        let mut others = Vec::new();
        for (index, schema) in schemas.iter().enumerate() {
            let mut part = ObjectPart {
                properties: Vec::new(),
                required: Vec::new(),
                object_only: false,
            };
            if !schema.collect_object_part(&mut part) {
                others.push(index);
                continue;
            }
            merged += 1;
            object_only |= part.object_only;
            for (name, node) in part.properties {
                entry(&mut names, name).nodes.push(node.clone());
            }
            for name in part.required {
                let entry = entry(&mut names, name);
                if entry.required == 0 {
                    entry.required = 1u64.checked_shl(required.count_ones())?;
                    required |= entry.required;
                }
            }
        }
        if merged < 2 {
            return None;
        }
        let lengths = NameLengths::of(names.iter().map(|(name, _)| name.as_str()));
        let names = if names.len() < HASHMAP_THRESHOLD {
            MergedNames::Scan(names)
        } else {
            MergedNames::Buckets(NameBuckets::new(names))
        };
        Some(ObjectBranches {
            names,
            lengths,
            required,
            object_only,
            others,
        })
    }

    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        let Some(object) = instance.as_object() else {
            return !self.object_only;
        };
        match &self.names {
            MergedNames::Scan(names) => self.members_valid(&object, ctx, |key| {
                let head = KeyHead::of(key);
                names
                    .iter()
                    .find(|(name, _)| name.matches(head, key))
                    .map(|(_, merged)| merged)
            }),
            MergedNames::Buckets(buckets) => self.members_valid(&object, ctx, |key| {
                buckets.find(key).map(|(_, merged)| merged)
            }),
        }
    }

    #[inline]
    fn members_valid<'s, 'i>(
        &'s self,
        object: &<F::Node<'i> as Node<'i, F>>::Object,
        ctx: &mut ValidationContext,
        find: impl Fn(&str) -> Option<&'s Merged<F>>,
    ) -> bool {
        let mut seen = 0u64;
        for (key, value) in object.members() {
            let key = key.as_ref();
            if !self.lengths.may_contain(key.len()) {
                continue;
            }
            if let Some(merged) = find(key) {
                if !merged.nodes.iter().all(|node| node.is_valid(&value, ctx)) {
                    return false;
                }
                seen |= merged.required;
            }
        }
        seen == self.required
    }
}

fn entry<'n, F: Json>(
    names: &'n mut Vec<(PropertyName, Merged<F>)>,
    name: &str,
) -> &'n mut Merged<F> {
    let position = names
        .iter()
        .position(|(merged, _)| merged.as_str() == name)
        .unwrap_or_else(|| {
            names.push((
                PropertyName::new(name.to_owned()),
                Merged {
                    nodes: Vec::new(),
                    required: 0,
                },
            ));
            names.len() - 1
        });
    &mut names[position].1
}

impl AllOfValidator<SerdeJson> {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        items: &'a [Value],
    ) -> CompilationResult<'a, F> {
        let ctx = ctx.new_at_location("allOf");
        let mut schemas = Vec::with_capacity(items.len());
        for (idx, item) in items.iter().enumerate() {
            let ctx = ctx.new_at_location(idx);
            let validators = compiler::compile(&ctx, ctx.as_resource_ref(item))?;
            schemas.push(validators);
        }
        Ok(Box::new(AllOfValidator {
            schemas,
            objects: OnceLock::new(),
        }))
    }
}

impl<F: Json> Validate<F> for AllOfValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        match self
            .objects
            .get_or_init(|| ObjectBranches::new(&self.schemas))
        {
            Some(objects) => {
                objects.is_valid(instance, ctx)
                    && objects
                        .others
                        .iter()
                        .all(|&index| self.schemas[index].is_valid(instance, ctx))
            }
            None => self.schemas.iter().all(|n| n.is_valid(instance, ctx)),
        }
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        for schema in &self.schemas {
            schema.validate(instance, location, tracker, ctx)?;
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
        for node in &self.schemas {
            node.collect_errors(instance, location, tracker, ctx, errors);
        }
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        self.evaluate_with_location(
            instance,
            location,
            ctx.instance_position(location),
            tracker,
            ctx,
        )
    }

    fn evaluate_with_location(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: InstancePosition,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let mut children = ChildList::default();
        for node in &self.schemas {
            let child =
                node.evaluate_instance_at(instance, location, instance_location, tracker, ctx);
            children.push(ctx.arena(), child);
        }
        EvaluationResult::from_children(children)
    }
}

pub(crate) struct SingleValueAllOfValidator<F: Json> {
    node: SchemaNode<F>,
}

impl SingleValueAllOfValidator<SerdeJson> {
    #[inline]
    pub(crate) fn compile<'a, F: Json>(
        ctx: &compiler::Context<F>,
        schema: &'a Value,
    ) -> CompilationResult<'a, F> {
        let ctx = ctx.new_at_location("allOf");
        let ctx = ctx.new_at_location(0);
        let node = compiler::compile(&ctx, ctx.as_resource_ref(schema))?;
        Ok(Box::new(SingleValueAllOfValidator { node }))
    }
}

impl<F: Json> Validate<F> for SingleValueAllOfValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        self.node.is_valid(instance, ctx)
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        self.node.validate(instance, location, tracker, ctx)
    }

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        self.node
            .collect_errors(instance, location, tracker, ctx, errors);
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        self.evaluate_with_location(
            instance,
            location,
            ctx.instance_position(location),
            tracker,
            ctx,
        )
    }

    fn evaluate_with_location(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: InstancePosition,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let node =
            self.node
                .evaluate_instance_at(instance, location, instance_location, tracker, ctx);
        EvaluationResult::from_node(ctx.arena(), node)
    }
}

#[inline]
pub(crate) fn compile<'a, F: Json>(
    ctx: &compiler::Context<F>,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a, F>> {
    if let Value::Array(items) = schema {
        if items.len() == 1 {
            let value = items.iter().next().expect("Vec is not empty");
            Some(SingleValueAllOfValidator::compile(ctx, value))
        } else {
            Some(AllOfValidator::compile(ctx, items))
        }
    } else {
        let location = ctx.location().join("allOf");
        Some(Err(ValidationError::single_type_error(
            location.clone(),
            location,
            Location::new(),
            LazyInstance::Ready(Cow::Borrowed(schema)),
            JsonType::Array,
        )))
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(&json!({"allOf": [{"type": "string"}]}), &json!(1), "/allOf/0/type")]
    #[test_case(&json!({"allOf": [{"type": "integer"}, {"maximum": 5}]}), &json!(6), "/allOf/1/maximum")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }

    // Seventeen merged names: past the threshold, `is_valid` finds them in buckets.
    fn many_names() -> Value {
        let properties: serde_json::Map<String, Value> = (0..16)
            .map(|index| (format!("p{index}"), json!({"type": "integer"})))
            .collect();
        json!({"allOf": [
            {"properties": properties},
            {"properties": {"q": {"type": "string"}}, "required": ["p15", "q"]}
        ]})
    }

    #[test_case(&json!({"p15": 1, "q": "x"}), true; "valid")]
    #[test_case(&json!({"p15": "x", "q": "x"}), false; "invalid member past the threshold")]
    #[test_case(&json!({"q": "x"}), false; "missing required name past the threshold")]
    fn merged_branches_past_the_threshold(instance: &Value, valid: bool) {
        if valid {
            tests_util::is_valid(&many_names(), instance);
        } else {
            tests_util::is_not_valid(&many_names(), instance);
        }
    }

    #[test]
    fn malformed() {
        tests_util::assert_compile_error(
            &json!({"allOf": 5}),
            "5 is not of type \"array\"",
            "/allOf",
        );
    }
}
