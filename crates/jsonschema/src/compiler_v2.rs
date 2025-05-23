use std::{borrow::Cow, collections::VecDeque, iter::once, ops::Range, sync::Arc};

use referencing::{uri, Registry};
use serde_json::Value;

use crate::{compiler::DEFAULT_BASE_URI, ir, ValidationOptions};

pub(crate) fn build(mut config: ValidationOptions, schema: &Value) -> Validator {
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

// TODO:
//  - mark object with `$ref` to apply draft-specific logic

fn compile_impl<'a>(schema: &ir::SchemaIR<'a>) -> Validator {
    // Main compilation loop
    //   - Compile assertions right away, so they are stored contiguously
    //   - Similarly, store annotations in a contiguous block of memory
    //   - Defer applicators, so their assertions appear after the ones coming from the current node
    let mut validator = Validator::new();
    let mut queue = VecDeque::new();
    queue.push_back(schema.root());
    // Traverse in BFS order
    //while let Some(node_id) = queue.pop_front() {
    //    match &schema[node_id].value {
    //        NodeValue::Bool(b) => {}
    //        NodeValue::Object => {
    //            let assertions_start = validator.assertions.len();
    //            for child_id in schema.children(node_id) {
    //                let node = schema.get(child_id);
    //                if let Some(EdgeLabel::Key(key)) = node.parent_label {
    //                    if key == "maxLength" {
    //                        if let NodeValue::Number(num) = node.value {
    //                            if let Some(limit) = num.as_u64() {
    //                                validator.push_assertion(Assertion::MaxLength {
    //                                    limit: limit as usize,
    //                                });
    //                            }
    //                        }
    //                    } else if key == "properties" {
    //                        // Compile properties - need to store keys + their schema ids. But
    //                        // schema ids are not yet available as they are not compiled yet - how
    //                        // to proceed here?
    //                        for property_id in schema.children(child_id) {
    //                            let property = schema.get(property_id);
    //                            queue.push_back(property_id);
    //                        }
    //                    }
    //                }
    //            }
    //            // TODO: store applicators for this schema, so we can apply subschemas during
    //            // validation
    //            let assertions_end = validator.assertions.len();
    //            //validator.push_schema(Schema {
    //            //    assertions: assertions_start..assertions_end,
    //            //});
    //        }
    //        _ => {}
    //    }
    //}
    validator
}

#[derive(Debug)]
struct Validator {
    schemas: Vec<Schema>,
    assertions: Vec<Assertion>,
    applicators: Vec<Applicator>,
}

#[derive(Debug)]
pub struct Schema {
    assertions: Range<usize>,
    applicators: Range<usize>,
}

impl Validator {
    pub fn new() -> Self {
        Validator {
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

    fn is_valid(&self, value: &Value) -> bool {
        //let root = &self.schemas[0];
        //for assertion in &self.assertions[root.assertions.clone()] {
        //    if !assertion.is_valid(value) {
        //        return false;
        //    }
        //}
        true
    }
}

#[derive(Debug)]
enum Assertion {
    MaxLength { limit: usize },
}

impl Assertion {
    fn is_valid(&self, value: &Value) -> bool {
        match self {
            Assertion::MaxLength { limit } => value.as_str().map_or(true, |s| s.len() <= *limit),
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
}

#[cfg(test)]
mod tests {
    use super::build;
    use serde_json::json;

    //#[test]
    //fn test_assertion_only() {
    //    let schema = json!({"type": "string", "maxLength": 5});
    //    let config = crate::options();
    //    let validator = build(config, &schema);
    //    assert!(validator.is_valid(&json!("abc")));
    //    assert!(!validator.is_valid(&json!("abcefg")));
    //}
    //
    //#[test]
    //fn test_properties() {
    //    let schema = json!({"properties": {"name": {"maxLength": 5}}});
    //    let config = crate::options();
    //    let validator = build(config, &schema);
    //    assert!(validator.is_valid(&json!({"name": "abc"})));
    //    assert!(!validator.is_valid(&json!({"name": "abcefg"})));
    //}
}
