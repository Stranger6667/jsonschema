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

/// Context for backpatching of applicator nodes.
#[derive(Debug)]
struct PendingPatch {
    schema_id: SchemaId,
    keyword_index: usize,
    kind: PatchKind,
}

impl PendingPatch {
    fn properties(schema_id: SchemaId, keyword_index: usize, node_id: ir::NodeId) -> Self {
        PendingPatch {
            schema_id,
            keyword_index,
            kind: PatchKind::Properties { node_id },
        }
    }

    fn r#ref(schema_id: SchemaId, keyword_index: usize, node_id: ir::NodeId) -> Self {
        PendingPatch {
            schema_id,
            keyword_index,
            kind: PatchKind::Ref { node_id },
        }
    }
}

#[derive(Debug)]
enum PatchKind {
    Properties { node_id: ir::NodeId },
    Ref { node_id: ir::NodeId },
}

struct Queue {
    seen: AHashSet<ir::NodeId>,
    items: VecDeque<ir::NodeId>,
}

impl Queue {
    fn new() -> Self {
        Queue {
            seen: AHashSet::new(),
            items: VecDeque::new(),
        }
    }
    fn push(&mut self, id: ir::NodeId) {
        if self.seen.insert(id) {
            self.items.push_back(id);
        }
    }
    fn pop(&mut self) -> Option<ir::NodeId> {
        self.items.pop_front()
    }
}

fn compile_impl<'a>(schema: &ir::SchemaIR<'a>) -> Validator {
    let mut validator = Validator::new();
    let mut queue = Queue::new();
    let mut node_to_schema = AHashMap::new();
    let mut pending_patches = Vec::new();

    queue.push(schema.root());

    while let Some(node_id) = queue.pop() {
        match &schema[node_id].value {
            IRValue::Bool(b) => {
                // TODO: should there be a node here?
            }
            IRValue::Object => {
                let schema_id = SchemaId(validator.schemas.len());
                node_to_schema.insert(node_id, schema_id);

                let mut keywords = Vec::new();

                for child_id in schema.children(node_id) {
                    let node = &schema[child_id];
                    if let Some(EdgeLabel::Key(key)) = node.label {
                        match key.as_str() {
                            "maxLength" => {
                                if let IRValue::Number(number) = node.value {
                                    if let Some(limit) = number.as_u64() {
                                        keywords.push(Keyword::MaxLength {
                                            limit: limit as usize,
                                        });
                                    }
                                }
                            }
                            "properties" => {
                                let keyword_index = keywords.len();
                                pending_patches.push(PendingPatch::properties(
                                    schema_id,
                                    keyword_index,
                                    child_id,
                                ));
                                keywords.push(Keyword::properties());
                                for id in schema.children(child_id) {
                                    queue.push(id);
                                }
                            }
                            "$ref" => {
                                let ir::IRValue::Reference(target_id) = node.value else {
                                    panic!()
                                };
                                let keyword_index = keywords.len();
                                pending_patches.push(PendingPatch::r#ref(
                                    schema_id,
                                    keyword_index,
                                    target_id,
                                ));
                                keywords.push(Keyword::r#ref());
                                queue.push(target_id);
                            }
                            _ => {}
                        }
                    }
                }
                let schema = Schema { keywords };

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
    let target = if patch.schema_id.0 == 0 {
        &mut validator.root
    } else {
        &mut validator.schemas[patch.schema_id.0]
    };

    apply_patch_impl(target, schema, &patch, node_to_schema);
}

fn apply_patch_impl(
    target: &mut Schema,
    schema: &ir::SchemaIR,
    patch: &PendingPatch,
    node_to_schema: &AHashMap<ir::NodeId, SchemaId>,
) {
    match patch.kind {
        PatchKind::Properties { node_id } => {
            if let Keyword::Properties { properties } = &mut target.keywords[patch.keyword_index] {
                for property_id in schema.children(node_id) {
                    let property = &schema[property_id];
                    if let Some(EdgeLabel::Key(key)) = property.label {
                        let schema_id = node_to_schema[&property_id];
                        properties.push((key.to_string(), schema_id));
                    }
                }
            }
        }
        PatchKind::Ref { node_id } => {
            if let Keyword::Ref { schema_id } = &mut target.keywords[patch.keyword_index] {
                *schema_id = node_to_schema[&node_id];
            }
        }
    }
}

#[derive(Debug)]
pub struct Validator {
    root: Schema,
    schemas: Vec<Schema>,
}

#[derive(Debug, Clone)]
pub struct Schema {
    keywords: Vec<Keyword>,
}

impl Validator {
    pub fn new() -> Self {
        Validator {
            root: Schema {
                keywords: Vec::new(),
            },
            schemas: Vec::new(),
        }
    }

    pub fn push_schema(&mut self, schema: Schema) {
        self.schemas.push(schema);
    }

    pub fn is_valid(&self, value: &Value) -> bool {
        for keyword in &self.root.keywords {
            if !self.apply_keyword(keyword, value) {
                return false;
            }
        }

        true
    }

    fn is_valid_for_schema(&self, value: &Value, schema_id: SchemaId) -> bool {
        // Evaluate all keywords and return on the first failed one
        let schema = if schema_id.0 == 0 {
            &self.root
        } else {
            &self.schemas[schema_id.0]
        };

        for keyword in &schema.keywords {
            if !self.apply_keyword(keyword, value) {
                return false;
            }
        }

        true
    }

    fn apply_keyword(&self, keyword: &Keyword, value: &Value) -> bool {
        match keyword {
            Keyword::MaxLength { limit } => {
                if let Value::String(item) = value {
                    if item.len() > *limit {
                        return false;
                    }
                }
                true
            }
            Keyword::MinLength { limit } => {
                if let Value::String(item) = value {
                    if item.len() < *limit {
                        return false;
                    }
                }
                true
            }
            Keyword::Properties { properties } => {
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
            Keyword::Ref { schema_id } => self.is_valid_for_schema(value, *schema_id),
        }
    }
}

#[derive(Debug, Clone)]
enum Keyword {
    MaxLength { limit: usize },
    MinLength { limit: usize },
    Properties { properties: Vec<(String, SchemaId)> },
    Ref { schema_id: SchemaId },
}

impl Keyword {
    fn properties() -> Self {
        Keyword::Properties { properties: vec![] }
    }

    fn r#ref() -> Self {
        Keyword::Ref {
            schema_id: SchemaId(0),
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

    #[test]
    fn test_nested_refs() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$defs": {
                "a": {"maxLength": 5},
                "b": {"$ref": "#/$defs/a"},
                "c": {"$ref": "#/$defs/b"}
            },
            "$ref": "#/$defs/c"
        });
        let config = crate::options();
        let validator = build(config, &schema);

        assert!(validator.is_valid(&json!("abc")));
        assert!(!validator.is_valid(&json!("abcdef")));
    }
}
