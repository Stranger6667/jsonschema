use serde_json::Value;

use crate::{Error, Resolver, ResourceRef, Segments};

use super::{
    draft202012::{self, ChildIterInner},
    Children,
};

pub(crate) fn children(key: &str, value: &Value) -> Children {
    match key {
        "items" if value.is_array() => Children::Array,
        "additionalItems"
        | "additionalProperties"
        | "contains"
        | "items"
        | "not"
        | "propertyNames" => Children::Single,
        "allOf" | "anyOf" | "oneOf" => Children::Array,
        "definitions" | "patternProperties" | "properties" => Children::Object,
        "dependencies" => Children::ObjectOfObjects,
        _ => Children::None,
    }
}

pub(crate) fn object_iter<'a>((key, value): (&'a String, &'a Value)) -> ChildIterInner<'a> {
    match key.as_str() {
        "additionalItems" | "additionalProperties" | "contains" | "not" | "propertyNames" => {
            ChildIterInner::Once(value)
        }
        "allOf" | "anyOf" | "oneOf" => {
            if let Some(arr) = value.as_array() {
                ChildIterInner::Array(arr.iter())
            } else {
                ChildIterInner::Empty
            }
        }
        "definitions" | "patternProperties" | "properties" => {
            if let Some(obj) = value.as_object() {
                ChildIterInner::Object(obj.values())
            } else {
                ChildIterInner::Empty
            }
        }
        "items" => match value {
            Value::Array(arr) => ChildIterInner::Array(arr.iter()),
            _ => ChildIterInner::Once(value),
        },
        "dependencies" => {
            if let Some(obj) = value.as_object() {
                ChildIterInner::FilteredObject(obj.values())
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
        "additionalItems",
        "additionalProperties",
        "contains",
        "not",
        "propertyNames",
    ];
    const IN_CHILD: &[&str] = &[
        "allOf",
        "anyOf",
        "oneOf",
        "definitions",
        "patternProperties",
        "properties",
    ];
    draft202012::maybe_in_subresource_with_items_and_dependencies(
        segments,
        resolver,
        subresource,
        IN_VALUE,
        IN_CHILD,
    )
}
