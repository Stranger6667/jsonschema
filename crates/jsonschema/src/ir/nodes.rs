use std::num::NonZeroUsize;

use super::number::Number;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct NodeId(NonZeroUsize);

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
    pub(crate) fn placeholder() -> NodeId {
        NodeId::new(usize::MAX)
    }
    #[inline]
    pub(crate) fn get(self) -> usize {
        self.0.get()
    }
}

#[derive(Debug)]
pub(crate) enum IRValue<'a> {
    Null,
    Bool(bool),
    Number(Number),
    String(&'a String),
    Object,
    Array,
    Reference(NodeId),
}

#[derive(Debug)]
pub enum EdgeLabel<'a> {
    Key(&'a String),
    Index(usize),
}

#[derive(Debug)]
pub(crate) struct IRNode<'a> {
    pub(super) parent: Option<NodeId>,
    pub(super) first_child: Option<NodeId>,
    pub(super) last_child: Option<NodeId>,
    pub(super) next_sibling: Option<NodeId>,
    pub(super) label: Option<EdgeLabel<'a>>,
    pub(super) value: IRValue<'a>,
}
