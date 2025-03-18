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
pub struct Block {
    id: BlockId,
    nodes: Vec<Node>,
}
