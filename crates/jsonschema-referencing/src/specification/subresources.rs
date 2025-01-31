use serde_json::Value;

use crate::{resource::InnerResourcePtr, segments::Segment, Error, Resolver, Segments};

pub(crate) type SubresourceIterator<'a> = Box<dyn Iterator<Item = &'a Value> + 'a>;

pub(crate) fn subresources_of(contents: &Value) -> SubresourceIterator<'_> {
    match contents.as_object() {
        Some(schema) => Box::new(schema.iter().flat_map(|(key, value)| match key.as_str() {
            "additionalProperties"
            | "contains"
            | "contentSchema"
            | "else"
            | "if"
            | "items"
            | "not"
            | "propertyNames"
            | "then"
            | "unevaluatedItems"
            | "unevaluatedProperties" => {
                Box::new(std::iter::once(value)) as SubresourceIterator<'_>
            }
            "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                Box::new(value.as_array().into_iter().flatten())
            }
            "$defs" | "definitions" | "dependentSchemas" | "patternProperties" | "properties" => {
                Box::new(value.as_object().into_iter().flat_map(|o| o.values()))
            }
            _ => Box::new(std::iter::empty()),
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
        "additionalProperties",
        "contains",
        "contentSchema",
        "else",
        "if",
        "items",
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
        "prefixItems",
        "$defs",
        "definitions",
        "dependentSchemas",
        "patternProperties",
        "properties",
    ];

    let mut iter = segments.iter();
    while let Some(segment) = iter.next() {
        if let Segment::Key(key) = segment {
            if !IN_VALUE.contains(&key.as_ref())
                && (!IN_CHILD.contains(&key.as_ref()) || iter.next().is_none())
            {
                return Ok(resolver.clone());
            }
        }
    }
    resolver.in_subresource_inner(subresource)
}

#[inline]
pub(crate) fn maybe_in_subresource_with_items_and_dependencies<'r>(
    segments: &Segments,
    resolver: &Resolver<'r>,
    subresource: &InnerResourcePtr,
    in_value: &[&str],
    in_child: &[&str],
) -> Result<Resolver<'r>, Error> {
    let mut iter = segments.iter();
    while let Some(segment) = iter.next() {
        if let Segment::Key(key) = segment {
            if (*key == "items" || *key == "dependencies") && subresource.contents().is_object() {
                return resolver.in_subresource_inner(subresource);
            }
            if !in_value.contains(&key.as_ref())
                && (!in_child.contains(&key.as_ref()) || iter.next().is_none())
            {
                return Ok(resolver.clone());
            }
        }
    }
    resolver.in_subresource_inner(subresource)
}

#[cfg(test)]
mod tests {
    use crate::Draft;

    use super::subresources_of;
    use ahash::HashSet;
    use serde_json::json;
    use test_case::test_case;

    #[test_case(&json!(true), &[] ; "boolean schema")]
    #[test_case(&json!(false), &[] ; "boolean schema false")]
    #[test_case(&json!({}), &[] ; "empty object")]
    #[test_case(&json!({"type": "string"}), &[] ; "no subresources")]
    #[test_case(
        &json!({"additionalProperties": {"type": "string"}}),
        &[json!({"type": "string"})] ;
        "in_value single"
    )]
    #[test_case(
        &json!({"if": {"type": "string"}, "then": {"minimum": 0}}),
        &[json!({"type": "string"}), json!({"minimum": 0})] ;
        "in_value multiple"
    )]
    #[test_case(
        &json!({"properties": {"foo": {"type": "string"}, "bar": {"type": "number"}}}),
        &[json!({"type": "string"}), json!({"type": "number"})] ;
        "in_subvalues"
    )]
    #[test_case(
        &json!({"allOf": [{"type": "string"}, {"minLength": 1}]}),
        &[json!({"type": "string"}), json!({"minLength": 1})] ;
        "in_subarray"
    )]
    #[test_case(
        &json!({
            "type": "object",
            "properties": {
                "foo": {"type": "string"},
                "bar": {"type": "number"}
            },
            "additionalProperties": {"type": "boolean"},
            "allOf": [
                {"required": ["foo"]},
                {"required": ["bar"]}
            ]
        }),
        &[
            json!({"type": "string"}),
            json!({"type": "number"}),
            json!({"type": "boolean"}),
            json!({"required": ["foo"]}),
            json!({"required": ["bar"]})
        ] ;
        "complex schema"
    )]
    #[test_case(
        &json!({
            "$defs": {
                "positiveInteger": {
                    "type": "integer",
                    "exclusiveMinimum": 0
                }
            },
            "properties": {
                "count": { "$ref": "#/$defs/positiveInteger" }
            }
        }),
        &[
            json!({"type": "integer", "exclusiveMinimum": 0}),
            json!({"$ref": "#/$defs/positiveInteger"})
        ] ;
        "with $defs"
    )]
    fn test_subresources_of(schema: &serde_json::Value, expected: &[serde_json::Value]) {
        let subresources: HashSet<&serde_json::Value> = subresources_of(schema).collect();
        let expected_set: HashSet<&serde_json::Value> = expected.iter().collect();

        assert_eq!(
            subresources.len(),
            expected.len(),
            "Number of subresources doesn't match"
        );
        assert_eq!(
            subresources, expected_set,
            "Subresources don't match expected values"
        );
    }

    #[test]
    fn test_all_keywords() {
        let schema = json!({
            "additionalProperties": {"type": "string"},
            "contains": {"minimum": 0},
            "contentSchema": {"format": "email"},
            "else": {"maximum": 100},
            "if": {"type": "number"},
            "items": {"type": "array"},
            "not": {"type": "null"},
            "propertyNames": {"minLength": 1},
            "then": {"multipleOf": 2},
            "unevaluatedItems": {"type": "boolean"},
            "unevaluatedProperties": {"type": "integer"},
            "allOf": [{"type": "object"}, {"required": ["foo"]}],
            "anyOf": [{"minimum": 0}, {"maximum": 100}],
            "oneOf": [{"type": "string"}, {"type": "number"}],
            "prefixItems": [{"type": "string"}, {"type": "number"}],
            "$defs": {
                "positiveInteger": {"type": "integer", "minimum": 1}
            },
            "definitions": {
                "negativeInteger": {"type": "integer", "maximum": -1}
            },
            "dependentSchemas": {
                "foo": {"required": ["bar"]}
            },
            "patternProperties": {
                "^S_": {"type": "string"},
                "^I_": {"type": "integer"}
            },
            "properties": {
                "prop1": {"type": "string"},
                "prop2": {"type": "number"}
            }
        });

        let subresources: Vec<&serde_json::Value> = subresources_of(&schema).collect();
        assert_eq!(subresources.len(), 26);

        assert!(subresources.contains(&&json!({"type": "string"})));
        assert!(subresources.contains(&&json!({"minimum": 0})));
        assert!(subresources.contains(&&json!({"format": "email"})));
        assert!(subresources.contains(&&json!({"maximum": 100})));
        assert!(subresources.contains(&&json!({"type": "number"})));
        assert!(subresources.contains(&&json!({"type": "array"})));
        assert!(subresources.contains(&&json!({"type": "null"})));
        assert!(subresources.contains(&&json!({"minLength": 1})));
        assert!(subresources.contains(&&json!({"multipleOf": 2})));
        assert!(subresources.contains(&&json!({"type": "boolean"})));
        assert!(subresources.contains(&&json!({"type": "integer"})));
        assert!(subresources.contains(&&json!({"type": "object"})));
        assert!(subresources.contains(&&json!({"required": ["foo"]})));
        assert!(subresources.contains(&&json!({"minimum": 0})));
        assert!(subresources.contains(&&json!({"maximum": 100})));
        assert!(subresources.contains(&&json!({"type": "string"})));
        assert!(subresources.contains(&&json!({"type": "number"})));
        assert!(subresources.contains(&&json!({"type": "integer", "minimum": 1})));
        assert!(subresources.contains(&&json!({"type": "integer", "maximum": -1})));
        assert!(subresources.contains(&&json!({"required": ["bar"]})));
        assert!(subresources.contains(&&json!({"type": "string"})));
        assert!(subresources.contains(&&json!({"type": "integer"})));
    }

    #[test_case(Draft::Draft4)]
    #[test_case(Draft::Draft6)]
    #[test_case(Draft::Draft7)]
    #[test_case(Draft::Draft201909)]
    #[test_case(Draft::Draft202012)]
    fn test_subresources_of_bool_schema(draft: Draft) {
        let bool_schema = json!(true);

        assert!(
            draft
                .subresources_of(&bool_schema)
                .collect::<Vec<_>>()
                .is_empty(),
            "Draft {draft:?} should return empty subresources for boolean schema",
        );
    }
}
