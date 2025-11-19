#[cfg(not(target_family = "wasm"))]
use std::sync::LazyLock;
use std::{borrow::Cow, collections::VecDeque, num::NonZeroUsize, sync::Arc};

use ahash::{AHashMap, AHashSet};
use fluent_uri::Uri;
use serde_json::Value;

use crate::{
    anchors::AnchorKey,
    cache::{SharedUriCache, UriCache},
    meta,
    resource::{unescape_segment, InnerResourcePtr, IntoDocument, JsonSchemaResource},
    uri,
    vocabularies::{self, VocabularySet},
    Anchor, DefaultRetriever, Draft, Error, Resource, Retrieve,
};

type DocumentStore<'doc> = AHashMap<Arc<Uri<String>>, (std::borrow::Cow<'doc, Value>, Draft)>;
pub(crate) type DocumentEntry<'doc> = (Arc<Uri<String>>, (Cow<'doc, Value>, Draft));
pub(crate) type DocumentVec<'doc> = Vec<DocumentEntry<'doc>>;
pub(crate) type ResourceMap = AHashMap<Arc<Uri<String>>, InnerResourcePtr>;

/// Pre-loaded registry containing all JSON Schema meta-schemas and their vocabularies.
pub static SPECIFICATIONS: Specifications = Specifications;

pub struct Specifications;

#[cfg(not(target_family = "wasm"))]
static SPECIFICATIONS_STORAGE: LazyLock<Registry<'static>> =
    LazyLock::new(|| Registry::build_from_meta_schemas(meta::META_SCHEMAS_ALL.as_slice()));

#[cfg(target_family = "wasm")]
thread_local! {
    static SPECIFICATIONS_STORAGE: std::cell::OnceCell<&'static Registry<'static>> = std::cell::OnceCell::new();
}

impl Specifications {
    fn get() -> &'static Registry<'static> {
        #[cfg(not(target_family = "wasm"))]
        {
            &SPECIFICATIONS_STORAGE
        }
        #[cfg(target_family = "wasm")]
        {
            SPECIFICATIONS_STORAGE.with(|cell| {
                cell.get_or_init(move || {
                    Box::leak(Box::new(Registry::build_from_meta_schemas(
                        meta::META_SCHEMAS_ALL.as_slice(),
                    )))
                })
            })
        }
    }
}

impl std::ops::Deref for Specifications {
    type Target = Registry<'static>;

    fn deref(&self) -> &Self::Target {
        Self::get()
    }
}

/// A registry of JSON Schema resources, each identified by their canonical URIs.
///
/// Registries store a collection of in-memory resources and their anchors.
/// They eagerly process all added resources, including their subresources and anchors.
/// This means that subresources contained within any added resources are immediately
/// discoverable and retrievable via their own IDs.
///
/// # Resource Retrieval
///
/// Registry supports both blocking and non-blocking retrieval of external resources.
///
/// ## Blocking Retrieval
///
/// ```rust
/// use referencing::{Registry, Resource, Retrieve, Uri};
/// use serde_json::{json, Value};
///
/// struct ExampleRetriever;
///
/// impl Retrieve for ExampleRetriever {
///     fn retrieve(
///         &self,
///         uri: &Uri<String>
///     ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
///         // Always return the same value for brevity
///         Ok(json!({"type": "string"}))
///     }
/// }
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let registry = Registry::options()
///     .retriever(ExampleRetriever)
///     .build([
///         // Initial schema that might reference external schemas
///         (
///             "https://example.com/user.json",
///             Resource::from_contents(json!({
///                 "type": "object",
///                 "properties": {
///                     // Should be retrieved by `ExampleRetriever`
///                     "role": {"$ref": "https://example.com/role.json"}
///                 }
///             }))
///         )
///     ])?;
/// # Ok(())
/// # }
/// ```
///
/// ## Non-blocking Retrieval
///
/// ```rust
/// # #[cfg(feature = "retrieve-async")]
/// # mod example {
/// use referencing::{Registry, Resource, AsyncRetrieve, Uri};
/// use serde_json::{json, Value};
///
/// struct ExampleRetriever;
///
/// #[cfg_attr(target_family = "wasm", async_trait::async_trait(?Send))]
/// #[cfg_attr(not(target_family = "wasm"), async_trait::async_trait)]
/// impl AsyncRetrieve for ExampleRetriever {
///     async fn retrieve(
///         &self,
///         uri: &Uri<String>
///     ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
///         // Always return the same value for brevity
///         Ok(json!({"type": "string"}))
///     }
/// }
///
///  # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let registry = Registry::options()
///     .async_retriever(ExampleRetriever)
///     .build([
///         (
///             "https://example.com/user.json",
///             Resource::from_contents(json!({
///                 // Should be retrieved by `ExampleRetriever`
///                 "$ref": "https://example.com/common/user.json"
///             }))
///         )
///     ])
///     .await?;
/// # Ok(())
/// # }
/// # }
/// ```
///
/// The registry will automatically:
///
/// - Resolve external references
/// - Cache retrieved schemas
/// - Handle nested references
/// - Process JSON Schema anchors
///
/// Registry stores JSON Schema documents.
///
/// Pure storage - contains only documents. Derived data (resources, anchors)
/// are computed by `ResolutionContext`.
pub struct Registry<'doc> {
    pub(crate) documents: DocumentStore<'doc>,
    pub(crate) resolution_cache: SharedUriCache,
    pub(crate) retriever: Arc<dyn Retrieve>,
    #[cfg(feature = "retrieve-async")]
    pub(crate) async_retriever: Option<Arc<dyn crate::AsyncRetrieve>>,
    pub(crate) draft: Draft,
}

impl std::fmt::Debug for Registry<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("documents", &self.documents)
            .field("resolution_cache", &self.resolution_cache)
            .field("retriever", &"<dyn Retrieve>")
            .field("async_retriever", &{
                #[cfg(feature = "retrieve-async")]
                {
                    if self.async_retriever.is_some() {
                        Some("<dyn AsyncRetrieve>")
                    } else {
                        None
                    }
                }
                #[cfg(not(feature = "retrieve-async"))]
                {
                    Option::<&str>::None
                }
            })
            .field("draft", &self.draft)
            .finish()
    }
}

impl Clone for Registry<'_> {
    fn clone(&self) -> Self {
        Self {
            documents: self.documents.clone(),
            resolution_cache: self.resolution_cache.clone(),
            retriever: Arc::clone(&self.retriever),
            #[cfg(feature = "retrieve-async")]
            async_retriever: self.async_retriever.as_ref().map(Arc::clone),
            draft: self.draft,
        }
    }
}

/// Configuration options for creating a [`Registry`].
pub struct RegistryOptions<R> {
    retriever: R,
    draft: Draft,
}

impl<R> RegistryOptions<R> {
    /// Set specification version under which the resources should be interpreted under.
    #[must_use]
    pub fn draft(mut self, draft: Draft) -> Self {
        self.draft = draft;
        self
    }
}

impl RegistryOptions<Arc<dyn Retrieve>> {
    /// Create a new [`RegistryOptions`] with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            retriever: Arc::new(DefaultRetriever),
            draft: Draft::default(),
        }
    }
    /// Set a custom retriever for the [`Registry`].
    #[must_use]
    pub fn retriever(mut self, retriever: impl IntoRetriever) -> Self {
        self.retriever = retriever.into_retriever();
        self
    }
    /// Set a custom async retriever for the [`Registry`].
    #[cfg(feature = "retrieve-async")]
    #[must_use]
    pub fn async_retriever(
        self,
        retriever: impl IntoAsyncRetriever,
    ) -> RegistryOptions<Arc<dyn crate::AsyncRetrieve>> {
        RegistryOptions {
            retriever: retriever.into_retriever(),
            draft: self.draft,
        }
    }
    /// Create a [`Registry`] from multiple resources using these options.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Any URI is invalid
    /// - Any referenced resources cannot be retrieved
    pub fn build(
        self,
        pairs: impl IntoIterator<Item = (impl AsRef<str>, Resource)>,
    ) -> Result<Registry<'static>, Error> {
        Registry::try_from_resources_with_retriever(pairs, self.retriever, self.draft)
    }
}

#[cfg(feature = "retrieve-async")]
impl RegistryOptions<Arc<dyn crate::AsyncRetrieve>> {
    /// Create a [`Registry`] from multiple resources using these options with async retrieval.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Any URI is invalid
    /// - Any referenced resources cannot be retrieved
    pub async fn build(
        self,
        pairs: impl IntoIterator<Item = (impl AsRef<str>, Resource)>,
    ) -> Result<Registry<'static>, Error> {
        Registry::try_from_resources_async_impl(pairs, Arc::clone(&self.retriever), self.draft)
            .await
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

impl Default for RegistryOptions<Arc<dyn Retrieve>> {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry<'static> {
    /// Create a new [`RegistryBuilder`](crate::RegistryBuilder) for constructing a registry.
    ///
    /// This is the recommended way to create new registries with the builder pattern.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use referencing::Registry;
    /// use serde_json::json;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let schema = json!({"type": "string"});
    ///
    /// let registry = Registry::builder()
    ///     .with_document("https://example.com/schema", &schema)?
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn builder<'doc>() -> crate::RegistryBuilder<'doc> {
        crate::RegistryBuilder::new()
    }
}

impl<'doc> Registry<'doc> {
    /// Get an iterator over the documents in this registry.
    ///
    /// Returns an iterator of `(&Uri, &(Cow<Value>, Draft))` pairs.
    #[must_use = "iterating documents is side-effect free; consume the iterator to observe data"]
    pub fn documents(
        &self,
    ) -> impl Iterator<
        Item = (
            &Arc<crate::Uri<String>>,
            &(std::borrow::Cow<'doc, Value>, Draft),
        ),
    > {
        self.documents.iter()
    }

    /// Get [`RegistryOptions`] for configuring a new [`Registry`].
    #[must_use]
    pub fn options() -> RegistryOptions<Arc<dyn Retrieve>> {
        RegistryOptions::new()
    }
    /// Create a new [`Registry`] with a single resource.
    ///
    /// # Arguments
    ///
    /// * `uri` - The URI of the resource.
    /// * `resource` - The resource to add.
    ///
    /// # Errors
    ///
    /// Returns an error if the URI is invalid or if there's an issue processing the resource.
    pub fn try_new<S, D>(uri: S, document: D) -> Result<Self, Error>
    where
        S: AsRef<str>,
        D: IntoDocument<'doc>,
    {
        Registry::builder()
            .with_document(uri.as_ref(), document)?
            .build()
    }
    /// Create a new [`Registry`] from an iterator of (URI, Resource) pairs.
    ///
    /// # Arguments
    ///
    /// * `pairs` - An iterator of (URI, Resource) pairs.
    ///
    /// # Errors
    ///
    /// Returns an error if any URI is invalid or if there's an issue processing the resources.
    pub fn try_from_resources<S, D, I>(pairs: I) -> Result<Self, Error>
    where
        I: IntoIterator<Item = (S, D)>,
        S: AsRef<str>,
        D: IntoDocument<'doc>,
    {
        Self::try_from_resources_impl(pairs, Draft::default())
    }
    fn try_from_resources_impl<S, D, I>(pairs: I, draft: Draft) -> Result<Self, Error>
    where
        I: IntoIterator<Item = (S, D)>,
        S: AsRef<str>,
        D: IntoDocument<'doc>,
    {
        Self::try_from_documents_with_retriever(pairs, Arc::new(DefaultRetriever), draft)
    }

    fn try_from_resources_with_retriever<S, D, I>(
        pairs: I,
        retriever: Arc<dyn Retrieve>,
        draft: Draft,
    ) -> Result<Self, Error>
    where
        I: IntoIterator<Item = (S, D)>,
        S: AsRef<str>,
        D: IntoDocument<'doc>,
    {
        Self::try_from_documents_with_retriever(pairs, retriever, draft)
    }
    /// Create a new [`Registry`] from an iterator of (URI, Resource) pairs using an async retriever.
    ///
    /// # Arguments
    ///
    /// * `pairs` - An iterator of (URI, Resource) pairs.
    ///
    /// # Errors
    ///
    /// Returns an error if any URI is invalid or if there's an issue processing the resources.
    #[cfg(feature = "retrieve-async")]
    async fn try_from_resources_async_impl<S, D, I>(
        pairs: I,
        retriever: Arc<dyn crate::AsyncRetrieve>,
        draft: Draft,
    ) -> Result<Self, Error>
    where
        I: IntoIterator<Item = (S, D)>,
        S: AsRef<str>,
        D: IntoDocument<'doc>,
    {
        Self::try_from_documents_async_impl(pairs, retriever, draft).await
    }

    fn try_from_documents_with_retriever<S, D, I>(
        pairs: I,
        retriever: Arc<dyn Retrieve>,
        draft: Draft,
    ) -> Result<Self, Error>
    where
        I: IntoIterator<Item = (S, D)>,
        S: AsRef<str>,
        D: IntoDocument<'doc>,
    {
        let converted = collect_documents(pairs)?;
        crate::builder::build_registry_with_retriever(converted, retriever, draft)
    }

    #[cfg(feature = "retrieve-async")]
    async fn try_from_documents_async_impl<S, D, I>(
        pairs: I,
        retriever: Arc<dyn crate::AsyncRetrieve>,
        draft: Draft,
    ) -> Result<Self, Error>
    where
        I: IntoIterator<Item = (S, D)>,
        S: AsRef<str>,
        D: IntoDocument<'doc>,
    {
        let converted = collect_documents(pairs)?;
        crate::builder::build_registry_with_async_retriever(converted, retriever, draft).await
    }
    /// Create a resolution context for this registry.
    ///
    /// The context can be used to create resolvers via `context.try_resolver()` or to add
    /// a root document for compilation via `context.with_root_document()`.
    #[must_use]
    pub fn context(&self) -> crate::ResolutionContext<'_> {
        crate::ResolutionContext::new(self)
    }

    // Note: We cannot provide a try_resolver convenience method because
    // the Resolver needs to hold a reference to the ResolutionContext.
    // Users must call registry.context().try_resolver(base_uri) instead.

    /// Create a new registry with an additional resource.
    ///
    /// This consumes the current registry and returns a new one with the resource added.
    ///
    /// # Errors
    ///
    /// Returns an error if the URI is invalid or there's an issue processing the resource.
    pub fn try_with_resource<S, D>(self, uri: S, document: D) -> Result<Self, Error>
    where
        S: AsRef<str>,
        D: IntoDocument<'doc>,
    {
        self.try_with_resources([(uri, document)])
    }

    /// Create a new registry with multiple additional resources.
    ///
    /// This consumes the current registry and returns a new one with the resources added.
    ///
    /// # Errors
    ///
    /// Returns an error if any URI is invalid or there's an issue processing the resources.
    pub fn try_with_resources<S, D, I>(self, pairs: I) -> Result<Self, Error>
    where
        I: IntoIterator<Item = (S, D)>,
        S: AsRef<str>,
        D: IntoDocument<'doc>,
    {
        self.rebuild_with_resources(pairs)
    }

    /// Create a new registry with an additional resource using async retrieval.
    ///
    /// # Errors
    ///
    /// Returns an error if the URI is invalid or retrieving referenced resources fails.
    #[cfg(feature = "retrieve-async")]
    pub async fn try_with_resource_async<S, D>(self, uri: S, document: D) -> Result<Self, Error>
    where
        S: AsRef<str>,
        D: IntoDocument<'doc>,
    {
        self.try_with_resources_async([(uri, document)]).await
    }

    /// Create a new registry with multiple resources using async retrieval.
    ///
    /// # Errors
    ///
    /// Returns an error if any URI is invalid or fetching their references fails.
    #[cfg(feature = "retrieve-async")]
    pub async fn try_with_resources_async<S, D, I>(self, pairs: I) -> Result<Self, Error>
    where
        I: IntoIterator<Item = (S, D)>,
        S: AsRef<str>,
        D: IntoDocument<'doc>,
    {
        self.rebuild_with_resources_async(pairs).await
    }

    fn rebuild_with_resources<S, D, I>(self, pairs: I) -> Result<Self, Error>
    where
        I: IntoIterator<Item = (S, D)>,
        S: AsRef<str>,
        D: IntoDocument<'doc>,
    {
        let Registry {
            documents,
            resolution_cache: _,
            retriever,
            #[cfg(feature = "retrieve-async")]
                async_retriever: _,
            draft,
        } = self;

        let mut all_documents: Vec<_> = documents.into_iter().collect();
        let new_documents = collect_documents(pairs)?;
        all_documents.extend(new_documents);

        crate::builder::build_registry_with_retriever(all_documents, retriever, draft)
    }

    #[cfg(feature = "retrieve-async")]
    async fn rebuild_with_resources_async<S, D, I>(self, pairs: I) -> Result<Self, Error>
    where
        I: IntoIterator<Item = (S, D)>,
        S: AsRef<str>,
        D: IntoDocument<'doc>,
    {
        let Registry {
            documents,
            resolution_cache: _,
            retriever,
            async_retriever,
            draft,
        } = self;

        let mut all_documents: Vec<_> = documents.into_iter().collect();
        let new_documents = collect_documents(pairs)?;
        all_documents.extend(new_documents);

        if let Some(async_retriever) = async_retriever {
            crate::builder::build_registry_with_async_retriever(
                all_documents,
                async_retriever,
                draft,
            )
            .await
        } else {
            crate::builder::build_registry_with_retriever(all_documents, retriever, draft)
        }
    }
    /// Resolves a reference URI against a base URI using registry's cache.
    ///
    /// # Errors
    ///
    /// Returns an error if base has not schema or there is a fragment.
    pub fn resolve_against(&self, base: &Uri<&str>, uri: &str) -> Result<Arc<Uri<String>>, Error> {
        self.resolution_cache.resolve_against(base, uri)
    }
    /// Returns vocabulary set configured for given draft and contents.
    ///
    /// For custom meta-schemas (`Draft::Unknown`), looks up the meta-schema in the registry
    /// and extracts its `$vocabulary` declaration. If the meta-schema is not registered,
    /// returns the default Draft 2020-12 vocabularies.
    #[must_use]
    pub fn find_vocabularies(&self, draft: Draft, contents: &Value) -> VocabularySet {
        match draft.detect(contents) {
            Draft::Unknown => {
                // Custom/unknown meta-schema - try to look it up in the registry
                if let Some(specification) = contents
                    .as_object()
                    .and_then(|obj| obj.get("$schema"))
                    .and_then(|s| s.as_str())
                {
                    if let Ok(mut uri) = uri::from_str(specification) {
                        // Remove fragment for lookup (e.g., "http://example.com/schema#" -> "http://example.com/schema")
                        // Documents are stored without fragments, so we must strip it to find the meta-schema
                        uri.set_fragment(None);
                        if let Some((doc_value, _)) = self.documents.get(&uri) {
                            // Found the custom meta-schema - extract vocabularies
                            if let Ok(Some(vocabularies)) = vocabularies::find(doc_value.as_ref()) {
                                return vocabularies;
                            }
                        }
                        // Meta-schema not registered - this will be caught during compilation
                        // For now, return default vocabularies to allow resource creation
                    }
                }
                // Default to Draft 2020-12 vocabularies for unknown meta-schemas
                Draft::Unknown.default_vocabularies()
            }
            draft => draft.default_vocabularies(),
        }
    }

    /// Build a registry with all the given meta-schemas from specs.
    pub(crate) fn build_from_meta_schemas(schemas: &[(&'static str, &'static Value)]) -> Self {
        let schemas_count = schemas.len();
        let mut documents = DocumentStore::with_capacity(schemas_count);
        let resolution_cache = UriCache::with_capacity(35);

        for (uri_str, schema) in schemas {
            let uri =
                uri::from_str(uri_str.trim_end_matches('#')).expect("Invalid URI in meta-schema");
            let draft = Draft::default().detect(schema);
            documents.insert(Arc::new(uri), (std::borrow::Cow::Borrowed(*schema), draft));
        }

        Self {
            documents,
            resolution_cache: resolution_cache.into_shared(),
            retriever: Arc::new(DefaultRetriever),
            #[cfg(feature = "retrieve-async")]
            async_retriever: None,
            draft: Draft::default(),
        }
    }
}

#[derive(Hash, Eq, PartialEq)]
pub(crate) struct ReferenceKey {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ReferenceKind {
    Ref,
    Schema,
}

pub(crate) struct ProcessingState {
    pub(crate) queue: VecDeque<(Arc<Uri<String>>, InnerResourcePtr)>,
    pub(crate) seen: ReferenceTracker,
    pub(crate) external: AHashSet<(String, Uri<String>, ReferenceKind, Draft)>,
    pub(crate) scratch: String,
    pub(crate) refers_metaschemas: bool,
    pub(crate) custom_metaschemas: Vec<Arc<Uri<String>>>,
}

impl ProcessingState {
    pub(crate) fn new() -> Self {
        Self {
            queue: VecDeque::with_capacity(32),
            seen: ReferenceTracker::new(),
            external: AHashSet::new(),
            scratch: String::new(),
            refers_metaschemas: false,
            custom_metaschemas: Vec::new(),
        }
    }
}

pub(crate) fn process_queue(
    state: &mut ProcessingState,
    resources: &mut ResourceMap,
    anchors: &mut AHashMap<AnchorKey, Anchor>,
    resolution_cache: &mut UriCache,
) -> Result<(), Error> {
    while let Some((mut base, resource)) = state.queue.pop_front() {
        if let Some(id) = resource.id() {
            base = resolution_cache.resolve_against(&base.borrow(), id)?;
            resources.insert(base.clone(), resource.clone());
        }

        for anchor in resource.anchors() {
            anchors.insert(AnchorKey::new(base.clone(), anchor.name()), anchor);
        }

        collect_external_resources(
            &base,
            resource.contents(),
            resource.draft(),
            &mut state.external,
            &mut state.seen,
            resolution_cache,
            &mut state.scratch,
            &mut state.refers_metaschemas,
        )?;

        for contents in resource.draft().subresources_of(resource.contents()) {
            let subresource = InnerResourcePtr::new(contents, resource.draft());
            state.queue.push_back((base.clone(), subresource));
        }
    }
    Ok(())
}

pub(crate) fn handle_fragment(
    uri: &Uri<String>,
    resource: &InnerResourcePtr,
    key: &Arc<Uri<String>>,
    default_draft: Draft,
    queue: &mut VecDeque<(Arc<Uri<String>>, InnerResourcePtr)>,
) {
    if let Some(fragment) = uri.fragment() {
        if let Some(resolved) = pointer(resource.contents(), fragment.as_str()) {
            let draft = default_draft.detect(resolved);
            let contents = std::ptr::addr_of!(*resolved);
            let resource = InnerResourcePtr::new(contents, draft);
            queue.push_back((Arc::clone(key), resource));
        }
    }
}

// Removed: handle_metaschemas is no longer needed since resources/anchors
// are computed by ResolutionContext, not stored in Registry

pub(crate) fn create_resource(
    retrieved: Value,
    fragmentless: Uri<String>,
    default_draft: Draft,
    documents: &mut DocumentStore<'_>,
    resources: &mut ResourceMap,
    custom_metaschemas: &mut Vec<Arc<Uri<String>>>,
) -> (Arc<Uri<String>>, InnerResourcePtr) {
    let draft = default_draft.detect(&retrieved);
    let key = Arc::new(fragmentless);

    // Store as Cow::Owned first
    documents.insert(
        Arc::clone(&key),
        (std::borrow::Cow::Owned(retrieved), draft),
    );

    // Get pointer to the stored value
    let stored_value = match &documents[&key].0 {
        std::borrow::Cow::Owned(v) => v as *const Value,
        std::borrow::Cow::Borrowed(v) => *v as *const Value,
    };
    let resource = InnerResourcePtr::new(stored_value, draft);
    resources.insert(Arc::clone(&key), resource.clone());

    // Track resources with custom meta-schemas for later validation
    if draft == Draft::Unknown {
        custom_metaschemas.push(Arc::clone(&key));
    }

    (key, resource)
}

fn collect_documents<'doc, I, S, D>(pairs: I) -> Result<DocumentVec<'doc>, Error>
where
    I: IntoIterator<Item = (S, D)>,
    S: AsRef<str>,
    D: IntoDocument<'doc>,
{
    pairs
        .into_iter()
        .map(|(uri_str, document)| {
            let uri_str = uri_str.as_ref().trim_end_matches('#');
            let uri = uri::from_str(uri_str)?;
            let (cow, draft) = document.into_document();
            Ok((Arc::new(uri), (cow, draft)))
        })
        .collect()
}

pub(crate) fn handle_retrieve_error(
    uri: &Uri<String>,
    original: &str,
    fragmentless: &Uri<String>,
    error: Box<dyn std::error::Error + Send + Sync>,
    kind: ReferenceKind,
) -> Result<(), Error> {
    match kind {
        ReferenceKind::Schema => {
            // $schema fetch failures are non-fatal during resource processing
            // Unregistered custom meta-schemas will be caught in validate_custom_metaschemas()
            Ok(())
        }
        ReferenceKind::Ref => {
            // $ref fetch failures are fatal - they're required for validation
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

pub(crate) fn validate_custom_metaschemas(
    custom_metaschemas: &[Arc<Uri<String>>],
    resources: &ResourceMap,
) -> Result<(), Error> {
    // Only validate resources with Draft::Unknown
    for uri in custom_metaschemas {
        if let Some(resource) = resources.get(uri) {
            // Extract the $schema value from this resource
            if let Some(schema_uri) = resource
                .contents()
                .as_object()
                .and_then(|obj| obj.get("$schema"))
                .and_then(|s| s.as_str())
            {
                // Check if this meta-schema is registered
                match uri::from_str(schema_uri) {
                    Ok(mut meta_uri) => {
                        // Remove fragment for lookup (e.g., "http://example.com/schema#" -> "http://example.com/schema")
                        meta_uri.set_fragment(None);
                        if !resources.contains_key(&meta_uri) {
                            return Err(Error::unknown_specification(schema_uri));
                        }
                    }
                    Err(_) => {
                        return Err(Error::unknown_specification(schema_uri));
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn collect_external_resources(
    base: &Arc<Uri<String>>,
    contents: &Value,
    draft: Draft,
    collected: &mut AHashSet<(String, Uri<String>, ReferenceKind, Draft)>,
    seen: &mut ReferenceTracker,
    resolution_cache: &mut UriCache,
    scratch: &mut String,
    refers_metaschemas: &mut bool,
) -> Result<(), Error> {
    // URN schemes are not supported for external resolution
    if base.scheme().as_str() == "urn" {
        return Ok(());
    }

    macro_rules! on_reference {
        ($reference:expr, $key:literal) => {
            // Skip well-known schema references
            if $reference.starts_with("https://json-schema.org/draft/")
                || $reference.starts_with("http://json-schema.org/draft-")
                || base.as_str().starts_with("https://json-schema.org/draft/")
            {
                if $key == "$ref" {
                    *refers_metaschemas = true;
                }
            } else if $reference != "#" {
                if mark_reference(seen, base, $reference) {
                    // Handle local references separately as they may have nested references to external resources
                    if $reference.starts_with('#') {
                        if let Some(referenced) =
                            pointer(contents, $reference.trim_start_matches('#'))
                        {
                            collect_external_resources(
                                base,
                                referenced,
                                draft,
                                collected,
                                seen,
                                resolution_cache,
                                scratch,
                                refers_metaschemas,
                            )?;
                        }
                    } else {
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
                            // Add the fragment back if present
                            if let Some(fragment) = fragment {
                                // It is cheaper to check if it is properly encoded than allocate given that
                                // the majority of inputs do not need to be additionally encoded
                                if let Some(encoded) = uri::EncodedString::new(fragment) {
                                    resolved = resolved.with_fragment(Some(encoded));
                                } else {
                                    uri::encode_to(fragment, scratch);
                                    resolved = resolved.with_fragment(Some(
                                        uri::EncodedString::new_or_panic(scratch),
                                    ));
                                    scratch.clear();
                                }
                            }
                            resolved
                        } else {
                            (*resolution_cache
                                .resolve_against(&base.borrow(), $reference)?)
                            .clone()
                        };

                        let kind = if $key == "$schema" {
                            ReferenceKind::Schema
                        } else {
                            ReferenceKind::Ref
                        };
                        collected.insert(($reference.to_string(), resolved, kind, draft));
                    }
                }
            }
        };
    }

    if let Some(object) = contents.as_object() {
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

fn mark_reference(seen: &mut ReferenceTracker, base: &Arc<Uri<String>>, reference: &str) -> bool {
    seen.insert(ReferenceKey::new(base, reference))
}

/// Look up a value by a JSON Pointer.
///
/// **NOTE**: A slightly faster version of pointer resolution based on `Value::pointer` from `serde_json`.
pub fn pointer<'a>(document: &'a Value, pointer: &str) -> Option<&'a Value> {
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
    use std::error::Error as _;

    use ahash::AHashMap;
    use fluent_uri::Uri;
    use serde_json::{json, Value};
    use test_case::test_case;

    use crate::{uri::from_str, Draft, Registry, Resource, Retrieve};

    use super::{pointer, RegistryOptions, SPECIFICATIONS};

    #[test]
    fn test_empty_pointer() {
        let document = json!({});
        assert_eq!(pointer(&document, ""), Some(&document));
    }

    #[test]
    fn test_invalid_uri_on_registry_creation() {
        let schema = Draft::Draft202012.create_resource(json!({}));
        let result = Registry::try_new(":/example.com", schema);
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
        let registry =
            Registry::try_new("http://example.com/schema1", schema).expect("Invalid resources");

        // Attempt to create a resolver for a URL not in the registry
        let context = registry.context();
        let resolver = context
            .try_resolver("http://example.com/non_existent_schema")
            .expect("Invalid base URI");

        let result = resolver.lookup("");

        assert_eq!(
            result.unwrap_err().to_string(),
            "Resource 'http://example.com/non_existent_schema' is not present in a registry and retrieving it failed: Retrieving external resources is not supported once the registry is populated"
        );
    }

    #[test]
    fn test_relative_uri_without_base() {
        let schema = Draft::Draft202012.create_resource(json!({"$ref": "./virtualNetwork.json"}));
        let error = Registry::try_new("json-schema:///", schema).expect_err("Should fail");
        assert_eq!(error.to_string(), "Resource './virtualNetwork.json' is not present in a registry and retrieving it failed: No base URI is available");
    }

    #[test]
    fn test_try_with_resources_requires_registered_custom_meta_schema() {
        let base_registry = Registry::try_new(
            "http://example.com/root",
            Resource::from_contents(json!({"type": "object"})),
        )
        .expect("Base registry should be created");

        let custom_schema = Resource::from_contents(json!({
            "$id": "http://example.com/custom",
            "$schema": "http://example.com/meta/custom",
            "type": "string"
        }));

        let error = base_registry
            .try_with_resources([("http://example.com/custom", custom_schema)])
            .expect_err("Extending registry must fail when the custom $schema is not registered");

        let error_msg = error.to_string();
        assert_eq!(
            error_msg,
            "Unknown meta-schema: 'http://example.com/meta/custom'. Custom meta-schemas must be registered in the registry before use"
        );
    }

    #[test]
    fn test_try_with_resources_accepts_registered_custom_meta_schema_fragment() {
        let meta_schema = Resource::from_contents(json!({
            "$id": "http://example.com/meta/custom#",
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object"
        }));

        let registry = Registry::try_new("http://example.com/meta/custom#", meta_schema)
            .expect("Meta-schema should be registered successfully");

        let schema = Resource::from_contents(json!({
            "$id": "http://example.com/schemas/my-schema",
            "$schema": "http://example.com/meta/custom#",
            "type": "string"
        }));

        registry
            .clone()
            .try_with_resources([("http://example.com/schemas/my-schema", schema)])
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
        Registry::try_from_resources([
            (
                "json-schema:///meta/level-b",
                Resource::from_contents(meta_schema_b),
            ),
            (
                "json-schema:///meta/level-a",
                Resource::from_contents(meta_schema_a),
            ),
            (
                "json-schema:///schemas/my-schema",
                Resource::from_contents(schema),
            ),
        ])
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

        let registry = Registry::options()
            .retriever(retriever)
            .build(input_pairs)
            .expect("Invalid resources");
        // Verify that all expected URIs are resolved and present in resources
        let context = registry.context();
        for uri in test_case.expected_resolved_uris {
            let resolver = context.try_resolver("").expect("Invalid base URI");
            assert!(resolver.lookup(uri).is_ok());
        }
    }

    #[test]
    fn test_default_retriever_with_remote_refs() {
        let result = Registry::try_from_resources([(
            "http://example.com/schema1",
            Resource::from_contents(json!({"$ref": "http://example.com/schema2"})),
        )]);
        let error = result.expect_err("Should fail");
        assert_eq!(error.to_string(), "Resource 'http://example.com/schema2' is not present in a registry and retrieving it failed: Default retriever does not fetch resources");
        assert!(error.source().is_some());
    }

    #[test]
    fn test_options() {
        let _registry = RegistryOptions::default()
            .build([("", Resource::from_contents(json!({})))])
            .expect("Invalid resources");
    }

    #[test]
    fn test_registry_with_duplicate_input_uris() {
        let input_resources = vec![
            (
                "http://example.com/schema",
                json!({
                    "type": "object",
                    "properties": {
                        "foo": { "type": "string" }
                    }
                }),
            ),
            (
                "http://example.com/schema",
                json!({
                    "type": "object",
                    "properties": {
                        "bar": { "type": "number" }
                    }
                }),
            ),
        ];

        let result = Registry::try_from_resources(
            input_resources
                .into_iter()
                .map(|(uri, value)| (uri, Draft::Draft202012.create_resource(value))),
        );

        assert!(
            result.is_ok(),
            "Failed to create registry with duplicate input URIs"
        );
        let registry = result.unwrap();

        // Get the resource via context
        let context = registry.context();
        let uri = from_str("http://example.com/schema").expect("Invalid URI");
        let resource = context.get_resource(&uri).unwrap();
        let properties = resource
            .contents()
            .get("properties")
            .and_then(|v| v.as_object())
            .unwrap();

        assert!(
            !properties.contains_key("bar"),
            "Registry should contain the earliest added schema"
        );
        assert!(
            properties.contains_key("foo"),
            "Registry should contain the overwritten schema"
        );
    }

    #[test]
    fn test_resolver_debug() {
        let registry = SPECIFICATIONS
            .clone()
            .try_with_resource("http://example.com", Resource::from_contents(json!({})))
            .expect("Invalid resource");
        let context = registry.context();
        let resolver = context
            .try_resolver("http://127.0.0.1/schema")
            .expect("Invalid base URI");
        assert_eq!(
            format!("{resolver:?}"),
            "Resolver { base_uri: \"http://127.0.0.1/schema\", scopes: \"[]\" }"
        );
    }

    #[test]
    fn test_try_with_resource() {
        let registry = SPECIFICATIONS
            .clone()
            .try_with_resource("http://example.com", Resource::from_contents(json!({})))
            .expect("Invalid resource");
        let context = registry.context();
        let resolver = context.try_resolver("").expect("Invalid base URI");
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
    fn root_document_registers_nested_ids() {
        let registry = Registry::builder()
            .build()
            .expect("Failed to build empty registry");
        let schema = json!({
            "$id": "https://example.com/root",
            "$defs": {
                "Foo": {
                    "$id": "Foo",
                    "type": "string"
                }
            }
        });
        let base_uri = from_str("https://example.com/root").expect("Invalid URI");
        let context = registry
            .context()
            .with_root_document(base_uri.clone(), &schema, Draft::Draft202012)
            .expect("Root document should be accepted");
        let resolver = context.resolver(base_uri.clone());
        let resolved = resolver
            .lookup("https://example.com/Foo")
            .expect("Nested $id should resolve");
        assert_eq!(
            resolved.contents(),
            schema
                .pointer("/$defs/Foo")
                .expect("Missing $defs.Foo definition")
        );
    }

    #[derive(Default)]
    struct MapRetriever {
        schemas: AHashMap<String, Value>,
    }

    impl Retrieve for MapRetriever {
        fn retrieve(
            &self,
            uri: &Uri<String>,
        ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
            if let Some(value) = self.schemas.get(uri.as_str()) {
                Ok(value.clone())
            } else {
                Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Missing {uri}"),
                )))
            }
        }
    }

    #[test]
    fn try_with_resource_fetches_external_dependencies() {
        let mut retriever = MapRetriever::default();
        retriever.schemas.insert(
            "http://example.com/external".to_string(),
            json!({"type": "string"}),
        );

        let registry = Registry::builder()
            .with_document("http://example.com/base", json!({}))
            .expect("Failed to add base document")
            .with_retriever(retriever)
            .build()
            .expect("Failed to build registry");

        let registry = registry
            .try_with_resource(
                "http://example.com/new",
                Resource::from_contents(json!({"$ref": "http://example.com/external"})),
            )
            .expect("Failed to extend registry");

        let context = registry.context();
        let resolver = context
            .try_resolver("http://example.com/new")
            .expect("Invalid base URI");
        let resolved = resolver
            .lookup("http://example.com/external")
            .expect("External reference should resolve");
        assert_eq!(resolved.contents(), &json!({"type": "string"}));
    }

    #[test]
    fn builder_preserves_borrowed_documents() {
        let schema = json!({
            "$id": "https://example.com/root",
            "$defs": {
                "Foo": { "type": "string" }
            },
            "$ref": "#/$defs/Foo"
        });

        let registry = Registry::builder()
            .with_document("https://example.com/root", (&schema, Draft::Draft202012))
            .expect("Failed to add document")
            .build()
            .expect("Failed to build registry");

        let context = registry.context();
        let uri = from_str("https://example.com/root").expect("Invalid URI");
        let resource = context
            .get_resource(&uri)
            .expect("Resource should exist for root document");
        assert!(
            resource.contents().pointer("/$defs/Foo").is_some(),
            "Borrowed schema lost definitions"
        );
    }

    #[test]
    fn test_invalid_reference() {
        // Found via fuzzing
        let resource = Draft::Draft202012.create_resource(json!({"$schema": "$##"}));
        let _ = Registry::try_new("http://#/", resource);
    }
}

#[cfg(all(test, feature = "retrieve-async"))]
mod async_tests {
    use crate::{uri, DefaultRetriever, Draft, Registry, Resource, Uri};
    use ahash::AHashMap;
    use serde_json::{json, Value};
    use std::error::Error;

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
        let result = Registry::options()
            .async_retriever(DefaultRetriever)
            .build([(
                "http://example.com/schema1",
                Resource::from_contents(json!({"$ref": "http://example.com/schema2"})),
            )])
            .await;

        let error = result.expect_err("Should fail");
        assert_eq!(error.to_string(), "Resource 'http://example.com/schema2' is not present in a registry and retrieving it failed: Default retriever does not fetch resources");
        assert!(error.source().is_some());
    }

    #[tokio::test]
    async fn test_async_options() {
        let _registry = Registry::options()
            .async_retriever(DefaultRetriever)
            .build([("", Draft::default().create_resource(json!({})))])
            .await
            .expect("Invalid resources");
    }

    #[tokio::test]
    async fn test_async_registry_with_duplicate_input_uris() {
        let input_resources = vec![
            (
                "http://example.com/schema",
                json!({
                    "type": "object",
                    "properties": {
                        "foo": { "type": "string" }
                    }
                }),
            ),
            (
                "http://example.com/schema",
                json!({
                    "type": "object",
                    "properties": {
                        "bar": { "type": "number" }
                    }
                }),
            ),
        ];

        let result = Registry::options()
            .async_retriever(DefaultRetriever)
            .build(
                input_resources
                    .into_iter()
                    .map(|(uri, value)| (uri, Draft::Draft202012.create_resource(value))),
            )
            .await;

        assert!(
            result.is_ok(),
            "Failed to create registry with duplicate input URIs"
        );
        let registry = result.unwrap();

        let uri = uri::from_str("http://example.com/schema").expect("Invalid URI");
        let (document, _) = registry
            .documents
            .get(&uri)
            .expect("Document should be registered");
        let properties = document
            .as_ref()
            .get("properties")
            .and_then(|v| v.as_object())
            .unwrap();

        assert!(
            !properties.contains_key("bar"),
            "Registry should contain the earliest added schema"
        );
        assert!(
            properties.contains_key("foo"),
            "Registry should contain the overwritten schema"
        );
    }

    #[tokio::test]
    async fn test_async_try_with_resource() {
        let retriever = TestAsyncRetriever::with_schema(
            "http://example.com/schema2",
            json!({"type": "object"}),
        );

        let registry = Registry::options()
            .async_retriever(retriever)
            .build([(
                "http://example.com",
                Resource::from_contents(json!({"$ref": "http://example.com/schema2"})),
            )])
            .await
            .expect("Invalid resource");

        let registry = registry
            .try_with_resource_async(
                "http://example.com/new",
                Resource::from_contents(json!({"$ref": "http://example.com/schema2"})),
            )
            .await
            .expect("Failed to extend registry");

        let context = registry.context();
        let resolver = context
            .try_resolver("http://example.com/new")
            .expect("Invalid base URI");
        let resolved = resolver
            .lookup("http://example.com/schema2")
            .expect("Lookup failed");
        assert_eq!(resolved.contents(), &json!({"type": "object"}));
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

        let registry = Registry::options()
            .async_retriever(retriever)
            .build([(
                "http://example.com/schema1",
                Resource::from_contents(json!({
                    "type": "object",
                    "properties": {
                        "obj": {"$ref": "http://example.com/schema2"},
                        "str": {"$ref": "http://example.com/schema3"}
                    }
                })),
            )])
            .await
            .expect("Invalid resource");

        let context = registry.context();
        let resolver = context.try_resolver("").expect("Invalid base URI");

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

        let registry = Registry::options()
            .async_retriever(retriever)
            .build([(
                "http://example.com/person",
                Resource::from_contents(json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "address": {"$ref": "http://example.com/address"}
                    }
                })),
            )])
            .await
            .expect("Invalid resource");

        let context = registry.context();
        let resolver = context.try_resolver("").expect("Invalid base URI");

        // Verify nested reference resolution
        let resolved = resolver
            .lookup("http://example.com/city")
            .expect("Lookup failed");
        assert_eq!(
            resolved.contents(),
            &json!({"type": "string", "minLength": 1})
        );
    }
}
