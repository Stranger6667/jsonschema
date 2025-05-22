use std::{borrow::Cow, iter::once, ops::Range, sync::Arc};

use jsonschema_ir::{EdgeLabel, NodeId, NodeValue, ResolvedSchema};
use referencing::{uri, Registry};
use serde_json::Value;

use crate::{compiler::DEFAULT_BASE_URI, ValidationOptions};

pub(crate) fn build(mut config: ValidationOptions, schema: &Value) {
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
    let schema = jsonschema_ir::build(base_uri, draft, &registry);
    let mut validator = Validator::new();
    compile_at(&schema, schema.root(), &mut validator);
    dbg!(&validator);
}

fn compile_at<'a>(schema: &ResolvedSchema<'a>, node_id: NodeId, v: &mut Validator) {
    match &schema.get(node_id).value {
        NodeValue::Bool(b) => {}
        NodeValue::Object => {
            for child_id in schema.children(node_id) {
                let node = schema.get(child_id);
                if let Some(EdgeLabel::Key(key)) = node.parent_label {
                    if key == "maxLength" {
                        if let NodeValue::Number(num) = node.value {
                            if let Some(limit) = num.as_u64() {
                                v.push(Assertion::MaxLength {
                                    limit: limit as usize,
                                });
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

#[derive(Debug)]
struct Validator {
    schemas: Vec<Schema>,
    assertions: Vec<Assertion>,
}

#[derive(Debug)]
pub struct Schema {
    assertions: Range<usize>,
}

impl Validator {
    pub fn new() -> Self {
        Validator {
            schemas: Vec::new(),
            assertions: Vec::new(),
        }
    }

    pub fn push(&mut self, a: Assertion) {
        self.assertions.push(a);
    }

    fn is_valid(&self, value: &Value) -> bool {
        let root = &self.schemas[0];
        for assertion in &self.assertions[root.assertions.clone()] {
            if !assertion.is_valid(value) {
                return false;
            }
        }
        true
    }
}

#[derive(Debug)]
enum Assertion {
    MaxLength { limit: usize },
}

impl Assertion {
    fn is_valid(&self, value: &Value) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::build;
    use serde_json::json;

    #[test]
    fn test_debug() {
        let schema = json!({"type": "string", "maxLength": 10});
        let config = crate::options();
        build(config, &schema);
    }
}
