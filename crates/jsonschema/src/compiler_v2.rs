use std::{borrow::Cow, collections::VecDeque, iter::once, ops::Range, sync::Arc};

use ahash::{AHashMap, AHashSet};
use referencing::{uri, Registry};
use serde_json::Value;

use crate::{
    compiler::DEFAULT_BASE_URI,
    ir::{self, EdgeLabel, IRValue},
    ValidationOptions,
};

pub fn build(mut config: ValidationOptions, schema: &Value) -> Validator {
    let draft = config.draft_for(schema).unwrap();
    let resource_ref = draft.create_resource_ref(schema);
    let resource = draft.create_resource(schema.clone());
    let base_uri = if let Some(base_uri) = config.base_uri.as_ref() {
        uri::from_str(base_uri).unwrap()
    } else {
        uri::from_str(resource_ref.id().unwrap_or(DEFAULT_BASE_URI)).unwrap()
    };

    // Build a registry & resolver needed for validator compilation
    let resources = &mut config.resources;
    let pairs = once((Cow::Borrowed(base_uri.as_str()), resource)).chain(
        resources
            .drain()
            .map(|(uri, resource)| (Cow::Owned(uri), resource)),
    );

    let registry = if let Some(registry) = config.registry.take() {
        registry
            .try_with_resources_and_retriever(pairs, &*config.retriever, draft)
            .unwrap()
    } else {
        Registry::options()
            .draft(draft)
            .retriever(Arc::clone(&config.retriever))
            .build(pairs)
            .unwrap()
    };
    let schema = ir::build(base_uri, draft, &registry);
    compile_impl(&schema)
}

// The idea is that a JSON Schema is stored as a tree in a flat layout.
//
// Each subschema (including the root one) may contain:
//   - assertions (like `minLength`)
//   - applicators (like `properties`)
//   - annotations (like `description`) - TODO
//
// There are four allocations, one for schemas and one for each of the entities above.
// Then schemas refer to them via ranges of IDs
struct PendingPatch {
    applicator_index: usize,
    patch_type: PatchType,
}

enum PatchType {
    Properties { properties_node_id: ir::NodeId },
    Ref { node_id: ir::NodeId },
}

fn compile_impl<'a>(schema: &ir::SchemaIR<'a>) -> Validator {
    let mut validator = Validator::new();
    let mut queue = VecDeque::new();
    let mut node_to_schema = AHashMap::new();
    let mut pending_patches = Vec::new();
    let mut seen = AHashSet::new();

    queue.push_back(schema.root());
    while let Some(node_id) = queue.pop_front() {
        match &schema[node_id].value {
            IRValue::Bool(b) => {}
            IRValue::Object => {
                let schema_id = SchemaId(validator.schemas.len());
                node_to_schema.insert(node_id, schema_id);
                let assertions_start = validator.assertions.len();
                let applicators_start = validator.applicators.len();
                for child_id in schema.children(node_id) {
                    let node = &schema[child_id];
                    if let Some(EdgeLabel::Key(key)) = node.label {
                        match key.as_str() {
                            "maxLength" => {
                                if let IRValue::Number(number) = node.value {
                                    if let Some(limit) = number.as_u64() {
                                        validator.push_assertion(Assertion::MaxLength {
                                            limit: limit as usize,
                                        });
                                    }
                                }
                            }
                            "properties" => {
                                let applicator_index = validator.applicators.len();
                                pending_patches.push(PendingPatch {
                                    applicator_index,
                                    patch_type: PatchType::Properties {
                                        properties_node_id: child_id,
                                    },
                                });
                                validator
                                    .push_applicator(Applicator::Properties { properties: vec![] });
                                for property_id in schema.children(child_id) {
                                    if seen.insert(property_id) {
                                        queue.push_back(property_id);
                                    }
                                }
                            }
                            "$ref" => {
                                let applicator_index = validator.applicators.len();
                                let ir::IRValue::Reference(target_id) = node.value else {
                                    panic!()
                                };
                                pending_patches.push(PendingPatch {
                                    applicator_index,
                                    patch_type: PatchType::Ref { node_id: target_id },
                                });
                                validator.push_applicator(Applicator::Ref {
                                    schema_id: SchemaId::new(0),
                                });
                                if seen.insert(target_id) {
                                    queue.push_back(target_id);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                let assertions_end = validator.assertions.len();
                let applicators_end = validator.applicators.len();
                let schema = Schema {
                    assertions: assertions_start..assertions_end,
                    applicators: applicators_start..applicators_end,
                };

                if node_id == ir::NodeId::root_id() {
                    validator.root = schema.clone();
                }
                validator.push_schema(schema);
            }
            _ => {}
        }
    }
    for patch in pending_patches {
        apply_patch(&mut validator, schema, patch, &node_to_schema);
    }
    validator
}

fn apply_patch(
    validator: &mut Validator,
    schema: &ir::SchemaIR,
    patch: PendingPatch,
    node_to_schema: &AHashMap<ir::NodeId, SchemaId>,
) {
    match patch.patch_type {
        PatchType::Properties { properties_node_id } => {
            let mut properties = vec![];
            for property_id in schema.children(properties_node_id) {
                let property = &schema[property_id];
                if let Some(EdgeLabel::Key(key)) = property.label {
                    let schema_id = node_to_schema[&property_id];
                    properties.push((key.to_string(), schema_id));
                }
            }

            if let Applicator::Properties {
                properties: ref mut props,
            } = &mut validator.applicators[patch.applicator_index]
            {
                *props = properties;
            }
        }
        PatchType::Ref { node_id } => {
            if let Applicator::Ref { schema_id } =
                &mut validator.applicators[patch.applicator_index]
            {
                *schema_id = node_to_schema[&node_id];
            }
        }
    }
}

#[derive(Debug)]
pub struct Validator {
    root: Schema,
    schemas: Vec<Schema>,
    assertions: Vec<Assertion>,
    applicators: Vec<Applicator>,
}

#[derive(Debug, Clone)]
pub struct Schema {
    assertions: Range<usize>,
    applicators: Range<usize>,
}

impl Validator {
    pub fn new() -> Self {
        Validator {
            root: Schema {
                assertions: 0..0,
                applicators: 0..0,
            },
            schemas: Vec::new(),
            assertions: Vec::new(),
            applicators: Vec::new(),
        }
    }

    pub fn push_schema(&mut self, schema: Schema) {
        self.schemas.push(schema);
    }

    pub fn push_assertion(&mut self, assertion: Assertion) {
        self.assertions.push(assertion);
    }
    pub fn push_applicator(&mut self, applicator: Applicator) {
        self.applicators.push(applicator);
    }

    pub fn is_valid(&self, value: &Value) -> bool {
        for assertion in &self.assertions[self.root.assertions.clone()] {
            if !assertion.is_valid(value) {
                return false;
            }
        }

        for applicator in &self.applicators[self.root.applicators.clone()] {
            if !self.apply_applicator(value, applicator) {
                return false;
            }
        }

        true
    }

    #[inline]
    fn is_valid_for_schema(&self, value: &Value, schema_id: SchemaId) -> bool {
        // Run all assertions & applicators and return on the first failed one
        let schema = &self.schemas[schema_id.0];

        if !schema.assertions.is_empty() {
            for assertion in &self.assertions[schema.assertions.clone()] {
                if !assertion.is_valid(value) {
                    return false;
                }
            }
        }

        if !schema.applicators.is_empty() {
            for applicator in &self.applicators[schema.applicators.clone()] {
                if !self.apply_applicator(value, applicator) {
                    return false;
                }
            }
        }

        true
    }

    fn apply_applicator(&self, value: &Value, applicator: &Applicator) -> bool {
        match applicator {
            Applicator::Properties { properties } => {
                let Some(object) = value.as_object() else {
                    return true;
                };
                for (key, subschema_id) in properties {
                    if let Some(subvalue) = object.get(key) {
                        if !self.is_valid_for_schema(subvalue, *subschema_id) {
                            return false;
                        }
                    }
                }
                true
            }
            Applicator::Ref { schema_id } => self.is_valid_for_schema(value, *schema_id),
        }
    }
}

#[derive(Debug)]
enum Assertion {
    MaxLength { limit: usize },
}

impl Assertion {
    fn is_valid(&self, value: &Value) -> bool {
        match self {
            Assertion::MaxLength { limit } => {
                if let Value::String(item) = value {
                    if item.len() > *limit {
                        return false;
                    }
                }
                true
            }
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SchemaId(usize);

impl SchemaId {
    fn new(index: usize) -> Self {
        SchemaId(index)
    }
}

#[derive(Debug)]
enum Applicator {
    Properties { properties: Vec<(String, SchemaId)> },
    Ref { schema_id: SchemaId },
}

#[cfg(test)]
mod tests {
    use super::build;
    use serde_json::json;

    #[test]
    fn test_properties() {
        let schema = json!({"properties": {"name": {"maxLength": 5}}});
        let config = crate::options();
        let validator = build(config, &schema);
        assert!(validator.is_valid(&json!({"name": "abc"})));
        assert!(!validator.is_valid(&json!({"name": "abcefg"})));
    }

    #[test]
    fn test_ref() {
        let schema = json!({
            "properties": {
                "name": {
                    "$ref": "#/$defs/Name"
                }
            },
            "$defs": {
                "Name": {
                    "maxLength": 5
                }
            }
        });
        let config = crate::options();
        let validator = build(config, &schema);
        assert!(validator.is_valid(&json!({"name": "abc"})));
        assert!(!validator.is_valid(&json!({"name": "abcefg"})));
    }

    #[test]
    fn test_self_ref_with_assertion() {
        let schema = json!({
            "properties": {
                "name": {"maxLength": 3},
                "child": {"$ref": "#"}
            }
        });
        let config = crate::options();
        let validator = build(config, &schema);

        assert!(validator.is_valid(&json!({"name": "Bob"})));
        assert!(validator.is_valid(&json!({
            "name": "Bob",
            "child": {"name": "Ann"}
        })));
        assert!(validator.is_valid(&json!({
            "name": "Bob",
            "child": {
                "name": "Ann",
                "child": {"name": "Joe"}
            }
        })));
        assert!(!validator.is_valid(&json!({"name": "Robert"})));
    }
}
