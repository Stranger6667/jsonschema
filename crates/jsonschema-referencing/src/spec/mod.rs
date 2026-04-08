//! Types used by the schema traversal machinery during registry building.
//!
//! When the registry walks a schema document, each JSON object is scanned to extract
//! relevant information per [`ObjectAnalysis`].
//!
//! The sub-modules contain draft-specific scanning logic.

pub(crate) mod draft201909;
pub(crate) mod draft202012;
pub(crate) mod draft4;
pub(crate) mod draft6;
pub(crate) mod draft7;
pub(crate) mod ids;

use serde_json::{Map, Value};

/// Shared metadata extracted from one schema object.
pub(crate) struct ObjectAnalysis<'a> {
    pub(crate) id: Option<&'a str>,
    pub(crate) has_anchor: bool,
    pub(crate) dollar_ref: Option<&'a str>,
    pub(crate) meta_schema: Option<&'a str>,
}

/// `analyze_object` implementation for drafts 2019-09 and 2020-12.
///
/// In these drafts anchors are a first-class `"$anchor"` keyword. Draft 2020-12 additionally
/// recognises `"$dynamicAnchor"` — pass `dynamic_anchor: true` for that draft.
pub(crate) fn analyze_object_modern(
    schema: &Map<String, Value>,
    dynamic_anchor: bool,
) -> ObjectAnalysis<'_> {
    let mut id = None;
    let mut has_anchor = false;
    let mut dollar_ref = None;
    let mut meta_schema = None;

    for (key, value) in schema {
        match key.as_str() {
            "$id" => id = value.as_str(),
            "$anchor" => has_anchor |= value.as_str().is_some(),
            "$dynamicAnchor" if dynamic_anchor => has_anchor |= value.as_str().is_some(),
            "$ref" => dollar_ref = value.as_str(),
            "$schema" => meta_schema = value.as_str(),
            _ => {}
        }
    }

    ObjectAnalysis {
        id,
        has_anchor,
        dollar_ref,
        meta_schema,
    }
}

/// `analyze_object` implementation for drafts 4, 6, and 7.
///
/// In these drafts anchors are encoded as a bare-fragment ID (`"#name"`), the
/// ID key differs between draft 4 (`"id"`) and drafts 6/7 (`"$id"`), and
/// `$ref` suppresses the ID.
pub(crate) fn analyze_object_classic<'a>(
    schema: &'a Map<String, Value>,
    id_key: &str,
) -> ObjectAnalysis<'a> {
    let mut raw_id = None;
    let mut dollar_ref = None;
    let mut meta_schema = None;

    for (key, value) in schema {
        let k = key.as_str();
        if k == id_key {
            raw_id = value.as_str();
        } else if k == "$ref" {
            dollar_ref = value.as_str();
        } else if k == "$schema" {
            meta_schema = value.as_str();
        }
    }

    let has_anchor = raw_id.is_some_and(|id| id.starts_with('#'));
    let id = match raw_id {
        Some(id) if !has_anchor && dollar_ref.is_none() => Some(id),
        _ => None,
    };

    ObjectAnalysis {
        id,
        has_anchor,
        dollar_ref,
        meta_schema,
    }
}
