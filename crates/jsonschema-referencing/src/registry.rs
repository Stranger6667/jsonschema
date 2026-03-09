use std::{
    collections::{hash_map::Entry, VecDeque},
    num::NonZeroUsize,
    sync::{Arc, LazyLock},
};

use ahash::{AHashMap, AHashSet};
use fluent_uri::{pct_enc::EStr, Uri};
use serde_json::Value;

use crate::{
    cache::{SharedUriCache, UriCache},
    index::{AnchorMap, ResourceMap},
    meta::{self, metas_for_draft},
    resource::{unescape_segment, PathStack},
    uri, DefaultRetriever, Draft, Error, Index, Resource, ResourceRef, Retrieve,
};

/// An owned-or-borrowed wrapper for JSON `Value`.
#[derive(Debug)]
pub(crate) enum ValueWrapper<'a> {
    Owned(Value),
    Borrowed(&'a Value),
}

impl AsRef<Value> for ValueWrapper<'_> {
    fn as_ref(&self) -> &Value {
        match self {
            ValueWrapper::Owned(value) => value,
            ValueWrapper::Borrowed(value) => value,
        }
    }
}

#[derive(Debug)]
struct StoredDocument<'a> {
    value: ValueWrapper<'a>,
    draft: Draft,
}

impl<'a> StoredDocument<'a> {
    fn owned(value: Value, draft: Draft) -> Self {
        Self {
            value: ValueWrapper::Owned(value),
            draft,
        }
    }

    fn borrowed(value: &'a Value, draft: Draft) -> Self {
        Self {
            value: ValueWrapper::Borrowed(value),
            draft,
        }
    }

    fn contents(&self) -> &Value {
        self.value.as_ref()
    }

    fn draft(&self) -> Draft {
        self.draft
    }
}

type DocumentStore<'a> = AHashMap<Arc<Uri<String>>, Arc<StoredDocument<'a>>>;

/// Pre-loaded registry containing all JSON Schema meta-schemas and their vocabularies
pub static SPECIFICATIONS: LazyLock<Registry<'static>> =
    LazyLock::new(|| Registry::build_from_meta_schemas(meta::META_SCHEMAS_ALL.as_slice()));

/// A registry of JSON Schema resources, each identified by their canonical URIs.
///
/// Registry is storage-only. It keeps original documents and URI resolution cache.
/// Build an [`Index`] from it via [`Registry::build_index`] when you need fast lookup.
#[derive(Debug, Clone)]
pub struct Registry<'a> {
    documents: DocumentStore<'a>,
    resolution_cache: SharedUriCache,
    known_resources: KnownResources,
    /// Skeleton built during `process_resources`: one entry per subresource that
    /// has its own `$id` or has `$anchor`/`$dynamicAnchor` entries.  Used by
    /// `build_index` to avoid a second full BFS traversal.
    skeleton: IndexSkeleton,
}

impl<'a> Registry<'a> {
    /// Create a borrowing registry from resource references and a custom retriever.
    ///
    /// # Errors
    ///
    /// Returns an error if any URI is invalid, retrieval fails, or custom meta-schemas
    /// cannot be validated.
    pub fn try_from_resources_and_retriever(
        pairs: impl IntoIterator<Item = (impl AsRef<str>, ResourceRef<'a>)>,
        retriever: &dyn Retrieve,
        draft: Draft,
    ) -> Result<Self, Error> {
        let mut documents = DocumentStore::new();
        let mut known_resources = KnownResources::new();
        let mut resolution_cache = UriCache::new();
        let (custom_metaschemas, skeleton) = process_resources_borrowed(
            pairs,
            retriever,
            &mut documents,
            &mut known_resources,
            &mut resolution_cache,
            draft,
        )?;
        validate_custom_metaschemas(&custom_metaschemas, &known_resources)?;

        Ok(Self {
            documents,
            resolution_cache: resolution_cache.into_shared(),
            known_resources,
            skeleton,
        })
    }

    #[cfg(feature = "retrieve-async")]
    /// Async version of [`Registry::try_from_resources_and_retriever`].
    ///
    /// # Errors
    ///
    /// Returns an error if any URI is invalid, retrieval fails, or custom meta-schemas
    /// cannot be validated.
    pub async fn try_from_resources_and_retriever_async(
        pairs: impl IntoIterator<Item = (impl AsRef<str>, ResourceRef<'a>)>,
        retriever: &dyn crate::AsyncRetrieve,
        draft: Draft,
    ) -> Result<Self, Error> {
        let mut documents = DocumentStore::new();
        let mut known_resources = KnownResources::new();
        let mut resolution_cache = UriCache::new();
        let (custom_metaschemas, skeleton) = process_resources_async_borrowed(
            pairs,
            retriever,
            &mut documents,
            &mut known_resources,
            &mut resolution_cache,
            draft,
        )
        .await?;
        validate_custom_metaschemas(&custom_metaschemas, &known_resources)?;

        Ok(Self {
            documents,
            resolution_cache: resolution_cache.into_shared(),
            known_resources,
            skeleton,
        })
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
        Registry::try_from_resources_impl(pairs, &*self.retriever, self.draft)
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
        Registry::try_from_resources_async_impl(pairs, &*self.retriever, self.draft).await
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
    /// Get [`RegistryOptions`] for configuring a new [`Registry`].
    #[must_use]
    pub fn options() -> RegistryOptions<Arc<dyn Retrieve>> {
        RegistryOptions::new()
    }

    /// Create a new [`Registry`] with a single resource.
    ///
    /// # Errors
    ///
    /// Returns an error if the URI is invalid or if there's an issue processing the resource.
    pub fn try_new(uri: impl AsRef<str>, resource: Resource) -> Result<Self, Error> {
        Self::try_new_impl(uri, resource, &DefaultRetriever, Draft::default())
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
    pub fn try_from_resources(
        pairs: impl IntoIterator<Item = (impl AsRef<str>, Resource)>,
    ) -> Result<Self, Error> {
        Self::try_from_resources_impl(pairs, &DefaultRetriever, Draft::default())
    }

    fn try_new_impl(
        uri: impl AsRef<str>,
        resource: Resource,
        retriever: &dyn Retrieve,
        draft: Draft,
    ) -> Result<Self, Error> {
        Self::try_from_resources_impl([(uri, resource)], retriever, draft)
    }

    fn try_from_resources_impl(
        pairs: impl IntoIterator<Item = (impl AsRef<str>, Resource)>,
        retriever: &dyn Retrieve,
        draft: Draft,
    ) -> Result<Self, Error> {
        let mut documents = DocumentStore::new();
        let mut known_resources = KnownResources::new();
        let mut resolution_cache = UriCache::new();

        let (custom_metaschemas, skeleton) = process_resources(
            pairs,
            retriever,
            &mut documents,
            &mut known_resources,
            &mut resolution_cache,
            draft,
        )?;

        validate_custom_metaschemas(&custom_metaschemas, &known_resources)?;

        Ok(Registry {
            documents,
            resolution_cache: resolution_cache.into_shared(),
            known_resources,
            skeleton,
        })
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
    async fn try_from_resources_async_impl(
        pairs: impl IntoIterator<Item = (impl AsRef<str>, Resource)>,
        retriever: &dyn crate::AsyncRetrieve,
        draft: Draft,
    ) -> Result<Self, Error> {
        let mut documents = DocumentStore::new();
        let mut known_resources = KnownResources::new();
        let mut resolution_cache = UriCache::new();

        let (custom_metaschemas, skeleton) = process_resources_async(
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
            documents,
            resolution_cache: resolution_cache.into_shared(),
            known_resources,
            skeleton,
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
        let skeleton = build_skeleton_for_documents(&documents, &mut resolution_cache)
            .expect("meta-schema skeleton must build");

        Self {
            documents,
            resolution_cache: resolution_cache.into_shared(),
            known_resources,
            skeleton,
        }
    }
}

impl<'a> Registry<'a> {
    /// Create a new registry with a new resource.
    ///
    /// # Errors
    ///
    /// Returns an error if the URI is invalid or if there's an issue processing the resource.
    pub fn try_with_resource(
        self,
        uri: impl AsRef<str>,
        resource: Resource,
    ) -> Result<Registry<'a>, Error> {
        let draft = resource.draft();
        self.try_with_resources([(uri, resource)], draft)
    }

    /// Create a new registry with new resources.
    ///
    /// # Errors
    ///
    /// Returns an error if any URI is invalid or if there's an issue processing the resources.
    pub fn try_with_resources(
        self,
        pairs: impl IntoIterator<Item = (impl AsRef<str>, Resource)>,
        draft: Draft,
    ) -> Result<Registry<'a>, Error> {
        self.try_with_resources_and_retriever(pairs, &DefaultRetriever, draft)
    }

    /// Create a new registry with new resources and using the given retriever.
    ///
    /// # Errors
    ///
    /// Returns an error if any URI is invalid or if there's an issue processing the resources.
    pub fn try_with_resources_and_retriever(
        self,
        pairs: impl IntoIterator<Item = (impl AsRef<str>, Resource)>,
        retriever: &dyn Retrieve,
        draft: Draft,
    ) -> Result<Registry<'a>, Error> {
        let mut documents = self.documents;
        let mut resolution_cache = self.resolution_cache.into_local();
        let mut known_resources = self.known_resources;
        let mut skeleton = self.skeleton;

        let (custom_metaschemas, mut new_skeleton) = process_resources(
            pairs,
            retriever,
            &mut documents,
            &mut known_resources,
            &mut resolution_cache,
            draft,
        )?;
        skeleton.append(&mut new_skeleton);
        validate_custom_metaschemas(&custom_metaschemas, &known_resources)?;

        Ok(Registry {
            documents,
            resolution_cache: resolution_cache.into_shared(),
            known_resources,
            skeleton,
        })
    }

    /// Create a new registry with new resources and using the given non-blocking retriever.
    ///
    /// # Errors
    ///
    /// Returns an error if any URI is invalid or if there's an issue processing the resources.
    #[cfg(feature = "retrieve-async")]
    pub async fn try_with_resources_and_retriever_async(
        self,
        pairs: impl IntoIterator<Item = (impl AsRef<str>, Resource)>,
        retriever: &dyn crate::AsyncRetrieve,
        draft: Draft,
    ) -> Result<Registry<'a>, Error> {
        let mut documents = self.documents;
        let mut resolution_cache = self.resolution_cache.into_local();
        let mut known_resources = self.known_resources;
        let mut skeleton = self.skeleton;

        let (custom_metaschemas, mut new_skeleton) = process_resources_async(
            pairs,
            retriever,
            &mut documents,
            &mut known_resources,
            &mut resolution_cache,
            draft,
        )
        .await?;
        skeleton.append(&mut new_skeleton);
        validate_custom_metaschemas(&custom_metaschemas, &known_resources)?;

        Ok(Registry {
            documents,
            resolution_cache: resolution_cache.into_shared(),
            known_resources,
            skeleton,
        })
    }

    /// Build a resolution index that borrows resources from this registry.
    ///
    /// # Errors
    ///
    /// Returns an error if URI resolution fails while indexing discovered resources.
    pub fn build_index(&self) -> Result<Index<'_>, Error> {
        let mut resources = ResourceMap::new();
        let mut anchors = AnchorMap::new();

        // Insert all root documents into the resources map and collect their anchors.
        for (uri, document) in &self.documents {
            let resource = ResourceRef::new(document.contents(), document.draft());
            resources.insert(Arc::clone(uri), resource);
            for anchor in document.draft().anchors(document.contents()) {
                anchors
                    .entry(Arc::clone(uri))
                    .or_default()
                    .insert(anchor.name(), anchor);
            }
        }

        // Process skeleton entries: subresources with their own `$id` or anchors.
        // No BFS traversal needed — the skeleton was built during `process_resources`.
        for entry in &self.skeleton {
            let Some(document) = self.documents.get(&entry.doc_key) else {
                continue;
            };
            let contents = if entry.pointer.is_empty() {
                document.contents()
            } else {
                match pointer(document.contents(), &entry.pointer) {
                    Some(v) => v,
                    None => continue,
                }
            };
            if entry.has_id {
                resources.insert(
                    Arc::clone(&entry.uri),
                    ResourceRef::new(contents, entry.draft),
                );
            }
            for anchor in entry.draft.anchors(contents) {
                anchors
                    .entry(Arc::clone(&entry.uri))
                    .or_default()
                    .insert(anchor.name(), anchor);
            }
        }

        Ok(Index::new(resources, anchors, &self.resolution_cache))
    }

    /// Resolves a reference URI against a base URI using registry's cache.
    ///
    /// # Errors
    ///
    /// Returns an error if base has not schema or there is a fragment.
    pub fn resolve_against(&self, base: &Uri<&str>, uri: &str) -> Result<Arc<Uri<String>>, Error> {
        self.resolution_cache.resolve_against(base, uri)
    }
}

/// Build skeleton entries for all documents already in `documents`.
/// Used by `build_from_meta_schemas` for the static SPECIFICATIONS registry.
fn build_skeleton_for_documents(
    documents: &DocumentStore<'_>,
    resolution_cache: &mut UriCache,
) -> Result<IndexSkeleton, Error> {
    let mut skeleton = IndexSkeleton::new();
    for (doc_uri, document) in documents {
        let root = document.contents();
        let initial_base = Arc::clone(doc_uri);
        // Stack: (base_uri, json_pointer_from_root, draft)
        let mut work: Vec<(Arc<Uri<String>>, String, Draft)> =
            vec![(initial_base, String::new(), document.draft())];
        while let Some((base, ptr_str, draft)) = work.pop() {
            let contents = if ptr_str.is_empty() {
                root
            } else {
                match pointer(root, &ptr_str) {
                    Some(v) => v,
                    None => continue,
                }
            };
            let mut current_base = base;
            let (id, has_anchors) = draft.id_and_has_anchors(contents);
            if let Some(id) = id {
                current_base = resolve_id(&current_base, id, resolution_cache)?;
                skeleton.push(SkeletonEntry {
                    uri: Arc::clone(&current_base),
                    doc_key: Arc::clone(doc_uri),
                    pointer: ptr_str.clone(),
                    draft,
                    has_id: true,
                });
            } else if has_anchors {
                skeleton.push(SkeletonEntry {
                    uri: Arc::clone(&current_base),
                    doc_key: Arc::clone(doc_uri),
                    pointer: ptr_str.clone(),
                    draft,
                    has_id: false,
                });
            }
            // Push children with their absolute paths
            let base_for_children = Arc::clone(&current_base);
            let mut path = PathStack::from_base(ptr_str);
            let _ = draft.walk_subresources_with_path(
                contents,
                &mut path,
                &mut |p, _child, child_draft| {
                    work.push((Arc::clone(&base_for_children), p.to_pointer(), child_draft));
                    Ok::<(), Error>(())
                },
            );
        }
    }
    Ok(skeleton)
}

type KnownResources = AHashSet<Uri<String>>;

/// An entry in the index skeleton, capturing a subresource that has its own
/// `$id` (needs a `resources` map entry) or has anchors (needs an `anchors` map entry).
#[derive(Debug, Clone)]
struct SkeletonEntry {
    /// Effective base URI at this node: the resolved `$id` if present, else the
    /// inherited base from the nearest ancestor with an `$id`.
    uri: Arc<Uri<String>>,
    /// Key of the root document in `DocumentStore` that contains this node.
    doc_key: Arc<Uri<String>>,
    /// JSON Pointer from the document root to this node (empty = root).
    pointer: String,
    draft: Draft,
    /// If `true`, this node has its own `$id` and must be inserted into the
    /// `resources` map under `uri`.
    has_id: bool,
}

type IndexSkeleton = Vec<SkeletonEntry>;

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
#[allow(unsafe_code)]
#[inline]
unsafe fn reuse_local_seen<'a, 'b>(mut s: LocalSeen<'a>) -> LocalSeen<'b> {
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

struct ProcessingState {
    queue: VecDeque<QueueEntry>,
    seen: ReferenceTracker,
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
    /// subresource targets are already in `visited_schemas` and return in O(1);
    /// non-subresource paths (e.g. `#/components/schemas/Foo`) are still fully traversed.
    deferred_refs: Vec<QueueEntry>,
    /// Skeleton entries accumulated during traversal; used by `build_index`.
    skeleton: IndexSkeleton,
}

impl ProcessingState {
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
            skeleton: IndexSkeleton::new(),
        }
    }
}

fn process_input_resources(
    pairs: impl IntoIterator<Item = (impl AsRef<str>, Resource)>,
    documents: &mut DocumentStore<'_>,
    known_resources: &mut KnownResources,
    state: &mut ProcessingState,
) -> Result<(), Error> {
    for (uri, resource) in pairs {
        let uri = uri::from_str(uri.as_ref().trim_end_matches('#'))?;
        let key = Arc::new(uri);
        match documents.entry(Arc::clone(&key)) {
            Entry::Occupied(_) => {}
            Entry::Vacant(entry) => {
                let (draft, contents) = resource.into_inner();
                entry.insert(Arc::new(StoredDocument::owned(contents, draft)));
                known_resources.insert((*key).clone());

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
    }
    Ok(())
}

fn process_input_resources_borrowed<'a>(
    pairs: impl IntoIterator<Item = (impl AsRef<str>, ResourceRef<'a>)>,
    documents: &mut DocumentStore<'a>,
    known_resources: &mut KnownResources,
    state: &mut ProcessingState,
) -> Result<(), Error> {
    for (uri, resource) in pairs {
        let uri = uri::from_str(uri.as_ref().trim_end_matches('#'))?;
        let key = Arc::new(uri);
        match documents.entry(Arc::clone(&key)) {
            Entry::Occupied(_) => {}
            Entry::Vacant(entry) => {
                entry.insert(Arc::new(StoredDocument::borrowed(
                    resource.contents(),
                    resource.draft(),
                )));
                known_resources.insert((*key).clone());

                if resource.draft() == Draft::Unknown {
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
                    .push_back((Arc::clone(&key), key, String::new(), resource.draft()));
            }
        }
    }
    Ok(())
}

fn process_queue<'a>(
    state: &mut ProcessingState,
    documents: &'a DocumentStore<'a>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    local_seen: &mut LocalSeen<'a>,
) -> Result<(), Error> {
    while let Some((base, document_root_uri, pointer_path, draft)) = state.queue.pop_front() {
        let Some(document) = documents.get(&document_root_uri) else {
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

        let resource = ResourceRef::new(contents, draft);
        let mut path = PathStack::from_base(pointer_path);
        process_resource_tree(
            base,
            root,
            resource,
            &mut path,
            &document_root_uri,
            state,
            known_resources,
            resolution_cache,
            local_seen,
        )?;
    }
    Ok(())
}

fn process_resource_tree<'a>(
    mut base: Arc<Uri<String>>,
    root: &'a Value,
    resource: ResourceRef<'a>,
    path: &mut PathStack<'a>,
    doc_key: &Arc<Uri<String>>,
    state: &mut ProcessingState,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    local_seen: &mut LocalSeen<'a>,
) -> Result<(), Error> {
    let (id, has_anchors) = resource.draft().id_and_has_anchors(resource.contents());
    if let Some(id) = id {
        base = resolve_id(&base, id, resolution_cache)?;
        known_resources.insert((*base).clone());
        state.skeleton.push(SkeletonEntry {
            uri: Arc::clone(&base),
            doc_key: Arc::clone(doc_key),
            pointer: path.to_pointer(),
            draft: resource.draft(),
            has_id: true,
        });
    } else if has_anchors {
        state.skeleton.push(SkeletonEntry {
            uri: Arc::clone(&base),
            doc_key: Arc::clone(doc_key),
            pointer: path.to_pointer(),
            draft: resource.draft(),
            has_id: false,
        });
    }

    let contents_ptr = std::ptr::from_ref::<Value>(resource.contents()) as usize;
    if state.visited_schemas.insert(contents_ptr) {
        collect_external_resources(
            &base,
            root,
            resource.contents(),
            &mut state.external,
            &mut state.seen,
            resolution_cache,
            &mut state.scratch,
            &mut state.refers_metaschemas,
            resource.draft(),
            doc_key,
            &mut state.deferred_refs,
            local_seen,
        )?;
    }

    resource.draft().walk_subresources_with_path(
        resource.contents(),
        path,
        &mut |child_path, child, child_draft| {
            process_resource_tree(
                Arc::clone(&base),
                root,
                ResourceRef::new(child, child_draft),
                child_path,
                doc_key,
                state,
                known_resources,
                resolution_cache,
                local_seen,
            )
        },
    )
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

fn handle_metaschemas(
    refers_metaschemas: bool,
    documents: &mut DocumentStore<'_>,
    known_resources: &mut KnownResources,
    draft_version: Draft,
    state: &mut ProcessingState,
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
        state
            .queue
            .push_back((Arc::clone(&key), Arc::clone(&key), String::new(), draft));
    }
    Ok(())
}

fn create_resource(
    retrieved: Value,
    fragmentless: Uri<String>,
    default_draft: Draft,
    documents: &mut DocumentStore<'_>,
    known_resources: &mut KnownResources,
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

#[allow(unsafe_code)]
fn process_resources(
    pairs: impl IntoIterator<Item = (impl AsRef<str>, Resource)>,
    retriever: &dyn Retrieve,
    documents: &mut DocumentStore<'_>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    default_draft: Draft,
) -> Result<(Vec<String>, IndexSkeleton), Error> {
    let mut state = ProcessingState::new();
    process_input_resources(pairs, documents, known_resources, &mut state)?;

    // Pre-size to the initial queue length to avoid repeated rehashing during traversal.
    let mut local_seen_buf: LocalSeen<'static> = LocalSeen::new();

    loop {
        if state.queue.is_empty() && state.external.is_empty() {
            break;
        }

        {
            // SAFETY: widens 'static → '_ (covariant); set is empty after reuse_local_seen clears it.
            let mut local_seen: LocalSeen<'_> = unsafe { reuse_local_seen(local_seen_buf) };
            process_queue(
                &mut state,
                documents,
                known_resources,
                resolution_cache,
                &mut local_seen,
            )?;
            process_deferred_refs(&mut state, documents, resolution_cache, &mut local_seen)?;
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
        &mut state,
    )?;

    if !state.queue.is_empty() {
        // SAFETY: widens 'static → '_ (covariant); set is empty after reuse_local_seen clears it.
        let mut local_seen: LocalSeen<'_> = unsafe { reuse_local_seen(local_seen_buf) };
        process_queue(
            &mut state,
            documents,
            known_resources,
            resolution_cache,
            &mut local_seen,
        )?;
        process_deferred_refs(&mut state, documents, resolution_cache, &mut local_seen)?;
    }

    Ok((state.custom_metaschemas, state.skeleton))
}

#[allow(unsafe_code)]
fn process_resources_borrowed<'a>(
    pairs: impl IntoIterator<Item = (impl AsRef<str>, ResourceRef<'a>)>,
    retriever: &dyn Retrieve,
    documents: &mut DocumentStore<'a>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    default_draft: Draft,
) -> Result<(Vec<String>, IndexSkeleton), Error> {
    let mut state = ProcessingState::new();
    process_input_resources_borrowed(pairs, documents, known_resources, &mut state)?;

    let mut local_seen_buf: LocalSeen<'static> = LocalSeen::new();

    loop {
        if state.queue.is_empty() && state.external.is_empty() {
            break;
        }

        {
            // SAFETY: widens 'static → '_ (covariant); set is empty after reuse_local_seen clears it.
            let mut local_seen: LocalSeen<'_> = unsafe { reuse_local_seen(local_seen_buf) };
            process_queue(
                &mut state,
                documents,
                known_resources,
                resolution_cache,
                &mut local_seen,
            )?;
            process_deferred_refs(&mut state, documents, resolution_cache, &mut local_seen)?;
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
        &mut state,
    )?;

    if !state.queue.is_empty() {
        // SAFETY: widens 'static → '_ (covariant); set is empty after reuse_local_seen clears it.
        let mut local_seen: LocalSeen<'_> = unsafe { reuse_local_seen(local_seen_buf) };
        process_queue(
            &mut state,
            documents,
            known_resources,
            resolution_cache,
            &mut local_seen,
        )?;
        process_deferred_refs(&mut state, documents, resolution_cache, &mut local_seen)?;
    }

    Ok((state.custom_metaschemas, state.skeleton))
}

#[cfg(feature = "retrieve-async")]
#[allow(unsafe_code)]
async fn process_resources_async(
    pairs: impl IntoIterator<Item = (impl AsRef<str>, Resource)>,
    retriever: &dyn crate::AsyncRetrieve,
    documents: &mut DocumentStore<'_>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    default_draft: Draft,
) -> Result<(Vec<String>, IndexSkeleton), Error> {
    type ExternalRefsByBase = AHashMap<Uri<String>, Vec<(String, Uri<String>, ReferenceKind)>>;

    let mut state = ProcessingState::new();
    process_input_resources(pairs, documents, known_resources, &mut state)?;

    let mut local_seen_buf: LocalSeen<'static> = LocalSeen::new();

    loop {
        if state.queue.is_empty() && state.external.is_empty() {
            break;
        }

        {
            // SAFETY: widens 'static → '_ (covariant); set is empty after reuse_local_seen clears it.
            let mut local_seen: LocalSeen<'_> = unsafe { reuse_local_seen(local_seen_buf) };
            process_queue(
                &mut state,
                documents,
                known_resources,
                resolution_cache,
                &mut local_seen,
            )?;
            process_deferred_refs(&mut state, documents, resolution_cache, &mut local_seen)?;
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
        &mut state,
    )?;

    if !state.queue.is_empty() {
        // SAFETY: widens 'static → '_ (covariant); set is empty after reuse_local_seen clears it.
        let mut local_seen: LocalSeen<'_> = unsafe { reuse_local_seen(local_seen_buf) };
        process_queue(
            &mut state,
            documents,
            known_resources,
            resolution_cache,
            &mut local_seen,
        )?;
        process_deferred_refs(&mut state, documents, resolution_cache, &mut local_seen)?;
    }

    Ok((state.custom_metaschemas, state.skeleton))
}

#[cfg(feature = "retrieve-async")]
#[allow(unsafe_code)]
async fn process_resources_async_borrowed<'a>(
    pairs: impl IntoIterator<Item = (impl AsRef<str>, ResourceRef<'a>)>,
    retriever: &dyn crate::AsyncRetrieve,
    documents: &mut DocumentStore<'a>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    default_draft: Draft,
) -> Result<(Vec<String>, IndexSkeleton), Error> {
    type ExternalRefsByBase = AHashMap<Uri<String>, Vec<(String, Uri<String>, ReferenceKind)>>;

    let mut state = ProcessingState::new();
    process_input_resources_borrowed(pairs, documents, known_resources, &mut state)?;

    let mut local_seen_buf: LocalSeen<'static> = LocalSeen::new();

    loop {
        if state.queue.is_empty() && state.external.is_empty() {
            break;
        }

        {
            // SAFETY: widens 'static → '_ (covariant); set is empty after reuse_local_seen clears it.
            let mut local_seen: LocalSeen<'_> = unsafe { reuse_local_seen(local_seen_buf) };
            process_queue(
                &mut state,
                documents,
                known_resources,
                resolution_cache,
                &mut local_seen,
            )?;
            process_deferred_refs(&mut state, documents, resolution_cache, &mut local_seen)?;
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
        &mut state,
    )?;

    if !state.queue.is_empty() {
        // SAFETY: widens 'static → '_ (covariant); set is empty after reuse_local_seen clears it.
        let mut local_seen: LocalSeen<'_> = unsafe { reuse_local_seen(local_seen_buf) };
        process_queue(
            &mut state,
            documents,
            known_resources,
            resolution_cache,
            &mut local_seen,
        )?;
        process_deferred_refs(&mut state, documents, resolution_cache, &mut local_seen)?;
    }

    Ok((state.custom_metaschemas, state.skeleton))
}
fn handle_retrieve_error(
    uri: &Uri<String>,
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
    deferred_refs: &mut Vec<QueueEntry>,
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
                    if mark_local_reference(local_seen, base, $reference) {
                        if let Some((referenced, resolved_base)) = pointer_with_base(
                            root,
                            $reference.trim_start_matches('#'),
                            base,
                            resolution_cache,
                            draft,
                        )? {
                            let target_draft = draft.detect(referenced);
                            deferred_refs.push((
                                resolved_base,
                                Arc::clone(doc_key),
                                $reference.trim_start_matches('#').to_string(),
                                target_draft,
                            ));
                        }
                    }
                } else if mark_reference(seen, base, $reference) {
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
    deferred_refs: &mut Vec<QueueEntry>,
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
/// `visited_schemas`. Subresource targets return in O(1); non-subresource targets
/// (e.g. `#/components/schemas/Foo`) are still fully traversed. New deferred entries
/// added during traversal are also processed iteratively until none remain.
fn process_deferred_refs<'a>(
    state: &mut ProcessingState,
    documents: &'a DocumentStore<'a>,
    resolution_cache: &mut UriCache,
    local_seen: &mut LocalSeen<'a>,
) -> Result<(), Error> {
    while !state.deferred_refs.is_empty() {
        let batch = std::mem::take(&mut state.deferred_refs);
        for (base, doc_key, pointer_path, draft) in batch {
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

#[allow(clippy::type_complexity)]
fn pointer_with_base<'a>(
    document: &'a Value,
    pointer: &str,
    base: &Arc<Uri<String>>,
    resolution_cache: &mut UriCache,
    draft: Draft,
) -> Result<Option<(&'a Value, Arc<Uri<String>>)>, Error> {
    if pointer.is_empty() {
        return Ok(Some((document, Arc::clone(base))));
    }
    if !pointer.starts_with('/') {
        return Ok(None);
    }

    let mut current = document;
    let mut current_base = Arc::clone(base);
    let mut current_draft = draft;

    for token in pointer.split('/').skip(1).map(unescape_segment) {
        current_draft = current_draft.detect(current);
        if let Some(id) = current_draft.id_of(current) {
            current_base = resolve_id(&current_base, id, resolution_cache)?;
        }

        current = match current {
            Value::Object(map) => match map.get(&*token) {
                Some(v) => v,
                None => return Ok(None),
            },
            Value::Array(list) => match parse_index(&token).and_then(|x| list.get(x)) {
                Some(v) => v,
                None => return Ok(None),
            },
            _ => return Ok(None),
        };
    }

    Ok(Some((current, current_base)))
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

    use crate::{uri::from_str, Draft, Registry, Resource, ResourceRef, Retrieve};

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
        let index = registry.build_index().expect("Invalid index");
        let resolver = index.resolver(
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
        let registry = Registry::try_from_resources_and_retriever(
            [("urn:root", ResourceRef::from_contents(&schema))],
            &crate::DefaultRetriever,
            Draft::default(),
        )
        .expect("Invalid resources");
        let index = registry.build_index().expect("Invalid index");
        assert!(index.contains_resource_uri("urn:root"));
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
            .try_with_resources(
                [("http://example.com/custom", custom_schema)],
                Draft::default(),
            )
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
            .try_with_resources(
                [("http://example.com/schemas/my-schema", schema)],
                Draft::default(),
            )
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
        let index = registry.build_index().expect("Invalid index");
        // Verify that all expected URIs are resolved and present in resources
        for uri in test_case.expected_resolved_uris {
            let resolver = index.resolver(from_str("").expect("Invalid base URI"));
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

        let index = registry.build_index().expect("Invalid index");
        let resource = index.resource("http://example.com/schema").unwrap();
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
        let index = registry.build_index().expect("Invalid index");
        let resolver =
            index.resolver(from_str("http://127.0.0.1/schema").expect("Invalid base URI"));
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
        let index = registry.build_index().expect("Invalid index");
        let resolver = index.resolver(from_str("").expect("Invalid base URI"));
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
    fn test_try_with_resources_preserves_existing_skeleton_entries() {
        let original = Registry::try_new(
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
        .expect("Invalid root schema");

        let extended = original
            .try_with_resource(
                "http://example.com/other",
                Resource::from_contents(json!({"type": "number"})),
            )
            .expect("Registry extension should succeed");

        let index = extended.build_index().expect("Invalid index");
        let resolver = index.resolver(from_str("").expect("Invalid base URI"));
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
    fn test_invalid_reference() {
        let resource = Draft::Draft202012.create_resource(json!({"$schema": "$##"}));
        let _ = Registry::try_new("http://#/", resource);
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

        let index = registry.build_index().expect("Invalid index");
        let resource = index.resource("http://example.com/schema").unwrap();
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

        let index = registry.build_index().expect("Invalid index");
        let resolver = index.resolver(uri::from_str("").expect("Invalid base URI"));
        let resolved = resolver
            .lookup("http://example.com/schema2")
            .expect("Lookup failed");
        assert_eq!(resolved.contents(), &json!({"type": "object"}));
    }

    #[tokio::test]
    async fn test_async_try_with_resources_preserves_existing_skeleton_entries() {
        let original = Registry::options()
            .async_retriever(DefaultRetriever)
            .build([(
                "http://example.com/root",
                Resource::from_contents(json!({
                    "$defs": {
                        "embedded": {
                            "$id": "http://example.com/embedded",
                            "type": "string"
                        }
                    }
                })),
            )])
            .await
            .expect("Invalid root schema");

        let extended = original
            .try_with_resources_and_retriever_async(
                [(
                    "http://example.com/other",
                    Resource::from_contents(json!({"type": "number"})),
                )],
                &DefaultRetriever,
                Draft::default(),
            )
            .await
            .expect("Registry extension should succeed");

        let index = extended.build_index().expect("Invalid index");
        let resolver = index.resolver(uri::from_str("").expect("Invalid base URI"));
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

        let index = registry.build_index().expect("Invalid index");
        let resolver = index.resolver(uri::from_str("").expect("Invalid base URI"));

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

        let index = registry.build_index().expect("Invalid index");
        let resolver = index.resolver(uri::from_str("").expect("Invalid base URI"));

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
        let registry = Registry::options()
            .async_retriever(retriever)
            .build([(
                "http://example.com/main",
                Resource::from_contents(json!({
                    "type": "object",
                    "properties": {
                        "name": { "$ref": "http://example.com/external#/$defs/foo" },
                        "age": { "$ref": "http://example.com/external#/$defs/bar" }
                    }
                })),
            )])
            .await
            .expect("Invalid resource");

        // Should only fetch the external schema once
        let fetches = FETCH_COUNT.load(Ordering::SeqCst);
        assert_eq!(
            fetches, 1,
            "External schema should be fetched only once, but was fetched {fetches} times"
        );

        let index = registry.build_index().expect("Invalid index");
        let resolver =
            index.resolver(uri::from_str("http://example.com/main").expect("Invalid base URI"));

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
