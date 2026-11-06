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
        | "else"
        | "items"
        | "if"
        | "not"
        | "propertyNames"
        | "then" => Children::Single,
        "allOf" | "anyOf" | "oneOf" => Children::Array,
        "definitions" | "patternProperties" | "properties" => Children::Object,
        "dependencies" => Children::ObjectOfObjects,
        _ => Children::None,
    }
}

pub(crate) fn object_iter<'a>((key, value): (&'a String, &'a Value)) -> ChildIterInner<'a> {
    match key.as_str() {
        // For these keys, yield the value once.
        "additionalItems"
        | "additionalProperties"
        | "contains"
        | "else"
        | "if"
        | "not"
        | "propertyNames"
        | "then" => ChildIterInner::Once(value),
        // For these keys, if the value is an array, iterate over its items.
        "allOf" | "anyOf" | "oneOf" => {
            if let Some(arr) = value.as_array() {
                // In the old draft, flatten() was used.
                // Here we simply iterate over the array.
                ChildIterInner::Array(arr.iter())
            } else {
                ChildIterInner::Empty
            }
        }
        // For these keys, if the value is an object, iterate over its values.
        "definitions" | "patternProperties" | "properties" => {
            if let Some(obj) = value.as_object() {
                // flat_map in the old draft: iterate over the object's values.
                ChildIterInner::Object(obj.values())
            } else {
                ChildIterInner::Empty
            }
        }
        // For "items": if it's an array, iterate over its items; otherwise, yield the value once.
        "items" => match value {
            Value::Array(arr) => ChildIterInner::Array(arr.iter()),
            _ => ChildIterInner::Once(value),
        },
        // For "dependencies": if the value is an object, iterate over its values filtered to only those that are objects.
        "dependencies" => {
            if let Some(obj) = value.as_object() {
                ChildIterInner::FilteredObject(obj.values())
            } else {
                ChildIterInner::Empty
            }
        }
        // For any other key, yield nothing.
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
        "else",
        "if",
        "not",
        "propertyNames",
        "then",
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
