use serde_json::Value;

mod draft201909;
mod draft4;
mod draft6;
mod draft7;
mod ids;
mod subresources;

use crate::{anchors, Anchor, Error, Resolver, Resource, ResourceRef, Segments};

/// JSON Schema specification versions.
#[non_exhaustive]
#[derive(Debug, Default, PartialEq, Copy, Clone, Hash, Eq)]
pub enum Draft {
    /// JSON Schema Draft 4
    Draft4,
    /// JSON Schema Draft 6
    Draft6,
    /// JSON Schema Draft 7
    #[default]
    Draft7,
    /// JSON Schema Draft 2019-09
    Draft201909,
    /// JSON Schema Draft 2020-12
    Draft202012,
}

impl Draft {
    #[must_use]
    pub fn create_resource(self, contents: Value) -> Resource {
        Resource::new(contents, self)
    }
    #[must_use]
    pub fn create_resource_ref(self, contents: &Value) -> ResourceRef<'_> {
        ResourceRef::new(contents, self)
    }
    /// Detect what specification could be applied to the given contents.
    ///
    /// # Errors
    ///
    /// On unknown `$schema` value it returns [`Error::UnknownSpecification`]
    pub fn detect(self, contents: &Value) -> Result<Draft, Error> {
        if let Some(schema) = contents
            .as_object()
            .and_then(|contents| contents.get("$schema"))
            .and_then(|schema| schema.as_str())
        {
            Ok(match schema.trim_end_matches('#') {
                "https://json-schema.org/draft/2020-12/schema" => Draft::Draft202012,
                "https://json-schema.org/draft/2019-09/schema" => Draft::Draft201909,
                "http://json-schema.org/draft-07/schema" => Draft::Draft7,
                "http://json-schema.org/draft-06/schema" => Draft::Draft6,
                "http://json-schema.org/draft-04/schema" => Draft::Draft4,
                value => return Err(Error::unknown_specification(value)),
            })
        } else {
            Ok(self)
        }
    }
    pub(crate) fn id_of(self, contents: &Value) -> Option<&str> {
        match self {
            Draft::Draft4 => ids::legacy_id(contents),
            Draft::Draft6 | Draft::Draft7 => ids::legacy_dollar_id(contents),
            Draft::Draft201909 | Draft::Draft202012 => ids::dollar_id(contents),
        }
    }
    #[must_use]
    pub fn subresources_of<'a>(
        self,
        contents: &'a Value,
    ) -> Box<dyn Iterator<Item = &'a Value> + 'a> {
        match self {
            Draft::Draft4 => draft4::subresources_of(contents),
            Draft::Draft6 => draft6::subresources_of(contents),
            Draft::Draft7 => draft7::subresources_of(contents),
            Draft::Draft201909 => draft201909::subresources_of(contents),
            Draft::Draft202012 => subresources::subresources_of(contents),
        }
    }
    pub(crate) fn anchors<'a>(self, contents: &'a Value) -> Box<dyn Iterator<Item = Anchor> + 'a> {
        match self {
            Draft::Draft4 => anchors::legacy_anchor_in_id(self, contents),
            Draft::Draft6 | Draft::Draft7 => anchors::legacy_anchor_in_dollar_id(self, contents),
            Draft::Draft201909 => anchors::anchor_2019(self, contents),
            Draft::Draft202012 => anchors::anchor(self, contents),
        }
    }
    pub(crate) fn maybe_in_subresource<'r>(
        self,
        segments: &Segments,
        resolver: &Resolver<'r>,
        subresource: ResourceRef<'r>,
    ) -> Result<Resolver<'r>, Error> {
        match self {
            Draft::Draft4 => draft4::maybe_in_subresource(segments, resolver, subresource),
            Draft::Draft6 => draft6::maybe_in_subresource(segments, resolver, subresource),
            Draft::Draft7 => draft7::maybe_in_subresource(segments, resolver, subresource),
            Draft::Draft201909 => {
                draft201909::maybe_in_subresource(segments, resolver, subresource)
            }
            Draft::Draft202012 => {
                subresources::maybe_in_subresource(segments, resolver, subresource)
            }
        }
    }
    /// Identifies known JSON schema keywords per draft.
    ///
    /// Optimized for unknown keywords and uses first-byte check and a small number
    /// of comparisons, which makes it ~2-3x faster than `AHashMap`.
    #[must_use]
    pub fn is_known_keyword(&self, keyword: &str) -> bool {
        let bytes = keyword.as_bytes();
        match bytes.first() {
            Some(b'$') => {
                bytes == b"$schema"
                    || bytes == b"$ref"
                    || (matches!(self, Draft::Draft201909 | Draft::Draft202012)
                        && (bytes == b"$anchor" || bytes == b"$defs"))
                    || (matches!(self, Draft::Draft201909)
                        && (bytes == b"$recursiveAnchor" || bytes == b"$recursiveRef"))
                    || (matches!(self, Draft::Draft202012)
                        && (bytes == b"$dynamicAnchor" || bytes == b"$dynamicRef"))
                    || (matches!(
                        self,
                        Draft::Draft6 | Draft::Draft7 | Draft::Draft201909 | Draft::Draft202012
                    ) && bytes == b"$id")
            }
            Some(b'a') => {
                bytes == b"additionalItems"
                    || bytes == b"additionalProperties"
                    || bytes == b"allOf"
                    || bytes == b"anyOf"
            }
            Some(b'c') => {
                ((bytes == b"const" || bytes == b"contains")
                    && matches!(
                        self,
                        Draft::Draft6 | Draft::Draft7 | Draft::Draft201909 | Draft::Draft202012
                    ))
                    || (matches!(self, Draft::Draft6 | Draft::Draft7)
                        && (bytes == b"contentEncoding" || bytes == b"contentMediaType"))
            }
            Some(b'd') => {
                bytes == b"dependencies"
                    || (matches!(self, Draft::Draft201909 | Draft::Draft202012)
                        && (bytes == b"dependentRequired" || bytes == b"dependentSchemas"))
            }
            Some(b'e') => {
                bytes == b"enum"
                    || bytes == b"exclusiveMaximum"
                    || bytes == b"exclusiveMinimum"
                    || (matches!(
                        self,
                        Draft::Draft7 | Draft::Draft201909 | Draft::Draft202012
                    ) && bytes == b"else")
            }
            Some(b'f') => bytes == b"format",
            Some(b'i') => {
                bytes == b"items"
                    || (matches!(
                        self,
                        Draft::Draft7 | Draft::Draft201909 | Draft::Draft202012
                    ) && bytes == b"if")
                    || (matches!(self, Draft::Draft4) && bytes == b"id")
            }
            Some(b'm') => {
                bytes == b"maximum"
                    || bytes == b"maxItems"
                    || bytes == b"maxLength"
                    || bytes == b"maxProperties"
                    || bytes == b"minimum"
                    || bytes == b"minItems"
                    || bytes == b"minLength"
                    || bytes == b"minProperties"
                    || bytes == b"multipleOf"
                    || (matches!(self, Draft::Draft201909 | Draft::Draft202012)
                        && (bytes == b"maxContains" || bytes == b"minContains"))
            }
            Some(b'n') => bytes == b"not",
            Some(b'o') => bytes == b"oneOf",
            Some(b'p') => {
                bytes == b"pattern"
                    || bytes == b"patternProperties"
                    || bytes == b"properties"
                    || (matches!(
                        self,
                        Draft::Draft6 | Draft::Draft7 | Draft::Draft201909 | Draft::Draft202012
                    ) && bytes == b"propertyNames")
                    || (matches!(self, Draft::Draft201909 | Draft::Draft202012)
                        && bytes == b"prefixItems")
            }
            Some(b'r') => bytes == b"required",
            Some(b't') => {
                bytes == b"type"
                    || (matches!(
                        self,
                        Draft::Draft7 | Draft::Draft201909 | Draft::Draft202012
                    ) && bytes == b"then")
            }
            Some(b'u') => {
                bytes == b"uniqueItems"
                    || (matches!(self, Draft::Draft201909 | Draft::Draft202012)
                        && (bytes == b"unevaluatedItems" || bytes == b"unevaluatedProperties"))
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Draft;
    use serde_json::json;
    use test_case::test_case;

    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema"}), Draft::Draft202012; "detect Draft 2020-12")]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2020-12/schema#"}), Draft::Draft202012; "detect Draft 2020-12 with fragment")]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft/2019-09/schema"}), Draft::Draft201909; "detect Draft 2019-09")]
    #[test_case(&json!({"$schema": "http://json-schema.org/draft-07/schema"}), Draft::Draft7; "detect Draft 7")]
    #[test_case(&json!({"$schema": "http://json-schema.org/draft-06/schema"}), Draft::Draft6; "detect Draft 6")]
    #[test_case(&json!({"$schema": "http://json-schema.org/draft-04/schema"}), Draft::Draft4; "detect Draft 4")]
    #[test_case(&json!({}), Draft::Draft7; "default to Draft 7 when no $schema")]
    fn test_detect(contents: &serde_json::Value, expected: Draft) {
        let result = Draft::Draft7
            .detect(contents)
            .expect("Unknown specification");
        assert_eq!(result, expected);
    }

    #[test]
    fn test_unknown_specification() {
        let error = Draft::Draft7
            .detect(&json!({"$schema": "invalid"}))
            .expect_err("Unknown specification");
        assert_eq!(error.to_string(), "Unknown specification: invalid");
    }

    #[test_case(Draft::Draft4; "Draft 4 stays Draft 4")]
    #[test_case(Draft::Draft6; "Draft 6 stays Draft 6")]
    #[test_case(Draft::Draft7; "Draft 7 stays Draft 7")]
    #[test_case(Draft::Draft201909; "Draft 2019-09 stays Draft 2019-09")]
    #[test_case(Draft::Draft202012; "Draft 2020-12 stays Draft 2020-12")]
    fn test_detect_no_change(draft: Draft) {
        let contents = json!({});
        let result = draft.detect(&contents).expect("Failed to detect draft");
        assert_eq!(result, draft);
    }
}