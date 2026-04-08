use std::{
    fmt,
    sync::{Arc, LazyLock},
};

use ahash::AHashMap;
use fluent_uri::Uri;
use serde_json::Value;

use crate::{
    cache::{SharedUriCache, UriCache},
    meta, uri,
    vocabularies::{self, VocabularySet},
    Anchor, DefaultRetriever, Draft, Error, Resolver, ResourceRef, Retrieve,
};

mod build;
use build::{KnownResources, ResourceStore, StoredResource};

mod index;
use index::{Index, IndexedAnchor, IndexedResource};

mod input;
#[cfg(feature = "retrieve-async")]
pub(crate) use input::IntoAsyncRetriever;
pub use input::IntoRegistryResource;
pub(crate) use input::{IntoRetriever, PendingResource};

/// Pre-loaded registry containing all JSON Schema meta-schemas and their vocabularies
pub static SPECIFICATIONS: LazyLock<Registry<'static>> =
    LazyLock::new(|| Registry::from_meta_schemas(meta::META_SCHEMAS_ALL.as_slice()));

#[derive(Clone)]
pub struct RegistryBuilder<'a> {
    baseline: Option<&'a Registry<'a>>,
    pending: AHashMap<Uri<String>, PendingResource<'a>>,
    retriever: Arc<dyn Retrieve>,
    #[cfg(feature = "retrieve-async")]
    async_retriever: Option<Arc<dyn crate::AsyncRetrieve>>,
    draft: Option<Draft>,
}

impl fmt::Debug for RegistryBuilder<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegistryBuilder")
            .field("has_baseline", &self.baseline.is_some())
            .field("pending_len", &self.pending.len())
            .field("draft", &self.draft)
            .finish()
    }
}

/// A registry of JSON Schema resources, each identified by their canonical URIs.
///
/// `Registry` is a prepared registry: add resources with [`Registry::new`] and
/// [`RegistryBuilder::add`], then call [`RegistryBuilder::prepare`] to build the
/// reusable registry. To resolve `$ref` references directly, create a [`Resolver`]
/// from the prepared registry:
///
/// ```rust
/// use referencing::Registry;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let schema = serde_json::json!({
///     "$schema": "https://json-schema.org/draft/2020-12/schema",
///     "$id": "https://example.com/root",
///     "$defs": { "item": { "type": "string" } },
///     "items": { "$ref": "#/$defs/item" }
/// });
///
/// let registry = Registry::new()
///     .add("https://example.com/root", schema)?
///     .prepare()?;
///
/// let resolver = registry.resolver(referencing::uri::from_str("https://example.com/root")?);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Registry<'a> {
    baseline: Option<&'a Registry<'a>>,
    resolution_cache: SharedUriCache,
    known_resources: KnownResources,
    index: Index<'a>,
}

impl<'a> RegistryBuilder<'a> {
    fn new() -> Self {
        Self {
            baseline: None,
            pending: AHashMap::new(),
            retriever: Arc::new(DefaultRetriever),
            #[cfg(feature = "retrieve-async")]
            async_retriever: None,
            draft: None,
        }
    }

    fn from_registry(registry: &'a Registry<'a>) -> Self {
        Self {
            baseline: Some(registry),
            pending: AHashMap::new(),
            retriever: Arc::new(DefaultRetriever),
            #[cfg(feature = "retrieve-async")]
            async_retriever: None,
            draft: None,
        }
    }

    #[must_use]
    pub fn draft(mut self, draft: Draft) -> Self {
        self.draft = Some(draft);
        self
    }

    #[must_use]
    pub fn retriever(mut self, retriever: impl IntoRetriever) -> Self {
        self.retriever = retriever.into_retriever();
        self
    }

    #[cfg(feature = "retrieve-async")]
    #[must_use]
    pub fn async_retriever(mut self, retriever: impl IntoAsyncRetriever) -> Self {
        self.async_retriever = Some(retriever.into_retriever());
        self
    }

    /// Add a resource to the registry builder.
    ///
    /// # Errors
    ///
    /// Returns an error if the URI is invalid.
    pub fn add<'b>(
        self,
        uri: impl AsRef<str>,
        resource: impl IntoRegistryResource<'b>,
    ) -> Result<RegistryBuilder<'b>, Error>
    where
        'a: 'b,
    {
        let parsed = uri::from_str(uri.as_ref().trim_end_matches('#'))?;
        let mut pending: AHashMap<Uri<String>, PendingResource<'b>> =
            self.pending.into_iter().collect();
        input::private::Sealed::insert_into(resource, &mut pending, parsed);
        Ok(RegistryBuilder {
            baseline: self.baseline,
            pending,
            retriever: self.retriever,
            #[cfg(feature = "retrieve-async")]
            async_retriever: self.async_retriever,
            draft: self.draft,
        })
    }

    /// Add multiple resources to the registry builder.
    ///
    /// # Errors
    ///
    /// Returns an error if any URI is invalid.
    pub fn extend<'b, I, U, T>(self, pairs: I) -> Result<RegistryBuilder<'b>, Error>
    where
        'a: 'b,
        I: IntoIterator<Item = (U, T)>,
        U: AsRef<str>,
        T: IntoRegistryResource<'b>,
    {
        let mut builder = RegistryBuilder {
            baseline: self.baseline,
            pending: self.pending.into_iter().collect(),
            retriever: self.retriever,
            #[cfg(feature = "retrieve-async")]
            async_retriever: self.async_retriever,
            draft: self.draft,
        };
        for (uri, resource) in pairs {
            builder = builder.add(uri, resource)?;
        }
        Ok(builder)
    }

    /// Prepare the registry for reuse.
    ///
    /// # Errors
    ///
    /// Returns an error if URI processing, retrieval, or custom meta-schema validation fails.
    pub fn prepare(self) -> Result<Registry<'a>, Error> {
        // When extending an existing registry, seed known resources from the baseline so the
        // retriever skips URIs already owned by the parent.
        let mut known_resources = self
            .baseline
            .map(|b| b.known_resources.clone())
            .unwrap_or_default();
        let mut documents = ResourceStore::new();
        let mut resolution_cache = UriCache::new();
        let (custom_metaschemas, index_data) = build::index_resources(
            self.pending,
            &*self.retriever,
            &mut documents,
            &mut known_resources,
            &mut resolution_cache,
            self.draft,
        )?;
        build::validate_custom_metaschemas(&custom_metaschemas, &known_resources)?;
        Ok(Registry {
            baseline: self.baseline,
            resolution_cache: resolution_cache.into_shared(),
            known_resources,
            index: index_data,
        })
    }

    #[cfg(feature = "retrieve-async")]
    /// Prepare the registry for reuse with async retrieval.
    ///
    /// # Errors
    ///
    /// Returns an error if URI processing, retrieval, or custom meta-schema validation fails.
    pub async fn async_prepare(self) -> Result<Registry<'a>, Error> {
        let retriever = self
            .async_retriever
            .unwrap_or_else(|| Arc::new(DefaultRetriever));
        let mut known_resources = self
            .baseline
            .map(|b| b.known_resources.clone())
            .unwrap_or_default();
        let mut documents = ResourceStore::new();
        let mut resolution_cache = UriCache::new();
        let (custom_metaschemas, index_data) = build::index_resources_async(
            self.pending,
            &*retriever,
            &mut documents,
            &mut known_resources,
            &mut resolution_cache,
            self.draft,
        )
        .await?;
        build::validate_custom_metaschemas(&custom_metaschemas, &known_resources)?;
        Ok(Registry {
            baseline: self.baseline,
            resolution_cache: resolution_cache.into_shared(),
            known_resources,
            index: index_data,
        })
    }
}

impl<'a> Registry<'a> {
    #[allow(clippy::new_ret_no_self)]
    #[must_use]
    pub fn new<'b>() -> RegistryBuilder<'b> {
        RegistryBuilder::new()
    }
    /// Add a resource to a prepared registry, returning a builder that must be prepared again.
    ///
    /// # Errors
    ///
    /// Returns an error if the URI is invalid.
    pub fn add<'b>(
        &'b self,
        uri: impl AsRef<str>,
        resource: impl IntoRegistryResource<'b>,
    ) -> Result<RegistryBuilder<'b>, Error>
    where
        'a: 'b,
    {
        RegistryBuilder::from_registry(self).add(uri, resource)
    }

    /// Add multiple resources to a prepared registry, returning a builder that
    /// must be prepared again.
    ///
    /// # Errors
    ///
    /// Returns an error if any URI is invalid.
    pub fn extend<'b, I, U, T>(&'b self, pairs: I) -> Result<RegistryBuilder<'b>, Error>
    where
        'a: 'b,
        I: IntoIterator<Item = (U, T)>,
        U: AsRef<str>,
        T: IntoRegistryResource<'b>,
    {
        RegistryBuilder::from_registry(self).extend(pairs)
    }

    /// Build a registry with all the given meta-schemas from specs.
    pub(crate) fn from_meta_schemas(schemas: &[(&'static str, &'static Value)]) -> Self {
        let mut documents = ResourceStore::with_capacity(schemas.len());
        let mut known_resources = KnownResources::with_capacity(schemas.len());

        for (uri, schema) in schemas {
            let parsed =
                uri::from_str(uri.trim_end_matches('#')).expect("meta-schema URI must be valid");
            let key = Arc::new(parsed);
            let draft = Draft::default().detect(schema);
            known_resources.insert((*key).clone());
            documents.insert(key, Arc::new(StoredResource::borrowed(schema, draft)));
        }

        let mut resolution_cache = UriCache::with_capacity(35);
        let index_data = build::build_index_from_stored(&documents, &mut resolution_cache)
            .expect("meta-schema index data must build");

        Self {
            baseline: None,
            resolution_cache: resolution_cache.into_shared(),
            known_resources,
            index: index_data,
        }
    }
    /// Returns `true` if the registry contains a resource at the given URI.
    ///
    /// Returns `false` if the URI is malformed.
    #[must_use]
    pub fn contains_resource(&self, uri: &str) -> bool {
        let Ok(uri) = uri::from_str(uri) else {
            return false;
        };
        self.resource_by_uri(&uri).is_some()
    }

    /// Creates a [`Resolver`] rooted at `base_uri`.
    ///
    /// The returned resolver borrows from this registry and cannot outlive it.
    #[must_use]
    pub fn resolver(&self, base_uri: Uri<String>) -> Resolver<'_> {
        Resolver::new(self, Arc::new(base_uri))
    }

    /// Returns the vocabulary set active for a schema with the given `contents`.
    ///
    /// Detects the draft from the `$schema` field. If no draft is detected or
    /// the draft has no registered vocabularies, returns the default vocabulary
    /// set — never errors.
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
                        if let Some(resource) = self.resource_by_uri(&uri) {
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

    /// Resolves `uri` against `base` and returns the resulting absolute URI.
    ///
    /// Results are cached. Returns an error if `base` has no scheme or if
    /// resolution fails.
    ///
    /// # Errors
    ///
    /// Returns an error if base has no schema or there is a fragment.
    pub fn resolve_uri(&self, base: &Uri<&str>, uri: &str) -> Result<Arc<Uri<String>>, Error> {
        self.resolution_cache.resolve_against(base, uri)
    }

    #[inline]
    pub(crate) fn resource_by_uri(&self, uri: &Uri<String>) -> Option<ResourceRef<'_>> {
        self.index
            .resources
            .get(uri)
            .and_then(IndexedResource::resolve)
            .or_else(|| {
                self.baseline
                    .and_then(|baseline| baseline.resource_by_uri(uri))
            })
    }

    pub(crate) fn anchor(&self, uri: &Uri<String>, name: &str) -> Result<Anchor<'_>, Error> {
        if let Some(anchor) = self.anchor_exact(uri, name) {
            return Ok(anchor);
        }

        if let Some(resource) = self.resource_by_uri(uri) {
            if let Some(id) = resource.id() {
                let canonical = uri::from_str(id)?;
                if let Some(anchor) = self.anchor_exact(&canonical, name) {
                    return Ok(anchor);
                }
            }
        }

        if name.contains('/') {
            Err(Error::invalid_anchor(name.to_string()))
        } else {
            Err(Error::no_such_anchor(name.to_string()))
        }
    }

    fn local_anchor_by_uri(&self, uri: &Uri<String>, name: &str) -> Option<Anchor<'_>> {
        self.index
            .anchors
            .get(uri)
            .and_then(|entries| entries.get(name))
            .and_then(IndexedAnchor::resolve)
    }

    fn anchor_exact(&self, uri: &Uri<String>, name: &str) -> Option<Anchor<'_>> {
        self.local_anchor_by_uri(uri, name).or_else(|| {
            self.baseline
                .and_then(|baseline| baseline.anchor_exact(uri, name))
        })
    }
}
