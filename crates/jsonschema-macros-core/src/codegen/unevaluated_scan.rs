use serde_json::Value;

/// Recursively scan `schema` for `unevaluatedProperties` / `unevaluatedItems` keyword usage.
/// Returns `(uses_unevaluated_properties, uses_unevaluated_items)`.
///
/// `properties`/`patternProperties`/`$defs`/`definitions` map instance/definition NAME -> schema;
/// their keys are data, not keywords, so a property literally named "unevaluatedProperties" must
/// not trip the flag. Only their *values* are scanned in schema position.
pub(crate) fn scan_uses_unevaluated(schema: &Value) -> (bool, bool) {
    let mut props = false;
    let mut items = false;
    scan(schema, &mut props, &mut items);
    (props, items)
}

/// Like [`scan_uses_unevaluated`] but ORs the result over multiple schema documents. The root and
/// every registry resource it can `$ref` into must all be covered: the keyword appearing in any one
/// resource means the generated validator needs the corresponding helpers. Short-circuits once both
/// flags are set.
pub(crate) fn scan_uses_unevaluated_over<'a>(
    schemas: impl Iterator<Item = &'a Value>,
) -> (bool, bool) {
    let mut props = false;
    let mut items = false;
    for schema in schemas {
        if props && items {
            break;
        }
        let (p, i) = scan_uses_unevaluated(schema);
        props |= p;
        items |= i;
    }
    (props, items)
}

/// Keywords whose value is a NAME -> schema map (as opposed to a schema or list of schemas).
fn is_name_map_keyword(key: &str) -> bool {
    matches!(
        key,
        "properties" | "patternProperties" | "$defs" | "definitions"
    )
}

fn scan(value: &Value, props: &mut bool, items: &mut bool) {
    match value {
        Value::Object(obj) => {
            if obj.contains_key("unevaluatedProperties") {
                *props = true;
            }
            if obj.contains_key("unevaluatedItems") {
                *items = true;
            }
            for (key, child) in obj {
                if *props && *items {
                    return;
                }
                if is_name_map_keyword(key) {
                    scan_name_map(child, props, items);
                } else {
                    scan(child, props, items);
                }
            }
        }
        Value::Array(arr) => {
            for child in arr {
                if *props && *items {
                    return;
                }
                scan(child, props, items);
            }
        }
        _ => {}
    }
}

/// Scans the schema VALUES of a NAME -> schema map; the map's own keys are never inspected.
fn scan_name_map(value: &Value, props: &mut bool, items: &mut bool) {
    if let Value::Object(map) = value {
        for schema in map.values() {
            if *props && *items {
                return;
            }
            scan(schema, props, items);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{scan_uses_unevaluated, scan_uses_unevaluated_over};
    use serde_json::json;

    #[test]
    fn none_present() {
        let s = json!({"type": "object", "properties": {"a": {"type": "string"}}});
        assert_eq!(scan_uses_unevaluated(&s), (false, false));
    }

    #[test]
    fn properties_at_root() {
        let s = json!({"type": "object", "unevaluatedProperties": false});
        assert_eq!(scan_uses_unevaluated(&s), (true, false));
    }

    #[test]
    fn items_nested_under_defs() {
        let s = json!({
            "$defs": {"inner": {"type": "array", "unevaluatedItems": false}},
            "allOf": [{"$ref": "#/$defs/inner"}]
        });
        assert_eq!(scan_uses_unevaluated(&s), (false, true));
    }

    #[test]
    fn both_present() {
        let s = json!({
            "properties": {"a": {"unevaluatedProperties": true}},
            "items": {"unevaluatedItems": true}
        });
        assert_eq!(scan_uses_unevaluated(&s), (true, true));
    }

    #[test]
    fn keyword_as_property_name_is_not_a_use() {
        // A property literally named "unevaluatedProperties" is data, not the keyword.
        let s = json!({"properties": {"unevaluatedProperties": {"type": "string"}}});
        assert_eq!(scan_uses_unevaluated(&s), (false, false));
    }

    #[test]
    fn over_single_root() {
        let root = json!({"type": "object", "unevaluatedProperties": false});
        assert_eq!(
            scan_uses_unevaluated_over(std::iter::once(&root)),
            (true, false)
        );
    }

    #[test]
    fn over_root_clean_resource_dirty() {
        // The keyword lives ONLY in a registry resource the root `$ref`s into.
        let root = json!({"type": "object", "allOf": [{"$ref": "urn:inner"}]});
        let resource = json!({"type": "array", "unevaluatedItems": false});
        assert_eq!(
            scan_uses_unevaluated_over([&root, &resource].into_iter()),
            (false, true)
        );
    }
}
