use serde_json::{Map, Value};

use crate::{draft::Draft, path::JsonPointerSegment, Error, Resolver, ResourceRef, Segments};

use super::draft202012::{self, ChildIterInner};

pub(crate) fn walk_children<'a>(
    schema: &'a Map<String, Value>,
    draft: Draft,
    f: &mut impl FnMut(&'a str, Option<JsonPointerSegment<'a>>, &'a Value, Draft) -> Result<(), Error>,
) -> Result<(), Error> {
    for (key, value) in schema {
        match key.as_str() {
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
            | "unevaluatedProperties" => {
                f(key, None, value, draft.detect(value))?;
            }
            "allOf" | "anyOf" | "oneOf" => {
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
            "items" => match value {
                Value::Array(arr) => {
                    for (index, item) in arr.iter().enumerate() {
                        f(key, Some(index.into()), item, draft.detect(item))?;
                    }
                }
                _ => f(key, None, value, draft.detect(value))?,
            },
            "dependencies" => {
                if let Some(obj) = value.as_object() {
                    for (child_key, child_value) in obj {
                        if !child_value.is_object() {
                            continue;
                        }
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
