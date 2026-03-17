use serde_json::Value;

use crate::{specification::Draft, Error, JsonPointerNode, Resolver, ResourceRef, Segments};

use super::subresources::{self, SubresourceIteratorInner};

pub(crate) struct BorrowedObjectProbe<'a> {
    pub(crate) id: Option<&'a str>,
    pub(crate) has_anchor: bool,
    pub(crate) has_ref_or_schema: bool,
}

pub(crate) fn probe_borrowed_object(contents: &Value) -> Option<BorrowedObjectProbe<'_>> {
    let schema = contents.as_object()?;

    let raw_id = schema.get("id").and_then(Value::as_str);
    let has_ref = schema.get("$ref").and_then(Value::as_str).is_some();
    let has_ref_or_schema = has_ref || schema.get("$schema").and_then(Value::as_str).is_some();
    let mut has_anchor = false;
    if let Some(id) = raw_id {
        has_anchor = id.starts_with('#');
    }

    let id = match raw_id {
        Some(id) if !id.starts_with('#') && !has_ref => Some(id),
        _ => None,
    };

    Some(BorrowedObjectProbe {
        id,
        has_anchor,
        has_ref_or_schema,
    })
}

pub(crate) fn scan_borrowed_object_into_scratch<'a>(
    contents: &'a Value,
    draft: Draft,
    references: &mut Vec<(&'a str, &'static str)>,
    children: &mut Vec<(&'a Value, Draft)>,
) -> Option<()> {
    let schema = contents.as_object()?;

    for (key, value) in schema {
        match key.as_str() {
            "$ref" => {
                if let Some(reference) = value.as_str() {
                    references.push((reference, "$ref"));
                }
            }
            "$schema" => {
                if let Some(reference) = value.as_str() {
                    references.push((reference, "$schema"));
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

    Some(())
}

pub(crate) fn walk_borrowed_subresources<'a, E, F>(
    contents: &'a Value,
    draft: Draft,
    f: &mut F,
) -> Result<(), E>
where
    F: FnMut(&'a Value, Draft) -> Result<(), E>,
{
    let Some(schema) = contents.as_object() else {
        return Ok(());
    };
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

pub(crate) fn walk_owned_subresources<'a, E, F>(
    contents: &'a Value,
    path: &JsonPointerNode<'_, '_>,
    draft: Draft,
    f: &mut F,
) -> Result<(), E>
where
    F: FnMut(&JsonPointerNode<'_, '_>, &'a Value, Draft) -> Result<(), E>,
{
    let Some(schema) = contents.as_object() else {
        return Ok(());
    };
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
    use super::{probe_borrowed_object, scan_borrowed_object_into_scratch};
    use crate::Draft;
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
        let analysis = probe_borrowed_object(&schema).expect("schema object should be analyzed");

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
        let mut references = Vec::new();
        let mut children = Vec::new();

        scan_borrowed_object_into_scratch(&schema, Draft::Draft4, &mut references, &mut children)
            .expect("schema object should be scanned");

        assert_eq!(
            references
                .iter()
                .map(|(reference, key): &(&str, &'static str)| {
                    (key.to_string(), reference.to_string())
                })
                .collect::<Vec<_>>(),
            vec![("$schema".to_string(), "http://example.com/meta".to_string())]
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
