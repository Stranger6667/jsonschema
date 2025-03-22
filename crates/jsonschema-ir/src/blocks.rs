use core::slice;

use crate::nodes::Node;

/// Unique identificator of a JSON Schema within `Schema`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct BlockId(u32);

impl BlockId {
    pub(crate) fn new(value: u32) -> BlockId {
        BlockId(value)
    }
}

/// A single JSON Schema instance without concrete metadata.
pub struct SubSchema {
    id: BlockId,
    nodes: Vec<Node>,
}

impl SubSchema {
    pub(crate) fn new(id: BlockId) -> SubSchema {
        Self {
            id,
            nodes: Vec::new(),
        }
    }

    pub(crate) fn push(&mut self, node: Node) {
        self.nodes.push(node);
    }

    pub(crate) fn nodes(&self) -> slice::Iter<'_, Node> {
        self.nodes.iter()
    }
}
