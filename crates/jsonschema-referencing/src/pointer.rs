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
            .try_fold(document, |target, token| match token {
                ParsedPointerSegment::Key(key) => match target {
                    Value::Object(map) => map.get(&**key),
                    _ => None,
                },
                ParsedPointerSegment::Index(index) => match target {
                    Value::Array(list) => list.get(*index),
                    _ => None,
                },
            })
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ParsedPointerSegment {
    Key(Box<str>),
    Index(usize),
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
