use crate::{
    output::{Annotations, ErrorDescription},
    paths::Location,
};
use referencing::Uri;
use serde::{
    ser::{SerializeMap, SerializeSeq, SerializeStruct},
    Serialize,
};
use std::{fmt::Write, sync::Arc};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EvaluationNode {
    pub(crate) keyword_location: Location,
    pub(crate) absolute_keyword_location: Option<Arc<Uri<String>>>,
    pub(crate) schema_location: String,
    pub(crate) instance_location: Location,
    pub(crate) valid: bool,
    pub(crate) annotations: Option<Annotations>,
    pub(crate) dropped_annotations: Option<Annotations>,
    pub(crate) errors: Vec<ErrorDescription>,
    pub(crate) children: Vec<EvaluationNode>,
}

impl EvaluationNode {
    pub(crate) fn valid(
        keyword_location: Location,
        absolute_keyword_location: Option<Arc<Uri<String>>>,
        schema_location: String,
        instance_location: Location,
        annotations: Option<Annotations>,
        children: Vec<EvaluationNode>,
    ) -> Self {
        EvaluationNode {
            keyword_location,
            absolute_keyword_location,
            schema_location,
            instance_location,
            valid: true,
            annotations,
            dropped_annotations: None,
            errors: Vec::new(),
            children,
        }
    }

    pub(crate) fn invalid(
        keyword_location: Location,
        absolute_keyword_location: Option<Arc<Uri<String>>>,
        schema_location: String,
        instance_location: Location,
        annotations: Option<Annotations>,
        errors: Vec<ErrorDescription>,
        children: Vec<EvaluationNode>,
    ) -> Self {
        EvaluationNode {
            keyword_location,
            absolute_keyword_location,
            schema_location,
            instance_location,
            valid: false,
            annotations: None,
            dropped_annotations: annotations,
            errors,
            children,
        }
    }
}

#[derive(Debug)]
pub struct Evaluation {
    root: EvaluationNode,
}

impl Evaluation {
    pub(crate) fn new(root: EvaluationNode) -> Self {
        Evaluation { root }
    }

    #[must_use]
    pub fn flag(&self) -> FlagOutput {
        FlagOutput {
            valid: self.root.valid,
        }
    }

    #[must_use]
    pub fn list(&self) -> ListOutput<'_> {
        ListOutput { root: &self.root }
    }

    #[must_use]
    pub fn hierarchical(&self) -> HierarchicalOutput<'_> {
        HierarchicalOutput { root: &self.root }
    }

    /// Iterates over every annotation produced during evaluation.
    #[must_use]
    pub fn iter_annotations(&self) -> AnnotationIter<'_> {
        AnnotationIter::new(&self.root)
    }

    /// Iterates over every error produced during evaluation.
    #[must_use]
    pub fn iter_errors(&self) -> ErrorIter<'_> {
        ErrorIter::new(&self.root)
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct FlagOutput {
    pub valid: bool,
}

#[derive(Debug)]
pub struct ListOutput<'a> {
    root: &'a EvaluationNode,
}

#[derive(Debug)]
pub struct HierarchicalOutput<'a> {
    root: &'a EvaluationNode,
}

impl Serialize for ListOutput<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serialize_list(self.root, serializer)
    }
}

impl Serialize for HierarchicalOutput<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serialize_hierarchical(self.root, serializer)
    }
}

fn serialize_list<S>(root: &EvaluationNode, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let mut state = serializer.serialize_struct("ListOutput", 2)?;
    state.serialize_field("valid", &root.valid)?;
    let mut entries = Vec::new();
    collect_list_entries(root, &mut Vec::new(), &mut entries);
    state.serialize_field("details", &entries)?;
    state.end()
}

fn serialize_hierarchical<S>(root: &EvaluationNode, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serialize_unit(root, "", serializer, true)
}

fn collect_list_entries<'a>(
    node: &'a EvaluationNode,
    path: &mut Vec<usize>,
    out: &mut Vec<ListEntry<'a>>,
) {
    out.push(ListEntry::new(node, path));
    for (idx, child) in node.children.iter().enumerate() {
        path.push(idx);
        collect_list_entries(child, path, out);
        path.pop();
    }
}

fn serialize_unit<S>(
    node: &EvaluationNode,
    evaluation_path: &str,
    serializer: S,
    include_children: bool,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let mut state = serializer.serialize_struct("OutputUnit", 7)?;
    state.serialize_field("valid", &node.valid)?;
    state.serialize_field("evaluationPath", evaluation_path)?;
    state.serialize_field("schemaLocation", &node.schema_location)?;
    state.serialize_field("instanceLocation", node.instance_location.as_str())?;
    if let Some(annotations) = &node.annotations {
        state.serialize_field("annotations", annotations)?;
    }
    if let Some(annotations) = &node.dropped_annotations {
        state.serialize_field("droppedAnnotations", annotations)?;
    }
    if !node.errors.is_empty() {
        state.serialize_field("errors", &ErrorEntries(&node.errors))?;
    }
    if include_children && !node.children.is_empty() {
        state.serialize_field(
            "details",
            &DetailsSerializer {
                parent_path: evaluation_path,
                children: &node.children,
            },
        )?;
    }
    state.end()
}

fn path_to_string(path: &[usize]) -> String {
    if path.is_empty() {
        String::new()
    } else {
        use itoa::Buffer;
        let mut result = String::with_capacity(path.len() * 2);
        let mut buf = Buffer::new();
        for segment in path {
            result.push('/');
            result.push_str(buf.format(*segment));
        }
        result
    }
}

pub(crate) fn format_schema_location(
    location: &Location,
    absolute: Option<&Arc<Uri<String>>>,
) -> String {
    if let Some(uri) = absolute {
        let base = uri.as_str();
        if base.contains('#') {
            base.to_string()
        } else if location.as_str().is_empty() {
            format!("{base}#")
        } else {
            format!("{base}#{}", location.as_str())
        }
    } else {
        location.as_str().to_string()
    }
}

struct ListEntry<'a> {
    node: &'a EvaluationNode,
    evaluation_path: String,
}

impl<'a> ListEntry<'a> {
    fn new(node: &'a EvaluationNode, path: &[usize]) -> Self {
        ListEntry {
            node,
            evaluation_path: path_to_string(path),
        }
    }
}

impl Serialize for ListEntry<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serialize_unit(self.node, &self.evaluation_path, serializer, false)
    }
}

struct DetailsSerializer<'a> {
    parent_path: &'a str,
    children: &'a [EvaluationNode],
}

impl Serialize for DetailsSerializer<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(self.children.len()))?;
        let mut buffer = String::new();
        for (idx, child) in self.children.iter().enumerate() {
            buffer.clear();
            if self.parent_path.is_empty() {
                write!(&mut buffer, "/{}", idx).expect("writing to string cannot fail");
            } else {
                buffer.push_str(self.parent_path);
                write!(&mut buffer, "/{}", idx).expect("writing to string cannot fail");
            }
            seq.serialize_element(&SeqEntry {
                node: child,
                path: &buffer,
            })?;
        }
        seq.end()
    }
}

struct SeqEntry<'a> {
    node: &'a EvaluationNode,
    path: &'a str,
}

impl Serialize for SeqEntry<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serialize_unit(self.node, self.path, serializer, true)
    }
}

/// Entry describing annotations emitted by a keyword during evaluation.
#[derive(Clone, Copy, Debug)]
pub struct AnnotationEntry<'a> {
    pub schema_location: &'a str,
    pub absolute_keyword_location: Option<&'a Uri<String>>,
    pub instance_location: &'a Location,
    pub annotations: &'a Annotations,
}

/// Entry describing errors emitted by a keyword during evaluation.
#[derive(Clone, Copy, Debug)]
pub struct ErrorEntry<'a> {
    pub schema_location: &'a str,
    pub absolute_keyword_location: Option<&'a Uri<String>>,
    pub instance_location: &'a Location,
    pub error: &'a ErrorDescription,
}

struct NodeIter<'a> {
    stack: Vec<&'a EvaluationNode>,
}

impl<'a> NodeIter<'a> {
    fn new(root: &'a EvaluationNode) -> Self {
        NodeIter { stack: vec![root] }
    }
}

impl<'a> Iterator for NodeIter<'a> {
    type Item = &'a EvaluationNode;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.stack.pop()?;
        for child in node.children.iter().rev() {
            self.stack.push(child);
        }
        Some(node)
    }
}

/// Iterator over annotations produced during evaluation.
pub struct AnnotationIter<'a> {
    nodes: NodeIter<'a>,
}

impl<'a> AnnotationIter<'a> {
    fn new(root: &'a EvaluationNode) -> Self {
        AnnotationIter {
            nodes: NodeIter::new(root),
        }
    }
}

impl<'a> Iterator for AnnotationIter<'a> {
    type Item = AnnotationEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        for node in self.nodes.by_ref() {
            if let Some(annotations) = node.annotations.as_ref() {
                return Some(AnnotationEntry {
                    schema_location: &node.schema_location,
                    absolute_keyword_location: node.absolute_keyword_location.as_deref(),
                    instance_location: &node.instance_location,
                    annotations,
                });
            }
        }
        None
    }
}

/// Iterator over errors produced during evaluation.
pub struct ErrorIter<'a> {
    nodes: NodeIter<'a>,
    current: Option<(&'a EvaluationNode, usize)>,
}

impl<'a> ErrorIter<'a> {
    fn new(root: &'a EvaluationNode) -> Self {
        ErrorIter {
            nodes: NodeIter::new(root),
            current: None,
        }
    }
}

impl<'a> Iterator for ErrorIter<'a> {
    type Item = ErrorEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((node, idx)) = self.current {
                if idx < node.errors.len() {
                    let entry = ErrorEntry {
                        schema_location: &node.schema_location,
                        absolute_keyword_location: node.absolute_keyword_location.as_deref(),
                        instance_location: &node.instance_location,
                        error: &node.errors[idx],
                    };
                    self.current = Some((node, idx + 1));
                    return Some(entry);
                }
                self.current = None;
            }

            match self.nodes.next() {
                Some(node) => {
                    if node.errors.is_empty() {
                        continue;
                    }
                    self.current = Some((node, 0));
                }
                None => return None,
            }
        }
    }
}

struct ErrorEntries<'a>(&'a [ErrorDescription]);

impl Serialize for ErrorEntries<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        let mut key = String::from("error0");
        for (idx, error) in self.0.iter().enumerate() {
            key.truncate(5);
            write!(&mut key, "{}", idx).expect("writing to string cannot fail");
            map.serialize_entry(&key, error)?;
        }

        map.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn loc() -> Location {
        Location::new()
    }

    fn annotation(value: serde_json::Value) -> Annotations {
        Annotations::from(value)
    }

    fn leaf_with_annotation(schema: &str, ann: serde_json::Value) -> EvaluationNode {
        EvaluationNode::valid(
            loc(),
            None,
            schema.to_string(),
            loc(),
            Some(annotation(ann)),
            Vec::new(),
        )
    }

    fn leaf_with_error(schema: &str, msg: &str) -> EvaluationNode {
        EvaluationNode::invalid(
            loc(),
            None,
            schema.to_string(),
            loc(),
            None,
            vec![ErrorDescription::from(msg)],
            Vec::new(),
        )
    }

    #[test]
    fn iter_annotations_visits_all_nodes() {
        let child = leaf_with_annotation("/child", json!({"k": "v"}));
        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            Some(annotation(json!({"root": true}))),
            vec![child],
        );
        let evaluation = Evaluation::new(root);
        let entries: Vec<_> = evaluation.iter_annotations().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].schema_location, "/root");
        assert_eq!(entries[1].schema_location, "/child");
    }

    #[test]
    fn iter_errors_visits_all_nodes() {
        let child = leaf_with_error("/child", "boom");
        let root = EvaluationNode::invalid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            None,
            vec![ErrorDescription::from("root error")],
            vec![child],
        );
        let evaluation = Evaluation::new(root);
        let entries: Vec<_> = evaluation.iter_errors().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].error.to_string(), "root error");
        assert_eq!(entries[1].error.to_string(), "boom");
    }
}
