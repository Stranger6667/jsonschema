use serde_json::{Map, Value};

use crate::{
    specification::{BorrowedReferenceSlots, Draft, OwnedObjectGate, OwnedScratchChild},
    Error, JsonPointerNode, Resolver, ResourceRef, Segments,
};

use super::subresources::{self, SubresourceIteratorInner};

pub(crate) fn owned_object_gate_map(schema: &Map<String, Value>) -> OwnedObjectGate<'_> {
    let mut raw_id = None;
    let mut ref_ = None;
    let mut schema_ref = None;
    let mut has_children = false;

    for (key, value) in schema {
        match key.as_str() {
            "id" => raw_id = value.as_str(),
            "$ref" => ref_ = value.as_str(),
            "$schema" => schema_ref = value.as_str(),
            "additionalItems" | "additionalProperties" => has_children |= value.is_object(),
            "contains"
            | "contentSchema"
            | "else"
            | "if"
            | "not"
            | "propertyNames"
            | "then"
            | "unevaluatedItems"
            | "unevaluatedProperties" => has_children = true,
            "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                has_children |= value.as_array().is_some_and(|items| !items.is_empty());
            }
            "$defs" | "definitions" | "dependentSchemas" | "patternProperties" | "properties" => {
                has_children |= value.as_object().is_some_and(|items| !items.is_empty());
            }
            "items" => {
                has_children |= match value {
                    Value::Array(items) => !items.is_empty(),
                    _ => true,
                };
            }
            "dependencies" => {
                has_children |= value
                    .as_object()
                    .is_some_and(|items| items.values().any(Value::is_object));
            }
            _ => {}
        }
    }

    let has_anchor = raw_id.is_some_and(|id| id.starts_with('#'));
    let id = match raw_id {
        Some(id) if !has_anchor && ref_.is_none() => Some(id),
        _ => None,
    };

    OwnedObjectGate {
        id,
        has_anchor,
        ref_,
        schema: schema_ref,
        has_children,
    }
}

pub(crate) fn scan_borrowed_object_into_scratch_map<'a>(
    schema: &'a Map<String, Value>,
    draft: Draft,
    references: &mut BorrowedReferenceSlots<'a>,
    children: &mut Vec<(&'a Value, Draft)>,
) {
    for (key, value) in schema {
        match key.as_str() {
            "$ref" => {
                if let Some(reference) = value.as_str() {
                    references.ref_ = Some(reference);
                }
            }
            "$schema" => {
                if let Some(reference) = value.as_str() {
                    references.schema = Some(reference);
                }
            }
            "additionalItems" | "additionalProperties" if value.is_object() => {
                crate::observe_registry!("registry.draft4.keyword={}", key);
                children.push((value, draft.detect(value)));
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
                crate::observe_registry!("registry.draft4.keyword={}", key);
                children.push((value, draft.detect(value)));
            }
            "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                crate::observe_registry!("registry.draft4.keyword={}", key);
                if let Some(arr) = value.as_array() {
                    for item in arr {
                        children.push((item, draft.detect(item)));
                    }
                }
            }
            "$defs" | "definitions" | "dependentSchemas" | "patternProperties" | "properties" => {
                crate::observe_registry!("registry.draft4.keyword={}", key);
                if let Some(obj) = value.as_object() {
                    for child_value in obj.values() {
                        children.push((child_value, draft.detect(child_value)));
                    }
                }
            }
            "items" => {
                crate::observe_registry!("registry.draft4.keyword=items");
                match value {
                    Value::Array(arr) => {
                        for item in arr {
                            children.push((item, draft.detect(item)));
                        }
                    }
                    _ => children.push((value, draft.detect(value))),
                }
            }
            "dependencies" => {
                crate::observe_registry!("registry.draft4.keyword=dependencies");
                if let Some(obj) = value.as_object() {
                    for child_value in obj.values() {
                        if !child_value.is_object() {
                            continue;
                        }
                        children.push((child_value, draft.detect(child_value)));
                    }
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn scan_owned_object_into_scratch_map<'a>(
    schema: &'a Map<String, Value>,
    draft: Draft,
    references: &mut BorrowedReferenceSlots<'a>,
    children: &mut Vec<OwnedScratchChild<'a>>,
) -> (Option<&'a str>, bool) {
    let mut raw_id = None;
    let mut has_ref = false;

    for (key, value) in schema {
        match key.as_str() {
            "id" => raw_id = value.as_str(),
            "$ref" => {
                if let Some(reference) = value.as_str() {
                    has_ref = true;
                    references.ref_ = Some(reference);
                }
            }
            "$schema" => {
                if let Some(reference) = value.as_str() {
                    references.schema = Some(reference);
                }
            }
            "additionalItems" | "additionalProperties" if value.is_object() => {
                children.push(OwnedScratchChild::key(
                    key.as_str(),
                    value,
                    draft.detect(value),
                ));
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
                children.push(OwnedScratchChild::key(
                    key.as_str(),
                    value,
                    draft.detect(value),
                ));
            }
            "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                if let Some(arr) = value.as_array() {
                    for (index, item) in arr.iter().enumerate() {
                        children.push(OwnedScratchChild::key_index(
                            key.as_str(),
                            index,
                            item,
                            draft.detect(item),
                        ));
                    }
                }
            }
            "$defs" | "definitions" | "dependentSchemas" | "patternProperties" | "properties" => {
                if let Some(obj) = value.as_object() {
                    for (child_key, child_value) in obj {
                        children.push(OwnedScratchChild::key_key(
                            key.as_str(),
                            child_key.as_str(),
                            child_value,
                            draft.detect(child_value),
                        ));
                    }
                }
            }
            "items" => match value {
                Value::Array(arr) => {
                    for (index, item) in arr.iter().enumerate() {
                        children.push(OwnedScratchChild::key_index(
                            "items",
                            index,
                            item,
                            draft.detect(item),
                        ));
                    }
                }
                _ => children.push(OwnedScratchChild::key("items", value, draft.detect(value))),
            },
            "dependencies" => {
                if let Some(obj) = value.as_object() {
                    for (child_key, child_value) in obj {
                        if !child_value.is_object() {
                            continue;
                        }
                        children.push(OwnedScratchChild::key_key(
                            key.as_str(),
                            child_key.as_str(),
                            child_value,
                            draft.detect(child_value),
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    let has_anchor = raw_id.is_some_and(|id| id.starts_with('#'));
    let id = match raw_id {
        Some(id) if !has_anchor && !has_ref => Some(id),
        _ => None,
    };
    (id, has_anchor)
}

pub(crate) fn walk_borrowed_subresources_map<'a, E, F>(
    schema: &'a Map<String, Value>,
    draft: Draft,
    f: &mut F,
) -> Result<(), E>
where
    F: FnMut(&'a Value, Draft) -> Result<(), E>,
{
    for (key, value) in schema {
        match key.as_str() {
            "additionalItems" | "additionalProperties" if value.is_object() => {
                crate::observe_registry!("registry.draft4.keyword={}", key);
                f(value, draft.detect(value))?;
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
                crate::observe_registry!("registry.draft4.keyword={}", key);
                f(value, draft.detect(value))?;
            }
            "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                crate::observe_registry!("registry.draft4.keyword={}", key);
                if let Some(arr) = value.as_array() {
                    for item in arr {
                        f(item, draft.detect(item))?;
                    }
                }
            }
            "$defs" | "definitions" | "dependentSchemas" | "patternProperties" | "properties" => {
                crate::observe_registry!("registry.draft4.keyword={}", key);
                if let Some(obj) = value.as_object() {
                    for child_value in obj.values() {
                        f(child_value, draft.detect(child_value))?;
                    }
                }
            }
            "items" => {
                crate::observe_registry!("registry.draft4.keyword=items");
                match value {
                    Value::Array(arr) => {
                        for item in arr {
                            f(item, draft.detect(item))?;
                        }
                    }
                    _ => f(value, draft.detect(value))?,
                }
            }
            "dependencies" => {
                crate::observe_registry!("registry.draft4.keyword=dependencies");
                if let Some(obj) = value.as_object() {
                    for child_value in obj.values() {
                        if !child_value.is_object() {
                            continue;
                        }
                        f(child_value, draft.detect(child_value))?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn walk_owned_subresources_map<'a, E, F>(
    schema: &'a Map<String, Value>,
    path: &JsonPointerNode<'_, '_>,
    draft: Draft,
    f: &mut F,
) -> Result<(), E>
where
    F: FnMut(&JsonPointerNode<'_, '_>, &'a Value, Draft) -> Result<(), E>,
{
    for (key, value) in schema {
        match key.as_str() {
            "additionalItems" | "additionalProperties" if value.is_object() => {
                let child_path = path.push(key.as_str());
                f(&child_path, value, draft.detect(value))?;
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
                let child_path = path.push(key.as_str());
                f(&child_path, value, draft.detect(value))?;
            }
            "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                if let Some(arr) = value.as_array() {
                    let parent_path = path.push(key.as_str());
                    for (i, item) in arr.iter().enumerate() {
                        let child_path = parent_path.push(i);
                        f(&child_path, item, draft.detect(item))?;
                    }
                }
            }
            "$defs" | "definitions" | "dependentSchemas" | "patternProperties" | "properties" => {
                if let Some(obj) = value.as_object() {
                    let parent_path = path.push(key.as_str());
                    for (child_key, child_value) in obj {
                        let child_path = parent_path.push(child_key.as_str());
                        f(&child_path, child_value, draft.detect(child_value))?;
                    }
                }
            }
            "items" => {
                let parent_path = path.push(key.as_str());
                match value {
                    Value::Array(arr) => {
                        for (i, item) in arr.iter().enumerate() {
                            let child_path = parent_path.push(i);
                            f(&child_path, item, draft.detect(item))?;
                        }
                    }
                    _ => f(&parent_path, value, draft.detect(value))?,
                }
            }
            "dependencies" => {
                if let Some(obj) = value.as_object() {
                    let parent_path = path.push(key.as_str());
                    for (child_key, child_value) in obj {
                        if !child_value.is_object() {
                            continue;
                        }
                        let child_path = parent_path.push(child_key.as_str());
                        f(&child_path, child_value, draft.detect(child_value))?;
                    }
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

#[cfg(test)]
mod tests {
    use crate::{specification::BorrowedReferenceSlots, Draft};
    use serde_json::json;

    #[test]
    fn test_probe_borrowed_object_collects_control_keys() {
        let schema = json!({
            "id": "http://example.com/node",
            "$schema": "http://example.com/meta",
            "properties": {
                "name": {"type": "string"}
            },
            "items": {"type": "integer"}
        });
        let analysis = Draft::Draft4.probe_borrowed_object_map(
            schema
                .as_object()
                .expect("schema object should be analyzed"),
        );

        assert_eq!(analysis.id, Some("http://example.com/node"));
        assert!(!analysis.has_anchor);
        assert!(analysis.has_ref_or_schema);
    }

    #[test]
    fn test_scan_borrowed_object_into_scratch_collects_refs_and_children() {
        let schema = json!({
            "id": "http://example.com/node",
            "$schema": "http://example.com/meta",
            "properties": {
                "name": {"type": "string"}
            },
            "items": {"type": "integer"}
        });
        let mut references = BorrowedReferenceSlots::default();
        let mut children = Vec::new();

        Draft::Draft4.scan_borrowed_object_into_scratch_map(
            schema.as_object().expect("schema object should be scanned"),
            &mut references,
            &mut children,
        );

        assert_eq!(
            (
                references.ref_.map(str::to_string),
                references.schema.map(str::to_string)
            ),
            vec![("$schema".to_string(), "http://example.com/meta".to_string())]
                .into_iter()
                .fold((None, None), |mut acc, (key, value)| {
                    if key == "$ref" {
                        acc.0 = Some(value);
                    } else {
                        acc.1 = Some(value);
                    }
                    acc
                })
        );
        let children: Vec<_> = children
            .iter()
            .map(|(child, child_draft)| ((*child).clone(), *child_draft))
            .collect();
        assert_eq!(children.len(), 2);
        assert!(children.contains(&(json!({"type": "string"}), Draft::Draft4)));
        assert!(children.contains(&(json!({"type": "integer"}), Draft::Draft4)));
    }
}
