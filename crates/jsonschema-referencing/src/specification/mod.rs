use serde_json::{Map, Value};
use subresources::SubresourceIterator;

mod draft201909;
mod draft4;
mod draft6;
mod draft7;
mod ids;
mod subresources;

use crate::{
    anchors,
    vocabularies::{VocabularySet, DRAFT_2019_09_VOCABULARIES, DRAFT_2020_12_VOCABULARIES},
    Anchor, Error, JsonPointerNode, Resolver, Resource, ResourceRef, Segments,
};

pub(crate) struct BorrowedObjectProbe<'a> {
    pub(crate) id: Option<&'a str>,
    pub(crate) has_anchor: bool,
    pub(crate) has_ref_or_schema: bool,
}

pub(crate) struct OwnedObjectGate<'a> {
    pub(crate) id: Option<&'a str>,
    pub(crate) has_anchor: bool,
    pub(crate) ref_: Option<&'a str>,
    pub(crate) schema: Option<&'a str>,
    pub(crate) has_children: bool,
}

#[derive(Copy, Clone)]
pub(crate) enum OwnedPathSegment<'a> {
    Key(&'a str),
    Index(usize),
}

#[derive(Copy, Clone)]
pub(crate) struct OwnedScratchChild<'a> {
    pub(crate) first: OwnedPathSegment<'a>,
    pub(crate) second: Option<OwnedPathSegment<'a>>,
    pub(crate) value: &'a Value,
    pub(crate) draft: Draft,
}

impl<'a> OwnedScratchChild<'a> {
    #[inline]
    pub(crate) fn key(key: &'a str, value: &'a Value, draft: Draft) -> Self {
        Self {
            first: OwnedPathSegment::Key(key),
            second: None,
            value,
            draft,
        }
    }

    #[inline]
    pub(crate) fn key_index(key: &'a str, index: usize, value: &'a Value, draft: Draft) -> Self {
        Self {
            first: OwnedPathSegment::Key(key),
            second: Some(OwnedPathSegment::Index(index)),
            value,
            draft,
        }
    }

    #[inline]
    pub(crate) fn key_key(
        key: &'a str,
        child_key: &'a str,
        value: &'a Value,
        draft: Draft,
    ) -> Self {
        Self {
            first: OwnedPathSegment::Key(key),
            second: Some(OwnedPathSegment::Key(child_key)),
            value,
            draft,
        }
    }
}

#[inline]
pub(crate) fn has_ref_or_schema_object(schema: &Map<String, Value>) -> bool {
    if schema.len() <= 3 {
        for (key, value) in schema {
            if (key == "$ref" || key == "$schema") && value.is_string() {
                return true;
            }
        }
        false
    } else {
        schema.get("$ref").and_then(Value::as_str).is_some()
            || schema.get("$schema").and_then(Value::as_str).is_some()
    }
}

#[derive(Default)]
pub(crate) struct BorrowedReferenceSlots<'a> {
    pub(crate) ref_: Option<&'a str>,
    pub(crate) schema: Option<&'a str>,
}

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
    #[must_use]
    pub fn create_resource(self, contents: Value) -> Resource {
        Resource::new(contents, self)
    }
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
            Draft::Draft4 => ids::legacy_id(contents),
            Draft::Draft6 | Draft::Draft7 => ids::legacy_dollar_id(contents),
            Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown => ids::dollar_id(contents),
        }
    }

    #[inline]
    pub(crate) fn probe_borrowed_object_map(
        self,
        obj: &Map<String, Value>,
    ) -> BorrowedObjectProbe<'_> {
        match self {
            Draft::Draft4 => analyze_legacy_id_object(obj),
            Draft::Draft6 | Draft::Draft7 => analyze_legacy_dollar_id_object(obj),
            Draft::Draft201909 => analyze_id_and_anchor_object(obj),
            Draft::Draft202012 | Draft::Unknown => analyze_id_and_any_anchor_object(obj),
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
                    Draft::Draft202012 | Draft::Unknown => subresources::object_iter,
                };
                SubresourceIterator::Object(schema.iter().flat_map(object_iter))
            }
            None => SubresourceIterator::Empty,
        }
    }
    pub(crate) fn scan_borrowed_object_into_scratch_map<'a>(
        self,
        contents: &'a Map<String, Value>,
        references: &mut BorrowedReferenceSlots<'a>,
        children: &mut Vec<(&'a Value, Draft)>,
    ) {
        match self {
            Draft::Draft4 => {
                draft4::scan_borrowed_object_into_scratch_map(contents, self, references, children);
            }
            Draft::Draft6 => {
                draft6::scan_borrowed_object_into_scratch_map(contents, self, references, children);
            }
            Draft::Draft7 => {
                draft7::scan_borrowed_object_into_scratch_map(contents, self, references, children);
            }
            Draft::Draft201909 => draft201909::scan_borrowed_object_into_scratch_map(
                contents, self, references, children,
            ),
            Draft::Draft202012 | Draft::Unknown => {
                subresources::scan_borrowed_object_into_scratch_map(
                    contents, self, references, children,
                );
            }
        }
    }
    pub(crate) fn scan_owned_object_into_scratch_map<'a>(
        self,
        contents: &'a Map<String, Value>,
        references: &mut BorrowedReferenceSlots<'a>,
        children: &mut Vec<OwnedScratchChild<'a>>,
    ) -> (Option<&'a str>, bool) {
        match self {
            Draft::Draft4 => {
                draft4::scan_owned_object_into_scratch_map(contents, self, references, children)
            }
            Draft::Draft6 => {
                draft6::scan_owned_object_into_scratch_map(contents, self, references, children)
            }
            Draft::Draft7 => {
                draft7::scan_owned_object_into_scratch_map(contents, self, references, children)
            }
            Draft::Draft201909 => draft201909::scan_owned_object_into_scratch_map(
                contents, self, references, children,
            ),
            Draft::Draft202012 | Draft::Unknown => {
                subresources::scan_owned_object_into_scratch_map(
                    contents, self, references, children,
                )
            }
        }
    }
    pub(crate) fn owned_object_gate_map(
        self,
        contents: &Map<String, Value>,
    ) -> OwnedObjectGate<'_> {
        match self {
            Draft::Draft4 => draft4::owned_object_gate_map(contents),
            Draft::Draft6 => draft6::owned_object_gate_map(contents),
            Draft::Draft7 => draft7::owned_object_gate_map(contents),
            Draft::Draft201909 => draft201909::owned_object_gate_map(contents),
            Draft::Draft202012 | Draft::Unknown => subresources::owned_object_gate_map(contents),
        }
    }
    pub(crate) fn walk_borrowed_subresources_map<'a, E, F>(
        self,
        contents: &'a Map<String, Value>,
        f: &mut F,
    ) -> Result<(), E>
    where
        F: FnMut(&'a Value, Draft) -> Result<(), E>,
    {
        match self {
            Draft::Draft4 => draft4::walk_borrowed_subresources_map(contents, self, f),
            Draft::Draft6 => draft6::walk_borrowed_subresources_map(contents, self, f),
            Draft::Draft7 => draft7::walk_borrowed_subresources_map(contents, self, f),
            Draft::Draft201909 => draft201909::walk_borrowed_subresources_map(contents, self, f),
            Draft::Draft202012 | Draft::Unknown => {
                subresources::walk_borrowed_subresources_map(contents, self, f)
            }
        }
    }
    pub(crate) fn walk_owned_subresources_map<'a, E, F>(
        self,
        contents: &'a Map<String, Value>,
        path: &JsonPointerNode<'_, '_>,
        f: &mut F,
    ) -> Result<(), E>
    where
        F: FnMut(&JsonPointerNode<'_, '_>, &'a Value, Draft) -> Result<(), E>,
    {
        match self {
            Draft::Draft4 => draft4::walk_owned_subresources_map(contents, path, self, f),
            Draft::Draft6 => draft6::walk_owned_subresources_map(contents, path, self, f),
            Draft::Draft7 => draft7::walk_owned_subresources_map(contents, path, self, f),
            Draft::Draft201909 => draft201909::walk_owned_subresources_map(contents, path, self, f),
            Draft::Draft202012 | Draft::Unknown => {
                subresources::walk_owned_subresources_map(contents, path, self, f)
            }
        }
    }
    pub(crate) fn anchors(self, contents: &Value) -> impl Iterator<Item = Anchor<'_>> {
        match self {
            Draft::Draft4 => anchors::legacy_anchor_in_id(self, contents),
            Draft::Draft6 | Draft::Draft7 => anchors::legacy_anchor_in_dollar_id(self, contents),
            Draft::Draft201909 => anchors::anchor_2019(self, contents),
            Draft::Draft202012 | Draft::Unknown => anchors::anchor(self, contents),
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
                subresources::maybe_in_subresource(segments, resolver, subresource)
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

fn analyze_legacy_id_object(obj: &Map<String, Value>) -> BorrowedObjectProbe<'_> {
    if obj.len() <= 3 {
        return scan_legacy_id_probe_small(obj);
    }

    let raw_id = obj.get("id").and_then(Value::as_str);
    let has_ref = obj.get("$ref").and_then(Value::as_str).is_some();
    let has_ref_or_schema = has_ref || obj.get("$schema").and_then(Value::as_str).is_some();
    let has_anchor = raw_id.is_some_and(|id| id.starts_with('#'));
    let id = match raw_id {
        Some(id) if !has_anchor && !has_ref => Some(id),
        _ => None,
    };

    BorrowedObjectProbe {
        id,
        has_anchor,
        has_ref_or_schema,
    }
}

fn scan_legacy_id_probe_small(obj: &Map<String, Value>) -> BorrowedObjectProbe<'_> {
    let mut raw_id = None;
    let mut has_ref = false;
    let mut has_schema = false;

    for (key, value) in obj {
        match key.as_str() {
            "id" => raw_id = value.as_str(),
            "$ref" => has_ref = value.is_string(),
            "$schema" => has_schema = value.is_string(),
            _ => {}
        }
    }

    let has_anchor = raw_id.is_some_and(|id| id.starts_with('#'));
    let id = match raw_id {
        Some(id) if !has_anchor && !has_ref => Some(id),
        _ => None,
    };

    BorrowedObjectProbe {
        id,
        has_anchor,
        has_ref_or_schema: has_ref || has_schema,
    }
}

fn analyze_legacy_dollar_id_object(obj: &Map<String, Value>) -> BorrowedObjectProbe<'_> {
    if obj.len() <= 3 {
        return scan_legacy_dollar_id_probe_small(obj);
    }

    let raw_id = obj.get("$id").and_then(Value::as_str);
    let has_ref = obj.get("$ref").and_then(Value::as_str).is_some();
    let has_ref_or_schema = has_ref || obj.get("$schema").and_then(Value::as_str).is_some();
    let has_anchor = raw_id.is_some_and(|id| id.starts_with('#'));
    let id = match raw_id {
        Some(id) if !has_anchor && !has_ref => Some(id),
        _ => None,
    };

    BorrowedObjectProbe {
        id,
        has_anchor,
        has_ref_or_schema,
    }
}

fn scan_legacy_dollar_id_probe_small(obj: &Map<String, Value>) -> BorrowedObjectProbe<'_> {
    let mut raw_id = None;
    let mut has_ref = false;
    let mut has_schema = false;

    for (key, value) in obj {
        match key.as_str() {
            "$id" => raw_id = value.as_str(),
            "$ref" => has_ref = value.is_string(),
            "$schema" => has_schema = value.is_string(),
            _ => {}
        }
    }

    let has_anchor = raw_id.is_some_and(|id| id.starts_with('#'));
    let id = match raw_id {
        Some(id) if !has_anchor && !has_ref => Some(id),
        _ => None,
    };

    BorrowedObjectProbe {
        id,
        has_anchor,
        has_ref_or_schema: has_ref || has_schema,
    }
}

fn analyze_id_and_anchor_object(obj: &Map<String, Value>) -> BorrowedObjectProbe<'_> {
    if obj.len() <= 2 {
        return scan_id_and_anchor_probe_small(obj);
    }

    BorrowedObjectProbe {
        id: obj.get("$id").and_then(Value::as_str),
        has_anchor: obj.get("$anchor").and_then(Value::as_str).is_some(),
        has_ref_or_schema: has_ref_or_schema_object(obj),
    }
}

fn scan_id_and_anchor_probe_small(obj: &Map<String, Value>) -> BorrowedObjectProbe<'_> {
    let mut id = None;
    let mut has_anchor = false;
    let mut has_ref_or_schema = false;

    for (key, value) in obj {
        match key.as_str() {
            "$id" => id = value.as_str(),
            "$anchor" => has_anchor |= value.as_str().is_some(),
            "$ref" | "$schema" => has_ref_or_schema |= value.is_string(),
            _ => {}
        }
    }

    BorrowedObjectProbe {
        id,
        has_anchor,
        has_ref_or_schema,
    }
}

fn analyze_id_and_any_anchor_object(obj: &Map<String, Value>) -> BorrowedObjectProbe<'_> {
    if obj.len() <= 3 {
        return scan_id_and_any_anchor_probe_small(obj);
    }

    BorrowedObjectProbe {
        id: obj.get("$id").and_then(Value::as_str),
        has_anchor: obj.get("$anchor").and_then(Value::as_str).is_some()
            || obj.get("$dynamicAnchor").and_then(Value::as_str).is_some(),
        has_ref_or_schema: has_ref_or_schema_object(obj),
    }
}

fn scan_id_and_any_anchor_probe_small(obj: &Map<String, Value>) -> BorrowedObjectProbe<'_> {
    let mut id = None;
    let mut has_anchor = false;
    let mut has_ref_or_schema = false;

    for (key, value) in obj {
        match key.as_str() {
            "$id" => id = value.as_str(),
            "$anchor" | "$dynamicAnchor" => has_anchor |= value.as_str().is_some(),
            "$ref" | "$schema" => has_ref_or_schema |= value.is_string(),
            _ => {}
        }
    }

    BorrowedObjectProbe {
        id,
        has_anchor,
        has_ref_or_schema,
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
