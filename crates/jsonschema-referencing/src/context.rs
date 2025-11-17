//! Resolution context for deriving resources and anchors from a registry.

use crate::{
    anchors::{Anchor, AnchorKey, AnchorKeyRef},
    resource::{InnerResourcePtr, JsonSchemaResource},
    Draft, Error, Registry, Resolver,
};
use ahash::AHashMap;
use fluent_uri::Uri;
use serde_json::Value;
use std::{collections::VecDeque, sync::Arc};

type ResourceMap = AHashMap<Arc<Uri<String>>, InnerResourcePtr>;

/// Resolution context providing a view over a registry with optional root document.
///
/// This type computes resources and anchors from registry documents on creation.
/// When resolving, it includes both registry documents and an optional root document.
///
/// # Architecture
///
/// - Registry: Stores documents only (pure storage)
/// - `ResolutionContext`: Computes resources + anchors from all documents
/// - Resolver: Uses `ResolutionContext` for URI resolution
///
/// # Lifetimes
///
/// The `'doc` lifetime represents the lifetime of the documents in the registry.
#[derive(Debug)]
pub struct ResolutionContext<'doc> {
    /// Reference to the registry this context was built from
    registry: &'doc Registry<'doc>,

    /// ALL resources (from registry documents + root)
    resources: ResourceMap,

    /// ALL anchors (from registry documents + root)
    anchors: AHashMap<AnchorKey, Anchor>,
}

impl<'doc> ResolutionContext<'doc> {
    /// Create a new resolution context from a registry.
    ///
    /// Computes resources and anchors from all registry documents.
    ///
    /// # Panics
    ///
    /// Panics if the registry documents cannot be converted into a valid context.
    pub fn new(registry: &'doc Registry<'doc>) -> Self {
        let mut context = Self {
            registry,
            resources: ResourceMap::new(),
            anchors: AHashMap::new(),
        };

        let initial = registry.documents.iter().map(|(uri, (cow_value, draft))| {
            let value_ptr = match cow_value {
                std::borrow::Cow::Borrowed(v) => *v as *const _,
                std::borrow::Cow::Owned(v) => v as *const _,
            };
            (
                Arc::clone(uri),
                InnerResourcePtr::new(value_ptr, *draft),
                true,
            )
        });
        context.extend_with_documents(initial);

        context
    }

    /// Get a resource by URI.
    pub(crate) fn get_resource(&self, uri: &Uri<String>) -> Option<&InnerResourcePtr> {
        self.resources.get(uri)
    }

    /// Add a root document to this context.
    ///
    /// This is used during compilation to add the schema being validated as
    /// a resolvable document without modifying the underlying registry.
    ///
    /// # Errors
    ///
    /// This method currently never fails; the `Result` is reserved for future diagnostics.
    pub fn with_root_document(
        mut self,
        uri: Uri<String>,
        schema: &'doc Value,
        draft: Draft,
    ) -> Result<Self, Error> {
        let uri_arc = Arc::new(uri);

        let resource_ptr = InnerResourcePtr::new(std::ptr::from_ref::<Value>(schema), draft);
        self.extend_with_documents([(Arc::clone(&uri_arc), resource_ptr, true)]);

        Ok(self)
    }

    /// Get the resolution cache from the underlying registry.
    #[must_use]
    pub(crate) fn resolution_cache(&self) -> &crate::cache::SharedUriCache {
        &self.registry.resolution_cache
    }

    /// Resolve a URI to the anchor.
    pub(crate) fn anchor(&self, uri: &Uri<String>, name: &str) -> Result<&Anchor, Error> {
        // Check if anchor name contains invalid characters
        if name.contains('/') {
            return Err(Error::invalid_anchor(name));
        }

        let key = AnchorKeyRef::new(uri, name);
        self.anchors
            .get(key.borrow_dyn())
            .ok_or_else(|| Error::no_such_anchor(name))
    }

    /// Resolve a URI against a base.
    ///
    /// # Errors
    ///
    /// If the reference is invalid.
    pub(crate) fn resolve_against(
        &self,
        base: &Uri<&str>,
        uri: &str,
    ) -> Result<Arc<Uri<String>>, Error> {
        self.resolution_cache().resolve_against(base, uri)
    }

    /// Create a resolver with the given base URI.
    ///
    /// # Errors
    ///
    /// Returns an error if the base URI is invalid.
    pub fn try_resolver(&self, base_uri: &str) -> Result<Resolver<'_>, Error> {
        let base = crate::uri::from_str(base_uri)?;
        Ok(self.resolver(base))
    }

    /// Create a resolver with a known valid base URI.
    #[must_use]
    pub fn resolver(&self, base_uri: Uri<String>) -> Resolver<'_> {
        Resolver::new(self, Arc::new(base_uri))
    }

    fn extend_with_documents(
        &mut self,
        docs: impl IntoIterator<Item = (Arc<Uri<String>>, InnerResourcePtr, bool)>,
    ) {
        let mut queue: VecDeque<(Arc<Uri<String>>, InnerResourcePtr, bool)> =
            docs.into_iter().collect();

        while let Some((original_base, resource, is_top_level)) = queue.pop_front() {
            // Register the resource under its original base only for top-level documents.
            if is_top_level {
                self.resources
                    .insert(original_base.clone(), resource.clone());
            }

            let final_base = if let Some(id) = resource.id() {
                match self
                    .registry
                    .resolution_cache
                    .resolve_against(&original_base.borrow(), id)
                {
                    Ok(resolved) => {
                        self.resources.insert(resolved.clone(), resource.clone());
                        resolved
                    }
                    Err(_) => original_base.clone(),
                }
            } else {
                original_base.clone()
            };

            for anchor in resource.anchors() {
                self.anchors
                    .entry(AnchorKey::new(final_base.clone(), anchor.name()))
                    .or_insert(anchor);
            }

            if is_top_level && final_base != original_base {
                for anchor in resource.anchors() {
                    self.anchors
                        .entry(AnchorKey::new(original_base.clone(), anchor.name()))
                        .or_insert(anchor);
                }
            }

            for subresource_contents in resource.draft().subresources_of(resource.contents()) {
                let subresource = InnerResourcePtr::new(subresource_contents, resource.draft());
                queue.push_back((final_base.clone(), subresource.clone(), false));
                if is_top_level && final_base != original_base {
                    queue.push_back((original_base.clone(), subresource, false));
                }
            }
        }
    }
}
