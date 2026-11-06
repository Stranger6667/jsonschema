use serde_json::Value;

use crate::{Error, Resolver, ResourceRef, Segments};

use super::{
    draft202012::{self, ChildIterInner},
    Children,
};

pub(crate) fn children(key: &str, value: &Value) -> Children {
    match key {
        "items" if value.is_array() => Children::Array,
        "additionalItems" | "additionalProperties" if value.is_object() => Children::Single,
        "contains"
        | "contentSchema"
        | "items"
        | "else"
        | "if"
        | "not"
        | "propertyNames"
        | "then"
        | "unevaluatedItems"
        | "unevaluatedProperties" => Children::Single,
        "allOf" | "anyOf" | "oneOf" | "prefixItems" => Children::Array,
        "$defs" | "definitions" | "dependentSchemas" | "patternProperties" | "properties" => {
            Children::Object
        }
        "dependencies" => Children::ObjectOfObjects,
        _ => Children::None,
    }
}

pub(crate) fn object_iter<'a>((key, value): (&'a String, &'a Value)) -> ChildIterInner<'a> {
    match key.as_str() {
        // For "items": if it's an array, iterate over it; otherwise, yield one element.
        "items" => match value {
            Value::Array(arr) => ChildIterInner::Array(arr.iter()),
            _ => ChildIterInner::Once(value),
        },
        // For "allOf", "anyOf", "oneOf", "prefixItems": if the value is an array, iterate over it.
        "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
            if let Some(arr) = value.as_array() {
                ChildIterInner::Array(arr.iter())
            } else {
                ChildIterInner::Empty
            }
        }
        // For "$defs", "definitions", "dependentSchemas", "patternProperties", "properties":
        // if the value is an object, iterate over its values.
        "$defs" | "definitions" | "dependentSchemas" | "patternProperties" | "properties" => {
            if let Some(obj) = value.as_object() {
                ChildIterInner::Object(obj.values())
            } else {
                ChildIterInner::Empty
            }
        }
        // For "dependencies": if the value is an object, iterate over its values filtered to only those that are objects.
        "dependencies" => {
            if let Some(obj) = value.as_object() {
                ChildIterInner::FilteredObject(obj.values())
            } else {
                ChildIterInner::Empty
            }
        }
        // For "additionalItems" and "additionalProperties", only if the value is an object.
        "additionalItems" | "additionalProperties" if value.is_object() => {
            ChildIterInner::Once(value)
        }
        // For other keys that were originally in the "single element" group:
        "contains"
        | "contentSchema"
        | "else"
        | "if"
        | "propertyNames"
        | "not"
        | "then"
        | "unevaluatedItems"
        | "unevaluatedProperties" => ChildIterInner::Once(value),
        _ => ChildIterInner::Empty,
    }
}

pub(crate) fn maybe_in_subresource<'r>(
    segments: &Segments,
    resolver: &Resolver<'r>,
    subresource: ResourceRef<'_>,
) -> Result<Resolver<'r>, Error> {
    const IN_VALUE: &[&str] = &["additionalItems", "additionalProperties", "not"];
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
