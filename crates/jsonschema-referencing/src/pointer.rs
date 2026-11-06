use serde_json::Value;

use crate::{
    path::{JsonPointerNode, JsonPointerSegment},
    resource::unescape_segment,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct ParsedPointer {
    pub(crate) segments: Vec<ParsedPointerSegment>,
}

impl ParsedPointer {
    pub(crate) fn from_json_pointer(pointer: &str) -> Option<Self> {
        if pointer.is_empty() {
            return Some(Self::default());
        }
        if !pointer.starts_with('/') {
            return None;
        }

        let mut segments = Vec::new();
        for token in pointer.split('/').skip(1).map(unescape_segment) {
            if let Some(index) = parse_index(&token) {
                segments.push(ParsedPointerSegment::Index(index));
            } else {
                segments.push(ParsedPointerSegment::Key(
                    token.into_owned().into_boxed_str(),
                ));
            }
        }
        Some(Self { segments })
    }

    pub(crate) fn from_pointer_node(path: &JsonPointerNode<'_, '_>) -> Self {
        let mut segments = Vec::new();
        let mut head = path;

        while let Some(parent) = head.parent() {
            segments.push(match head.segment() {
                JsonPointerSegment::Key(key) => ParsedPointerSegment::Key(key.as_ref().into()),
                JsonPointerSegment::Index(idx) => ParsedPointerSegment::Index(*idx),
            });
            head = parent;
        }

        segments.reverse();
        Self { segments }
    }

    pub(crate) fn lookup<'a>(&self, document: &'a Value) -> Option<&'a Value> {
        self.segments
            .iter()
            .try_fold(document, |target, token| token.step(target))
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ParsedPointerSegment {
    Key(Box<str>),
    Index(usize),
}

impl ParsedPointerSegment {
    /// The child of `target` this segment names.
    #[inline]
    pub(crate) fn step<'a>(&self, target: &'a Value) -> Option<&'a Value> {
        match (self, target) {
            (ParsedPointerSegment::Key(key), Value::Object(map)) => map.get(&**key),
            (ParsedPointerSegment::Index(index), Value::Array(list)) => list.get(*index),
            // A numeric token also names an object member, e.g. `/responses/200`
            (ParsedPointerSegment::Index(index), Value::Object(map)) => {
                map.get(index.to_string().as_str())
            }
            _ => None,
        }
    }
}

/// Look up a value by a JSON Pointer.
///
/// **NOTE**: A slightly faster version of pointer resolution based on `Value::pointer` from `serde_json`.
pub fn pointer<'a>(document: &'a Value, pointer: &str) -> Option<&'a Value> {
    if pointer.is_empty() {
        return Some(document);
    }
    if !pointer.starts_with('/') {
        return None;
    }
    pointer.split('/').skip(1).map(unescape_segment).try_fold(
        document,
        |target, token| match target {
            Value::Object(map) => map.get(&*token),
            Value::Array(list) => parse_index(&token).and_then(|x| list.get(x)),
            _ => None,
        },
    )
}

// Taken from `serde_json`.
#[must_use]
pub fn parse_index(s: &str) -> Option<usize> {
    if s.starts_with('+') || (s.starts_with('0') && s.len() != 1) {
        return None;
    }
    s.parse().ok()
}
