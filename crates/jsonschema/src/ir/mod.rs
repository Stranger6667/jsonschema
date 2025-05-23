mod display;
mod nodes;
mod number;

use std::{collections::VecDeque, iter::successors};

use ahash::{AHashMap, AHashSet};
pub(crate) use nodes::{EdgeLabel, IRNode, IRValue, NodeId};
use number::Number;
use referencing::{Draft, Registry, ResourceRef, Uri};
use serde_json::Value;

#[derive(Debug)]
pub(crate) struct SchemaIR<'a> {
    nodes: Vec<IRNode<'a>>,
}

impl<'a> SchemaIR<'a> {
    fn new() -> Self {
        // dummy slot at index 0
        let nodes = vec![IRNode {
            parent: None,
            first_child: None,
            last_child: None,
            next_sibling: None,
            label: None,
            value: IRValue::Null,
        }];
        SchemaIR { nodes }
    }
    /// Append a new child node to a parent node.
    fn append(
        &mut self,
        parent: Option<NodeId>,
        label: Option<EdgeLabel<'a>>,
        value: IRValue<'a>,
    ) -> NodeId {
        let id = NodeId::new(self.nodes.len());
        self.nodes.push(IRNode {
            parent,
            first_child: None,
            last_child: None,
            next_sibling: None,
            label,
            value,
        });

        if let Some(parent_id) = parent {
            if let Some(last_child_id) = self[parent_id].last_child.take() {
                self[last_child_id].next_sibling = Some(id);
            } else {
                self[parent_id].first_child = Some(id);
            }
            self[parent_id].last_child = Some(id);
        }

        id
    }

    pub(crate) fn root(&self) -> NodeId {
        NodeId::root_id()
    }

    pub(crate) fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        successors(self[id].first_child, |&node| self[node].next_sibling)
    }

    pub(crate) fn as_json(&self) -> display::IRJsonAdapter {
        display::IRJsonAdapter {
            schema: self,
            node_id: self.root(),
        }
    }
    pub(crate) fn as_json_at(&self, node_id: NodeId) -> display::IRJsonAdapter {
        display::IRJsonAdapter {
            schema: self,
            node_id,
        }
    }
}

impl<'a> std::ops::Index<NodeId> for SchemaIR<'a> {
    type Output = IRNode<'a>;

    #[inline]
    fn index(&self, id: NodeId) -> &Self::Output {
        &self.nodes[id.get()]
    }
}

impl<'a> std::ops::IndexMut<NodeId> for SchemaIR<'a> {
    #[inline]
    fn index_mut(&mut self, id: NodeId) -> &mut Self::Output {
        &mut self.nodes[id.get()]
    }
}

pub fn build(document_uri: Uri<String>, draft: Draft, registry: &Registry) -> SchemaIR<'_> {
    let value = registry
        .get_document(&document_uri)
        .expect("Schema is not present in the registry");
    let mut schema = SchemaIR::new();
    let mut value_to_node_id = AHashMap::new();
    let mut pending_references = Vec::new();
    let mut seen: AHashSet<*const Value> = AHashSet::new();
    seen.insert(value);
    let resolver = registry.resolver(document_uri);
    let mut stack = VecDeque::new();
    stack.push_back((resolver, None, None, value));

    while let Some((mut resolver, parent, label, value)) = stack.pop_front() {
        let node_id = match value {
            Value::Null => schema.append(parent, label, IRValue::Null),
            Value::Bool(b) => schema.append(parent, label, IRValue::Bool(*b)),
            Value::Number(n) => schema.append(parent, label, IRValue::Number(Number::from(n))),
            Value::String(s) => schema.append(parent, label, IRValue::String(s)),
            Value::Array(arr) => {
                let node_id = schema.append(parent, label, IRValue::Array);
                for (idx, item) in arr.iter().enumerate() {
                    if seen.insert(item) {
                        stack.push_back((
                            resolver.clone(),
                            Some(node_id),
                            Some(EdgeLabel::Index(idx)),
                            item,
                        ));
                    }
                }
                node_id
            }
            Value::Object(object) => {
                let node_id = schema.append(parent, label, IRValue::Object);
                let resource = ResourceRef::new(value, draft);

                if let Some(id) = resource.id() {
                    if !id.starts_with('#') {
                        resolver = resolver.in_subresource(resource).expect("Invalid URI");
                    }
                }
                if let Some((key, Value::String(reference))) = object.get_key_value("$ref") {
                    let resolved = resolver.lookup(reference).expect("Unresolvable reference");
                    let resolved_value = resolved.contents();
                    let ref_node_id = schema.append(
                        Some(node_id),
                        Some(EdgeLabel::Key(key)),
                        IRValue::Reference(NodeId::placeholder()),
                    );

                    pending_references.push((ref_node_id, resolved_value as *const Value));
                    if seen.insert(resolved_value) {
                        stack.push_back((resolved.resolver().clone(), None, None, resolved_value));
                    }
                }

                for (key, value) in object.iter().rev() {
                    if key != "$ref" && seen.insert(value) {
                        stack.push_back((
                            resolver.clone(),
                            Some(node_id),
                            Some(EdgeLabel::Key(key)),
                            value,
                        ));
                    }
                }
                node_id
            }
        };
        value_to_node_id.insert(value as *const Value, node_id);
    }

    for (node_id, target_ptr) in pending_references {
        if let Some(target_id) = value_to_node_id.get(&target_ptr) {
            if let IRValue::Reference(reference_target_id) = &mut schema[node_id].value {
                *reference_target_id = *target_id;
            } else {
                panic!("Node is not a reference")
            }
        } else {
            panic!("Reference not found")
        }
    }

    schema
}

#[cfg(test)]
mod tests {
    use super::*;
    use referencing::{uri, Draft, Registry};
    use serde_json::{json, Value};
    use test_case::test_case;

    const BASE_URI: &str = "https://example.com/";

    fn init_registry(input: Value) -> (Uri<String>, Registry) {
        let base_uri = uri::from_str(BASE_URI).expect("Invalid URI");
        let registry = Registry::try_new(BASE_URI, Draft::Draft202012.create_resource(input))
            .expect("Failed to build a registry");
        (base_uri, registry)
    }

    fn roundtrip_test(input: Value) {
        let (base_uri, registry) = init_registry(input.clone());

        let ir = build(base_uri, Draft::Draft202012, &registry);
        let json_string = ir.as_json().to_string();

        let reparsed: Value =
            serde_json::from_str(&json_string).expect("Failed to parse IR-generated JSON");

        assert_eq!(input, reparsed, "Roundtrip failed\nIR JSON: {json_string}");
    }

    #[test_case(json!(null))]
    #[test_case(json!(true))]
    #[test_case(json!(false))]
    #[test_case(json!(42))]
    #[test_case(json!(-17))]
    #[test_case(json!(0))]
    #[test_case(json!(1.23))]
    #[test_case(json!("hello"))]
    #[test_case(json!(""))]
    #[test_case(json!({}); "object")]
    #[test_case(json!([]); "array")]
    #[test_case(json!({"key": "value"}))]
    #[test_case(json!({"a": 1, "b": 2}))]
    #[test_case(json!([1, 2, 3]))]
    #[test_case(json!([true, null, "mixed"]))]
    fn test_basic_rountrip(input: Value) {
        roundtrip_test(input);
    }

    #[test]
    fn test_nested_structure() {
        let input = json!({
            "metadata": {
                "version": "1.0",
                "tags": ["test", "schema"]
            },
            "schema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "items": [1, 2, {"nested": true}]
                }
            },
            "examples": [
                {"name": "test1"},
                {"name": "test2", "extra": [1, 2, 3]}
            ]
        });
        roundtrip_test(input);
    }

    #[test]
    fn test_deterministic_output() {
        let input = json!({"z": 1, "a": 2, "m": 3});

        let (base_uri, registry) = init_registry(input.clone());

        let ir = build(base_uri, Draft::Draft202012, &registry);
        let json1 = ir.as_json().to_string();
        let json2 = ir.as_json().to_string();

        assert_eq!(json1, json2);

        let reparsed: Value =
            serde_json::from_str(&json1).expect("Failed to parse IR-generated JSON");
        assert_eq!(input, reparsed);
    }

    fn get_ref_target(parsed_json: &Value, pointer: &str) -> Option<u64> {
        parsed_json.pointer(pointer).and_then(|v| v.as_u64())
    }

    #[test]
    fn test_ref_to_root() {
        let schema = json!({
            "type": "object",
            "properties": {
                "self": {"$ref": "#"}
            }
        });

        let (base_uri, registry) = init_registry(schema);
        let ir = build(base_uri, Draft::Draft202012, &registry);
        let json_output = ir.as_json().to_string();

        let parsed: Value =
            serde_json::from_str(&json_output).expect("Should parse IR output as JSON");

        let target_node_id =
            get_ref_target(&parsed, "/properties/self/$ref").expect("Should find $ref target");

        assert_eq!(target_node_id, 1, "Reference should point to root node");
    }

    #[test]
    fn test_ref_to_definition() {
        let schema = json!({
            "properties": {
                "user": {"$ref": "#/definitions/person"}
            },
            "definitions": {
                "person": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"}
                    }
                }
            }
        });

        let (base_uri, registry) = init_registry(schema.clone());
        let ir = build(base_uri, Draft::Draft202012, &registry);
        let json_output = ir.as_json().to_string();

        let parsed: Value =
            serde_json::from_str(&json_output).expect("Should parse IR output as JSON");

        let target_node_id =
            get_ref_target(&parsed, "/properties/user/$ref").expect("Should find $ref target");

        let target_json = ir
            .as_json_at(NodeId::new(target_node_id as usize))
            .to_string();
        let target_parsed: Value =
            serde_json::from_str(&target_json).expect("Target should be valid JSON");

        let expected_person = &schema["definitions"]["person"];
        assert_eq!(target_parsed, *expected_person);
    }

    #[test]
    fn test_nested_refs() {
        let schema = json!({
            "allOf": [
                {"$ref": "#/definitions/base"}
            ],
            "definitions": {
                "base": {
                    "properties": {
                        "nested": {"$ref": "#/definitions/nested"}
                    }
                },
                "nested": {"type": "number"}
            }
        });

        let (base_uri, registry) = init_registry(schema.clone());
        let ir = build(base_uri, Draft::Draft202012, &registry);
        let json_output = ir.as_json().to_string();

        let parsed: Value =
            serde_json::from_str(&json_output).expect("Should parse IR output as JSON");

        let base_target =
            get_ref_target(&parsed, "/allOf/0/$ref").expect("Should find base $ref target");

        let base_json = ir.as_json_at(NodeId::new(base_target as usize)).to_string();
        let base_parsed: Value =
            serde_json::from_str(&base_json).expect("Base target should be valid JSON");

        let mut expected_base = schema["definitions"]["base"].clone();
        expected_base["properties"]["nested"]["$ref"] = json!(4);
        assert_eq!(base_parsed, expected_base);

        let nested_target = get_ref_target(&base_parsed, "/properties/nested/$ref")
            .expect("Should find nested $ref target");

        let nested_json = ir
            .as_json_at(NodeId::new(nested_target as usize))
            .to_string();
        let nested_parsed: Value =
            serde_json::from_str(&nested_json).expect("Nested target should be valid JSON");

        let expected_nested = &schema["definitions"]["nested"];
        assert_eq!(nested_parsed, *expected_nested);
    }

    #[test]
    fn test_multiple_refs_same_target() {
        let schema = json!({
            "properties": {
                "user1": {"$ref": "#/definitions/person"},
                "user2": {"$ref": "#/definitions/person"}
            },
            "definitions": {
                "person": {"type": "string"}
            }
        });

        let (base_uri, registry) = init_registry(schema.clone());
        let ir = build(base_uri, Draft::Draft202012, &registry);
        let json_output = ir.as_json().to_string();

        let parsed: Value =
            serde_json::from_str(&json_output).expect("Should parse IR output as JSON");

        let target1 = get_ref_target(&parsed, "/properties/user1/$ref")
            .expect("Should find first $ref target");
        let target2 = get_ref_target(&parsed, "/properties/user2/$ref")
            .expect("Should find second $ref target");

        assert_eq!(target1, target2);

        // Verify both targets match the original person definition
        let target1_json = ir.as_json_at(NodeId::new(target1 as usize)).to_string();
        let target1_parsed: Value =
            serde_json::from_str(&target1_json).expect("Target1 should be valid JSON");

        let target2_json = ir.as_json_at(NodeId::new(target2 as usize)).to_string();
        let target2_parsed: Value =
            serde_json::from_str(&target2_json).expect("Target2 should be valid JSON");

        let expected_person = &schema["definitions"]["person"];
        assert_eq!(target1_parsed, *expected_person);
        assert_eq!(target2_parsed, *expected_person);
        assert_eq!(target1_parsed, target2_parsed);
    }
}
