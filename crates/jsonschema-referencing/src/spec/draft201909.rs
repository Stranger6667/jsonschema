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
        | "contentSchema"
        | "items"
        | "else"
        | "if"
        | "not"
        | "propertyNames"
        | "then"
        | "unevaluatedItems"
        | "unevaluatedProperties" => Children::Single,
        "allOf" | "anyOf" | "oneOf" => Children::Array,
        "$defs" | "definitions" | "dependentSchemas" | "patternProperties" | "properties" => {
            Children::Object
        }
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
        | "contentSchema"
        | "else"
        | "if"
        | "not"
        | "propertyNames"
        | "then"
        | "unevaluatedItems"
        | "unevaluatedProperties" => ChildIterInner::Once(value),
        // For these keys, if the value is an array, iterate over its items.
        "allOf" | "anyOf" | "oneOf" => {
            if let Some(arr) = value.as_array() {
                ChildIterInner::Array(arr.iter())
            } else {
                ChildIterInner::Empty
            }
        }
        // For these keys, if the value is an object, iterate over its values.
        "$defs" | "definitions" | "dependentSchemas" | "patternProperties" | "properties" => {
            if let Some(obj) = value.as_object() {
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
        "contentSchema",
        "else",
        "if",
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
        "$defs",
        "definitions",
        "dependentSchemas",
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
