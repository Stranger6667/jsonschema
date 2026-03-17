use std::{
    borrow::Cow,
    collections::VecDeque,
    fmt,
    num::NonZeroUsize,
    sync::{Arc, LazyLock},
};

use ahash::{AHashMap, AHashSet};
use fluent_uri::{pct_enc::EStr, Uri};
use serde_json::Value;

use crate::{
    cache::{SharedUriCache, UriCache},
    meta::{self, metas_for_draft},
    resource::unescape_segment,
    small_map::SmallMap,
    uri,
    vocabularies::{self, VocabularySet},
    Anchor, DefaultRetriever, Draft, Error, JsonPointerNode, JsonPointerSegment, Resolver,
    Resource, ResourceRef, Retrieve,
};

#[derive(Debug)]
struct StoredDocument<'a> {
    value: Cow<'a, Value>,
    draft: Draft,
}

impl<'a> StoredDocument<'a> {
    #[inline]
    fn owned(value: Value, draft: Draft) -> Self {
        Self {
            value: Cow::Owned(value),
            draft,
        }
    }

    #[inline]
    fn borrowed(value: &'a Value, draft: Draft) -> Self {
        Self {
            value: Cow::Borrowed(value),
            draft,
        }
    }

    #[inline]
    fn contents(&self) -> &Value {
        &self.value
    }

    #[inline]
    fn borrowed_contents(&self) -> Option<&'a Value> {
        match &self.value {
            Cow::Borrowed(value) => Some(value),
            Cow::Owned(_) => None,
        }
    }

    #[inline]
    fn draft(&self) -> Draft {
        self.draft
    }
}

type DocumentStore<'a> = AHashMap<Arc<Uri<String>>, Arc<StoredDocument<'a>>>;
type AnchorKey = Box<str>;

#[derive(Debug, Clone, Default)]
struct PreparedIndex<'a> {
    resources: SmallMap<Arc<Uri<String>>, IndexedResource<'a>>,
    anchors: SmallMap<Arc<Uri<String>>, SmallMap<AnchorKey, IndexedAnchor<'a>>>,
}

#[derive(Debug, Clone)]
enum IndexedResource<'a> {
    Borrowed(ResourceRef<'a>),
    Owned {
        document: Arc<StoredDocument<'a>>,
        pointer: ParsedPointer,
        draft: Draft,
    },
}

impl IndexedResource<'_> {
    #[inline]
    fn resolve(&self) -> Option<ResourceRef<'_>> {
        match self {
            IndexedResource::Borrowed(resource) => {
                Some(ResourceRef::new(resource.contents(), resource.draft()))
            }
            IndexedResource::Owned {
                document,
                pointer,
                draft,
            } => {
                let contents = pointer.lookup(document.contents())?;
                Some(ResourceRef::new(contents, *draft))
            }
        }
    }
}

type BorrowedAnchor<'a> = Anchor<'a>;

#[derive(Debug, Clone)]
enum IndexedAnchor<'a> {
    Borrowed(BorrowedAnchor<'a>),
    Owned {
        document: Arc<StoredDocument<'a>>,
        pointer: ParsedPointer,
        draft: Draft,
        kind: IndexedAnchorKind,
        name: Box<str>,
    },
}

impl IndexedAnchor<'_> {
    #[inline]
    fn resolve(&self) -> Option<Anchor<'_>> {
        match self {
            IndexedAnchor::Borrowed(anchor) => Some(match anchor {
                Anchor::Default { name, resource } => Anchor::Default {
                    name,
                    resource: ResourceRef::new(resource.contents(), resource.draft()),
                },
                Anchor::Dynamic { name, resource } => Anchor::Dynamic {
                    name,
                    resource: ResourceRef::new(resource.contents(), resource.draft()),
                },
            }),
            IndexedAnchor::Owned {
                document,
                pointer,
                draft,
                kind,
                name,
            } => {
                let contents = pointer.lookup(document.contents())?;
                let resource = ResourceRef::new(contents, *draft);
                Some(match kind {
                    IndexedAnchorKind::Default => Anchor::Default { name, resource },
                    IndexedAnchorKind::Dynamic => Anchor::Dynamic { name, resource },
                })
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndexedAnchorKind {
    Default,
    Dynamic,
}

#[derive(Debug, Clone, Default)]
struct ParsedPointer {
    segments: Vec<ParsedPointerSegment>,
}

impl ParsedPointer {
    fn from_json_pointer(pointer: &str) -> Option<Self> {
        if pointer.is_empty() {
            return Some(Self::default());
        }
        if !pointer.starts_with('/') {
            return None;
        }

        let mut segments = Vec::new();
        for token in pointer.split('/').skip(1).map(unescape_segment) {
            if let Some(index) = parse_index(&token) {
                segments.push(ParsedPointerSegment::Index(index));
            } else {
                segments.push(ParsedPointerSegment::Key(
                    token.into_owned().into_boxed_str(),
                ));
            }
        }
        Some(Self { segments })
    }

    fn from_pointer_node(path: &JsonPointerNode<'_, '_>) -> Self {
        let mut segments = Vec::new();
        let mut head = path;

        while let Some(parent) = head.parent() {
            segments.push(match head.segment() {
                JsonPointerSegment::Key(key) => ParsedPointerSegment::Key(key.as_ref().into()),
                JsonPointerSegment::Index(idx) => ParsedPointerSegment::Index(*idx),
            });
            head = parent;
        }

        segments.reverse();
        Self { segments }
    }

    fn lookup<'a>(&self, document: &'a Value) -> Option<&'a Value> {
        self.segments
            .iter()
            .try_fold(document, |target, token| match token {
                ParsedPointerSegment::Key(key) => match target {
                    Value::Object(map) => map.get(&**key),
                    _ => None,
                },
                ParsedPointerSegment::Index(index) => match target {
                    Value::Array(list) => list.get(*index),
                    _ => None,
                },
            })
    }
}

#[derive(Debug, Clone)]
enum ParsedPointerSegment {
    Key(Box<str>),
    Index(usize),
}

/// Pre-loaded registry containing all JSON Schema meta-schemas and their vocabularies
pub static SPECIFICATIONS: LazyLock<Registry<'static>> =
    LazyLock::new(|| Registry::build_from_meta_schemas(meta::META_SCHEMAS_ALL.as_slice()));

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
    index_data: PreparedIndex<'a>,
}

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

#[derive(Clone)]
pub(crate) enum PendingResource<'a> {
    OwnedValue(Value),
    BorrowedValue(&'a Value),
    OwnedResource(Resource),
    BorrowedResource(ResourceRef<'a>),
}

pub(crate) mod private {
    use ahash::AHashMap;
    use fluent_uri::Uri;

    use super::PendingResource;

    pub(crate) trait Sealed<'a> {
        fn insert_into(
            self,
            pending: &mut AHashMap<Uri<String>, PendingResource<'a>>,
            uri: Uri<String>,
        );
    }
}

#[allow(private_bounds)]
pub trait IntoRegistryResource<'a>: private::Sealed<'a> {}

impl<'a, T> IntoRegistryResource<'a> for T where T: private::Sealed<'a> {}

impl<'a> private::Sealed<'a> for Resource {
    fn insert_into(
        self,
        pending: &mut AHashMap<Uri<String>, PendingResource<'a>>,
        uri: Uri<String>,
    ) {
        pending.insert(uri, PendingResource::OwnedResource(self));
    }
}

impl<'a> private::Sealed<'a> for &'a Resource {
    fn insert_into(
        self,
        pending: &mut AHashMap<Uri<String>, PendingResource<'a>>,
        uri: Uri<String>,
    ) {
        pending.insert(
            uri,
            PendingResource::BorrowedResource(ResourceRef::new(self.contents(), self.draft())),
        );
    }
}

impl<'a> private::Sealed<'a> for &'a Value {
    fn insert_into(
        self,
        pending: &mut AHashMap<Uri<String>, PendingResource<'a>>,
        uri: Uri<String>,
    ) {
        pending.insert(uri, PendingResource::BorrowedValue(self));
    }
}

impl<'a> private::Sealed<'a> for ResourceRef<'a> {
    fn insert_into(
        self,
        pending: &mut AHashMap<Uri<String>, PendingResource<'a>>,
        uri: Uri<String>,
    ) {
        pending.insert(uri, PendingResource::BorrowedResource(self));
    }
}

impl<'a> private::Sealed<'a> for Value {
    fn insert_into(
        self,
        pending: &mut AHashMap<Uri<String>, PendingResource<'a>>,
        uri: Uri<String>,
    ) {
        pending.insert(uri, PendingResource::OwnedValue(self));
    }
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
        private::Sealed::insert_into(resource, &mut pending, parsed);
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
        if let Some(baseline) = self.baseline {
            baseline.try_with_pending_resources_and_retriever(
                self.pending,
                &*self.retriever,
                self.draft,
            )
        } else {
            Registry::try_from_pending_resources_impl(self.pending, &*self.retriever, self.draft)
        }
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
        if let Some(baseline) = self.baseline {
            baseline
                .try_with_pending_resources_and_retriever_async(
                    self.pending,
                    &*retriever,
                    self.draft,
                )
                .await
        } else {
            Registry::try_from_pending_resources_async_impl(self.pending, &*retriever, self.draft)
                .await
        }
    }
}

impl<'a> Registry<'a> {
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
}

pub trait IntoRetriever {
    fn into_retriever(self) -> Arc<dyn Retrieve>;
}

impl<T: Retrieve + 'static> IntoRetriever for T {
    fn into_retriever(self) -> Arc<dyn Retrieve> {
        Arc::new(self)
    }
}

impl IntoRetriever for Arc<dyn Retrieve> {
    fn into_retriever(self) -> Arc<dyn Retrieve> {
        self
    }
}

#[cfg(feature = "retrieve-async")]
pub trait IntoAsyncRetriever {
    fn into_retriever(self) -> Arc<dyn crate::AsyncRetrieve>;
}

#[cfg(feature = "retrieve-async")]
impl<T: crate::AsyncRetrieve + 'static> IntoAsyncRetriever for T {
    fn into_retriever(self) -> Arc<dyn crate::AsyncRetrieve> {
        Arc::new(self)
    }
}

#[cfg(feature = "retrieve-async")]
impl IntoAsyncRetriever for Arc<dyn crate::AsyncRetrieve> {
    fn into_retriever(self) -> Arc<dyn crate::AsyncRetrieve> {
        self
    }
}

impl Registry<'static> {
    #[allow(clippy::new_ret_no_self)]
    #[must_use]
    pub fn new<'a>() -> RegistryBuilder<'a> {
        RegistryBuilder::new()
    }

    fn try_from_pending_resources_impl<'a>(
        pairs: impl IntoIterator<Item = (Uri<String>, PendingResource<'a>)>,
        retriever: &dyn Retrieve,
        draft: Option<Draft>,
    ) -> Result<Registry<'a>, Error> {
        let mut documents = DocumentStore::new();
        let mut known_resources = KnownResources::new();
        let mut resolution_cache = UriCache::new();

        let (custom_metaschemas, index_data) = process_resources_mixed(
            pairs,
            retriever,
            &mut documents,
            &mut known_resources,
            &mut resolution_cache,
            draft,
        )?;

        validate_custom_metaschemas(&custom_metaschemas, &known_resources)?;

        Ok(Registry {
            baseline: None,
            resolution_cache: resolution_cache.into_shared(),
            known_resources,
            index_data,
        })
    }

    #[cfg(feature = "retrieve-async")]
    async fn try_from_pending_resources_async_impl<'a>(
        pairs: impl IntoIterator<Item = (Uri<String>, PendingResource<'a>)>,
        retriever: &dyn crate::AsyncRetrieve,
        draft: Option<Draft>,
    ) -> Result<Registry<'a>, Error> {
        let mut documents = DocumentStore::new();
        let mut known_resources = KnownResources::new();
        let mut resolution_cache = UriCache::new();

        let (custom_metaschemas, index_data) = process_resources_async_mixed(
            pairs,
            retriever,
            &mut documents,
            &mut known_resources,
            &mut resolution_cache,
            draft,
        )
        .await?;

        validate_custom_metaschemas(&custom_metaschemas, &known_resources)?;

        Ok(Registry {
            baseline: None,
            resolution_cache: resolution_cache.into_shared(),
            known_resources,
            index_data,
        })
    }

    /// Build a registry with all the given meta-schemas from specs.
    pub(crate) fn build_from_meta_schemas(schemas: &[(&'static str, &'static Value)]) -> Self {
        let mut documents = DocumentStore::with_capacity(schemas.len());
        let mut known_resources = KnownResources::with_capacity(schemas.len());

        for (uri, schema) in schemas {
            let parsed =
                uri::from_str(uri.trim_end_matches('#')).expect("meta-schema URI must be valid");
            let key = Arc::new(parsed);
            let draft = Draft::default().detect(schema);
            known_resources.insert((*key).clone());
            documents.insert(key, Arc::new(StoredDocument::borrowed(schema, draft)));
        }

        let mut resolution_cache = UriCache::with_capacity(35);
        let index_data = build_prepared_index_for_documents(&documents, &mut resolution_cache)
            .expect("meta-schema index data must build");

        Self {
            baseline: None,
            resolution_cache: resolution_cache.into_shared(),
            known_resources,
            index_data,
        }
    }
}

impl<'a> Registry<'a> {
    fn try_with_pending_resources_and_retriever(
        &'a self,
        pairs: impl IntoIterator<Item = (Uri<String>, PendingResource<'a>)>,
        retriever: &dyn Retrieve,
        draft: Option<Draft>,
    ) -> Result<Registry<'a>, Error> {
        let mut documents = DocumentStore::new();
        let mut resolution_cache = UriCache::new();
        let mut known_resources = self.known_resources.clone();

        let (custom_metaschemas, index_data) = process_resources_mixed(
            pairs,
            retriever,
            &mut documents,
            &mut known_resources,
            &mut resolution_cache,
            draft,
        )?;
        validate_custom_metaschemas(&custom_metaschemas, &known_resources)?;

        Ok(Registry {
            baseline: Some(self),
            resolution_cache: resolution_cache.into_shared(),
            known_resources,
            index_data,
        })
    }

    #[cfg(feature = "retrieve-async")]
    async fn try_with_pending_resources_and_retriever_async(
        &'a self,
        pairs: impl IntoIterator<Item = (Uri<String>, PendingResource<'a>)>,
        retriever: &dyn crate::AsyncRetrieve,
        draft: Option<Draft>,
    ) -> Result<Registry<'a>, Error> {
        let mut documents = DocumentStore::new();
        let mut resolution_cache = UriCache::new();
        let mut known_resources = self.known_resources.clone();

        let (custom_metaschemas, index_data) = process_resources_async_mixed(
            pairs,
            retriever,
            &mut documents,
            &mut known_resources,
            &mut resolution_cache,
            draft,
        )
        .await?;
        validate_custom_metaschemas(&custom_metaschemas, &known_resources)?;

        Ok(Registry {
            baseline: Some(self),
            resolution_cache: resolution_cache.into_shared(),
            known_resources,
            index_data,
        })
    }

    /// Resolves a reference URI against a base URI using registry's cache.
    ///
    /// # Errors
    ///
    /// Returns an error if base has not schema or there is a fragment.
    pub fn resolve_against(&self, base: &Uri<&str>, uri: &str) -> Result<Arc<Uri<String>>, Error> {
        self.resolution_cache.resolve_against(base, uri)
    }

    #[must_use]
    pub fn contains_resource_uri(&self, uri: &str) -> bool {
        let Ok(uri) = uri::from_str(uri) else {
            return false;
        };
        self.resource_by_uri(&uri).is_some()
    }

    #[must_use]
    pub fn contains_anchor(&self, uri: &str, name: &str) -> bool {
        let Ok(uri) = uri::from_str(uri) else {
            return false;
        };
        self.contains_anchor_uri(&uri, name)
    }

    #[must_use]
    pub fn resolver(&self, base_uri: Uri<String>) -> Resolver<'_> {
        Resolver::new(self, Arc::new(base_uri))
    }

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

    #[inline]
    pub(crate) fn resource_by_uri(&self, uri: &Uri<String>) -> Option<ResourceRef<'_>> {
        self.index_data
            .resources
            .get(uri)
            .and_then(IndexedResource::resolve)
            .or_else(|| {
                self.baseline
                    .and_then(|baseline| baseline.resource_by_uri(uri))
            })
    }

    pub(crate) fn contains_anchor_uri(&self, uri: &Uri<String>, name: &str) -> bool {
        self.index_data
            .anchors
            .get(uri)
            .is_some_and(|entries| entries.contains_key(name))
            || self
                .baseline
                .is_some_and(|baseline| baseline.contains_anchor_uri(uri, name))
    }

    fn local_anchor_by_uri(&self, uri: &Uri<String>, name: &str) -> Option<Anchor<'_>> {
        self.index_data
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
}

/// Build prepared local index data for all documents already in `documents`.
/// Used by `build_from_meta_schemas` for the static SPECIFICATIONS registry.
fn build_prepared_index_for_documents<'a>(
    documents: &DocumentStore<'a>,
    resolution_cache: &mut UriCache,
) -> Result<PreparedIndex<'a>, Error> {
    let mut state = ProcessingState::new();
    let mut known_resources = KnownResources::default();

    for (doc_uri, document) in documents {
        known_resources.insert((**doc_uri).clone());
        insert_root_index_entries(&mut state.index_data, doc_uri, document);
    }

    for (doc_uri, document) in documents {
        if document.borrowed_contents().is_some() {
            let mut local_seen = LocalSeen::new();
            process_borrowed_document(
                Arc::clone(doc_uri),
                doc_uri,
                document,
                "",
                document.draft(),
                &mut state,
                &mut known_resources,
                resolution_cache,
                &mut local_seen,
            )?;
        } else {
            let mut local_seen = LocalSeen::new();
            process_owned_document(
                Arc::clone(doc_uri),
                doc_uri,
                document,
                "",
                document.draft(),
                &mut state,
                &mut known_resources,
                resolution_cache,
                &mut local_seen,
            )?;
        }
    }
    Ok(state.index_data)
}

type KnownResources = AHashSet<Uri<String>>;

#[derive(Hash, Eq, PartialEq)]
struct ReferenceKey {
    base_ptr: NonZeroUsize,
    reference: String,
}

impl ReferenceKey {
    fn new(base: &Arc<Uri<String>>, reference: &str) -> Self {
        Self {
            base_ptr: NonZeroUsize::new(Arc::as_ptr(base) as usize)
                .expect("Arc pointer should never be null"),
            reference: reference.to_owned(),
        }
    }
}

type ReferenceTracker = AHashSet<ReferenceKey>;

/// Allocation-free local-ref deduplication: stores (`base_arc_ptr`, &`str_borrowed_from_json`).
type LocalSeen<'a> = AHashSet<(NonZeroUsize, &'a str)>;

/// Clears a [`LocalSeen`] set and reinterprets it with a different borrow lifetime,
/// reusing the backing heap allocation across processing phases.
///
/// # Safety
/// - The set is cleared before the lifetime change, so no `'a` references remain live.
/// - `(NonZeroUsize, &'a str)` and `(NonZeroUsize, &'b str)` have identical memory layouts
///   for any two lifetimes (`&str` is a fat pointer whose size/alignment are lifetime-independent).
/// - After `clear()` the heap allocation holds no initialized `T` values, so no pointer in
///   the allocation is ever read through the wrong lifetime.
/// - Verified under MIRI (tree borrows): no undefined behaviour detected.
#[allow(unsafe_code)]
#[inline]
unsafe fn reuse_local_seen<'b>(mut s: LocalSeen<'_>) -> LocalSeen<'b> {
    s.clear();
    // SAFETY: see above — layouts identical, no live 'a refs after clear()
    std::mem::transmute(s)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ReferenceKind {
    Ref,
    Schema,
}

/// An entry in the processing queue.
/// `(base_uri, document_root_uri, pointer, draft)`
///
/// `pointer` is a JSON Pointer relative to the document root (`""` means root).
/// Local `$ref`s are always resolved against the document root.
type QueueEntry = (Arc<Uri<String>>, Arc<Uri<String>>, String, Draft);

/// A deferred local `$ref` target.
///
/// Like [`QueueEntry`] but carries the pre-resolved value address (`value_addr`) obtained
/// for free during the `pointer()` call at push time. Used in [`process_deferred_refs`] to
/// skip already-visited targets without a second `pointer()` traversal.
///
/// `(base_uri, document_root_uri, pointer, draft, value_addr)`
type DeferredRef = (Arc<Uri<String>>, Arc<Uri<String>>, String, Draft, usize);

fn insert_borrowed_anchor_entries<'a>(
    index_data: &mut PreparedIndex<'a>,
    uri: &Arc<Uri<String>>,
    draft: Draft,
    contents: &'a Value,
) {
    let anchors = index_data.anchors.get_or_insert_default(Arc::clone(uri));
    for anchor in draft.anchors(contents) {
        anchors.insert(
            anchor.name().to_string().into_boxed_str(),
            IndexedAnchor::Borrowed(anchor),
        );
    }
}

fn insert_owned_anchor_entries<'a>(
    index_data: &mut PreparedIndex<'a>,
    uri: &Arc<Uri<String>>,
    document: &Arc<StoredDocument<'a>>,
    pointer: &ParsedPointer,
    draft: Draft,
    contents: &Value,
) {
    let anchors = index_data.anchors.get_or_insert_default(Arc::clone(uri));
    for anchor in draft.anchors(contents) {
        let (name, kind) = match anchor {
            Anchor::Default { name, .. } => (name, IndexedAnchorKind::Default),
            Anchor::Dynamic { name, .. } => (name, IndexedAnchorKind::Dynamic),
        };
        anchors.insert(
            name.to_string().into_boxed_str(),
            IndexedAnchor::Owned {
                document: Arc::clone(document),
                pointer: pointer.clone(),
                draft,
                kind,
                name: name.to_string().into_boxed_str(),
            },
        );
    }
}

fn insert_root_index_entries<'a>(
    index_data: &mut PreparedIndex<'a>,
    doc_key: &Arc<Uri<String>>,
    document: &Arc<StoredDocument<'a>>,
) {
    if let Some(contents) = document.borrowed_contents() {
        index_data.resources.insert(
            Arc::clone(doc_key),
            IndexedResource::Borrowed(ResourceRef::new(contents, document.draft())),
        );
        insert_borrowed_anchor_entries(index_data, doc_key, document.draft(), contents);
    } else {
        let pointer = ParsedPointer::default();
        index_data.resources.insert(
            Arc::clone(doc_key),
            IndexedResource::Owned {
                document: Arc::clone(document),
                pointer: pointer.clone(),
                draft: document.draft(),
            },
        );
        insert_owned_anchor_entries(
            index_data,
            doc_key,
            document,
            &pointer,
            document.draft(),
            document.contents(),
        );
    }
}

fn insert_borrowed_discovered_index_entries<'a>(
    index_data: &mut PreparedIndex<'a>,
    uri: &Arc<Uri<String>>,
    draft: Draft,
    has_id: bool,
    contents: &'a Value,
) {
    if has_id {
        index_data.resources.insert(
            Arc::clone(uri),
            IndexedResource::Borrowed(ResourceRef::new(contents, draft)),
        );
    }
    insert_borrowed_anchor_entries(index_data, uri, draft, contents);
}

fn insert_owned_discovered_index_entries<'a>(
    index_data: &mut PreparedIndex<'a>,
    uri: &Arc<Uri<String>>,
    document: &Arc<StoredDocument<'a>>,
    pointer: &ParsedPointer,
    draft: Draft,
    has_id: bool,
    contents: &Value,
) {
    if has_id {
        index_data.resources.insert(
            Arc::clone(uri),
            IndexedResource::Owned {
                document: Arc::clone(document),
                pointer: pointer.clone(),
                draft,
            },
        );
    }
    insert_owned_anchor_entries(index_data, uri, document, pointer, draft, contents);
}

struct ProcessingState<'a> {
    queue: VecDeque<QueueEntry>,
    seen: ReferenceTracker,
    // The String is the original reference text (e.g. "./foo.json"), kept solely for
    // `json-schema://`-scheme error messages where the resolved URI is not user-friendly.
    external: AHashSet<(String, Uri<String>, ReferenceKind)>,
    scratch: String,
    refers_metaschemas: bool,
    custom_metaschemas: Vec<String>,
    /// Tracks schema pointers we've visited during recursive external resource collection.
    /// This prevents infinite recursion when schemas reference each other.
    visited_schemas: AHashSet<usize>,
    /// Deferred local-ref targets. During the main traversal, instead of calling
    /// `collect_external_resources_recursive` immediately when a local `$ref` is found,
    /// the target is pushed here. After `process_queue` completes (full document traversal),
    /// subresource targets are already in `visited_schemas` and skipped in O(1) via the
    /// pre-stored value address; non-subresource paths (e.g. `#/components/schemas/Foo`)
    /// are still fully traversed.
    deferred_refs: Vec<DeferredRef>,
    borrowed_reference_scratch: crate::specification::BorrowedReferenceSlots<'a>,
    borrowed_child_scratch: Vec<(&'a Value, Draft)>,
    index_data: PreparedIndex<'a>,
}

impl ProcessingState<'_> {
    fn new() -> Self {
        Self {
            queue: VecDeque::with_capacity(32),
            seen: ReferenceTracker::new(),
            external: AHashSet::new(),
            scratch: String::new(),
            refers_metaschemas: false,
            custom_metaschemas: Vec::new(),
            visited_schemas: AHashSet::new(),
            deferred_refs: Vec::new(),
            borrowed_reference_scratch: crate::specification::BorrowedReferenceSlots::default(),
            borrowed_child_scratch: Vec::new(),
            index_data: PreparedIndex::default(),
        }
    }
}

fn process_input_resources_mixed<'a>(
    pairs: impl IntoIterator<Item = (Uri<String>, PendingResource<'a>)>,
    documents: &mut DocumentStore<'a>,
    known_resources: &mut KnownResources,
    state: &mut ProcessingState<'a>,
    draft_override: Option<Draft>,
) {
    for (uri, resource) in pairs {
        let key = Arc::new(uri);
        let draft = match &resource {
            PendingResource::OwnedValue(value) => {
                draft_override.unwrap_or_else(|| Draft::default().detect(value))
            }
            PendingResource::BorrowedValue(value) => {
                draft_override.unwrap_or_else(|| Draft::default().detect(value))
            }
            PendingResource::OwnedResource(resource) => resource.draft(),
            PendingResource::BorrowedResource(resource) => resource.draft(),
        };

        let r = Arc::new(match resource {
            PendingResource::OwnedValue(value) => {
                let (draft, contents) = draft.create_resource(value).into_inner();
                StoredDocument::owned(contents, draft)
            }
            PendingResource::BorrowedValue(value) => {
                let resource = draft.create_resource_ref(value);
                StoredDocument::borrowed(resource.contents(), resource.draft())
            }
            PendingResource::OwnedResource(resource) => {
                let (draft, contents) = resource.into_inner();
                StoredDocument::owned(contents, draft)
            }
            PendingResource::BorrowedResource(resource) => {
                StoredDocument::borrowed(resource.contents(), resource.draft())
            }
        });

        documents.insert(Arc::clone(&key), Arc::clone(&r));
        known_resources.insert((*key).clone());
        insert_root_index_entries(&mut state.index_data, &key, &r);

        if draft == Draft::Unknown {
            let contents = documents
                .get(&key)
                .expect("document was just inserted")
                .contents();
            if let Some(meta_schema) = contents
                .as_object()
                .and_then(|obj| obj.get("$schema"))
                .and_then(|schema| schema.as_str())
            {
                state.custom_metaschemas.push(meta_schema.to_string());
            }
        }

        state
            .queue
            .push_back((Arc::clone(&key), key, String::new(), draft));
    }
}

fn process_queue<'r>(
    state: &mut ProcessingState<'r>,
    documents: &DocumentStore<'r>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
) -> Result<(), Error> {
    while let Some((base, document_root_uri, pointer_path, draft)) = state.queue.pop_front() {
        let Some(document) = documents.get(&document_root_uri) else {
            continue;
        };
        if document.borrowed_contents().is_some() {
            let mut document_local_seen = LocalSeen::new();
            process_borrowed_document(
                base,
                &document_root_uri,
                document,
                &pointer_path,
                draft,
                state,
                known_resources,
                resolution_cache,
                &mut document_local_seen,
            )?;
            continue;
        }
        let mut document_local_seen = LocalSeen::new();
        process_owned_document(
            base,
            &document_root_uri,
            document,
            &pointer_path,
            draft,
            state,
            known_resources,
            resolution_cache,
            &mut document_local_seen,
        )?;
    }
    Ok(())
}

fn process_borrowed_document<'r>(
    current_base_uri: Arc<Uri<String>>,
    document_root_uri: &Arc<Uri<String>>,
    document: &Arc<StoredDocument<'r>>,
    pointer_path: &str,
    draft: Draft,
    state: &mut ProcessingState<'r>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    local_seen: &mut LocalSeen<'r>,
) -> Result<(), Error> {
    let Some(document_root) = document.borrowed_contents() else {
        return Ok(());
    };
    let Some(subschema) = (if pointer_path.is_empty() {
        Some(document_root)
    } else {
        pointer(document_root, pointer_path)
    }) else {
        return Ok(());
    };

    explore_borrowed_subtree(
        current_base_uri,
        document_root,
        subschema,
        draft,
        pointer_path.is_empty(),
        document_root_uri,
        state,
        known_resources,
        resolution_cache,
        local_seen,
    )
}

fn explore_borrowed_subtree<'r>(
    mut current_base_uri: Arc<Uri<String>>,
    document_root: &'r Value,
    subschema: &'r Value,
    draft: Draft,
    is_root_entry: bool,
    document_root_uri: &Arc<Uri<String>>,
    state: &mut ProcessingState<'r>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    local_seen: &mut LocalSeen<'r>,
) -> Result<(), Error> {
    let object = subschema.as_object();
    #[cfg(feature = "perf-observe-registry")]
    if let Some(object) = object {
        crate::observe_registry!("registry.borrowed.object_len={}", object.len());
    }
    let probe = object.map(|schema| draft.probe_borrowed_object_map(schema));
    if let Some(probe) = probe.as_ref() {
        #[cfg(feature = "perf-observe-registry")]
        {
            let id_scan = match (probe.id.is_some(), probe.has_anchor) {
                (false, false) => "none",
                (true, false) => "id_only",
                (false, true) => "anchor_only",
                (true, true) => "id_and_anchor",
            };
            crate::observe_registry!("registry.id_scan={id_scan}");
        }
        if let Some(id) = probe.id {
            let original_base_uri = Arc::clone(&current_base_uri);
            current_base_uri = resolve_id(&current_base_uri, id, resolution_cache)?;
            known_resources.insert((*current_base_uri).clone());
            let insert_resource = current_base_uri != original_base_uri;
            if !(is_root_entry && current_base_uri == *document_root_uri) {
                insert_borrowed_discovered_index_entries(
                    &mut state.index_data,
                    &current_base_uri,
                    draft,
                    insert_resource,
                    subschema,
                );
            }
        } else if probe.has_anchor && !is_root_entry {
            insert_borrowed_discovered_index_entries(
                &mut state.index_data,
                &current_base_uri,
                draft,
                false,
                subschema,
            );
        }
    }

    if let (Some(schema), Some(probe)) = (object, probe.as_ref()) {
        if probe.has_ref_or_schema {
            let child_start = state.borrowed_child_scratch.len();
            draft.scan_borrowed_object_into_scratch_map(
                schema,
                &mut state.borrowed_reference_scratch,
                &mut state.borrowed_child_scratch,
            );
            let child_end = state.borrowed_child_scratch.len();

            let subschema_ptr = std::ptr::from_ref::<Value>(subschema) as usize;
            if state.visited_schemas.insert(subschema_ptr) {
                for (reference, key) in [
                    (state.borrowed_reference_scratch.ref_, "$ref"),
                    (state.borrowed_reference_scratch.schema, "$schema"),
                ] {
                    let Some(reference) = reference else {
                        continue;
                    };
                    if reference.starts_with("https://json-schema.org/draft/")
                        || reference.starts_with("http://json-schema.org/draft-")
                        || current_base_uri
                            .as_str()
                            .starts_with("https://json-schema.org/draft/")
                    {
                        if key == "$ref" {
                            state.refers_metaschemas = true;
                        }
                        continue;
                    }
                    if reference == "#" {
                        continue;
                    }
                    if reference.starts_with('#') {
                        if mark_local_reference(local_seen, &current_base_uri, reference) {
                            let ptr = reference.trim_start_matches('#');
                            if let Some(referenced) = pointer(document_root, ptr) {
                                let target_draft = draft.detect(referenced);
                                let value_addr = std::ptr::from_ref::<Value>(referenced) as usize;
                                state.deferred_refs.push((
                                    Arc::clone(&current_base_uri),
                                    Arc::clone(document_root_uri),
                                    ptr.to_string(),
                                    target_draft,
                                    value_addr,
                                ));
                            }
                        }
                        continue;
                    }
                    if mark_reference(&mut state.seen, &current_base_uri, reference) {
                        let resolved = if current_base_uri.has_fragment() {
                            let mut base_without_fragment = current_base_uri.as_ref().clone();
                            base_without_fragment.set_fragment(None);

                            let (path, fragment) = match reference.split_once('#') {
                                Some((path, fragment)) => (path, Some(fragment)),
                                None => (reference, None),
                            };

                            let mut resolved = (*resolution_cache
                                .resolve_against(&base_without_fragment.borrow(), path)?)
                            .clone();
                            if let Some(fragment) = fragment {
                                if let Some(encoded) = uri::EncodedString::new(fragment) {
                                    resolved = resolved.with_fragment(Some(encoded));
                                } else {
                                    uri::encode_to(fragment, &mut state.scratch);
                                    resolved = resolved.with_fragment(Some(
                                        uri::EncodedString::new_or_panic(&state.scratch),
                                    ));
                                    state.scratch.clear();
                                }
                            }
                            resolved
                        } else {
                            (*resolution_cache
                                .resolve_against(&current_base_uri.borrow(), reference)?)
                            .clone()
                        };

                        let kind = if key == "$schema" {
                            ReferenceKind::Schema
                        } else {
                            ReferenceKind::Ref
                        };
                        state
                            .external
                            .insert((reference.to_string(), resolved, kind));
                    }
                }
            }

            let mut idx = child_start;
            while idx < child_end {
                let (child, child_draft) = state.borrowed_child_scratch[idx];
                idx += 1;
                explore_borrowed_subtree(
                    Arc::clone(&current_base_uri),
                    document_root,
                    child,
                    child_draft,
                    false,
                    document_root_uri,
                    state,
                    known_resources,
                    resolution_cache,
                    local_seen,
                )?;
            }

            state.borrowed_reference_scratch.ref_ = None;
            state.borrowed_reference_scratch.schema = None;
            state.borrowed_child_scratch.truncate(child_start);
            return Ok(());
        }
    }
    let subschema_ptr = std::ptr::from_ref::<Value>(subschema) as usize;
    if state.visited_schemas.insert(subschema_ptr)
        && probe.as_ref().is_none_or(|probe| probe.has_ref_or_schema)
    {
        collect_external_resources(
            &current_base_uri,
            document_root,
            subschema,
            &mut state.external,
            &mut state.seen,
            resolution_cache,
            &mut state.scratch,
            &mut state.refers_metaschemas,
            draft,
            document_root_uri,
            &mut state.deferred_refs,
            local_seen,
        )?;
    }

    if let Some(schema) = object {
        draft.walk_borrowed_subresources_map(schema, &mut |child, child_draft| {
            explore_borrowed_subtree(
                Arc::clone(&current_base_uri),
                document_root,
                child,
                child_draft,
                false,
                document_root_uri,
                state,
                known_resources,
                resolution_cache,
                local_seen,
            )
        })
    } else {
        Ok(())
    }
}

fn process_owned_document<'a, 'r>(
    current_base_uri: Arc<Uri<String>>,
    document_root_uri: &Arc<Uri<String>>,
    document: &'a Arc<StoredDocument<'r>>,
    pointer_path: &str,
    draft: Draft,
    state: &mut ProcessingState<'r>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    local_seen: &mut LocalSeen<'a>,
) -> Result<(), Error> {
    let document_root = document.contents();
    let Some(subschema) = (if pointer_path.is_empty() {
        Some(document_root)
    } else {
        pointer(document_root, pointer_path)
    }) else {
        return Ok(());
    };
    let parsed_pointer = ParsedPointer::from_json_pointer(pointer_path);

    with_pointer_node_from_parsed(parsed_pointer.as_ref(), |path| {
        explore_owned_subtree(
            current_base_uri,
            document_root,
            subschema,
            draft,
            pointer_path.is_empty(),
            path,
            document_root_uri,
            document,
            state,
            known_resources,
            resolution_cache,
            local_seen,
        )
    })
}

fn with_pointer_node_from_parsed<R>(
    pointer: Option<&ParsedPointer>,
    f: impl FnOnce(&JsonPointerNode<'_, '_>) -> R,
) -> R {
    fn descend<'a, 'node, R>(
        segments: &'a [ParsedPointerSegment],
        current: &'node JsonPointerNode<'a, 'node>,
        f: impl FnOnce(&JsonPointerNode<'_, '_>) -> R,
    ) -> R {
        if let Some((head, tail)) = segments.split_first() {
            let next = match head {
                ParsedPointerSegment::Key(key) => current.push(key.as_ref()),
                ParsedPointerSegment::Index(idx) => current.push(*idx),
            };
            descend(tail, &next, f)
        } else {
            f(current)
        }
    }

    let root = JsonPointerNode::new();
    match pointer {
        Some(pointer) => descend(&pointer.segments, &root, f),
        None => f(&root),
    }
}

fn explore_owned_subtree<'a, 'r>(
    mut current_base_uri: Arc<Uri<String>>,
    document_root: &'a Value,
    subschema: &'a Value,
    draft: Draft,
    is_root_entry: bool,
    path: &JsonPointerNode<'_, '_>,
    document_root_uri: &Arc<Uri<String>>,
    document: &Arc<StoredDocument<'r>>,
    state: &mut ProcessingState<'r>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    local_seen: &mut LocalSeen<'a>,
) -> Result<(), Error> {
    let object = subschema.as_object();
    let (id, has_anchors) = object.map_or((None, false), |schema| {
        draft.id_and_has_anchors_object(schema)
    });
    if let Some(id) = id {
        let original_base_uri = Arc::clone(&current_base_uri);
        current_base_uri = resolve_id(&current_base_uri, id, resolution_cache)?;
        known_resources.insert((*current_base_uri).clone());
        let insert_resource = current_base_uri != original_base_uri;
        if !(is_root_entry && current_base_uri == *document_root_uri)
            && (insert_resource || has_anchors)
        {
            let pointer = ParsedPointer::from_pointer_node(path);
            insert_owned_discovered_index_entries(
                &mut state.index_data,
                &current_base_uri,
                document,
                &pointer,
                draft,
                insert_resource,
                subschema,
            );
        }
    } else if has_anchors && !is_root_entry {
        let pointer = ParsedPointer::from_pointer_node(path);
        insert_owned_discovered_index_entries(
            &mut state.index_data,
            &current_base_uri,
            document,
            &pointer,
            draft,
            false,
            subschema,
        );
    }

    let subschema_ptr = std::ptr::from_ref::<Value>(subschema) as usize;
    if state.visited_schemas.insert(subschema_ptr) {
        collect_external_resources(
            &current_base_uri,
            document_root,
            subschema,
            &mut state.external,
            &mut state.seen,
            resolution_cache,
            &mut state.scratch,
            &mut state.refers_metaschemas,
            draft,
            document_root_uri,
            &mut state.deferred_refs,
            local_seen,
        )?;
    }

    if let Some(schema) = object {
        draft.walk_owned_subresources_map(schema, path, &mut |child_path, child, child_draft| {
            explore_owned_subtree(
                Arc::clone(&current_base_uri),
                document_root,
                child,
                child_draft,
                false,
                child_path,
                document_root_uri,
                document,
                state,
                known_resources,
                resolution_cache,
                local_seen,
            )
        })
    } else {
        Ok(())
    }
}

fn enqueue_fragment_entry(
    uri: &Uri<String>,
    key: &Arc<Uri<String>>,
    default_draft: Draft,
    documents: &DocumentStore<'_>,
    queue: &mut VecDeque<QueueEntry>,
) {
    if let Some(fragment) = uri.fragment() {
        let Some(document) = documents.get(key) else {
            return;
        };
        if let Some(resolved) = pointer(document.contents(), fragment.as_str()) {
            let fragment_draft = default_draft.detect(resolved);
            queue.push_back((
                Arc::clone(key),
                Arc::clone(key),
                fragment.as_str().to_string(),
                fragment_draft,
            ));
        }
    }
}

fn handle_metaschemas<'a>(
    refers_metaschemas: bool,
    documents: &mut DocumentStore<'a>,
    known_resources: &mut KnownResources,
    draft_version: Draft,
    state: &mut ProcessingState<'a>,
) -> Result<(), Error> {
    if !refers_metaschemas {
        return Ok(());
    }

    let schemas = metas_for_draft(draft_version);
    for (uri, schema) in schemas {
        let key = Arc::new(uri::from_str(uri.trim_end_matches('#'))?);
        if documents.contains_key(&key) {
            continue;
        }
        let draft = Draft::default().detect(schema);
        documents.insert(
            Arc::clone(&key),
            Arc::new(StoredDocument::borrowed(schema, draft)),
        );
        known_resources.insert((*key).clone());
        insert_root_index_entries(
            &mut state.index_data,
            &key,
            documents
                .get(&key)
                .expect("meta-schema document was just inserted into the store"),
        );
        state
            .queue
            .push_back((Arc::clone(&key), Arc::clone(&key), String::new(), draft));
    }
    Ok(())
}

fn create_resource<'a>(
    retrieved: Value,
    fragmentless: Uri<String>,
    default_draft: Draft,
    documents: &mut DocumentStore<'a>,
    known_resources: &mut KnownResources,
    index_data: &mut PreparedIndex<'a>,
    custom_metaschemas: &mut Vec<String>,
) -> (Arc<Uri<String>>, Draft) {
    let draft = default_draft.detect(&retrieved);
    let key = Arc::new(fragmentless);
    documents.insert(
        Arc::clone(&key),
        Arc::new(StoredDocument::owned(retrieved, draft)),
    );

    let contents = documents
        .get(&key)
        .expect("document was just inserted")
        .contents();
    known_resources.insert((*key).clone());
    insert_root_index_entries(
        index_data,
        &key,
        documents
            .get(&key)
            .expect("retrieved document was just inserted into the store"),
    );

    if draft == Draft::Unknown {
        if let Some(meta_schema) = contents
            .as_object()
            .and_then(|obj| obj.get("$schema"))
            .and_then(|schema| schema.as_str())
        {
            custom_metaschemas.push(meta_schema.to_string());
        }
    }

    (key, draft)
}

/// Shared sync processing loop used during registry preparation. After the
/// initial input has been ingested into `state`, this function drives the
/// BFS-fetch cycle until all reachable external resources have been retrieved,
/// then handles meta-schema injection and runs a final queue pass.
#[allow(unsafe_code)]
fn run_sync_processing_loop<'a>(
    state: &mut ProcessingState<'a>,
    documents: &mut DocumentStore<'a>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    default_draft: Draft,
    retriever: &dyn Retrieve,
) -> Result<(), Error> {
    let mut local_seen_buf: LocalSeen<'static> = LocalSeen::new();

    loop {
        if state.queue.is_empty() && state.external.is_empty() {
            break;
        }

        {
            // SAFETY: widens 'static → '_ (covariant); set is empty after reuse_local_seen clears it.
            let mut local_seen: LocalSeen<'_> = unsafe { reuse_local_seen(local_seen_buf) };
            process_queue(state, documents, known_resources, resolution_cache)?;
            process_deferred_refs(state, documents, resolution_cache, &mut local_seen)?;
            // SAFETY: clears all '_ refs before narrowing back to 'static to reclaim the buffer.
            local_seen_buf = unsafe { reuse_local_seen(local_seen) };
        }

        for (original, uri, kind) in state.external.drain() {
            let mut fragmentless = uri.clone();
            fragmentless.set_fragment(None);
            if !known_resources.contains(&fragmentless) {
                let retrieved = match retriever.retrieve(&fragmentless) {
                    Ok(retrieved) => retrieved,
                    Err(error) => {
                        handle_retrieve_error(&uri, &original, &fragmentless, error, kind)?;
                        continue;
                    }
                };

                let (key, draft) = create_resource(
                    retrieved,
                    fragmentless,
                    default_draft,
                    documents,
                    known_resources,
                    &mut state.index_data,
                    &mut state.custom_metaschemas,
                );
                enqueue_fragment_entry(&uri, &key, default_draft, documents, &mut state.queue);
                state
                    .queue
                    .push_back((Arc::clone(&key), key, String::new(), draft));
            }
        }
    }

    handle_metaschemas(
        state.refers_metaschemas,
        documents,
        known_resources,
        default_draft,
        state,
    )?;

    if !state.queue.is_empty() {
        // SAFETY: widens 'static → '_ (covariant); set is empty after reuse_local_seen clears it.
        let mut local_seen: LocalSeen<'_> = unsafe { reuse_local_seen(local_seen_buf) };
        process_queue(state, documents, known_resources, resolution_cache)?;
        process_deferred_refs(state, documents, resolution_cache, &mut local_seen)?;
    }

    Ok(())
}

fn process_resources_mixed<'a>(
    pairs: impl IntoIterator<Item = (Uri<String>, PendingResource<'a>)>,
    retriever: &dyn Retrieve,
    documents: &mut DocumentStore<'a>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    draft_override: Option<Draft>,
) -> Result<(Vec<String>, PreparedIndex<'a>), Error> {
    let mut state = ProcessingState::new();
    process_input_resources_mixed(
        pairs,
        documents,
        known_resources,
        &mut state,
        draft_override,
    );
    run_sync_processing_loop(
        &mut state,
        documents,
        known_resources,
        resolution_cache,
        draft_override.unwrap_or_default(),
        retriever,
    )?;
    Ok((state.custom_metaschemas, state.index_data))
}

#[cfg(feature = "retrieve-async")]
async fn process_resources_async_mixed<'a>(
    pairs: impl IntoIterator<Item = (Uri<String>, PendingResource<'a>)>,
    retriever: &dyn crate::AsyncRetrieve,
    documents: &mut DocumentStore<'a>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    draft_override: Option<Draft>,
) -> Result<(Vec<String>, PreparedIndex<'a>), Error> {
    let mut state = ProcessingState::new();
    process_input_resources_mixed(
        pairs,
        documents,
        known_resources,
        &mut state,
        draft_override,
    );
    run_async_processing_loop(
        &mut state,
        documents,
        known_resources,
        resolution_cache,
        draft_override.unwrap_or_default(),
        retriever,
    )
    .await?;
    Ok((state.custom_metaschemas, state.index_data))
}

/// Shared async processing loop used during registry preparation. Batches
/// concurrent external retrievals with `join_all` and otherwise mirrors
/// [`run_sync_processing_loop`].
#[cfg(feature = "retrieve-async")]
#[allow(unsafe_code)]
async fn run_async_processing_loop<'a>(
    state: &mut ProcessingState<'a>,
    documents: &mut DocumentStore<'a>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    default_draft: Draft,
    retriever: &dyn crate::AsyncRetrieve,
) -> Result<(), Error> {
    type ExternalRefsByBase = AHashMap<Uri<String>, Vec<(String, Uri<String>, ReferenceKind)>>;

    let mut local_seen_buf: LocalSeen<'static> = LocalSeen::new();

    loop {
        if state.queue.is_empty() && state.external.is_empty() {
            break;
        }

        {
            // SAFETY: widens 'static → '_ (covariant); set is empty after reuse_local_seen clears it.
            let mut local_seen: LocalSeen<'_> = unsafe { reuse_local_seen(local_seen_buf) };
            process_queue(state, documents, known_resources, resolution_cache)?;
            process_deferred_refs(state, documents, resolution_cache, &mut local_seen)?;
            // SAFETY: clears all '_ refs before narrowing back to 'static to reclaim the buffer.
            local_seen_buf = unsafe { reuse_local_seen(local_seen) };
        }

        if !state.external.is_empty() {
            let mut grouped = ExternalRefsByBase::new();
            for (original, uri, kind) in state.external.drain() {
                let mut fragmentless = uri.clone();
                fragmentless.set_fragment(None);
                if !known_resources.contains(&fragmentless) {
                    grouped
                        .entry(fragmentless)
                        .or_default()
                        .push((original, uri, kind));
                }
            }

            let entries: Vec<_> = grouped.into_iter().collect();
            let results = {
                let futures = entries
                    .iter()
                    .map(|(fragmentless, _)| retriever.retrieve(fragmentless));
                futures::future::join_all(futures).await
            };

            for ((fragmentless, refs), result) in entries.into_iter().zip(results) {
                let retrieved = match result {
                    Ok(retrieved) => retrieved,
                    Err(error) => {
                        if let Some((original, uri, kind)) = refs.into_iter().next() {
                            handle_retrieve_error(&uri, &original, &fragmentless, error, kind)?;
                        }
                        continue;
                    }
                };

                let (key, draft) = create_resource(
                    retrieved,
                    fragmentless,
                    default_draft,
                    documents,
                    known_resources,
                    &mut state.index_data,
                    &mut state.custom_metaschemas,
                );

                for (_, uri, _) in &refs {
                    enqueue_fragment_entry(uri, &key, default_draft, documents, &mut state.queue);
                }

                state
                    .queue
                    .push_back((Arc::clone(&key), key, String::new(), draft));
            }
        }
    }

    handle_metaschemas(
        state.refers_metaschemas,
        documents,
        known_resources,
        default_draft,
        state,
    )?;

    if !state.queue.is_empty() {
        // SAFETY: widens 'static → '_ (covariant); set is empty after reuse_local_seen clears it.
        let mut local_seen: LocalSeen<'_> = unsafe { reuse_local_seen(local_seen_buf) };
        process_queue(state, documents, known_resources, resolution_cache)?;
        process_deferred_refs(state, documents, resolution_cache, &mut local_seen)?;
    }

    Ok(())
}

fn handle_retrieve_error(
    uri: &Uri<String>,
    // The original reference string is used in error messages for `json-schema://` URIs
    // where the resolved URI is not user-friendly (e.g. "./foo.json" vs "json-schema:///foo.json").
    original: &str,
    fragmentless: &Uri<String>,
    error: Box<dyn std::error::Error + Send + Sync>,
    kind: ReferenceKind,
) -> Result<(), Error> {
    match kind {
        ReferenceKind::Schema => Ok(()),
        ReferenceKind::Ref => {
            if uri.scheme().as_str() == "json-schema" {
                Err(Error::unretrievable(
                    original,
                    "No base URI is available".into(),
                ))
            } else {
                Err(Error::unretrievable(fragmentless.as_str(), error))
            }
        }
    }
}

fn validate_custom_metaschemas(
    custom_metaschemas: &[String],
    known_resources: &KnownResources,
) -> Result<(), Error> {
    for schema_uri in custom_metaschemas {
        match uri::from_str(schema_uri) {
            Ok(mut meta_uri) => {
                meta_uri.set_fragment(None);
                if !known_resources.contains(&meta_uri) {
                    return Err(Error::unknown_specification(schema_uri));
                }
            }
            Err(_) => {
                return Err(Error::unknown_specification(schema_uri));
            }
        }
    }
    Ok(())
}

fn collect_external_resources<'doc>(
    base: &Arc<Uri<String>>,
    root: &'doc Value,
    contents: &'doc Value,
    collected: &mut AHashSet<(String, Uri<String>, ReferenceKind)>,
    seen: &mut ReferenceTracker,
    resolution_cache: &mut UriCache,
    scratch: &mut String,
    refers_metaschemas: &mut bool,
    draft: Draft,
    doc_key: &Arc<Uri<String>>,
    deferred_refs: &mut Vec<DeferredRef>,
    local_seen: &mut LocalSeen<'doc>,
) -> Result<(), Error> {
    if base.scheme().as_str() == "urn" {
        return Ok(());
    }

    macro_rules! on_reference {
        ($reference:expr, $key:literal) => {
            if $reference.starts_with("https://json-schema.org/draft/")
                || $reference.starts_with("http://json-schema.org/draft-")
                || base.as_str().starts_with("https://json-schema.org/draft/")
            {
                if $key == "$ref" {
                    *refers_metaschemas = true;
                }
            } else if $reference != "#" {
                if $reference.starts_with('#') {
                    crate::observe_registry!("registry.local_ref={}", $reference);
                    if draft == Draft::Draft4 || mark_local_reference(local_seen, base, $reference)
                    {
                        let ptr = $reference.trim_start_matches('#');
                        if let Some(referenced) = pointer(root, ptr) {
                            let target_draft = draft.detect(referenced);
                            let value_addr = std::ptr::from_ref::<Value>(referenced) as usize;
                            deferred_refs.push((
                                Arc::clone(base),
                                Arc::clone(doc_key),
                                ptr.to_string(),
                                target_draft,
                                value_addr,
                            ));
                        }
                    }
                } else if mark_reference(seen, base, $reference) {
                    if $key == "$schema" {
                        crate::observe_registry!("registry.schema_ref={}", $reference);
                    } else {
                        crate::observe_registry!("registry.external_ref={}", $reference);
                    }
                    let resolved = if base.has_fragment() {
                        let mut base_without_fragment = base.as_ref().clone();
                        base_without_fragment.set_fragment(None);

                        let (path, fragment) = match $reference.split_once('#') {
                            Some((path, fragment)) => (path, Some(fragment)),
                            None => ($reference, None),
                        };

                        let mut resolved = (*resolution_cache
                            .resolve_against(&base_without_fragment.borrow(), path)?)
                        .clone();
                        if let Some(fragment) = fragment {
                            if let Some(encoded) = uri::EncodedString::new(fragment) {
                                resolved = resolved.with_fragment(Some(encoded));
                            } else {
                                uri::encode_to(fragment, scratch);
                                resolved = resolved
                                    .with_fragment(Some(uri::EncodedString::new_or_panic(scratch)));
                                scratch.clear();
                            }
                        }
                        resolved
                    } else {
                        (*resolution_cache.resolve_against(&base.borrow(), $reference)?).clone()
                    };

                    let kind = if $key == "$schema" {
                        ReferenceKind::Schema
                    } else {
                        ReferenceKind::Ref
                    };
                    collected.insert(($reference.to_string(), resolved, kind));
                }
            }
        };
    }

    if let Some(object) = contents.as_object() {
        crate::observe_registry!("registry.ref_scan.object_len={}", object.len());
        if object.len() < 3 {
            for (key, value) in object {
                if key == "$ref" {
                    if let Some(reference) = value.as_str() {
                        on_reference!(reference, "$ref");
                    }
                } else if key == "$schema" {
                    if let Some(reference) = value.as_str() {
                        on_reference!(reference, "$schema");
                    }
                }
            }
        } else {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                on_reference!(reference, "$ref");
            }
            if let Some(reference) = object.get("$schema").and_then(Value::as_str) {
                on_reference!(reference, "$schema");
            }
        }
    }
    Ok(())
}

fn collect_external_resources_recursive<'doc>(
    base: &Arc<Uri<String>>,
    root: &'doc Value,
    contents: &'doc Value,
    collected: &mut AHashSet<(String, Uri<String>, ReferenceKind)>,
    seen: &mut ReferenceTracker,
    resolution_cache: &mut UriCache,
    scratch: &mut String,
    refers_metaschemas: &mut bool,
    draft: Draft,
    visited: &mut AHashSet<usize>,
    doc_key: &Arc<Uri<String>>,
    deferred_refs: &mut Vec<DeferredRef>,
    local_seen: &mut LocalSeen<'doc>,
) -> Result<(), Error> {
    let ptr = std::ptr::from_ref::<Value>(contents) as usize;
    if !visited.insert(ptr) {
        return Ok(());
    }

    let current_base = match draft.id_of(contents) {
        Some(id) => resolve_id(base, id, resolution_cache)?,
        None => Arc::clone(base),
    };

    collect_external_resources(
        &current_base,
        root,
        contents,
        collected,
        seen,
        resolution_cache,
        scratch,
        refers_metaschemas,
        draft,
        doc_key,
        deferred_refs,
        local_seen,
    )?;

    for subresource in draft.subresources_of(contents) {
        let subresource_draft = draft.detect(subresource);
        collect_external_resources_recursive(
            &current_base,
            root,
            subresource,
            collected,
            seen,
            resolution_cache,
            scratch,
            refers_metaschemas,
            subresource_draft,
            visited,
            doc_key,
            deferred_refs,
            local_seen,
        )?;
    }
    Ok(())
}

/// Process deferred local-ref targets collected during the main traversal.
///
/// Called after `process_queue` finishes so that all subresource nodes are already in
/// `visited_schemas`. Targets that were visited by the main BFS (e.g. `#/definitions/Foo`
/// under a JSON Schema keyword) are skipped in O(1) via the pre-stored value address,
/// avoiding a redundant `pointer()` traversal. Non-subresource targets
/// (e.g. `#/components/schemas/Foo`) are still fully traversed. New deferred entries
/// added during traversal are also processed iteratively until none remain.
fn process_deferred_refs<'a>(
    state: &mut ProcessingState<'_>,
    documents: &'a DocumentStore<'a>,
    resolution_cache: &mut UriCache,
    local_seen: &mut LocalSeen<'a>,
) -> Result<(), Error> {
    while !state.deferred_refs.is_empty() {
        let batch = std::mem::take(&mut state.deferred_refs);
        for (base, doc_key, pointer_path, draft, value_addr) in batch {
            // Fast path: if this target was already visited by the main BFS traversal
            // (e.g. a `#/definitions/Foo` that `walk_subresources_with_path` descended into),
            // all its subresources were processed and `collect_external_resources` was already
            // called on each — skip without a redundant `pointer()` traversal.
            if state.visited_schemas.contains(&value_addr) {
                continue;
            }
            let Some(document) = documents.get(&doc_key) else {
                continue;
            };
            let root = document.contents();
            let Some(contents) = (if pointer_path.is_empty() {
                Some(root)
            } else {
                pointer(root, &pointer_path)
            }) else {
                continue;
            };
            collect_external_resources_recursive(
                &base,
                root,
                contents,
                &mut state.external,
                &mut state.seen,
                resolution_cache,
                &mut state.scratch,
                &mut state.refers_metaschemas,
                draft,
                &mut state.visited_schemas,
                &doc_key,
                &mut state.deferred_refs,
                local_seen,
            )?;
        }
    }
    Ok(())
}

fn mark_reference(seen: &mut ReferenceTracker, base: &Arc<Uri<String>>, reference: &str) -> bool {
    seen.insert(ReferenceKey::new(base, reference))
}

fn mark_local_reference<'a>(
    local_seen: &mut LocalSeen<'a>,
    base: &Arc<Uri<String>>,
    reference: &'a str,
) -> bool {
    let base_ptr =
        NonZeroUsize::new(Arc::as_ptr(base) as usize).expect("Arc pointer should never be null");
    local_seen.insert((base_ptr, reference))
}

fn resolve_id(
    base: &Arc<Uri<String>>,
    id: &str,
    resolution_cache: &mut UriCache,
) -> Result<Arc<Uri<String>>, Error> {
    if id.starts_with('#') {
        return Ok(Arc::clone(base));
    }
    let mut resolved = (*resolution_cache.resolve_against(&base.borrow(), id)?).clone();
    if resolved.fragment().is_some_and(EStr::is_empty) {
        resolved.set_fragment(None);
    }
    Ok(Arc::new(resolved))
}

/// Look up a value by a JSON Pointer.
///
/// **NOTE**: A slightly faster version of pointer resolution based on `Value::pointer` from `serde_json`.
pub fn pointer<'a>(document: &'a Value, pointer: &str) -> Option<&'a Value> {
    crate::observe_registry!(
        "registry.pointer_segments={}",
        bytecount::count(pointer.as_bytes(), b'/')
    );
    if pointer.is_empty() {
        return Some(document);
    }
    if !pointer.starts_with('/') {
        return None;
    }
    pointer.split('/').skip(1).map(unescape_segment).try_fold(
        document,
        |target, token| match target {
            Value::Object(map) => map.get(&*token),
            Value::Array(list) => parse_index(&token).and_then(|x| list.get(x)),
            _ => None,
        },
    )
}

// Taken from `serde_json`.
#[must_use]
pub fn parse_index(s: &str) -> Option<usize> {
    if s.starts_with('+') || (s.starts_with('0') && s.len() != 1) {
        return None;
    }
    s.parse().ok()
}
#[cfg(test)]
mod tests {
    use std::{error::Error as _, sync::Arc};

    use ahash::AHashMap;
    use fluent_uri::Uri;
    use serde_json::{json, Value};
    use test_case::test_case;

    use crate::{uri::from_str, Anchor, Draft, JsonPointerNode, Registry, Resource, Retrieve};

    use super::{
        insert_root_index_entries, pointer, process_borrowed_document, process_owned_document,
        IndexedResource, KnownResources, LocalSeen, ParsedPointer, ProcessingState, StoredDocument,
        SPECIFICATIONS,
    };
    use crate::cache::UriCache;

    #[test]
    fn test_empty_pointer() {
        let document = json!({});
        assert_eq!(pointer(&document, ""), Some(&document));
    }

    #[test]
    fn test_parsed_pointer_from_json_pointer_node_matches_pointer_lookup() {
        let document = json!({
            "$defs": {
                "foo/bar": [
                    {"value": true}
                ]
            }
        });
        let root = JsonPointerNode::new();
        let defs = root.push("$defs");
        let entry = defs.push("foo/bar");
        let node = entry.push(0);

        let parsed = ParsedPointer::from_pointer_node(&node);
        assert_eq!(
            parsed.lookup(&document),
            pointer(&document, "/$defs/foo~1bar/0")
        );
    }

    #[test]
    fn test_invalid_uri_on_registry_creation() {
        let schema = Draft::Draft202012.create_resource(json!({}));
        let result = Registry::new().add(":/example.com", schema);
        let error = result.expect_err("Should fail");

        assert_eq!(
            error.to_string(),
            "Invalid URI reference ':/example.com': unexpected character at index 0"
        );
        let source_error = error.source().expect("Should have a source");
        let inner_source = source_error.source().expect("Should have a source");
        assert_eq!(inner_source.to_string(), "unexpected character at index 0");
    }

    #[test]
    fn test_lookup_unresolvable_url() {
        // Create a registry with a single resource
        let schema = Draft::Draft202012.create_resource(json!({
            "type": "object",
            "properties": {
                "foo": { "type": "string" }
            }
        }));
        let registry = Registry::new()
            .add("http://example.com/schema1", schema)
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");

        // Attempt to create a resolver for a URL not in the registry
        let resolver = registry.resolver(
            from_str("http://example.com/non_existent_schema").expect("Invalid base URI"),
        );

        let result = resolver.lookup("");

        assert_eq!(
            result.unwrap_err().to_string(),
            "Resource 'http://example.com/non_existent_schema' is not present in a registry and retrieving it failed: Retrieving external resources is not supported once the registry is populated"
        );
    }

    #[test]
    fn test_registry_can_be_built_from_borrowed_resources() {
        let schema = json!({"type": "string"});
        let registry = Registry::new()
            .add("urn:root", &schema)
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");
        assert!(registry.contains_resource_uri("urn:root"));
    }

    #[test]
    fn test_prepare_builds_local_entries_for_borrowed_and_owned() {
        let root = json!({"$ref": "http://example.com/remote"});
        let remote = json!({"type": "string"});
        let registry = Registry::new()
            .retriever(create_test_retriever(&[(
                "http://example.com/remote",
                remote.clone(),
            )]))
            .add("http://example.com/root", &root)
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");

        let root_uri = from_str("http://example.com/root").expect("Invalid root URI");
        let remote_uri = from_str("http://example.com/remote").expect("Invalid remote URI");

        let root_resource = registry
            .resource_by_uri(&root_uri)
            .expect("Borrowed root should be available from prepared local entries");
        let remote_resource = registry
            .resource_by_uri(&remote_uri)
            .expect("Owned retrieved document should be available from prepared local entries");

        assert_eq!(root_resource.contents(), &root);
        assert_eq!(remote_resource.contents(), &remote);
    }

    #[test]
    fn test_prepare_populates_local_entries_for_subresources_and_anchors() {
        let registry = Registry::new()
            .add(
                "http://example.com/root",
                json!({
                    "$defs": {
                        "embedded": {
                            "$id": "http://example.com/embedded",
                            "$anchor": "node",
                            "type": "string"
                        }
                    }
                }),
            )
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");

        let embedded_uri = from_str("http://example.com/embedded").expect("Invalid embedded URI");
        let embedded_resource = registry
            .resource_by_uri(&embedded_uri)
            .expect("Embedded subresource should be available from prepared local entries");
        assert_eq!(
            embedded_resource.contents(),
            &json!({
                "$id": "http://example.com/embedded",
                "$anchor": "node",
                "type": "string"
            })
        );

        let embedded_anchor = registry
            .anchor(&embedded_uri, "node")
            .expect("Embedded anchor should be available from prepared local entries");
        match embedded_anchor {
            Anchor::Default { resource, .. } => assert_eq!(
                resource.contents(),
                &json!({
                    "$id": "http://example.com/embedded",
                    "$anchor": "node",
                    "type": "string"
                })
            ),
            Anchor::Dynamic { .. } => panic!("Expected a default anchor"),
        }
    }

    #[test]
    fn test_process_borrowed_document_indexes_embedded_resource_as_borrowed() {
        let schema = json!({
            "$defs": {
                "embedded": {
                    "$id": "http://example.com/embedded",
                    "type": "string"
                }
            }
        });
        let doc_key = Arc::new(from_str("http://example.com/root").expect("valid root URI"));
        let document = Arc::new(StoredDocument::borrowed(&schema, Draft::Draft202012));
        let mut state = ProcessingState::new();
        let mut known_resources = KnownResources::default();
        let mut resolution_cache = UriCache::new();
        let mut local_seen = LocalSeen::new();

        known_resources.insert((*doc_key).clone());
        insert_root_index_entries(&mut state.index_data, &doc_key, &document);

        process_borrowed_document(
            Arc::clone(&doc_key),
            &doc_key,
            &document,
            "",
            Draft::Draft202012,
            &mut state,
            &mut known_resources,
            &mut resolution_cache,
            &mut local_seen,
        )
        .expect("borrowed document traversal should succeed");

        let embedded_uri =
            Arc::new(from_str("http://example.com/embedded").expect("valid embedded URI"));
        match state.index_data.resources.get(&embedded_uri) {
            Some(IndexedResource::Borrowed(resource)) => {
                assert_eq!(
                    resource.contents(),
                    &json!({"$id": "http://example.com/embedded", "type": "string"})
                );
            }
            other => panic!("expected borrowed embedded resource entry, got {other:?}"),
        }
    }

    #[test]
    fn test_process_owned_document_indexes_embedded_resource_as_owned() {
        let schema = json!({
            "$defs": {
                "embedded": {
                    "$id": "http://example.com/embedded",
                    "type": "string"
                }
            }
        });
        let doc_key = Arc::new(from_str("http://example.com/root").expect("valid root URI"));
        let document = Arc::new(StoredDocument::owned(schema, Draft::Draft202012));
        let mut state = ProcessingState::new();
        let mut known_resources = KnownResources::default();
        let mut resolution_cache = UriCache::new();
        let mut local_seen = LocalSeen::new();

        known_resources.insert((*doc_key).clone());
        insert_root_index_entries(&mut state.index_data, &doc_key, &document);

        process_owned_document(
            Arc::clone(&doc_key),
            &doc_key,
            &document,
            "",
            Draft::Draft202012,
            &mut state,
            &mut known_resources,
            &mut resolution_cache,
            &mut local_seen,
        )
        .expect("owned document traversal should succeed");

        let embedded_uri =
            Arc::new(from_str("http://example.com/embedded").expect("valid embedded URI"));
        match state.index_data.resources.get(&embedded_uri) {
            Some(IndexedResource::Owned { .. }) => {}
            other => panic!("expected owned embedded resource entry, got {other:?}"),
        }
    }

    #[test]
    fn test_process_owned_document_indexes_fragment_root_with_pointer_prefix() {
        let schema = json!({
            "$defs": {
                "embedded": {
                    "$id": "http://example.com/embedded",
                    "type": "string"
                }
            }
        });
        let doc_key = Arc::new(from_str("http://example.com/root").expect("valid root URI"));
        let document = Arc::new(StoredDocument::owned(schema, Draft::Draft202012));
        let mut state = ProcessingState::new();
        let mut known_resources = KnownResources::default();
        let mut resolution_cache = UriCache::new();
        let mut local_seen = LocalSeen::new();

        known_resources.insert((*doc_key).clone());
        insert_root_index_entries(&mut state.index_data, &doc_key, &document);

        process_owned_document(
            Arc::clone(&doc_key),
            &doc_key,
            &document,
            "/$defs/embedded",
            Draft::Draft202012,
            &mut state,
            &mut known_resources,
            &mut resolution_cache,
            &mut local_seen,
        )
        .expect("owned fragment traversal should succeed");

        let embedded_uri =
            Arc::new(from_str("http://example.com/embedded").expect("valid embedded URI"));
        match state.index_data.resources.get(&embedded_uri) {
            Some(IndexedResource::Owned { pointer, .. }) => {
                assert_eq!(
                    pointer.lookup(document.contents()),
                    Some(&json!({"$id": "http://example.com/embedded", "type": "string"}))
                );
            }
            other => panic!("expected owned embedded resource entry, got {other:?}"),
        }
    }

    #[test]
    fn test_prepare_merges_anchor_entries_for_shared_effective_uri() {
        let registry = Registry::new()
            .add(
                "http://example.com/root",
                json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "$defs": {
                        "first": {
                            "$anchor": "first",
                            "type": "string"
                        },
                        "second": {
                            "$anchor": "second",
                            "type": "integer"
                        }
                    }
                }),
            )
            .expect("Invalid resources")
            .prepare()
            .expect("Invalid resources");

        let resolver = registry.resolver(from_str("http://example.com/root").expect("Invalid URI"));

        assert_eq!(
            resolver
                .lookup("#first")
                .expect("First anchor should resolve")
                .contents(),
            &json!({
                "$anchor": "first",
                "type": "string"
            })
        );
        assert_eq!(
            resolver
                .lookup("#second")
                .expect("Second anchor should resolve")
                .contents(),
            &json!({
                "$anchor": "second",
                "type": "integer"
            })
        );
    }

    #[test]
    fn test_relative_uri_without_base() {
        let schema = Draft::Draft202012.create_resource(json!({"$ref": "./virtualNetwork.json"}));
        let error = Registry::new()
            .add("json-schema:///", schema)
            .expect("Root resource should be accepted")
            .prepare()
            .expect_err("Should fail");
        assert_eq!(error.to_string(), "Resource './virtualNetwork.json' is not present in a registry and retrieving it failed: No base URI is available");
    }

    #[test]
    fn test_prepare_requires_registered_custom_meta_schema() {
        let base_registry = Registry::new()
            .add(
                "http://example.com/root",
                Resource::from_contents(json!({"type": "object"})),
            )
            .expect("Base registry should be created")
            .prepare()
            .expect("Base registry should be created");

        let custom_schema = Resource::from_contents(json!({
            "$id": "http://example.com/custom",
            "$schema": "http://example.com/meta/custom",
            "type": "string"
        }));

        let error = base_registry
            .add("http://example.com/custom", custom_schema)
            .expect("Schema should be accepted")
            .prepare()
            .expect_err("Extending registry must fail when the custom $schema is not registered");

        let error_msg = error.to_string();
        assert_eq!(
            error_msg,
            "Unknown meta-schema: 'http://example.com/meta/custom'. Custom meta-schemas must be registered in the registry before use"
        );
    }

    #[test]
    fn test_prepare_accepts_registered_custom_meta_schema_fragment() {
        let meta_schema = Resource::from_contents(json!({
            "$id": "http://example.com/meta/custom#",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object"
        }));

        let registry = Registry::new()
            .add("http://example.com/meta/custom#", meta_schema)
            .expect("Meta-schema should be registered successfully")
            .prepare()
            .expect("Meta-schema should be registered successfully");

        let schema = Resource::from_contents(json!({
            "$id": "http://example.com/schemas/my-schema",
            "$schema": "http://example.com/meta/custom#",
            "type": "string"
        }));

        registry
            .add("http://example.com/schemas/my-schema", schema)
            .expect("Schema should be accepted")
            .prepare()
            .expect("Schema should accept registered meta-schema URI with trailing '#'");
    }

    #[test]
    fn test_chained_custom_meta_schemas() {
        // Meta-schema B (uses standard Draft 2020-12)
        let meta_schema_b = json!({
            "$id": "json-schema:///meta/level-b",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$vocabulary": {
                "https://json-schema.org/draft/2020-12/vocab/core": true,
                "https://json-schema.org/draft/2020-12/vocab/validation": true,
            },
            "type": "object",
            "properties": {
                "customProperty": {"type": "string"}
            }
        });

        // Meta-schema A (uses Meta-schema B)
        let meta_schema_a = json!({
            "$id": "json-schema:///meta/level-a",
            "$schema": "json-schema:///meta/level-b",
            "customProperty": "level-a-meta",
            "type": "object"
        });

        // Schema (uses Meta-schema A)
        let schema = json!({
            "$id": "json-schema:///schemas/my-schema",
            "$schema": "json-schema:///meta/level-a",
            "customProperty": "my-schema",
            "type": "string"
        });

        // Register all meta-schemas and schema in a chained manner
        // All resources are provided upfront, so no external retrieval should occur
        Registry::new()
            .add(
                "json-schema:///meta/level-b",
                Resource::from_contents(meta_schema_b),
            )
            .expect("Meta-schema should be accepted")
            .add(
                "json-schema:///meta/level-a",
                Resource::from_contents(meta_schema_a),
            )
            .expect("Meta-schema should be accepted")
            .add(
                "json-schema:///schemas/my-schema",
                Resource::from_contents(schema),
            )
            .expect("Schema should be accepted")
            .prepare()
            .expect("Chained custom meta-schemas should be accepted when all are registered");
    }

    struct TestRetriever {
        schemas: AHashMap<String, Value>,
    }

    impl TestRetriever {
        fn new(schemas: AHashMap<String, Value>) -> Self {
            TestRetriever { schemas }
        }
    }

    impl Retrieve for TestRetriever {
        fn retrieve(
            &self,
            uri: &Uri<String>,
        ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
            if let Some(value) = self.schemas.get(uri.as_str()) {
                Ok(value.clone())
            } else {
                Err(format!("Failed to find {uri}").into())
            }
        }
    }

    fn create_test_retriever(schemas: &[(&str, Value)]) -> TestRetriever {
        TestRetriever::new(
            schemas
                .iter()
                .map(|&(k, ref v)| (k.to_string(), v.clone()))
                .collect(),
        )
    }

    #[test]
    fn test_registry_builder_uses_custom_draft() {
        let registry = Registry::new()
            .draft(Draft::Draft4)
            .add("urn:test", json!({}))
            .expect("Resource should be accepted")
            .prepare()
            .expect("Registry should prepare");

        let uri = from_str("urn:test").expect("Invalid test URI");
        assert_eq!(
            registry.resource_by_uri(&uri).unwrap().draft(),
            Draft::Draft4
        );
    }

    #[test]
    fn test_registry_builder_uses_custom_retriever() {
        let registry = Registry::new()
            .retriever(create_test_retriever(&[(
                "http://example.com/remote",
                json!({"type": "string"}),
            )]))
            .add(
                "http://example.com/root",
                json!({"$ref": "http://example.com/remote"}),
            )
            .expect("Resource should be accepted")
            .prepare()
            .expect("Registry should prepare");

        assert!(registry.contains_resource_uri("http://example.com/remote"));
    }

    struct TestCase {
        input_resources: Vec<(&'static str, Value)>,
        remote_resources: Vec<(&'static str, Value)>,
        expected_resolved_uris: Vec<&'static str>,
    }

    #[test_case(
        TestCase {
            input_resources: vec![
                ("http://example.com/schema1", json!({"$ref": "http://example.com/schema2"})),
            ],
            remote_resources: vec![
                ("http://example.com/schema2", json!({"type": "object"})),
            ],
            expected_resolved_uris: vec!["http://example.com/schema1", "http://example.com/schema2"],
        }
    ;"External ref at top")]
    #[test_case(
        TestCase {
            input_resources: vec![
                ("http://example.com/schema1", json!({
                    "$defs": {
                        "subschema": {"type": "string"}
                    },
                    "$ref": "#/$defs/subschema"
                })),
            ],
            remote_resources: vec![],
            expected_resolved_uris: vec!["http://example.com/schema1"],
        }
    ;"Internal ref at top")]
    #[test_case(
        TestCase {
            input_resources: vec![
                ("http://example.com/schema1", json!({"$ref": "http://example.com/schema2"})),
                ("http://example.com/schema2", json!({"type": "object"})),
            ],
            remote_resources: vec![],
            expected_resolved_uris: vec!["http://example.com/schema1", "http://example.com/schema2"],
        }
    ;"Ref to later resource")]
    #[test_case(
    TestCase {
            input_resources: vec![
                ("http://example.com/schema1", json!({
                    "type": "object",
                    "properties": {
                        "prop1": {"$ref": "http://example.com/schema2"}
                    }
                })),
            ],
            remote_resources: vec![
                ("http://example.com/schema2", json!({"type": "string"})),
            ],
            expected_resolved_uris: vec!["http://example.com/schema1", "http://example.com/schema2"],
        }
    ;"External ref in subresource")]
    #[test_case(
        TestCase {
            input_resources: vec![
                ("http://example.com/schema1", json!({
                    "type": "object",
                    "properties": {
                        "prop1": {"$ref": "#/$defs/subschema"}
                    },
                    "$defs": {
                        "subschema": {"type": "string"}
                    }
                })),
            ],
            remote_resources: vec![],
            expected_resolved_uris: vec!["http://example.com/schema1"],
        }
    ;"Internal ref in subresource")]
    #[test_case(
        TestCase {
            input_resources: vec![
                ("file:///schemas/main.json", json!({"$ref": "file:///schemas/external.json"})),
            ],
            remote_resources: vec![
                ("file:///schemas/external.json", json!({"type": "object"})),
            ],
            expected_resolved_uris: vec!["file:///schemas/main.json", "file:///schemas/external.json"],
        }
    ;"File scheme: external ref at top")]
    #[test_case(
        TestCase {
            input_resources: vec![
                ("file:///schemas/main.json", json!({"$ref": "subfolder/schema.json"})),
            ],
            remote_resources: vec![
                ("file:///schemas/subfolder/schema.json", json!({"type": "string"})),
            ],
            expected_resolved_uris: vec!["file:///schemas/main.json", "file:///schemas/subfolder/schema.json"],
        }
    ;"File scheme: relative path ref")]
    #[test_case(
        TestCase {
            input_resources: vec![
                ("file:///schemas/main.json", json!({
                    "type": "object",
                    "properties": {
                        "local": {"$ref": "local.json"},
                        "remote": {"$ref": "http://example.com/schema"}
                    }
                })),
            ],
            remote_resources: vec![
                ("file:///schemas/local.json", json!({"type": "string"})),
                ("http://example.com/schema", json!({"type": "number"})),
            ],
            expected_resolved_uris: vec![
                "file:///schemas/main.json",
                "file:///schemas/local.json",
                "http://example.com/schema"
            ],
        }
    ;"File scheme: mixing with http scheme")]
    #[test_case(
        TestCase {
            input_resources: vec![
                ("file:///C:/schemas/main.json", json!({"$ref": "/D:/other_schemas/schema.json"})),
            ],
            remote_resources: vec![
                ("file:///D:/other_schemas/schema.json", json!({"type": "boolean"})),
            ],
            expected_resolved_uris: vec![
                "file:///C:/schemas/main.json",
                "file:///D:/other_schemas/schema.json"
            ],
        }
    ;"File scheme: absolute path in Windows style")]
    #[test_case(
        TestCase {
            input_resources: vec![
                ("http://example.com/schema1", json!({"$ref": "http://example.com/schema2"})),
            ],
            remote_resources: vec![
                ("http://example.com/schema2", json!({"$ref": "http://example.com/schema3"})),
                ("http://example.com/schema3", json!({"$ref": "http://example.com/schema4"})),
                ("http://example.com/schema4", json!({"$ref": "http://example.com/schema5"})),
                ("http://example.com/schema5", json!({"type": "object"})),
            ],
            expected_resolved_uris: vec![
                "http://example.com/schema1",
                "http://example.com/schema2",
                "http://example.com/schema3",
                "http://example.com/schema4",
                "http://example.com/schema5",
            ],
        }
    ;"Four levels of external references")]
    #[test_case(
        TestCase {
            input_resources: vec![
                ("http://example.com/schema1", json!({"$ref": "http://example.com/schema2"})),
            ],
            remote_resources: vec![
                ("http://example.com/schema2", json!({"$ref": "http://example.com/schema3"})),
                ("http://example.com/schema3", json!({"$ref": "http://example.com/schema4"})),
                ("http://example.com/schema4", json!({"$ref": "http://example.com/schema5"})),
                ("http://example.com/schema5", json!({"$ref": "http://example.com/schema6"})),
                ("http://example.com/schema6", json!({"$ref": "http://example.com/schema1"})),
            ],
            expected_resolved_uris: vec![
                "http://example.com/schema1",
                "http://example.com/schema2",
                "http://example.com/schema3",
                "http://example.com/schema4",
                "http://example.com/schema5",
                "http://example.com/schema6",
            ],
        }
    ;"Five levels of external references with circular reference")]
    fn test_references_processing(test_case: TestCase) {
        let retriever = create_test_retriever(&test_case.remote_resources);

        let input_pairs = test_case
            .input_resources
            .clone()
            .into_iter()
            .map(|(uri, value)| (uri, Resource::from_contents(value)));

        let mut registry = Registry::new().retriever(retriever);
        for (uri, resource) in input_pairs {
            registry = registry.add(uri, resource).expect("Invalid resources");
        }
        let registry = registry.prepare().expect("Invalid resources");
        // Verify that all expected URIs are resolved and present in resources
        for uri in test_case.expected_resolved_uris {
            let resolver = registry.resolver(from_str("").expect("Invalid base URI"));
            assert!(resolver.lookup(uri).is_ok());
        }
    }

    #[test]
    fn test_default_retriever_with_remote_refs() {
        let result = Registry::new()
            .add(
                "http://example.com/schema1",
                Resource::from_contents(json!({"$ref": "http://example.com/schema2"})),
            )
            .expect("Resource should be accepted")
            .prepare();
        let error = result.expect_err("Should fail");
        assert_eq!(error.to_string(), "Resource 'http://example.com/schema2' is not present in a registry and retrieving it failed: Default retriever does not fetch resources");
        assert!(error.source().is_some());
    }

    #[test]
    fn test_registry_new_can_add_and_prepare() {
        let registry = Registry::new()
            .add("urn:test", json!({"type": "string"}))
            .expect("Resource should be accepted")
            .prepare()
            .expect("Registry should prepare");

        assert!(registry.contains_resource_uri("urn:test"));
    }

    #[test]
    fn test_prepared_registry_can_be_extended_via_add() {
        let original = Registry::new()
            .add("urn:one", json!({"type": "string"}))
            .expect("Resource should be accepted")
            .prepare()
            .expect("Registry should prepare");

        let registry = original
            .add("urn:two", json!({"type": "integer"}))
            .expect("Resource should be accepted")
            .prepare()
            .expect("Registry should prepare");

        assert!(original.contains_resource_uri("urn:one"));
        assert!(!original.contains_resource_uri("urn:two"));
        assert!(registry.contains_resource_uri("urn:one"));
        assert!(registry.contains_resource_uri("urn:two"));
    }

    #[test]
    fn test_registry_builder_accepts_borrowed_values() {
        let schema = json!({"type": "string"});
        let registry = Registry::new()
            .add("urn:test", &schema)
            .expect("Resource should be accepted")
            .prepare()
            .expect("Registry should prepare");

        assert!(registry.contains_resource_uri("urn:test"));
    }

    #[test]
    fn test_registry_builder_accepts_borrowed_resources() {
        let schema = Draft::Draft4.create_resource(json!({"type": "string"}));
        let registry = Registry::new()
            .add("urn:test", &schema)
            .expect("Resource should be accepted")
            .prepare()
            .expect("Registry should prepare");

        let uri = from_str("urn:test").expect("Invalid test URI");
        assert_eq!(
            registry.resource_by_uri(&uri).unwrap().draft(),
            Draft::Draft4
        );
    }

    #[test]
    fn test_registry_with_duplicate_input_uris() {
        let registry = Registry::new()
            .add(
                "http://example.com/schema",
                json!({
                    "type": "object",
                    "properties": {
                        "foo": { "type": "string" }
                    }
                }),
            )
            .expect("First resource should be accepted")
            .add(
                "http://example.com/schema",
                json!({
                    "type": "object",
                    "properties": {
                        "bar": { "type": "number" }
                    }
                }),
            )
            .expect("Second resource should overwrite the first")
            .prepare()
            .expect("Registry should prepare");

        let uri = from_str("http://example.com/schema").expect("Invalid schema URI");
        let resource = registry.resource_by_uri(&uri).unwrap();
        let properties = resource
            .contents()
            .get("properties")
            .and_then(|v| v.as_object())
            .unwrap();

        assert!(
            !properties.contains_key("foo"),
            "Registry should replace the earlier explicit input resource"
        );
        assert!(properties.contains_key("bar"));
    }

    #[test]
    fn test_resolver_debug() {
        let registry = SPECIFICATIONS
            .add("http://example.com", json!({}))
            .expect("Invalid resource")
            .prepare()
            .expect("Invalid resource");
        let resolver =
            registry.resolver(from_str("http://127.0.0.1/schema").expect("Invalid base URI"));
        assert_eq!(
            format!("{resolver:?}"),
            "Resolver { base_uri: \"http://127.0.0.1/schema\", scopes: \"[]\" }"
        );
    }

    #[test]
    fn test_prepare_with_specifications_registry() {
        let registry = SPECIFICATIONS
            .add("http://example.com", json!({}))
            .expect("Invalid resource")
            .prepare()
            .expect("Invalid resource");
        let resolver = registry.resolver(from_str("").expect("Invalid base URI"));
        let resolved = resolver
            .lookup("http://json-schema.org/draft-06/schema#/definitions/schemaArray")
            .expect("Lookup failed");
        assert_eq!(
            resolved.contents(),
            &json!({
                "type": "array",
                "minItems": 1,
                "items": { "$ref": "#" }
            })
        );
    }

    #[test]
    fn test_prepare_preserves_existing_local_entries() {
        let original = Registry::new()
            .add(
                "http://example.com/root",
                Resource::from_contents(json!({
                    "$defs": {
                        "embedded": {
                            "$id": "http://example.com/embedded",
                            "type": "string"
                        }
                    }
                })),
            )
            .expect("Invalid root schema")
            .prepare()
            .expect("Invalid root schema");

        let extended = original
            .add(
                "http://example.com/other",
                Resource::from_contents(json!({"type": "number"})),
            )
            .expect("Registry extension should succeed")
            .prepare()
            .expect("Registry extension should succeed");

        let resolver = extended.resolver(from_str("").expect("Invalid base URI"));
        let embedded = resolver
            .lookup("http://example.com/embedded")
            .expect("Embedded subresource URI should stay indexed after extension");
        assert_eq!(
            embedded.contents(),
            &json!({
                "$id": "http://example.com/embedded",
                "type": "string"
            })
        );
    }

    #[test]
    fn test_prepared_registry_can_be_extended_via_extend() {
        let original = Registry::new()
            .add("urn:one", json!({"type": "string"}))
            .expect("Resource should be accepted")
            .prepare()
            .expect("Registry should prepare");

        let registry = original
            .extend([("urn:two", json!({"type": "integer"}))])
            .expect("Resources should be accepted")
            .prepare()
            .expect("Registry should prepare");

        assert!(original.contains_resource_uri("urn:one"));
        assert!(!original.contains_resource_uri("urn:two"));
        assert!(registry.contains_resource_uri("urn:one"));
        assert!(registry.contains_resource_uri("urn:two"));
    }

    #[test]
    fn test_invalid_reference() {
        let resource = Draft::Draft202012.create_resource(json!({"$schema": "$##"}));
        let _ = Registry::new()
            .add("http://#/", resource)
            .and_then(super::RegistryBuilder::prepare);
    }
}

#[cfg(all(test, feature = "retrieve-async"))]
mod async_tests {
    use crate::{uri, DefaultRetriever, Draft, Registry, Resource, Uri};
    use ahash::AHashMap;
    use serde_json::{json, Value};
    use std::{
        error::Error,
        sync::atomic::{AtomicUsize, Ordering},
    };

    struct TestAsyncRetriever {
        schemas: AHashMap<String, Value>,
    }

    impl TestAsyncRetriever {
        fn with_schema(uri: impl Into<String>, schema: Value) -> Self {
            TestAsyncRetriever {
                schemas: { AHashMap::from_iter([(uri.into(), schema)]) },
            }
        }
    }

    #[cfg_attr(target_family = "wasm", async_trait::async_trait(?Send))]
    #[cfg_attr(not(target_family = "wasm"), async_trait::async_trait)]
    impl crate::AsyncRetrieve for TestAsyncRetriever {
        async fn retrieve(
            &self,
            uri: &Uri<String>,
        ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
            self.schemas
                .get(uri.as_str())
                .cloned()
                .ok_or_else(|| "Schema not found".into())
        }
    }

    #[tokio::test]
    async fn test_default_async_retriever_with_remote_refs() {
        let result = Registry::new()
            .async_retriever(DefaultRetriever)
            .add(
                "http://example.com/schema1",
                Resource::from_contents(json!({"$ref": "http://example.com/schema2"})),
            )
            .expect("Resource should be accepted")
            .async_prepare()
            .await;

        let error = result.expect_err("Should fail");
        assert_eq!(error.to_string(), "Resource 'http://example.com/schema2' is not present in a registry and retrieving it failed: Default retriever does not fetch resources");
        assert!(error.source().is_some());
    }

    #[tokio::test]
    async fn test_async_prepare() {
        let _registry = Registry::new()
            .async_retriever(DefaultRetriever)
            .add("", Draft::default().create_resource(json!({})))
            .expect("Invalid resources")
            .async_prepare()
            .await
            .expect("Invalid resources");
    }

    #[tokio::test]
    async fn test_async_registry_with_duplicate_input_uris() {
        let registry = Registry::new()
            .async_retriever(DefaultRetriever)
            .add(
                "http://example.com/schema",
                json!({
                    "type": "object",
                    "properties": {
                        "foo": { "type": "string" }
                    }
                }),
            )
            .expect("First resource should be accepted")
            .add(
                "http://example.com/schema",
                json!({
                    "type": "object",
                    "properties": {
                        "bar": { "type": "number" }
                    }
                }),
            )
            .expect("Second resource should overwrite the first")
            .async_prepare()
            .await
            .expect("Registry should prepare");

        let uri = uri::from_str("http://example.com/schema").expect("Invalid schema URI");
        let resource = registry.resource_by_uri(&uri).unwrap();
        let properties = resource
            .contents()
            .get("properties")
            .and_then(|v| v.as_object())
            .unwrap();

        assert!(
            !properties.contains_key("foo"),
            "Registry should replace the earlier explicit input resource"
        );
        assert!(properties.contains_key("bar"));
    }

    #[tokio::test]
    async fn test_registry_builder_async_prepare_uses_async_retriever() {
        let registry = Registry::new()
            .async_retriever(TestAsyncRetriever::with_schema(
                "http://example.com/schema2",
                json!({"type": "object"}),
            ))
            .add(
                "http://example.com",
                json!({"$ref": "http://example.com/schema2"}),
            )
            .expect("Resource should be accepted")
            .async_prepare()
            .await
            .expect("Registry should prepare");

        let resolver = registry.resolver(uri::from_str("").expect("Invalid base URI"));
        let resolved = resolver
            .lookup("http://example.com/schema2")
            .expect("Lookup failed");
        assert_eq!(resolved.contents(), &json!({"type": "object"}));
    }

    #[tokio::test]
    async fn test_async_prepare_with_remote_resource() {
        let retriever = TestAsyncRetriever::with_schema(
            "http://example.com/schema2",
            json!({"type": "object"}),
        );

        let registry = Registry::new()
            .async_retriever(retriever)
            .add(
                "http://example.com",
                Resource::from_contents(json!({"$ref": "http://example.com/schema2"})),
            )
            .expect("Invalid resource")
            .async_prepare()
            .await
            .expect("Invalid resource");

        let resolver = registry.resolver(uri::from_str("").expect("Invalid base URI"));
        let resolved = resolver
            .lookup("http://example.com/schema2")
            .expect("Lookup failed");
        assert_eq!(resolved.contents(), &json!({"type": "object"}));
    }

    #[tokio::test]
    async fn test_async_prepare_preserves_existing_local_entries() {
        let original = Registry::new()
            .async_retriever(DefaultRetriever)
            .add(
                "http://example.com/root",
                Resource::from_contents(json!({
                    "$defs": {
                        "embedded": {
                            "$id": "http://example.com/embedded",
                            "type": "string"
                        }
                    }
                })),
            )
            .expect("Invalid root schema")
            .async_prepare()
            .await
            .expect("Invalid root schema");

        let extended = original
            .add(
                "http://example.com/other",
                Resource::from_contents(json!({"type": "number"})),
            )
            .expect("Registry extension should succeed")
            .async_prepare()
            .await
            .expect("Registry extension should succeed");

        let resolver = extended.resolver(uri::from_str("").expect("Invalid base URI"));
        let embedded = resolver
            .lookup("http://example.com/embedded")
            .expect("Embedded subresource URI should stay indexed after async extension");
        assert_eq!(
            embedded.contents(),
            &json!({
                "$id": "http://example.com/embedded",
                "type": "string"
            })
        );
    }

    #[tokio::test]
    async fn test_async_registry_with_multiple_refs() {
        let retriever = TestAsyncRetriever {
            schemas: AHashMap::from_iter([
                (
                    "http://example.com/schema2".to_string(),
                    json!({"type": "object"}),
                ),
                (
                    "http://example.com/schema3".to_string(),
                    json!({"type": "string"}),
                ),
            ]),
        };

        let registry = Registry::new()
            .async_retriever(retriever)
            .add(
                "http://example.com/schema1",
                Resource::from_contents(json!({
                    "type": "object",
                    "properties": {
                        "obj": {"$ref": "http://example.com/schema2"},
                        "str": {"$ref": "http://example.com/schema3"}
                    }
                })),
            )
            .expect("Invalid resource")
            .async_prepare()
            .await
            .expect("Invalid resource");

        let resolver = registry.resolver(uri::from_str("").expect("Invalid base URI"));

        // Check both references are resolved correctly
        let resolved2 = resolver
            .lookup("http://example.com/schema2")
            .expect("Lookup failed");
        assert_eq!(resolved2.contents(), &json!({"type": "object"}));

        let resolved3 = resolver
            .lookup("http://example.com/schema3")
            .expect("Lookup failed");
        assert_eq!(resolved3.contents(), &json!({"type": "string"}));
    }

    #[tokio::test]
    async fn test_async_registry_with_nested_refs() {
        let retriever = TestAsyncRetriever {
            schemas: AHashMap::from_iter([
                (
                    "http://example.com/address".to_string(),
                    json!({
                        "type": "object",
                        "properties": {
                            "street": {"type": "string"},
                            "city": {"$ref": "http://example.com/city"}
                        }
                    }),
                ),
                (
                    "http://example.com/city".to_string(),
                    json!({
                        "type": "string",
                        "minLength": 1
                    }),
                ),
            ]),
        };

        let registry = Registry::new()
            .async_retriever(retriever)
            .add(
                "http://example.com/person",
                Resource::from_contents(json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "address": {"$ref": "http://example.com/address"}
                    }
                })),
            )
            .expect("Invalid resource")
            .async_prepare()
            .await
            .expect("Invalid resource");

        let resolver = registry.resolver(uri::from_str("").expect("Invalid base URI"));

        // Verify nested reference resolution
        let resolved = resolver
            .lookup("http://example.com/city")
            .expect("Lookup failed");
        assert_eq!(
            resolved.contents(),
            &json!({"type": "string", "minLength": 1})
        );
    }

    // Multiple refs to the same external schema with different fragments were fetched multiple times in async mode.
    #[tokio::test]
    async fn test_async_registry_with_duplicate_fragment_refs() {
        static FETCH_COUNT: AtomicUsize = AtomicUsize::new(0);

        struct CountingRetriever {
            inner: TestAsyncRetriever,
        }

        #[cfg_attr(target_family = "wasm", async_trait::async_trait(?Send))]
        #[cfg_attr(not(target_family = "wasm"), async_trait::async_trait)]
        impl crate::AsyncRetrieve for CountingRetriever {
            async fn retrieve(
                &self,
                uri: &Uri<String>,
            ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
                FETCH_COUNT.fetch_add(1, Ordering::SeqCst);
                self.inner.retrieve(uri).await
            }
        }

        FETCH_COUNT.store(0, Ordering::SeqCst);

        let retriever = CountingRetriever {
            inner: TestAsyncRetriever::with_schema(
                "http://example.com/external",
                json!({
                    "$defs": {
                        "foo": {
                            "type": "object",
                            "properties": {
                                "nested": { "type": "string" }
                            }
                        },
                        "bar": {
                            "type": "object",
                            "properties": {
                                "value": { "type": "integer" }
                            }
                        }
                    }
                }),
            ),
        };

        // Schema references the same external URL with different fragments
        let registry = Registry::new()
            .async_retriever(retriever)
            .add(
                "http://example.com/main",
                Resource::from_contents(json!({
                    "type": "object",
                    "properties": {
                        "name": { "$ref": "http://example.com/external#/$defs/foo" },
                        "age": { "$ref": "http://example.com/external#/$defs/bar" }
                    }
                })),
            )
            .expect("Invalid resource")
            .async_prepare()
            .await
            .expect("Invalid resource");

        // Should only fetch the external schema once
        let fetches = FETCH_COUNT.load(Ordering::SeqCst);
        assert_eq!(
            fetches, 1,
            "External schema should be fetched only once, but was fetched {fetches} times"
        );

        let resolver =
            registry.resolver(uri::from_str("http://example.com/main").expect("Invalid base URI"));

        // Verify both fragment references resolve correctly
        let foo = resolver
            .lookup("http://example.com/external#/$defs/foo")
            .expect("Lookup failed");
        assert_eq!(
            foo.contents(),
            &json!({
                "type": "object",
                "properties": {
                    "nested": { "type": "string" }
                }
            })
        );

        let bar = resolver
            .lookup("http://example.com/external#/$defs/bar")
            .expect("Lookup failed");
        assert_eq!(
            bar.contents(),
            &json!({
                "type": "object",
                "properties": {
                    "value": { "type": "integer" }
                }
            })
        );
    }
}
