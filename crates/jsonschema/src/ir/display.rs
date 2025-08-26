use crate::ir::EdgeLabel;

use super::{IRValue, NodeId, SchemaIR};

pub(crate) struct IRJsonAdapter<'s, 'a> {
    pub(super) schema: &'s SchemaIR<'a>,
    pub(super) node_id: NodeId,
}

impl<'s, 'a> std::fmt::Display for IRJsonAdapter<'s, 'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.write_node(f, self.node_id, 0)
    }
}

impl<'s, 'a> IRJsonAdapter<'s, 'a> {
    fn write_node(
        &self,
        f: &mut std::fmt::Formatter<'_>,
        node_id: NodeId,
        depth: usize,
    ) -> std::fmt::Result {
        match &self.schema[node_id].value {
            IRValue::Null => write!(f, "null"),
            IRValue::Bool(b) => write!(f, "{}", b),
            IRValue::Number(n) => write!(f, "{}", n),
            IRValue::String(s) => write!(f, "\"{}\"", s),
            IRValue::Object => self.write_object(f, node_id, depth),
            IRValue::Array => self.write_array(f, node_id, depth),
            IRValue::Reference(target) => write!(f, "{}", target.get()),
        }
    }

    fn write_object(
        &self,
        f: &mut std::fmt::Formatter<'_>,
        node_id: NodeId,
        depth: usize,
    ) -> std::fmt::Result {
        writeln!(f, "{{")?;
        for child_id in self.schema.children(node_id) {
            let child = &self.schema[child_id];
            if let Some(EdgeLabel::Key(key)) = &child.label {
                for _ in 0..=depth {
                    write!(f, "  ")?;
                }

                write!(f, "\"{}\": ", key)?;
                self.write_node(f, child_id, depth + 1)?;

                if child.next_sibling.is_some() {
                    write!(f, ",")?;
                }
                writeln!(f)?;
            }
        }
        for _ in 0..depth {
            write!(f, "  ")?;
        }
        write!(f, "}}")
    }

    fn write_array(
        &self,
        f: &mut std::fmt::Formatter<'_>,
        node_id: NodeId,
        depth: usize,
    ) -> std::fmt::Result {
        write!(f, "[")?;
        for (i, child_id) in self.schema.children(node_id).enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            self.write_node(f, child_id, depth)?;
        }
        write!(f, "]")
    }
}
