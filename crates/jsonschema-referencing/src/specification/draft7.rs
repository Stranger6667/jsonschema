use serde_json::Value;

use crate::{resource::InnerResourcePtr, Error, Resolver, Segments};

use super::subresources::{self, SubresourceIterator};

pub(crate) fn subresources_of(contents: &Value) -> SubresourceIterator<'_> {
    match contents.as_object() {
        Some(schema) => Box::new(schema.iter().flat_map(|(key, value)| {
            match key.as_str() {
                "additionalItems"
                | "additionalProperties"
                | "contains"
                | "else"
                | "if"
                | "not"
                | "propertyNames"
                | "then" => Box::new(std::iter::once(value)) as SubresourceIterator<'_>,
                "allOf" | "anyOf" | "oneOf" => Box::new(value.as_array().into_iter().flatten()),
                "definitions" | "patternProperties" | "properties" => {
                    Box::new(value.as_object().into_iter().flat_map(|o| o.values()))
                }
                "items" => match value {
                    Value::Array(arr) => Box::new(arr.iter()) as SubresourceIterator<'_>,
                    _ => Box::new(std::iter::once(value)),
                },
                "dependencies" => Box::new(
                    value
                        .as_object()
                        .into_iter()
                        .flat_map(|o| o.values())
                        .filter(|v| v.is_object()),
                ),
                _ => Box::new(std::iter::empty()),
            }
        })),
        None => Box::new(std::iter::empty()),
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
