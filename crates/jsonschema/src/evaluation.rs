use crate::{paths::Location, ValidationError};
use ahash::AHashMap;
use referencing::Uri;
use serde::{
    ser::{SerializeMap, SerializeSeq, SerializeStruct},
    Serialize,
};
use std::{
    fmt::{self, Write},
    sync::Arc,
};

/// Annotations associated with an output unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Annotations(Arc<serde_json::Value>);

impl Annotations {
    /// Create a new `Annotations` instance.
    #[must_use]
    pub(crate) fn new(v: serde_json::Value) -> Self {
        Annotations(Arc::new(v))
    }

    /// Create a new `Annotations` instance from an Arc.
    #[must_use]
    pub(crate) fn from_arc(v: Arc<serde_json::Value>) -> Self {
        Annotations(v)
    }

    /// Returns the inner [`serde_json::Value`] of the annotation.
    #[inline]
    #[must_use]
    pub fn into_inner(self) -> serde_json::Value {
        Arc::try_unwrap(self.0).unwrap_or_else(|arc| (*arc).clone())
    }

    /// The `serde_json::Value` of the annotation.
    #[must_use]
    pub fn value(&self) -> &serde_json::Value {
        &self.0
    }
}

impl serde::Serialize for Annotations {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

/// Description of a validation error used within evaluation outputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorDescription {
    keyword: String,
    message: String,
}

impl ErrorDescription {
    #[inline]
    #[must_use]
    pub(crate) fn new(keyword: impl Into<String>, message: String) -> Self {
        Self {
            keyword: keyword.into(),
            message,
        }
    }

    /// Create an `ErrorDescription` from a `ValidationError`.
    #[inline]
    #[must_use]
    pub(crate) fn from_validation_error(e: &ValidationError<'_>) -> Self {
        ErrorDescription {
            keyword: e.kind().keyword().to_owned(),
            message: e.to_string(),
        }
    }

    /// Returns the keyword associated with this error.
    #[inline]
    #[must_use]
    pub fn keyword(&self) -> &str {
        &self.keyword
    }

    /// Returns the inner [`String`] of the error description.
    #[inline]
    #[must_use]
    pub fn into_inner(self) -> String {
        self.message
    }

    /// Returns the message of the error description.
    #[inline]
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ErrorDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

pub(crate) const NO_NODE: u32 = u32::MAX;

#[derive(Debug, PartialEq)]
pub(crate) struct EvaluationNode {
    pub(crate) keyword_location: Location,
    pub(crate) absolute_keyword_location: Option<Arc<Uri<String>>>,
    pub(crate) schema_location: Arc<str>,
    pub(crate) instance_location: Location,
    pub(crate) valid: bool,
    pub(crate) annotations: Option<Annotations>,
    pub(crate) dropped_annotations: Option<Annotations>,
    pub(crate) errors: Vec<ErrorDescription>,
    pub(crate) first_child: u32,
    pub(crate) next_sibling: u32,
}

/// Nodes per chunk. A power of two so the index split is a shift and a mask.
const CHUNK: usize = 32;

/// Every node of one evaluation, in chunks that are never resized, so growth copies nothing.
///
/// Children are a sibling chain rather than a `Vec` per node: subtrees are finished in the order
/// they are evaluated, so a parent cannot know where its children sit until they all exist.
#[derive(Debug, Default)]
pub(crate) struct EvaluationArena {
    chunks: Vec<Vec<EvaluationNode>>,
    len: usize,
}

impl EvaluationArena {
    #[inline]
    pub(crate) fn push(&mut self, node: EvaluationNode) -> u32 {
        let index = u32::try_from(self.len).expect("evaluation exceeded u32 nodes");
        if self.len % CHUNK == 0 {
            self.chunks.push(Vec::with_capacity(CHUNK));
        }
        self.chunks
            .last_mut()
            .expect("a chunk was just reserved")
            .push(node);
        self.len += 1;
        index
    }

    #[inline]
    pub(crate) fn node(&self, index: u32) -> &EvaluationNode {
        let index = index as usize;
        &self.chunks[index / CHUNK][index % CHUNK]
    }

    #[inline]
    fn node_mut(&mut self, index: u32) -> &mut EvaluationNode {
        let index = index as usize;
        &mut self.chunks[index / CHUNK][index % CHUNK]
    }

    pub(crate) fn child_indices(&self, index: u32) -> ChildIndices<'_> {
        ChildIndices {
            arena: self,
            current: self.node(index).first_child,
        }
    }
}

pub(crate) struct ChildIndices<'a> {
    arena: &'a EvaluationArena,
    current: u32,
}

impl Iterator for ChildIndices<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        if self.current == NO_NODE {
            return None;
        }
        let index = self.current;
        self.current = self.arena.node(index).next_sibling;
        Some(index)
    }
}

/// The children collected for one node, as a chain into [`EvaluationArena`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChildList {
    first: u32,
    last: u32,
    len: u32,
    /// Whether every child pushed so far is valid.
    valid: bool,
}

impl Default for ChildList {
    fn default() -> Self {
        ChildList {
            first: NO_NODE,
            last: NO_NODE,
            len: 0,
            valid: true,
        }
    }
}

impl ChildList {
    #[inline]
    pub(crate) fn push(&mut self, arena: &mut EvaluationArena, node: EvaluationNode) {
        self.valid &= node.valid;
        let index = arena.push(node);
        if self.last == NO_NODE {
            self.first = index;
        } else {
            arena.node_mut(self.last).next_sibling = index;
        }
        self.last = index;
        self.len += 1;
    }

    pub(crate) fn len(self) -> usize {
        self.len as usize
    }

    pub(crate) fn all_valid(self) -> bool {
        self.valid
    }

    /// Move `other`'s children onto the end of this list.
    pub(crate) fn append(&mut self, arena: &mut EvaluationArena, other: ChildList) {
        if other.first == NO_NODE {
            return;
        }
        if self.last == NO_NODE {
            self.first = other.first;
        } else {
            arena.node_mut(self.last).next_sibling = other.first;
        }
        self.last = other.last;
        self.len += other.len;
        self.valid &= other.valid;
    }

    /// A list of nodes that were collected before the arena was reachable.
    #[cfg(test)]
    pub(crate) fn from_nodes(
        arena: &mut EvaluationArena,
        nodes: impl IntoIterator<Item = EvaluationNode>,
    ) -> Self {
        let mut list = ChildList::default();
        for node in nodes {
            list.push(arena, node);
        }
        list
    }

    /// A list holding `node` alone.
    #[inline]
    pub(crate) fn of(arena: &mut EvaluationArena, node: EvaluationNode) -> Self {
        let mut list = ChildList::default();
        list.push(arena, node);
        list
    }
}

impl EvaluationNode {
    pub(crate) fn valid(
        keyword_location: Location,
        absolute_keyword_location: Option<Arc<Uri<String>>>,
        schema_location: impl Into<Arc<str>>,
        instance_location: Location,
        annotations: Option<Annotations>,
        children: ChildList,
    ) -> Self {
        let schema_location = schema_location.into();
        EvaluationNode {
            keyword_location,
            absolute_keyword_location,
            schema_location,
            instance_location,
            valid: true,
            annotations,
            dropped_annotations: None,
            errors: Vec::new(),
            first_child: children.first,
            next_sibling: NO_NODE,
        }
    }

    pub(crate) fn invalid(
        keyword_location: Location,
        absolute_keyword_location: Option<Arc<Uri<String>>>,
        schema_location: impl Into<Arc<str>>,
        instance_location: Location,
        annotations: Option<Annotations>,
        errors: Vec<ErrorDescription>,
        children: ChildList,
    ) -> Self {
        let schema_location = schema_location.into();
        EvaluationNode {
            keyword_location,
            absolute_keyword_location,
            schema_location,
            instance_location,
            valid: false,
            annotations: None,
            dropped_annotations: annotations,
            errors,
            first_child: children.first,
            next_sibling: NO_NODE,
        }
    }
}

/// Result of evaluating a JSON instance against a schema.
///
/// This type provides access to structured output formats as defined in the
/// [JSON Schema specification](https://json-schema.org/draft/2020-12/json-schema-core#name-output-structure).
///
/// # Output Formats
///
/// The evaluation result can be accessed in three standard formats:
///
/// - **Flag**: Simple boolean validity indicator via [`flag()`](Self::flag)
/// - **List**: Flat list of all evaluation units via [`list()`](Self::list)
/// - **Hierarchical**: Nested tree structure via [`hierarchical()`](Self::hierarchical)
///
/// All formats are serializable to JSON using `serde_json`.
///
/// # Examples
///
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({"type": "string", "minLength": 3});
/// let validator = jsonschema::validator_for(&schema)?;
///
/// // Evaluate an instance
/// let instance = json!("ab");
/// let evaluation = validator.evaluate(&instance);
///
/// // Check validity with flag format
/// let flag = evaluation.flag();
/// assert!(!flag.valid);
///
/// // Get structured output as JSON
/// let list_output = serde_json::to_value(evaluation.list())?;
/// println!("{}", serde_json::to_string_pretty(&list_output)?);
///
/// // Iterate over errors
/// for error in evaluation.iter_errors() {
///     println!("Error at {}: {}", error.instance_location, error.error);
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct Evaluation {
    arena: EvaluationArena,
    root: u32,
}

impl Evaluation {
    pub(crate) fn new(arena: EvaluationArena, root: u32) -> Self {
        Evaluation { arena, root }
    }

    #[cfg(test)]
    fn with_root(mut arena: EvaluationArena, root: EvaluationNode) -> Self {
        let root = arena.push(root);
        Evaluation::new(arena, root)
    }

    fn root(&self) -> &EvaluationNode {
        self.arena.node(self.root)
    }
    /// Returns the flag output format.
    ///
    /// This is the simplest output format, containing only a boolean indicating
    /// whether the instance is valid according to the schema.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({"type": "number"});
    /// let validator = jsonschema::validator_for(&schema)?;
    ///
    /// let evaluation = validator.evaluate(&json!(42));
    /// let flag = evaluation.flag();
    /// assert!(flag.valid);
    ///
    /// let evaluation = validator.evaluate(&json!("not a number"));
    /// let flag = evaluation.flag();
    /// assert!(!flag.valid);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn flag(&self) -> FlagOutput {
        FlagOutput {
            valid: self.root().valid,
        }
    }
    /// Whether the instance is valid against the schema.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let validator = jsonschema::validator_for(&json!({"type": "number"}))?;
    ///
    /// assert!(validator.evaluate(&json!(42)).is_valid());
    /// assert!(!validator.evaluate(&json!("oops")).is_valid());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.root().valid
    }
    /// Returns the list output format.
    ///
    /// This format provides a flat list of all evaluation units, where each unit
    /// contains information about a specific validation step including its location,
    /// validity, annotations, and errors.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({
    ///     "type": "array",
    ///     "prefixItems": [{"type": "string"}],
    ///     "items": {"type": "integer"}
    /// });
    /// let validator = jsonschema::validator_for(&schema)?;
    /// let evaluation = validator.evaluate(&json!(["hello", "oops"]));
    ///
    /// assert_eq!(
    ///     serde_json::to_value(evaluation.list())?,
    ///     json!({
    ///         "valid": false,
    ///         "details": [
    ///             {"evaluationPath": "", "instanceLocation": "", "schemaLocation": "", "valid": false},
    ///             {
    ///                 "valid": true,
    ///                 "evaluationPath": "/type",
    ///                 "instanceLocation": "",
    ///                 "schemaLocation": "/type"
    ///             },
    ///             {
    ///                 "valid": false,
    ///                 "evaluationPath": "/items",
    ///                 "instanceLocation": "",
    ///                 "schemaLocation": "/items",
    ///                 "droppedAnnotations": true
    ///             },
    ///             {
    ///                 "valid": false,
    ///                 "evaluationPath": "/items",
    ///                 "instanceLocation": "/1",
    ///                 "schemaLocation": "/items"
    ///             },
    ///             {
    ///                 "valid": false,
    ///                 "evaluationPath": "/items/type",
    ///                 "instanceLocation": "/1",
    ///                 "schemaLocation": "/items/type",
    ///                 "errors": {"type": "\"oops\" is not of type \"integer\""}
    ///             },
    ///             {
    ///                 "valid": true,
    ///                 "evaluationPath": "/prefixItems",
    ///                 "instanceLocation": "",
    ///                 "schemaLocation": "/prefixItems",
    ///                 "annotations": 0
    ///             },
    ///             {
    ///                 "valid": true,
    ///                 "evaluationPath": "/prefixItems/0",
    ///                 "instanceLocation": "/0",
    ///                 "schemaLocation": "/prefixItems/0"
    ///             },
    ///             {
    ///                 "valid": true,
    ///                 "evaluationPath": "/prefixItems/0/type",
    ///                 "instanceLocation": "/0",
    ///                 "schemaLocation": "/prefixItems/0/type"
    ///             }
    ///         ]
    ///     })
    /// );
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn list(&self) -> ListOutput<'_> {
        ListOutput {
            arena: &self.arena,
            root: self.root,
        }
    }
    /// Returns the hierarchical output format.
    ///
    /// This format represents the evaluation as a tree structure that mirrors the
    /// schema's logical structure. Each node contains its validation result along
    /// with nested child nodes representing sub-schema evaluations.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({
    ///     "type": "array",
    ///     "prefixItems": [{"type": "string"}],
    ///     "items": {"type": "integer"}
    /// });
    /// let validator = jsonschema::validator_for(&schema)?;
    /// let evaluation = validator.evaluate(&json!(["hello", "oops"]));
    ///
    /// assert_eq!(
    ///     serde_json::to_value(evaluation.hierarchical())?,
    ///     json!({
    ///         "valid": false,
    ///         "evaluationPath": "",
    ///         "schemaLocation": "",
    ///         "instanceLocation": "",
    ///         "details": [
    ///             {
    ///                 "valid": true,
    ///                 "evaluationPath": "/type",
    ///                 "instanceLocation": "",
    ///                 "schemaLocation": "/type"
    ///             },
    ///             {
    ///                 "valid": false,
    ///                 "evaluationPath": "/items",
    ///                 "instanceLocation": "",
    ///                 "schemaLocation": "/items",
    ///                 "droppedAnnotations": true,
    ///                 "details": [
    ///                     {
    ///                         "valid": false,
    ///                         "evaluationPath": "/items",
    ///                         "instanceLocation": "/1",
    ///                         "schemaLocation": "/items",
    ///                         "details": [
    ///                             {
    ///                                 "valid": false,
    ///                                 "evaluationPath": "/items/type",
    ///                                 "instanceLocation": "/1",
    ///                                 "schemaLocation": "/items/type",
    ///                                 "errors": {"type": "\"oops\" is not of type \"integer\""}
    ///                             }
    ///                         ]
    ///                     }
    ///                 ]
    ///             },
    ///             {
    ///                 "valid": true,
    ///                 "evaluationPath": "/prefixItems",
    ///                 "instanceLocation": "",
    ///                 "schemaLocation": "/prefixItems",
    ///                 "annotations": 0,
    ///                 "details": [
    ///                     {
    ///                         "valid": true,
    ///                         "evaluationPath": "/prefixItems/0",
    ///                         "instanceLocation": "/0",
    ///                         "schemaLocation": "/prefixItems/0",
    ///                         "details": [
    ///                             {
    ///                                 "valid": true,
    ///                                 "evaluationPath": "/prefixItems/0/type",
    ///                                 "instanceLocation": "/0",
    ///                                 "schemaLocation": "/prefixItems/0/type"
    ///                             }
    ///                         ]
    ///                     }
    ///                 ]
    ///             }
    ///         ]
    ///     })
    /// );
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn hierarchical(&self) -> HierarchicalOutput<'_> {
        HierarchicalOutput {
            arena: &self.arena,
            root: self.root,
        }
    }
    /// Returns an iterator over all annotations produced during evaluation.
    ///
    /// Annotations are metadata emitted by keywords during successful validation.
    /// They can be used to collect information about which parts of a schema
    /// matched the instance.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({
    ///     "type": "object",
    ///     "properties": {"name": {"type": "string"}, "age": {"type": "number", "minimum": 0}},
    ///     "required": ["name"]
    /// });
    /// let validator = jsonschema::validator_for(&schema)?;
    /// let evaluation = validator.evaluate(&json!({"name": "Alice", "age": 30}));
    ///
    /// let entries: Vec<_> = evaluation.iter_annotations().collect();
    /// assert_eq!(entries.len(), 1);
    /// assert_eq!(entries[0].schema_location, "/properties");
    /// assert_eq!(entries[0].instance_location.as_str(), "");
    ///
    /// let mut annotation_names: Vec<_> = entries[0]
    ///     .annotations
    ///     .value()
    ///     .as_array()
    ///     .expect("annotation should be an array")
    ///     .iter()
    ///     .map(|value| value.as_str().expect("annotation items should be strings"))
    ///     .collect();
    /// annotation_names.sort_unstable();
    /// assert_eq!(annotation_names, vec!["age", "name"]);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn iter_annotations(&self) -> AnnotationIter<'_> {
        AnnotationIter::new(&self.arena, self.root)
    }
    /// Returns an iterator over all errors produced during evaluation.
    ///
    /// Each error entry contains information about a validation failure,
    /// including its location in both the schema and instance.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use serde_json::json;
    ///
    /// let schema = json!({
    ///     "type": "object",
    ///     "required": ["name"],
    ///     "properties": {
    ///         "age": {"type": "number"}
    ///     }
    /// });
    /// let validator = jsonschema::validator_for(&schema)?;
    /// let evaluation = validator.evaluate(&json!({"name": "Bob", "age": "oops"}));
    ///
    /// let errors: Vec<_> = evaluation.iter_errors().collect();
    /// assert_eq!(errors.len(), 1);
    /// assert_eq!(errors[0].schema_location, "/properties/age/type");
    /// assert_eq!(errors[0].instance_location.as_str(), "/age");
    /// assert_eq!(errors[0].error.to_string(), "\"oops\" is not of type \"number\"");
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn iter_errors(&self) -> ErrorIter<'_> {
        ErrorIter::new(&self.arena, self.root)
    }
}

/// Flag output format containing only a validity indicator.
///
/// This is the simplest output format defined in the JSON Schema specification.
/// It contains only a single boolean field indicating whether validation succeeded.
///
/// # JSON Structure
///
/// ```json
/// {
///   "valid": true
/// }
/// ```
///
/// # Examples
///
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({"type": "string"});
/// let validator = jsonschema::validator_for(&schema)?;
/// let evaluation = validator.evaluate(&json!("hello"));
///
/// let flag = evaluation.flag();
/// assert_eq!(flag.valid, true);
///
/// let output = serde_json::to_value(flag)?;
/// assert_eq!(output, json!({"valid": true}));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Serialize)]
pub struct FlagOutput {
    /// Whether the instance is valid according to the schema.
    pub valid: bool,
}

/// List output format providing a flat list of evaluation units.
///
/// This format represents the evaluation result as a flat sequence where each
/// entry corresponds to a validation step. Each unit includes its evaluation path,
/// schema location, instance location, validity, and any annotations or errors.
///
/// See [`Evaluation::list`] for an example JSON payload produced by this type.
#[derive(Debug)]
pub struct ListOutput<'a> {
    arena: &'a EvaluationArena,
    root: u32,
}

/// Hierarchical output format providing a tree structure of evaluation results.
///
/// This format represents the evaluation as a nested tree that mirrors the logical
/// structure of the schema. Each node contains validation results and child nodes
/// representing nested sub-schema evaluations.
///
/// See [`Evaluation::hierarchical`] for an example JSON payload produced by this type.
#[derive(Debug)]
pub struct HierarchicalOutput<'a> {
    arena: &'a EvaluationArena,
    root: u32,
}

impl Serialize for ListOutput<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serialize_list(self.arena, self.root, serializer)
    }
}

impl Serialize for HierarchicalOutput<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serialize_hierarchical(self.arena, self.root, serializer)
    }
}

fn serialize_list<S>(arena: &EvaluationArena, root: u32, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let mut state = serializer.serialize_struct("ListOutput", 2)?;
    state.serialize_field("valid", &arena.node(root).valid)?;
    let mut entries = Vec::new();
    collect_list_entries(arena, root, &mut entries);
    state.serialize_field("details", &entries)?;
    state.end()
}

fn serialize_hierarchical<S>(
    arena: &EvaluationArena,
    root: u32,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serialize_unit(arena, root, serializer, true)
}

fn collect_list_entries<'a>(arena: &'a EvaluationArena, index: u32, out: &mut Vec<ListEntry<'a>>) {
    // Note: The spec says "Output units which do not contain errors or annotations SHOULD be
    // excluded" but the official test suite includes all nodes. We include all nodes to match
    // the reference implementation and test suite expectations.
    out.push(ListEntry::new(arena, index));
    for child in arena.child_indices(index) {
        collect_list_entries(arena, child, out);
    }
}

fn serialize_unit<S>(
    arena: &EvaluationArena,
    index: u32,
    serializer: S,
    include_children: bool,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let node = arena.node(index);
    let mut state = serializer.serialize_struct("OutputUnit", 7)?;
    state.serialize_field("valid", &node.valid)?;
    state.serialize_field("evaluationPath", node.keyword_location.as_str())?;
    state.serialize_field("schemaLocation", node.schema_location.as_ref())?;
    state.serialize_field("instanceLocation", node.instance_location.as_str())?;
    if let Some(annotations) = &node.annotations {
        state.serialize_field("annotations", annotations)?;
    }
    if let Some(annotations) = &node.dropped_annotations {
        state.serialize_field("droppedAnnotations", annotations)?;
    }
    if !node.errors.is_empty() {
        state.serialize_field("errors", &ErrorEntriesSerializer(&node.errors))?;
    }
    if include_children && node.first_child != NO_NODE {
        state.serialize_field("details", &DetailsSerializer { arena, index })?;
    }
    state.end()
}

pub(crate) fn format_schema_location(
    location: &Location,
    absolute: Option<&Arc<Uri<String>>>,
) -> Arc<str> {
    if let Some(uri) = absolute {
        let base = uri.strip_fragment();
        let suffix = location.as_str();
        crate::paths::build_arc_str(suffix.len() + 1, |buffer| {
            write!(buffer, "{base}#{suffix}").expect("writing to a String cannot fail");
        })
    } else {
        location.as_arc()
    }
}

struct ListEntry<'a> {
    arena: &'a EvaluationArena,
    index: u32,
}

impl<'a> ListEntry<'a> {
    fn new(arena: &'a EvaluationArena, index: u32) -> Self {
        ListEntry { arena, index }
    }
}

impl Serialize for ListEntry<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serialize_unit(self.arena, self.index, serializer, false)
    }
}

struct DetailsSerializer<'a> {
    arena: &'a EvaluationArena,
    index: u32,
}

impl Serialize for DetailsSerializer<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut seq = serializer.serialize_seq(None)?;
        for child in self.arena.child_indices(self.index) {
            seq.serialize_element(&SeqEntry {
                arena: self.arena,
                index: child,
            })?;
        }
        seq.end()
    }
}

struct SeqEntry<'a> {
    arena: &'a EvaluationArena,
    index: u32,
}

impl Serialize for SeqEntry<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serialize_unit(self.arena, self.index, serializer, true)
    }
}

/// Entry describing annotations emitted by a keyword during evaluation.
///
/// Annotations are metadata produced by keywords during successful validation.
/// They provide additional information about which schema keywords matched
/// and what values they produced.
///
/// # Examples
///
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({
///     "type": "object",
///     "properties": {
///         "name": {"type": "string"},
///         "age": {"type": "number"}
///     }
/// });
/// let validator = jsonschema::validator_for(&schema)?;
/// let instance = json!({"name": "Alice", "age": 30});
/// let evaluation = validator.evaluate(&instance);
/// let entry = evaluation.iter_annotations().next().unwrap();
/// assert_eq!(entry.schema_location, "/properties");
/// assert_eq!(entry.instance_location.as_str(), "");
///
/// let mut annotation_names: Vec<_> = entry
///     .annotations
///     .value()
///     .as_array()
///     .expect("annotation should be an array")
///     .iter()
///     .map(|value| value.as_str().expect("annotation items should be strings"))
///     .collect();
/// annotation_names.sort_unstable();
/// assert_eq!(annotation_names, vec!["age", "name"]);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug)]
pub struct AnnotationEntry<'a> {
    /// The JSON Pointer to the schema keyword that produced the annotation.
    pub schema_location: &'a str,
    /// The absolute URI of the keyword location, if available.
    pub absolute_keyword_location: Option<&'a Uri<String>>,
    /// The JSON Pointer to the instance location being validated.
    pub instance_location: &'a Location,
    /// The annotations produced by the keyword.
    pub annotations: &'a Annotations,
}

/// Entry describing errors emitted by a keyword during evaluation.
///
/// Error entries contain information about validation failures, including
/// the locations in both the schema and instance where the error occurred.
///
/// # Examples
///
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({
///     "type": "object",
///     "required": ["name"],
///     "properties": {
///         "age": {"type": "number"}
///     }
/// });
/// let validator = jsonschema::validator_for(&schema)?;
/// let instance = json!({"name": "test", "age": "oops"});
/// let evaluation = validator.evaluate(&instance);
/// let entry = evaluation.iter_errors().next().unwrap();
/// assert_eq!(entry.schema_location, "/properties/age/type");
/// assert_eq!(entry.instance_location.as_str(), "/age");
/// assert_eq!(entry.error.to_string(), "\"oops\" is not of type \"number\"");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug)]
pub struct ErrorEntry<'a> {
    /// The JSON Pointer to the schema keyword that produced the error.
    pub schema_location: &'a str,
    /// The absolute URI of the keyword location, if available.
    pub absolute_keyword_location: Option<&'a Uri<String>>,
    /// The JSON Pointer to the instance location that failed validation.
    pub instance_location: &'a Location,
    /// The error description.
    pub error: &'a ErrorDescription,
}

impl fmt::Display for ErrorEntry<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.instance_location.is_empty() {
            self.error.fmt(f)
        } else {
            write!(f, "{}: {}", self.instance_location, self.error)
        }
    }
}

struct NodeIter<'a> {
    arena: &'a EvaluationArena,
    stack: Vec<u32>,
}

impl<'a> NodeIter<'a> {
    fn new(arena: &'a EvaluationArena, root: u32) -> Self {
        NodeIter {
            arena,
            stack: vec![root],
        }
    }
}

impl<'a> Iterator for NodeIter<'a> {
    type Item = &'a EvaluationNode;

    fn next(&mut self) -> Option<Self::Item> {
        let index = self.stack.pop()?;
        let start = self.stack.len();
        self.stack.extend(self.arena.child_indices(index));
        self.stack[start..].reverse();
        Some(self.arena.node(index))
    }
}

/// Iterator over annotations produced during evaluation.
///
/// This iterator traverses the evaluation tree and yields [`AnnotationEntry`]
/// for each node that produced annotations during validation.
///
/// Annotations are only present for nodes where validation succeeded.
///
/// # Examples
///
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({
///     "type": "object",
///     "properties": {
///         "name": {"type": "string"},
///         "age": {"type": "number"}
///     }
/// });
/// let validator = jsonschema::validator_for(&schema)?;
/// let evaluation = validator.evaluate(&json!({"name": "Alice", "age": 30}));
///
/// let annotations: Vec<_> = evaluation.iter_annotations().collect();
/// assert_eq!(annotations.len(), 1);
/// assert_eq!(annotations[0].instance_location.as_str(), "");
///
/// let mut annotation_names: Vec<_> = annotations[0]
///     .annotations
///     .value()
///     .as_array()
///     .expect("annotation should be an array")
///     .iter()
///     .map(|value| value.as_str().expect("annotation items should be strings"))
///     .collect();
/// annotation_names.sort_unstable();
/// assert_eq!(annotation_names, vec!["age", "name"]);
/// # Ok(())
/// # }
/// ```
pub struct AnnotationIter<'a> {
    nodes: NodeIter<'a>,
}

impl<'a> AnnotationIter<'a> {
    fn new(arena: &'a EvaluationArena, root: u32) -> Self {
        AnnotationIter {
            nodes: NodeIter::new(arena, root),
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
///
/// This iterator traverses the evaluation tree and yields [`ErrorEntry`]
/// for each error encountered during validation.
///
/// Nodes can have multiple errors, and this iterator will yield all of them
/// in depth-first order.
///
/// # Examples
///
/// ```rust
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use serde_json::json;
///
/// let schema = json!({
///     "type": "object",
///     "required": ["name"],
///     "properties": {
///         "name": {"type": "string"},
///         "age": {"type": "number", "minimum": 0}
///     }
/// });
/// let validator = jsonschema::validator_for(&schema)?;
/// let evaluation = validator.evaluate(&json!({"age": -5}));
///
/// let errors: Vec<_> = evaluation.iter_errors().collect();
/// assert_eq!(errors.len(), 2);
/// assert_eq!(errors[0].schema_location, "/required");
/// assert_eq!(errors[0].instance_location.as_str(), "");
/// assert_eq!(errors[1].schema_location, "/properties/age/minimum");
/// assert_eq!(errors[1].instance_location.as_str(), "/age");
/// # Ok(())
/// # }
/// ```
pub struct ErrorIter<'a> {
    nodes: NodeIter<'a>,
    current: Option<(&'a EvaluationNode, usize)>,
}

impl<'a> ErrorIter<'a> {
    fn new(arena: &'a EvaluationArena, root: u32) -> Self {
        ErrorIter {
            nodes: NodeIter::new(arena, root),
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

            {
                let node = self.nodes.next()?;
                if node.errors.is_empty() {
                    continue;
                }
                self.current = Some((node, 0));
            }
        }
    }
}

struct ErrorEntriesSerializer<'a>(&'a [ErrorDescription]);

impl<'a> Serialize for ErrorEntriesSerializer<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut grouped: Vec<(&'a str, Vec<&'a str>)> = Vec::new();
        let mut indexes: AHashMap<&'a str, usize> = AHashMap::new();

        for error in self.0 {
            let keyword = error.keyword();
            let msg = error.message();
            if let Some(&idx) = indexes.get(keyword) {
                grouped[idx].1.push(msg);
            } else {
                indexes.insert(keyword, grouped.len());
                grouped.push((keyword, vec![msg]));
            }
        }

        let mut map = serializer.serialize_map(Some(grouped.len()))?;
        for (keyword, messages) in grouped {
            if messages.len() == 1 {
                map.serialize_entry(keyword, messages[0])?;
            } else {
                map.serialize_entry(keyword, &messages)?;
            }
        }
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::sync::Arc;
    use test_case::test_case;

    fn loc() -> Location {
        Location::new()
    }

    fn annotation(value: serde_json::Value) -> Annotations {
        Annotations::new(value)
    }

    impl ErrorDescription {
        fn from_string(s: &str) -> Self {
            ErrorDescription {
                keyword: "error".into(),
                message: s.to_string(),
            }
        }
    }

    fn leaf_with_annotation(schema: &str, ann: serde_json::Value) -> EvaluationNode {
        EvaluationNode::valid(
            loc(),
            None,
            schema.to_string(),
            loc(),
            Some(annotation(ann)),
            ChildList::default(),
        )
    }

    fn leaf_with_error(schema: &str, msg: &str) -> EvaluationNode {
        EvaluationNode::invalid(
            loc(),
            None,
            schema.to_string(),
            loc(),
            None,
            vec![ErrorDescription::from_string(msg)],
            ChildList::default(),
        )
    }

    #[test]
    fn iter_annotations_visits_all_nodes() {
        let mut arena = EvaluationArena::default();
        let child = leaf_with_annotation("/child", json!({"k": "v"}));
        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            Some(annotation(json!({"root": true}))),
            ChildList::from_nodes(&mut arena, vec![child]),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let entries: Vec<_> = evaluation.iter_annotations().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].schema_location, "/root");
        assert_eq!(entries[1].schema_location, "/child");
    }

    #[test]
    fn iter_errors_visits_all_nodes() {
        let mut arena = EvaluationArena::default();
        let child = leaf_with_error("/child", "boom");
        let root = EvaluationNode::invalid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            None,
            vec![ErrorDescription::from_string("root error")],
            ChildList::from_nodes(&mut arena, vec![child]),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let entries: Vec<_> = evaluation.iter_errors().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].error.to_string(), "root error");
        assert_eq!(entries[1].error.to_string(), "boom");
    }

    #[test]
    fn flag_output_valid() {
        let arena = EvaluationArena::default();
        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            None,
            ChildList::default(),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let flag = evaluation.flag();
        assert!(flag.valid);
    }

    #[test]
    fn flag_output_invalid() {
        let arena = EvaluationArena::default();
        let root = EvaluationNode::invalid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            None,
            vec![ErrorDescription::from_string("error")],
            ChildList::default(),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let flag = evaluation.flag();
        assert!(!flag.valid);
    }

    #[test]
    fn flag_output_serialization() {
        let arena = EvaluationArena::default();
        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            None,
            ChildList::default(),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let flag = evaluation.flag();
        let serialized = serde_json::to_value(flag).expect("serialization succeeds");
        assert_eq!(serialized, json!({"valid": true}));
    }

    #[test]
    fn list_output_serialization_valid() {
        let arena = EvaluationArena::default();
        let root = EvaluationNode::valid(
            loc(),
            None,
            "#".to_string(),
            loc(),
            None,
            ChildList::default(),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let list = evaluation.list();
        let serialized = serde_json::to_value(list).expect("serialization succeeds");
        assert_eq!(
            serialized,
            json!({
                "valid": true,
                "details": [
                    {
                        "valid": true,
                        "evaluationPath": "",
                        "schemaLocation": "#",
                        "instanceLocation": ""
                    }
                ]
            })
        );
    }

    #[test]
    fn list_output_serialization_with_children() {
        let mut arena = EvaluationArena::default();
        let child1 = leaf_with_annotation("/child1", json!({"key": "value"}));
        let child2 = leaf_with_error("/child2", "child error");
        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            Some(annotation(json!({"root": true}))),
            ChildList::from_nodes(&mut arena, vec![child1, child2]),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let list = evaluation.list();
        let serialized = serde_json::to_value(list).expect("serialization succeeds");
        assert_eq!(
            serialized,
            json!({
                "valid": true,
                "details": [
                    {
                        "valid": true,
                        "evaluationPath": "",
                        "schemaLocation": "/root",
                        "instanceLocation": "",
                        "annotations": {"root": true}
                    },
                    {
                        "valid": true,
                        "evaluationPath": "",
                        "schemaLocation": "/child1",
                        "instanceLocation": "",
                        "annotations": {"key": "value"}
                    },
                    {
                        "valid": false,
                        "evaluationPath": "",
                        "schemaLocation": "/child2",
                        "instanceLocation": "",
                        "errors": {"error": "child error"}
                    }
                ]
            })
        );
    }

    #[test]
    fn hierarchical_output_serialization() {
        let mut arena = EvaluationArena::default();
        let child = leaf_with_annotation("/child", json!({"nested": "data"}));
        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            Some(annotation(json!({"root": "annotation"}))),
            ChildList::from_nodes(&mut arena, vec![child]),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let hierarchical = evaluation.hierarchical();
        let serialized = serde_json::to_value(hierarchical).expect("serialization succeeds");
        assert_eq!(
            serialized,
            json!({
                "valid": true,
                "evaluationPath": "",
                "schemaLocation": "/root",
                "instanceLocation": "",
                "annotations": {"root": "annotation"},
                "details": [
                    {
                        "valid": true,
                        "evaluationPath": "",
                        "schemaLocation": "/child",
                        "instanceLocation": "",
                        "annotations": {"nested": "data"}
                    }
                ]
            })
        );
    }

    #[test]
    fn outputs_include_errors_and_dropped_annotations() {
        let mut arena = EvaluationArena::default();
        let invalid_child = EvaluationNode::invalid(
            loc(),
            None,
            "/items/type".to_string(),
            Location::new().join(1usize),
            None,
            vec![ErrorDescription::from_string("child error")],
            ChildList::default(),
        );
        let prefix_child = leaf_with_annotation("/prefix", json!(0));
        let root = EvaluationNode::invalid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            Some(annotation(json!({"dropped": true}))),
            vec![ErrorDescription::from_string("root failure")],
            ChildList::from_nodes(&mut arena, vec![invalid_child, prefix_child]),
        );
        let evaluation = Evaluation::with_root(arena, root);
        let list = serde_json::to_value(evaluation.list()).expect("serialization succeeds");
        assert_eq!(
            list,
            json!({
                "valid": false,
                "details": [
                    {
                        "valid": false,
                        "evaluationPath": "",
                        "schemaLocation": "/root",
                        "instanceLocation": "",
                        "droppedAnnotations": {"dropped": true},
                        "errors": {"error": "root failure"}
                    },
                    {
                        "valid": false,
                        "evaluationPath": "",
                        "schemaLocation": "/items/type",
                        "instanceLocation": "/1",
                        "errors": {"error": "child error"}
                    },
                    {
                        "valid": true,
                        "evaluationPath": "",
                        "schemaLocation": "/prefix",
                        "instanceLocation": "",
                        "annotations": 0
                    }
                ]
            })
        );
        let hierarchical =
            serde_json::to_value(evaluation.hierarchical()).expect("serialization succeeds");
        assert_eq!(
            hierarchical,
            json!({
                "valid": false,
                "evaluationPath": "",
                "schemaLocation": "/root",
                "instanceLocation": "",
                "droppedAnnotations": {"dropped": true},
                "errors": {"error": "root failure"},
                "details": [
                    {
                        "valid": false,
                        "evaluationPath": "",
                        "schemaLocation": "/items/type",
                        "instanceLocation": "/1",
                        "errors": {"error": "child error"}
                    },
                    {
                        "valid": true,
                        "evaluationPath": "",
                        "schemaLocation": "/prefix",
                        "instanceLocation": "",
                        "annotations": 0
                    }
                ]
            })
        );
    }

    #[test]
    fn empty_evaluation_tree() {
        let arena = EvaluationArena::default();
        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            None,
            ChildList::default(),
        );
        let evaluation = Evaluation::with_root(arena, root);

        // No annotations
        assert_eq!(evaluation.iter_annotations().count(), 0);
        // No errors
        assert_eq!(evaluation.iter_errors().count(), 0);

        let flag = evaluation.flag();
        assert!(flag.valid);
    }

    #[test]
    fn deep_nesting() {
        let mut arena = EvaluationArena::default();
        // Create a deeply nested tree: root -> level1 -> level2 -> level3
        let level3 = leaf_with_annotation("/level3", json!({"level": 3}));
        let level2 = EvaluationNode::valid(
            loc(),
            None,
            "/level2".to_string(),
            loc(),
            Some(annotation(json!({"level": 2}))),
            ChildList::from_nodes(&mut arena, vec![level3]),
        );
        let level1 = EvaluationNode::valid(
            loc(),
            None,
            "/level1".to_string(),
            loc(),
            Some(annotation(json!({"level": 1}))),
            ChildList::from_nodes(&mut arena, vec![level2]),
        );
        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            Some(annotation(json!({"level": 0}))),
            ChildList::from_nodes(&mut arena, vec![level1]),
        );

        let evaluation = Evaluation::with_root(arena, root);
        let annotations: Vec<_> = evaluation.iter_annotations().collect();
        assert_eq!(annotations.len(), 4);

        // Check depth-first order
        assert_eq!(annotations[0].schema_location, "/root");
        assert_eq!(annotations[1].schema_location, "/level1");
        assert_eq!(annotations[2].schema_location, "/level2");
        assert_eq!(annotations[3].schema_location, "/level3");
    }

    #[test]
    fn wide_tree() {
        let mut arena = EvaluationArena::default();
        // Create a wide tree with many siblings
        let children = ChildList::from_nodes(
            &mut arena,
            (0..10).map(|i| leaf_with_annotation(&format!("/child{i}"), json!({"index": i}))),
        );

        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            Some(annotation(json!({"root": true}))),
            children,
        );

        let evaluation = Evaluation::with_root(arena, root);
        let annotations: Vec<_> = evaluation.iter_annotations().collect();
        assert_eq!(annotations.len(), 11); // root + 10 children
    }

    #[test]
    fn multiple_errors_per_node() {
        let arena = EvaluationArena::default();
        let errors = vec![
            ErrorDescription::from_string("error 1"),
            ErrorDescription::from_string("error 2"),
            ErrorDescription::from_string("error 3"),
        ];
        let root = EvaluationNode::invalid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            None,
            errors,
            ChildList::default(),
        );

        let evaluation = Evaluation::with_root(arena, root);
        let error_entries: Vec<_> = evaluation.iter_errors().collect();
        assert_eq!(error_entries.len(), 3);
        assert_eq!(error_entries[0].error.to_string(), "error 1");
        assert_eq!(error_entries[1].error.to_string(), "error 2");
        assert_eq!(error_entries[2].error.to_string(), "error 3");
    }

    #[test]
    fn mixed_valid_and_invalid_nodes() {
        let mut arena = EvaluationArena::default();
        let valid_child = leaf_with_annotation("/valid", json!({"ok": true}));
        let invalid_child = leaf_with_error("/invalid", "failed");

        let root = EvaluationNode::invalid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            Some(annotation(json!({"attempted": true}))),
            vec![ErrorDescription::from_string("root failed")],
            ChildList::from_nodes(&mut arena, vec![valid_child, invalid_child]),
        );

        let evaluation = Evaluation::with_root(arena, root);

        // Should have 1 annotation (from valid child only; root has dropped annotations)
        let annotations: Vec<_> = evaluation.iter_annotations().collect();
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].schema_location, "/valid");

        // Should have 2 errors (root + invalid child)
        let errors: Vec<_> = evaluation.iter_errors().collect();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn annotations_iterator_skips_nodes_without_annotations() {
        let mut arena = EvaluationArena::default();
        let no_annotation = EvaluationNode::valid(
            loc(),
            None,
            "/no_ann".to_string(),
            loc(),
            None,
            ChildList::default(),
        );
        let with_annotation = leaf_with_annotation("/with_ann", json!({"present": true}));

        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            None,
            ChildList::from_nodes(&mut arena, vec![no_annotation, with_annotation]),
        );

        let evaluation = Evaluation::with_root(arena, root);
        let annotations: Vec<_> = evaluation.iter_annotations().collect();
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].schema_location, "/with_ann");
    }

    #[test]
    fn errors_iterator_skips_nodes_without_errors() {
        let mut arena = EvaluationArena::default();
        let no_error = EvaluationNode::valid(
            loc(),
            None,
            "/no_error".to_string(),
            loc(),
            Some(annotation(json!({"ok": true}))),
            ChildList::default(),
        );
        let with_error = leaf_with_error("/with_error", "failed");

        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            None,
            ChildList::from_nodes(&mut arena, vec![no_error, with_error]),
        );

        let evaluation = Evaluation::with_root(arena, root);
        let errors: Vec<_> = evaluation.iter_errors().collect();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].schema_location, "/with_error");
    }

    #[test]
    fn error_entries_serialization_empty() {
        let entries = ErrorEntriesSerializer(&[]);
        let serialized = serde_json::to_value(&entries).expect("serialization succeeds");
        assert!(serialized.is_object());
        assert_eq!(serialized.as_object().unwrap().len(), 0);
    }

    #[test]
    fn error_entries_serialization_single() {
        let errors = vec![ErrorDescription::from_string("test error")];
        let entries = ErrorEntriesSerializer(&errors);
        let serialized = serde_json::to_value(&entries).expect("serialization succeeds");
        assert!(serialized.is_object());
        assert_eq!(serialized.as_object().unwrap().len(), 1);
        assert!(serialized.get("error").is_some());
    }

    #[test]
    fn error_entries_serialization_multiple() {
        let errors = vec![
            ErrorDescription::new("alpha", "error 1".to_string()),
            ErrorDescription::new("beta", "error 2".to_string()),
            ErrorDescription::new("gamma", "error 3".to_string()),
        ];
        let entries = ErrorEntriesSerializer(&errors);
        let serialized = serde_json::to_value(&entries).expect("serialization succeeds");
        assert_eq!(serialized.as_object().unwrap().len(), 3);
        assert!(serialized.get("alpha").is_some());
        assert!(serialized.get("beta").is_some());
        assert!(serialized.get("gamma").is_some());
    }

    #[test]
    fn error_entries_serialization_preserves_duplicates() {
        let errors = vec![
            ErrorDescription::new("required", "\"foo\" is required".to_string()),
            ErrorDescription::new("required", "\"bar\" is required".to_string()),
        ];
        let entries = ErrorEntriesSerializer(&errors);
        let serialized = serde_json::to_value(&entries).expect("serialization succeeds");
        let value = serialized
            .get("required")
            .expect("required keyword present")
            .as_array()
            .expect("multiple errors serialized as array");
        assert_eq!(value.len(), 2);
        assert_eq!(value[0], "\"foo\" is required");
        assert_eq!(value[1], "\"bar\" is required");
    }

    #[test]
    fn list_output_preserves_multiple_errors_per_keyword() {
        let arena = EvaluationArena::default();
        let errors = vec![
            ErrorDescription::new("required", "\"foo\" is required".to_string()),
            ErrorDescription::new("required", "\"bar\" is required".to_string()),
        ];
        let root = EvaluationNode::invalid(
            loc(),
            None,
            "/required".to_string(),
            loc(),
            None,
            errors,
            ChildList::default(),
        );

        let evaluation = Evaluation::with_root(arena, root);
        let list = serde_json::to_value(evaluation.list()).expect("serialization succeeds");
        let root_unit = list
            .get("details")
            .and_then(|value| value.as_array())
            .and_then(|details| details.first())
            .expect("list output contains root unit");
        let errors = root_unit
            .get("errors")
            .and_then(|errors| errors.get("required"))
            .and_then(|value| value.as_array())
            .expect("errors serialized as array");
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0], "\"foo\" is required");
        assert_eq!(errors[1], "\"bar\" is required");
    }

    #[test]
    fn format_schema_location_without_absolute() {
        let location = Location::new().join("properties").join("name");
        let formatted = format_schema_location(&location, None);
        assert_eq!(formatted.as_ref(), "/properties/name");
    }

    #[test]
    fn format_schema_location_with_absolute_no_fragment() {
        let location = Location::new().join("properties");
        let uri = Arc::new(
            Uri::parse("http://example.com/schema.json")
                .unwrap()
                .to_owned(),
        );
        let formatted = format_schema_location(&location, Some(&uri));
        assert_eq!(
            formatted.as_ref(),
            "http://example.com/schema.json#/properties"
        );
    }

    #[test]
    fn format_schema_location_with_absolute_empty_location() {
        let location = Location::new();
        let uri = Arc::new(
            Uri::parse("http://example.com/schema.json")
                .unwrap()
                .to_owned(),
        );
        let formatted = format_schema_location(&location, Some(&uri));
        assert_eq!(formatted.as_ref(), "http://example.com/schema.json#");
    }

    #[test]
    fn format_schema_location_with_absolute_existing_fragment() {
        let location = Location::new().join("properties");
        let uri = Arc::new(
            Uri::parse("http://example.com/schema.json#/defs/myDef")
                .unwrap()
                .to_owned(),
        );
        let formatted = format_schema_location(&location, Some(&uri));
        // When URI has a fragment, it's replaced with the location
        assert_eq!(
            formatted.as_ref(),
            "http://example.com/schema.json#/properties"
        );
    }

    #[test]
    fn dropped_annotations_on_invalid_node() {
        let annotations = Some(annotation(json!({"dropped": true})));
        let root = EvaluationNode::invalid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            annotations.clone(),
            vec![ErrorDescription::from_string("failed")],
            ChildList::default(),
        );

        assert!(!root.valid);
        assert!(root.annotations.is_none());
        assert!(root.dropped_annotations.is_some());
        assert_eq!(
            root.dropped_annotations.as_ref().unwrap(),
            annotations.as_ref().unwrap()
        );
    }

    #[test]
    fn valid_node_has_no_dropped_annotations() {
        let annotations = Some(annotation(json!({"kept": true})));
        let root = EvaluationNode::valid(
            loc(),
            None,
            "/root".to_string(),
            loc(),
            annotations.clone(),
            ChildList::default(),
        );

        assert!(root.valid);
        assert!(root.annotations.is_some());
        assert!(root.dropped_annotations.is_none());
        assert_eq!(
            root.annotations.as_ref().unwrap(),
            annotations.as_ref().unwrap()
        );
    }

    #[test]
    fn absolute_keyword_location_populated_with_id() {
        use serde_json::json;

        // Schema with $id should populate absoluteKeywordLocation
        let schema = json!({
            "$id": "https://example.com/schema",
            "type": "object",
            "properties": {
                "name": {"type": "string"}
            }
        });

        let validator = crate::validator_for(&schema).expect("schema compiles");
        let evaluation = validator.evaluate(&json!({"name": "test"}));

        // Verify that absoluteKeywordLocation is populated for nodes
        let annotations: Vec<_> = evaluation.iter_annotations().collect();
        assert!(!annotations.is_empty());

        // At least one annotation should have an absolute keyword location
        let with_absolute = annotations
            .iter()
            .filter(|a| a.absolute_keyword_location.is_some())
            .count();

        assert!(with_absolute > 0);

        // Verify the absolute locations start with the schema's $id
        for annotation in annotations
            .iter()
            .filter(|a| a.absolute_keyword_location.is_some())
        {
            let uri_str = annotation.absolute_keyword_location.unwrap().as_str();
            assert!(uri_str.starts_with("https://example.com/schema"));
        }
    }

    #[test]
    fn annotations_value_returns_reference() {
        let expected = json!({"key": "value"});
        let annotations = Annotations::new(expected.clone());

        // value() should return a reference to the inner value
        assert_eq!(annotations.value(), &expected);
    }

    #[test]
    fn annotations_into_inner_consumes_and_returns_value() {
        let expected = json!({"key": "value", "nested": {"array": [1, 2, 3]}});
        let annotations = Annotations::new(expected.clone());

        // into_inner() should consume self and return the owned value
        let inner = annotations.into_inner();
        assert_eq!(inner, expected);
    }

    #[test]
    fn error_description_into_inner_consumes_and_returns_message() {
        let expected_message = "test error message";
        let error = ErrorDescription::from_string(expected_message);

        // into_inner() should consume self and return the owned message
        let message = error.into_inner();
        assert_eq!(message, expected_message);
    }

    #[test_case(json!(42), true)]
    #[test_case(json!("not a number"), false)]
    #[allow(clippy::needless_pass_by_value)]
    fn test_evaluation_is_valid(instance: Value, expected: bool) {
        let validator = crate::validator_for(&json!({"type": "number"})).expect("valid schema");
        assert_eq!(validator.evaluate(&instance).is_valid(), expected);
    }

    #[test]
    fn test_error_entry_display() {
        let schema = json!({
            "type": "object",
            "properties": {"age": {"type": "number"}},
            "required": ["name"]
        });
        let validator = crate::validator_for(&schema).expect("valid schema");
        let evaluation = validator.evaluate(&json!({"age": "oops"}));
        let rendered: Vec<String> = evaluation.iter_errors().map(|e| e.to_string()).collect();
        assert_eq!(
            rendered,
            vec![
                "\"name\" is a required property".to_string(),
                "/age: \"oops\" is not of type \"number\"".to_string(),
            ]
        );
    }
}

#[cfg(test)]
mod public_api {
    use super::{Evaluation, HierarchicalOutput, ListOutput};

    // These are returned from the public API, so dropping a derive breaks downstream `{:?}`.
    #[test]
    fn output_types_are_debug() {
        fn assert_debug<T: std::fmt::Debug>() {}
        assert_debug::<Evaluation>();
        assert_debug::<ListOutput<'_>>();
        assert_debug::<HierarchicalOutput<'_>>();
    }
}
