use std::{
    collections::{HashMap, HashSet, VecDeque},
    iter::successors,
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
    pub parent: Option<NodeId>,
    pub first_child: Option<NodeId>,
    pub next_sibling: Option<NodeId>,
    pub parent_label: Option<EdgeLabel<'a>>,
    pub value: NodeValue<'a>,
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
) -> ResolvedSchema<'a> {
    let mut schema = ResolvedSchema::new();
    let mut value_to_node_id = HashMap::new();
    let mut pending_references = Vec::new();
    let mut seen = HashSet::new();
    seen.insert(value);
    let resolver = registry.resolver(base_uri);
    let mut stack = VecDeque::new();
    stack.push_back((resolver, None, None, value));

    while let Some((mut resolver, parent, label, value)) = stack.pop_front() {
        let node_id = match value {
            Value::Null => schema.push(parent, label, NodeValue::Null),
            Value::Bool(b) => schema.push(parent, label, NodeValue::Bool(*b)),
            Value::Number(n) => schema.push(parent, label, NodeValue::Number(n)),
            Value::String(s) => schema.push(parent, label, NodeValue::String(s)),
            Value::Array(arr) => {
                let node_id = schema.push(parent, label, NodeValue::Array);
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
                let node_id = schema.push(parent, label, NodeValue::Object);
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
                        let ref_node_id = schema.push(
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
            if let NodeValue::Reference(_, reference_target_id) = &mut schema.get_mut(node_id).value
            {
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

#[derive(Debug)]
pub struct ResolvedSchema<'a> {
    nodes: Vec<Node<'a>>,
}

impl<'a> ResolvedSchema<'a> {
    fn new() -> Self {
        // dummy slot at index 0
        let nodes = vec![Node {
            parent: None,
            first_child: None,
            next_sibling: None,
            parent_label: None,
            value: NodeValue::Null,
        }];
        ResolvedSchema { nodes }
    }
    /// Append a new node, link it into its parent, and return its `NodeId`.
    pub fn push(
        &mut self,
        parent: Option<NodeId>,
        label: Option<EdgeLabel<'a>>,
        value: NodeValue<'a>,
    ) -> NodeId {
        let id = NodeId::new(self.nodes.len());
        self.nodes.push(Node {
            parent,
            first_child: None,
            next_sibling: None,
            parent_label: label,
            value,
        });

        if let Some(parent_id) = parent {
            let parent = &mut self.nodes[parent_id.get()];
            if let Some(child) = parent.first_child {
                // find tail of sibling chain
                let mut child_id = child.get();
                while let Some(next) = self.nodes[child_id].next_sibling {
                    child_id = next.get();
                }
                self.nodes[child_id].next_sibling = Some(id);
            } else {
                parent.first_child = Some(id);
            }
        }

        id
    }

    /// The `NodeId` of the actual root (skips index 0).
    pub fn root(&self) -> NodeId {
        NodeId::root_id()
    }

    /// Immutably get a node by its `NodeId`.
    pub fn get(&self, id: NodeId) -> &Node<'a> {
        &self.nodes[id.get()]
    }

    fn get_mut(&mut self, id: NodeId) -> &mut Node<'a> {
        &mut self.nodes[id.get()]
    }

    pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        successors(self.get(id).first_child, |&node| {
            self.get(node).next_sibling
        })
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.get(id).parent
    }

    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        self.get(id).next_sibling
    }
}
