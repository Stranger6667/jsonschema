use serde_json::{Map, Value};

use crate::{
    anchor,
    path::JsonPointerSegment,
    spec::{self, draft201909, draft202012, draft4, draft6, draft7, ObjectAnalysis},
    vocabularies::{VocabularySet, DRAFT_2019_09_VOCABULARIES, DRAFT_2020_12_VOCABULARIES},
    Anchor, Error, Resolver, Resource, ResourceRef, Segments,
};

/// JSON Schema specification versions.
#[non_exhaustive]
#[derive(Debug, Default, PartialEq, Copy, Clone, Hash, Eq, PartialOrd, Ord)]
pub enum Draft {
    /// JSON Schema Draft 4
    Draft4,
    /// JSON Schema Draft 6
    Draft6,
    /// JSON Schema Draft 7
    Draft7,
    /// JSON Schema Draft 2019-09
    Draft201909,
    /// JSON Schema Draft 2020-12
    #[default]
    Draft202012,
    /// Internal use only: Represents custom/unrecognized meta-schemas.
    /// Do not use directly. Custom meta-schemas are resolved automatically
    /// when registered in the Registry.
    #[doc(hidden)]
    Unknown,
}

impl Draft {
    /// Wraps `contents` in a [`Resource`] tagged with this draft version.
    #[must_use]
    pub fn create_resource(self, contents: Value) -> Resource {
        Resource::new(contents, self)
    }
    /// Wraps a reference to `contents` in a [`ResourceRef`] tagged with this draft version.
    #[must_use]
    pub fn create_resource_ref(self, contents: &Value) -> ResourceRef<'_> {
        ResourceRef::new(contents, self)
    }
    /// Detect a draft from a `$schema` URI string.
    ///
    /// Accepts the URI with or without a trailing `#`. Returns `Draft::Unknown`
    /// for custom/unknown meta-schema URIs.
    #[must_use]
    pub fn from_schema_uri(uri: &str) -> Draft {
        match uri.trim_end_matches('#') {
            // Accept both HTTPS and HTTP for all known drafts
            "https://json-schema.org/draft/2020-12/schema"
            | "http://json-schema.org/draft/2020-12/schema" => Draft::Draft202012,
            "https://json-schema.org/draft/2019-09/schema"
            | "http://json-schema.org/draft/2019-09/schema" => Draft::Draft201909,
            "https://json-schema.org/draft-07/schema"
            | "http://json-schema.org/draft-07/schema" => Draft::Draft7,
            "https://json-schema.org/draft-06/schema"
            | "http://json-schema.org/draft-06/schema" => Draft::Draft6,
            "https://json-schema.org/draft-04/schema"
            | "http://json-schema.org/draft-04/schema" => Draft::Draft4,
            // Custom/unknown meta-schemas return Unknown
            // Validation of custom meta-schemas happens during registry building
            _ => Draft::Unknown,
        }
    }
    /// Detect what specification could be applied to the given contents.
    ///
    /// Inspects the `$schema` field and returns the matching draft. If no `$schema`
    /// field is present, returns `self` unchanged — the caller's current draft is preserved.
    ///
    /// Returns `Draft::Unknown` for custom/unknown `$schema` values.
    /// Validation of custom meta-schemas happens during registry building.
    #[must_use]
    pub fn detect(self, contents: &Value) -> Draft {
        if let Some(uri) = contents
            .as_object()
            .and_then(|contents| contents.get("$schema"))
            .and_then(|schema| schema.as_str())
        {
            Draft::from_schema_uri(uri)
        } else {
            self
        }
    }
    pub(crate) fn id_of(self, contents: &Value) -> Option<&str> {
        match self {
            Draft::Draft4 => spec::ids::legacy_id(contents),
            Draft::Draft6 | Draft::Draft7 => spec::ids::legacy_dollar_id(contents),
            Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown => {
                spec::ids::dollar_id(contents)
            }
        }
    }

    pub fn subresources_of(self, contents: &Value) -> impl Iterator<Item = &Value> {
        match contents.as_object() {
            Some(schema) => {
                let object_iter = match self {
                    Draft::Draft4 => draft4::object_iter,
                    Draft::Draft6 => draft6::object_iter,
                    Draft::Draft7 => draft7::object_iter,
                    Draft::Draft201909 => draft201909::object_iter,
                    Draft::Draft202012 | Draft::Unknown => draft202012::object_iter,
                };
                draft202012::ChildIter::Object(schema.iter().flat_map(object_iter))
            }
            None => draft202012::ChildIter::Empty,
        }
    }
    pub(crate) fn analyze_object(self, contents: &Map<String, Value>) -> ObjectAnalysis<'_> {
        match self {
            Draft::Draft4 => spec::analyze_object_classic(contents, "id"),
            Draft::Draft6 | Draft::Draft7 => spec::analyze_object_classic(contents, "$id"),
            Draft::Draft201909 => spec::analyze_object_modern(contents, false),
            Draft::Draft202012 | Draft::Unknown => spec::analyze_object_modern(contents, true),
        }
    }
    pub(crate) fn walk_children<'a>(
        self,
        contents: &'a Map<String, Value>,
        f: &mut impl FnMut(
            &'a str,
            Option<JsonPointerSegment<'a>>,
            &'a Value,
            Draft,
        ) -> Result<(), Error>,
    ) -> Result<(), Error> {
        match self {
            Draft::Draft4 => draft4::walk_children(contents, self, f),
            Draft::Draft6 => draft6::walk_children(contents, self, f),
            Draft::Draft7 => draft7::walk_children(contents, self, f),
            Draft::Draft201909 => draft201909::walk_children(contents, self, f),
            Draft::Draft202012 | Draft::Unknown => draft202012::walk_children(contents, self, f),
        }
    }
    pub(crate) fn anchors(self, contents: &Value) -> impl Iterator<Item = Anchor<'_>> {
        match self {
            Draft::Draft4 => anchor::legacy_anchor_in_id(self, contents),
            Draft::Draft6 | Draft::Draft7 => anchor::legacy_anchor_in_dollar_id(self, contents),
            Draft::Draft201909 => anchor::anchor_2019(self, contents),
            Draft::Draft202012 | Draft::Unknown => anchor::anchor(self, contents),
        }
    }
    pub(crate) fn maybe_in_subresource<'r>(
        self,
        segments: &Segments,
        resolver: &Resolver<'r>,
        subresource: ResourceRef<'_>,
    ) -> Result<Resolver<'r>, Error> {
        match self {
            Draft::Draft4 => draft4::maybe_in_subresource(segments, resolver, subresource),
            Draft::Draft6 => draft6::maybe_in_subresource(segments, resolver, subresource),
            Draft::Draft7 => draft7::maybe_in_subresource(segments, resolver, subresource),
            Draft::Draft201909 => {
                draft201909::maybe_in_subresource(segments, resolver, subresource)
            }
            Draft::Draft202012 | Draft::Unknown => {
                draft202012::maybe_in_subresource(segments, resolver, subresource)
            }
        }
    }
    /// Identifies known JSON schema keywords per draft.
    #[must_use]
    pub fn is_known_keyword(&self, keyword: &str) -> bool {
        match keyword {
            "$ref"
            | "$schema"
            | "additionalItems"
            | "additionalProperties"
            | "allOf"
            | "anyOf"
            | "dependencies"
            | "enum"
            | "exclusiveMaximum"
            | "exclusiveMinimum"
            | "format"
            | "items"
            | "maxItems"
            | "maxLength"
            | "maxProperties"
            | "maximum"
            | "minItems"
            | "minLength"
            | "minProperties"
            | "minimum"
            | "multipleOf"
            | "not"
            | "oneOf"
            | "pattern"
            | "patternProperties"
            | "properties"
            | "required"
            | "type"
            | "uniqueItems" => true,

            "id" if *self == Draft::Draft4 => true,

            "$id" | "const" | "contains" | "propertyNames"
                if *self >= Draft::Draft6 || *self == Draft::Unknown =>
            {
                true
            }

            "contentEncoding" | "contentMediaType"
                if matches!(self, Draft::Draft6 | Draft::Draft7) =>
            {
                true
            }

            "contentEncoding" | "contentMediaType" | "contentSchema"
                if matches!(
                    self,
                    Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
                ) =>
            {
                true
            }

            "else" | "if" | "then" if *self >= Draft::Draft7 || *self == Draft::Unknown => true,

            "$anchor"
            | "$defs"
            | "$recursiveAnchor"
            | "$recursiveRef"
            | "dependentRequired"
            | "dependentSchemas"
            | "maxContains"
            | "minContains"
            | "prefixItems"
            | "unevaluatedItems"
            | "unevaluatedProperties"
                if *self >= Draft::Draft201909 || *self == Draft::Unknown =>
            {
                true
            }

            "$dynamicAnchor" | "$dynamicRef"
                if *self == Draft::Draft202012 || *self == Draft::Unknown =>
            {
                true
            }

            _ => false,
        }
    }

    pub(crate) fn default_vocabularies(self) -> VocabularySet {
        match self {
            Draft::Draft4 | Draft::Draft6 | Draft::Draft7 => VocabularySet::new(),
            Draft::Draft201909 => VocabularySet::from_known(DRAFT_2019_09_VOCABULARIES),
            Draft::Draft202012 | Draft::Unknown => {
                VocabularySet::from_known(DRAFT_2020_12_VOCABULARIES)
            }
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
    #[test_case(&json!({"$schema": "https://json-schema.org/draft-07/schema"}), Draft::Draft7; "detect Draft 7 https")]
    #[test_case(&json!({"$schema": "http://json-schema.org/draft-06/schema"}), Draft::Draft6; "detect Draft 6")]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft-06/schema"}), Draft::Draft6; "detect Draft 6 https")]
    #[test_case(&json!({"$schema": "http://json-schema.org/draft-04/schema"}), Draft::Draft4; "detect Draft 4")]
    #[test_case(&json!({"$schema": "https://json-schema.org/draft-04/schema"}), Draft::Draft4; "detect Draft 4 https")]
    #[test_case(&json!({}), Draft::Draft7; "default to Draft 7 when no $schema")]
    fn test_detect(contents: &serde_json::Value, expected: Draft) {
        let result = Draft::Draft7.detect(contents);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_unknown_specification() {
        let draft = Draft::Draft7.detect(&json!({"$schema": "invalid"}));
        assert_eq!(draft, Draft::Unknown);
    }

    #[test_case(Draft::Draft4; "Draft 4 stays Draft 4")]
    #[test_case(Draft::Draft6; "Draft 6 stays Draft 6")]
    #[test_case(Draft::Draft7; "Draft 7 stays Draft 7")]
    #[test_case(Draft::Draft201909; "Draft 2019-09 stays Draft 2019-09")]
    #[test_case(Draft::Draft202012; "Draft 2020-12 stays Draft 2020-12")]
    fn test_detect_no_change(draft: Draft) {
        let contents = json!({});
        let result = draft.detect(&contents);
        assert_eq!(result, draft);
    }
}
