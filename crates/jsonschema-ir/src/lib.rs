use std::{
    collections::{HashMap, HashSet, VecDeque},
    num::NonZeroUsize,
};

use referencing::{Draft, Registry, ResourceRef, Uri};
use serde_json::Value;

#[derive(Debug)]
pub enum NodeValue<'a> {
    Null,
    Object,
    Array,
    Bool(bool),
    Number(&'a serde_json::Number),
    String(&'a String),
    Reference(&'a String, NodeId),
}

#[derive(Debug)]
pub enum EdgeLabel<'a> {
    Key(&'a String),
    Index(usize),
}

#[derive(Debug)]
pub struct Node<'a> {
    parent: Option<NodeId>,
    first_child: Option<NodeId>,
    next_sibling: Option<NodeId>,
    parent_label: Option<EdgeLabel<'a>>,
    value: NodeValue<'a>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(NonZeroUsize);

impl NodeId {
    #[inline]
    pub(crate) fn new(value: usize) -> NodeId {
        NodeId(NonZeroUsize::new(value).expect("Value is zero"))
    }
    #[inline]
    pub(crate) fn root_id() -> NodeId {
        NodeId::new(1)
    }
    #[inline]
    pub(crate) fn get(self) -> usize {
        self.0.get()
    }
}

pub fn build<'a>(
    base_uri: Uri<String>,
    draft: Draft,
    value: &'a Value,
    registry: &'a Registry,
) -> Vec<Node<'a>> {
    let mut arena = Vec::new();
    arena.push(Node {
        parent: None,
        first_child: None,
        next_sibling: None,
        parent_label: None,
        value: NodeValue::Null,
    });
    let mut value_to_node_id = HashMap::new();
    let mut pending_references = Vec::new();
    let mut seen = HashSet::new();
    seen.insert(value);
    let resolver = registry.resolver(base_uri);
    let mut stack = VecDeque::new();
    stack.push_back((resolver, None, None, value));

    while let Some((mut resolver, parent, label, value)) = stack.pop_front() {
        let node_id = match value {
            Value::Null => new_node(&mut arena, parent, label, NodeValue::Null),
            Value::Bool(b) => new_node(&mut arena, parent, label, NodeValue::Bool(*b)),
            Value::Number(n) => new_node(&mut arena, parent, label, NodeValue::Number(n)),
            Value::String(s) => new_node(&mut arena, parent, label, NodeValue::String(s)),
            Value::Array(arr) => {
                let node_id = new_node(&mut arena, parent, label, NodeValue::Array);
                for (idx, item) in arr.iter().enumerate().rev() {
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
                let node_id = new_node(&mut arena, parent, label, NodeValue::Object);
                let resource = ResourceRef::new(value, draft);

                if let Some(id) = resource.id() {
                    if !id.starts_with('#') {
                        resolver = resolver.in_subresource(resource).expect("Invalid URI");
                    }
                }
                if let Some((key, Value::String(reference))) = object.get_key_value("$ref") {
                    let resolved = resolver.lookup(reference).expect("Unresolvable reference");
                    let value = resolved.contents();

                    if seen.insert(value) {
                        let ref_node_id = new_node(
                            &mut arena,
                            Some(node_id),
                            Some(EdgeLabel::Key(key)),
                            NodeValue::Reference(reference, NodeId::root_id()),
                        );

                        pending_references.push((ref_node_id, value as *const Value));
                        stack.push_back((resolved.resolver().clone(), None, None, value));
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
            if let NodeValue::Reference(_, reference_target_id) = &mut arena[node_id.get()].value {
                *reference_target_id = *target_id;
            } else {
                panic!("Node is not a reference")
            }
        } else {
            panic!("Reference not found")
        }
    }

    arena
}

fn new_node<'a>(
    arena: &mut Vec<Node<'a>>,
    parent: Option<NodeId>,
    label: Option<EdgeLabel<'a>>,
    value: NodeValue<'a>,
) -> NodeId {
    let id = NodeId::new(arena.len());
    arena.push(Node {
        parent,
        first_child: None,
        next_sibling: None,
        parent_label: label,
        value,
    });
    // Link into parent's child list
    if let Some(parent_id) = parent {
        let parent = &mut arena[parent_id.get()];
        if let Some(child_id) = parent.first_child {
            // Walk to last sibling
            let mut sibling_id = child_id.get();
            while let Some(next) = arena[sibling_id].next_sibling {
                sibling_id = next.get();
            }
            arena[sibling_id].next_sibling = Some(id);
        } else {
            parent.first_child = Some(id);
        }
    }
    id
}
