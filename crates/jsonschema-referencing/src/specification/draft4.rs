use serde_json::Value;

use crate::{resource::PathStack, specification::Draft, Error, Resolver, ResourceRef, Segments};

use super::subresources::{self, SubresourceIteratorInner};

pub(crate) fn walk_subresources_with_path<'a, E, F>(
    contents: &'a Value,
    path: &mut PathStack<'a>,
    draft: Draft,
    f: &mut F,
) -> Result<(), E>
where
    F: FnMut(&mut PathStack<'a>, &'a Value, Draft) -> Result<(), E>,
{
    let Some(schema) = contents.as_object() else {
        return Ok(());
    };
    for (key, value) in schema {
        match key.as_str() {
            "additionalItems" | "additionalProperties" if value.is_object() => {
                let c = path.push_key(key);
                f(path, value, draft.detect(value))?;
                path.truncate(c);
            }
            "contains"
            | "contentSchema"
            | "else"
            | "if"
            | "not"
            | "propertyNames"
            | "then"
            | "unevaluatedItems"
            | "unevaluatedProperties" => {
                let c = path.push_key(key);
                f(path, value, draft.detect(value))?;
                path.truncate(c);
            }
            "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                if let Some(arr) = value.as_array() {
                    let c1 = path.push_key(key);
                    for (i, item) in arr.iter().enumerate() {
                        let c2 = path.push_index(i);
                        f(path, item, draft.detect(item))?;
                        path.truncate(c2);
                    }
                    path.truncate(c1);
                }
            }
            "$defs" | "definitions" | "dependentSchemas" | "patternProperties" | "properties" => {
                if let Some(obj) = value.as_object() {
                    let c1 = path.push_key(key);
                    for (child_key, child_value) in obj {
                        let c2 = path.push_key(child_key);
                        f(path, child_value, draft.detect(child_value))?;
                        path.truncate(c2);
                    }
                    path.truncate(c1);
                }
            }
            "items" => {
                let c1 = path.push_key(key);
                match value {
                    Value::Array(arr) => {
                        for (i, item) in arr.iter().enumerate() {
                            let c2 = path.push_index(i);
                            f(path, item, draft.detect(item))?;
                            path.truncate(c2);
                        }
                    }
                    _ => f(path, value, draft.detect(value))?,
                }
                path.truncate(c1);
            }
            "dependencies" => {
                if let Some(obj) = value.as_object() {
                    let c1 = path.push_key(key);
                    for (child_key, child_value) in obj {
                        if !child_value.is_object() {
                            continue;
                        }
                        let c2 = path.push_key(child_key);
                        f(path, child_value, draft.detect(child_value))?;
                        path.truncate(c2);
                    }
                    path.truncate(c1);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn object_iter<'a>(
    (key, value): (&'a String, &'a Value),
) -> SubresourceIteratorInner<'a> {
    match key.as_str() {
        // For "items": if it’s an array, iterate over it; otherwise, yield one element.
        "items" => match value {
            Value::Array(arr) => SubresourceIteratorInner::Array(arr.iter()),
            _ => SubresourceIteratorInner::Once(value),
        },
        // For "allOf", "anyOf", "oneOf", "prefixItems": if the value is an array, iterate over it.
        "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
            if let Some(arr) = value.as_array() {
                SubresourceIteratorInner::Array(arr.iter())
            } else {
                SubresourceIteratorInner::Empty
            }
        }
        // For "$defs", "definitions", "dependentSchemas", "patternProperties", "properties":
        // if the value is an object, iterate over its values.
        "$defs" | "definitions" | "dependentSchemas" | "patternProperties" | "properties" => {
            if let Some(obj) = value.as_object() {
                SubresourceIteratorInner::Object(obj.values())
            } else {
                SubresourceIteratorInner::Empty
            }
        }
        // For "dependencies": if the value is an object, iterate over its values filtered to only those that are objects.
        "dependencies" => {
            if let Some(obj) = value.as_object() {
                SubresourceIteratorInner::FilteredObject(obj.values())
            } else {
                SubresourceIteratorInner::Empty
            }
        }
        // For "additionalItems" and "additionalProperties", only if the value is an object.
        "additionalItems" | "additionalProperties" if value.is_object() => {
            SubresourceIteratorInner::Once(value)
        }
        // For other keys that were originally in the “single element” group:
        "contains"
        | "contentSchema"
        | "else"
        | "if"
        | "propertyNames"
        | "not"
        | "then"
        | "unevaluatedItems"
        | "unevaluatedProperties" => SubresourceIteratorInner::Once(value),
        _ => SubresourceIteratorInner::Empty,
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
    subresources::maybe_in_subresource_with_items_and_dependencies(
        segments,
        resolver,
        subresource,
        IN_VALUE,
        IN_CHILD,
    )
}
