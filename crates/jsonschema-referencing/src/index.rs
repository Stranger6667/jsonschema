use std::sync::Arc;

use ahash::AHashMap;
use fluent_uri::Uri;
use serde_json::Value;

use crate::{
    cache::SharedUriCache,
    uri,
    vocabularies::{self, VocabularySet},
    Anchor, Draft, Error, Resolver, ResourceRef,
};

pub(crate) type ResourceMap<'r> = AHashMap<Arc<Uri<String>>, ResourceRef<'r>>;
pub(crate) type AnchorMap<'r> = AHashMap<Arc<Uri<String>>, AHashMap<&'r str, Anchor<'r>>>;

/// Resolution index built from a [`Registry`].
#[derive(Debug)]
pub struct Index<'r> {
    pub(crate) resources: ResourceMap<'r>,
    pub(crate) anchors: AnchorMap<'r>,
    pub(crate) resolution_cache: &'r SharedUriCache,
}

impl<'r> Index<'r> {
    pub(crate) fn new(
        resources: ResourceMap<'r>,
        anchors: AnchorMap<'r>,
        resolution_cache: &'r SharedUriCache,
    ) -> Self {
        Self {
            resources,
            anchors,
            resolution_cache,
        }
    }

    #[must_use]
    pub fn contains_resource_uri(&self, uri: &str) -> bool {
        let Ok(uri) = uri::from_str(uri) else {
            return false;
        };
        self.resources.contains_key(&uri)
    }

    #[cfg(test)]
    pub(crate) fn resource(&self, uri: &str) -> Option<ResourceRef<'r>> {
        let uri = uri::from_str(uri).ok()?;
        self.resources.get(&uri).copied()
    }

    pub(crate) fn resource_by_uri(&self, uri: &Uri<String>) -> Option<ResourceRef<'r>> {
        self.resources.get(uri).copied()
    }

    #[must_use]
    pub fn contains_anchor(&self, uri: &str, name: &str) -> bool {
        let Ok(uri) = uri::from_str(uri) else {
            return false;
        };
        self.anchors
            .get(&uri)
            .is_some_and(|entries| entries.contains_key(name))
    }

    #[cfg(test)]
    pub(crate) fn is_dynamic_anchor(&self, uri: &str, name: &str) -> bool {
        let Ok(uri) = uri::from_str(uri) else {
            return false;
        };
        matches!(
            self.anchors.get(&uri).and_then(|entries| entries.get(name)),
            Some(Anchor::Dynamic { .. })
        )
    }

    pub(crate) fn anchor<'a>(
        &self,
        uri: &'a Uri<String>,
        name: &'a str,
    ) -> Result<&Anchor<'r>, Error> {
        if let Some(entries) = self.anchors.get(uri) {
            if let Some(value) = entries.get(name) {
                return Ok(value);
            }
        }

        // Fallback: if the document was retrieved under a different URI than its declared
        // `$id`, anchors are indexed under the canonical `$id` URI.  Look up the resource
        // by the supplied URI, resolve its `$id`, and retry under the canonical URI.
        if let Some(resource) = self.resources.get(uri) {
            if let Some(id) = resource.id() {
                let uri = uri::from_str(id)?;
                if let Some(entries) = self.anchors.get(&uri) {
                    if let Some(value) = entries.get(name) {
                        return Ok(value);
                    }
                }
            }
        }

        if name.contains('/') {
            Err(Error::invalid_anchor(name.to_string()))
        } else {
            Err(Error::no_such_anchor(name.to_string()))
        }
    }

    /// Resolves a reference URI against a base URI using index cache.
    ///
    /// # Errors
    ///
    /// Returns an error if base has no scheme or there is a fragment.
    pub fn resolve_against(&self, base: &Uri<&str>, uri: &str) -> Result<Arc<Uri<String>>, Error> {
        self.resolution_cache.resolve_against(base, uri)
    }

    #[must_use]
    pub fn resolver(&'r self, base_uri: Uri<String>) -> Resolver<'r> {
        Resolver::new(self, Arc::new(base_uri))
    }

    /// Returns vocabulary set configured for given draft and contents.
    ///
    /// For custom meta-schemas (`Draft::Unknown`), looks up the meta-schema in the index
    /// and extracts its `$vocabulary` declaration. If the meta-schema is not present,
    /// returns the default Draft 2020-12 vocabularies.
    #[must_use]
    pub fn find_vocabularies(&self, draft: Draft, contents: &Value) -> VocabularySet {
        match draft.detect(contents) {
            Draft::Unknown => {
                if let Some(specification) = contents
                    .as_object()
                    .and_then(|obj| obj.get("$schema"))
                    .and_then(|s| s.as_str())
                {
                    if let Ok(mut uri) = uri::from_str(specification) {
                        uri.set_fragment(None);
                        if let Some(resource) = self.resources.get(&uri) {
                            if let Ok(Some(vocabularies)) = vocabularies::find(resource.contents())
                            {
                                return vocabularies;
                            }
                        }
                    }
                }
                Draft::Unknown.default_vocabularies()
            }
            draft => draft.default_vocabularies(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{uri, Draft, Registry, Resource};
    use serde_json::json;

    #[test]
    fn build_index_contains_root_resource() {
        let registry = Registry::try_new(
            "urn:root",
            Resource::from_contents(json!({"type": "string"})),
        )
        .unwrap();
        let index = registry.build_index().unwrap();
        assert!(index.contains_resource_uri("urn:root"));
    }

    #[test]
    fn build_index_keeps_resource_draft() {
        let schema = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "string"
        });
        let registry = Registry::try_new("urn:r", Resource::from_contents(schema)).unwrap();
        let index = registry.build_index().unwrap();
        assert_eq!(index.resource("urn:r").unwrap().draft(), Draft::Draft7);
    }

    #[test]
    fn build_index_keeps_dynamic_anchor_kind() {
        let schema = json!({"$dynamicAnchor": "node"});
        let registry = Registry::try_new("urn:r", Resource::from_contents(schema)).unwrap();
        let index = registry.build_index().unwrap();
        assert!(index.is_dynamic_anchor("urn:r", "node"));
    }

    #[test]
    fn build_index_indexes_subresources_and_anchors() {
        let schema = json!({
            "$id": "http://example.com/root",
            "$defs": {
                "child": {
                    "$id": "nested.json",
                    "$anchor": "node",
                    "type": "integer"
                }
            },
            "$ref": "nested.json#node"
        });
        let registry =
            Registry::try_new("http://example.com/root", Resource::from_contents(schema)).unwrap();
        let index = registry.build_index().unwrap();
        assert!(index.contains_resource_uri("http://example.com/nested.json"));
        assert!(index.contains_anchor("http://example.com/nested.json", "node"));
    }

    #[test]
    fn build_index_anchor_lookup_via_storage_uri() {
        // Schema stored under "urn:root" but declares $id "http://example.com/root".
        // Anchors are indexed under the canonical URI; the fallback in Index::anchor
        // translates the storage URI to the canonical URI on lookup.
        let schema = json!({
            "$id": "http://example.com/root",
            "$anchor": "entry"
        });
        let registry = Registry::try_new("urn:root", Resource::from_contents(schema)).unwrap();
        let index = registry.build_index().unwrap();
        let resolver = index.resolver(uri::from_str("urn:root").unwrap());
        let resolved = resolver.lookup("#entry").unwrap();
        assert!(resolved.contents().is_object());
    }

    #[test]
    fn build_index_keeps_resolve_against_behavior() {
        let registry = Registry::try_new(
            "http://example.com/root",
            Resource::from_contents(json!({"type": "object"})),
        )
        .unwrap();
        let index = registry.build_index().unwrap();
        let base = uri::from_str("http://example.com/root").unwrap();
        let resolved = index.resolve_against(&base.borrow(), "child.json").unwrap();
        assert_eq!(resolved.as_str(), "http://example.com/child.json");
    }
}
