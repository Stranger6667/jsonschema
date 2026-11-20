use serde_json::Value;

use crate::Segments;

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
        "items" => match value {
            Value::Array(arr) => ChildIterInner::Array(arr.iter()),
            _ => ChildIterInner::Once(value),
        },
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
        "dependencies" => {
            if let Some(obj) = value.as_object() {
                ChildIterInner::FilteredObject(obj.values())
            } else {
                ChildIterInner::Empty
            }
        }
        "additionalItems" | "additionalProperties" if value.is_object() => {
            ChildIterInner::Once(value)
        }
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

pub(crate) fn is_subschema(segments: &Segments, contents: &Value) -> bool {
    const IN_VALUE: &[&str] = &["additionalItems", "additionalProperties", "not"];
    const IN_CHILD: &[&str] = &[
        "allOf",
        "anyOf",
        "oneOf",
        "definitions",
        "patternProperties",
        "properties",
    ];
    draft202012::is_subschema_with_items_and_dependencies(segments, contents, IN_VALUE, IN_CHILD)
}
