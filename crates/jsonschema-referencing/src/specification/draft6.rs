use serde_json::Value;

use crate::{resource::InnerResourcePtr, Error, Resolver, Segments};

use super::subresources::{self, SubIterBranch, SubresourceIterator};

fn object_iter<'a>((key, value): (&'a String, &'a Value)) -> SubIterBranch<'a> {
    match key.as_str() {
        "additionalItems" | "additionalProperties" | "contains" | "not" | "propertyNames" => {
            SubIterBranch::Once(value)
        }
        "allOf" | "anyOf" | "oneOf" => {
            if let Some(arr) = value.as_array() {
                SubIterBranch::Array(arr.iter())
            } else {
                SubIterBranch::Empty
            }
        }
        "definitions" | "patternProperties" | "properties" => {
            if let Some(obj) = value.as_object() {
                SubIterBranch::Object(obj.values())
            } else {
                SubIterBranch::Empty
            }
        }
        "items" => match value {
            Value::Array(arr) => SubIterBranch::Array(arr.iter()),
            _ => SubIterBranch::Once(value),
        },
        "dependencies" => {
            if let Some(obj) = value.as_object() {
                SubIterBranch::FilteredObject(obj.values())
            } else {
                SubIterBranch::Empty
            }
        }
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
    subresources::maybe_in_subresource_with_items_and_dependencies(
        segments,
        resolver,
        subresource,
        IN_VALUE,
        IN_CHILD,
    )
}
