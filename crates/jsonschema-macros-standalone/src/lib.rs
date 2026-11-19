//! Only `jsonschema` is in scope here, so any path a generated validator names outside the
//! alias table fails to resolve.

use jsonschema::__private::serde_json::{Map, Value};

#[jsonschema::validator(
    schema = r##"{
        "type": "object",
        "properties": {
            "kind": {"const": "a"},
            "exact": {"const": 1.5},
            "choice": {"enum": [1, 2, {"nested": true}]},
            "count": {"type": "integer", "minimum": 1, "maximum": 10, "multipleOf": 2},
            "ratio": {"type": "number", "exclusiveMinimum": 0.5, "multipleOf": 0.1},
            "name": {"type": "string", "minLength": 1, "maxLength": 8, "pattern": "^\\S*$"},
            "free": {"type": "string", "pattern": "a.+b"},
            "when": {"type": "string", "format": "date-time"},
            "tags": {"type": "array", "uniqueItems": true, "minItems": 1},
            "notNull": {"not": {"type": "null"}},
            "either": {"oneOf": [{"type": "string"}, {"type": "integer"}]},
            "both": {"allOf": [{"type": "object"}, {"required": ["x"]}]},
            "self": {"$ref": "#"}
        },
        "patternProperties": {"^x-": {"type": "string"}},
        "propertyNames": {"maxLength": 32},
        "additionalProperties": false,
        "required": ["kind"],
        "if": {"required": ["count"]},
        "then": {"required": ["ratio"]},
        "else": {"required": ["name"]}
    }"##,
    validate_formats = true
)]
pub struct Schema;

// `contentEncoding` and `contentMediaType` assert only on drafts 6 and 7.
#[jsonschema::validator(
    schema = r#"{"type": "string", "contentEncoding": "base64", "contentMediaType": "application/json"}"#,
    draft = Draft7
)]
pub struct Content;

struct Even;

impl<'i> jsonschema::Keyword<'i> for Even {
    fn validate(&self, instance: &'i Value) -> Result<(), jsonschema::ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(jsonschema::ValidationError::custom("number must be even"))
        }
    }

    fn is_valid(&self, instance: &'i Value) -> bool {
        instance.as_u64().is_none_or(|number| number % 2 == 0)
    }
}

// The `Result` is required by the keyword factory signature.
#[allow(clippy::unnecessary_wraps)]
fn even<'a>(
    _parent: &'a Map<String, Value>,
    _value: &'a Value,
    _path: jsonschema::paths::Location,
) -> Result<Box<dyn for<'i> jsonschema::Keyword<'i>>, jsonschema::ValidationError<'a>> {
    Ok(Box::new(Even))
}

#[jsonschema::validator(
    schema = r#"{"type": "integer", "even": true}"#,
    keywords = { "even" => crate::even }
)]
pub struct Custom;

#[cfg(test)]
mod tests {
    use super::{Custom, Schema};
    use jsonschema::__private::serde_json::json;

    #[test]
    fn entry_points_are_callable_without_a_serde_json_dependency() {
        let instance = json!({"kind": "a", "name": "abc"});
        assert!(Schema::is_valid(&instance));
        assert!(Schema::validate(&instance).is_ok());
        assert_eq!(Schema::iter_errors(&instance).count(), 0);

        let invalid = json!({"kind": "b"});
        assert!(!Schema::is_valid(&invalid));
        assert!(Schema::validate(&invalid).is_err());
        assert_eq!(Schema::iter_errors(&invalid).count(), 2);

        assert!(Custom::is_valid(&json!(2)));
        assert_eq!(
            Custom::validate(&json!(3))
                .expect_err("odd number")
                .to_string(),
            "number must be even"
        );
    }
}
