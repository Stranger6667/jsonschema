use serde_json::Value;

use crate::{resource::InnerResourcePtr, Error, Resolver, Segments};

use super::subresources::{self, SubIterBranch, SubresourceIterator};

fn object_iter<'a>((key, value): (&'a String, &'a Value)) -> SubIterBranch<'a> {
    match key.as_str() {
        // For these keys, yield the value once.
        "additionalItems"
        | "additionalProperties"
        | "contains"
        | "else"
        | "if"
        | "not"
        | "propertyNames"
        | "then" => SubIterBranch::Once(value),
        // For these keys, if the value is an array, iterate over its items.
        "allOf" | "anyOf" | "oneOf" => {
            if let Some(arr) = value.as_array() {
                // In the old draft, flatten() was used.
                // Here we simply iterate over the array.
                SubIterBranch::Array(arr.iter())
            } else {
                SubIterBranch::Empty
            }
        }
        // For these keys, if the value is an object, iterate over its values.
        "definitions" | "patternProperties" | "properties" => {
            if let Some(obj) = value.as_object() {
                // flat_map in the old draft: iterate over the object's values.
                SubIterBranch::Object(obj.values())
            } else {
                SubIterBranch::Empty
            }
        }
        // For "items": if it's an array, iterate over its items; otherwise, yield the value once.
        "items" => match value {
            Value::Array(arr) => SubIterBranch::Array(arr.iter()),
            _ => SubIterBranch::Once(value),
        },
        // For "dependencies": if the value is an object, iterate over its values filtered to only those that are objects.
        "dependencies" => {
            if let Some(obj) = value.as_object() {
                SubIterBranch::FilteredObject(obj.values())
            } else {
                SubIterBranch::Empty
            }
        }
        // For any other key, yield nothing.
        _ => SubIterBranch::Empty,
    }
}

pub(crate) fn subresources_of(contents: &Value) -> SubresourceIterator<'_> {
    match contents.as_object() {
        Some(schema) => SubresourceIterator::Object(schema.iter().flat_map(object_iter)),
        None => SubresourceIterator::Empty,
    }
}

pub(crate) fn maybe_in_subresource<'r>(
    segments: &Segments,
    resolver: &Resolver<'r>,
    subresource: &InnerResourcePtr,
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
    subresources::maybe_in_subresource_with_items_and_dependencies(
        segments,
        resolver,
        subresource,
        IN_VALUE,
        IN_CHILD,
    )
}
