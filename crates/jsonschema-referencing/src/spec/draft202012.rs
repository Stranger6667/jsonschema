use core::slice;
use std::iter::FlatMap;

use serde_json::{Map, Value};

use crate::{
    draft::Draft, path::JsonPointerSegment, segments::Segment, Error, Resolver, ResourceRef,
    Segments,
};

pub(crate) fn walk_children<'a>(
    schema: &'a Map<String, Value>,
    draft: Draft,
    f: &mut impl FnMut(&'a str, Option<JsonPointerSegment<'a>>, &'a Value, Draft) -> Result<(), Error>,
) -> Result<(), Error> {
    for (key, value) in schema {
        match key.as_str() {
            "additionalProperties"
            | "contains"
            | "contentSchema"
            | "else"
            | "if"
            | "items"
            | "not"
            | "propertyNames"
            | "then"
            | "unevaluatedItems"
            | "unevaluatedProperties" => {
                f(key, None, value, draft.detect(value))?;
            }
            "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                if let Some(arr) = value.as_array() {
                    for (index, item) in arr.iter().enumerate() {
                        f(key, Some(index.into()), item, draft.detect(item))?;
                    }
                }
            }
            "$defs" | "definitions" | "dependentSchemas" | "patternProperties" | "properties" => {
                if let Some(obj) = value.as_object() {
                    for (child_key, child_value) in obj {
                        f(
                            key,
                            Some(child_key.as_str().into()),
                            child_value,
                            draft.detect(child_value),
                        )?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

type ObjectChildIter<'a> = FlatMap<
    serde_json::map::Iter<'a>,
    ChildIterInner<'a>,
    fn((&'a std::string::String, &'a Value)) -> ChildIterInner<'a>,
>;

/// A simple iterator that either wraps an iterator producing &Value or is empty.
/// NOTE: It is noticeably slower if `Object` is boxed.
#[allow(clippy::large_enum_variant)]
pub(crate) enum ChildIter<'a> {
    Object(ObjectChildIter<'a>),
    Empty,
}

impl<'a> Iterator for ChildIter<'a> {
    type Item = &'a Value;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            ChildIter::Object(iter) => iter.next(),
            ChildIter::Empty => None,
        }
    }
}

pub(crate) enum ChildIterInner<'a> {
    Once(&'a Value),
    Array(slice::Iter<'a, Value>),
    Object(serde_json::map::Values<'a>),
    FilteredObject(serde_json::map::Values<'a>),
    Empty,
}

impl<'a> Iterator for ChildIterInner<'a> {
    type Item = &'a Value;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            ChildIterInner::Once(_) => {
                let ChildIterInner::Once(value) = std::mem::replace(self, ChildIterInner::Empty)
                else {
                    unreachable!()
                };
                Some(value)
            }
            ChildIterInner::Array(iter) => iter.next(),
            ChildIterInner::Object(iter) => iter.next(),
            ChildIterInner::FilteredObject(iter) => {
                for next in iter.by_ref() {
                    if !next.is_object() {
                        continue;
                    }
                    return Some(next);
                }
                None
            }
            ChildIterInner::Empty => None,
        }
    }
}

pub(crate) fn object_iter<'a>((key, value): (&'a String, &'a Value)) -> ChildIterInner<'a> {
    match key.as_str() {
        "additionalProperties"
        | "contains"
        | "contentSchema"
        | "else"
        | "if"
        | "items"
        | "not"
        | "propertyNames"
        | "then"
        | "unevaluatedItems"
        | "unevaluatedProperties" => ChildIterInner::Once(value),
        "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
            if let Some(arr) = value.as_array() {
                ChildIterInner::Array(arr.iter())
            } else {
                ChildIterInner::Empty
            }
        }
        "$defs" | "definitions" | "dependentSchemas" | "patternProperties" | "properties" => {
            if let Some(obj) = value.as_object() {
                ChildIterInner::Object(obj.values())
            } else {
                ChildIterInner::Empty
            }
        }
        _ => ChildIterInner::Empty,
    }
}

pub(crate) fn maybe_in_subresource<'r>(
    segments: &Segments,
    resolver: &Resolver<'r>,
    subresource: ResourceRef<'_>,
) -> Result<Resolver<'r>, Error> {
    const IN_VALUE: &[&str] = &[
        "additionalProperties",
        "contains",
        "contentSchema",
        "else",
        "if",
        "items",
        "not",
        "propertyNames",
        "then",
        "unevaluatedItems",
        "unevaluatedProperties",
    ];
    const IN_CHILD: &[&str] = &[
        "allOf",
        "anyOf",
        "oneOf",
        "prefixItems",
        "$defs",
        "definitions",
        "dependentSchemas",
        "patternProperties",
        "properties",
    ];

    let mut iter = segments.iter();
    while let Some(segment) = iter.next() {
        if let Segment::Key(key) = segment {
            if !IN_VALUE.contains(&key.as_ref())
                && (!IN_CHILD.contains(&key.as_ref()) || iter.next().is_none())
            {
                return Ok(resolver.clone());
            }
        }
    }
    resolver.in_subresource(subresource)
}

#[inline]
pub(crate) fn maybe_in_subresource_with_items_and_dependencies<'r>(
    segments: &Segments,
    resolver: &Resolver<'r>,
    subresource: ResourceRef<'_>,
    in_value: &[&str],
    in_child: &[&str],
) -> Result<Resolver<'r>, Error> {
    let mut iter = segments.iter();
    while let Some(segment) = iter.next() {
        if let Segment::Key(key) = segment {
            if (*key == "items" || *key == "dependencies") && subresource.contents().is_object() {
                return resolver.in_subresource(subresource);
            }
            if !in_value.contains(&key.as_ref())
                && (!in_child.contains(&key.as_ref()) || iter.next().is_none())
            {
                return Ok(resolver.clone());
            }
        }
    }
    resolver.in_subresource(subresource)
}
