//! BFS pipeline that processes pending resources into the prepared index.
//!
//! Entry points:
//! - [`index_resources`]: processes pending resources and returns a prepared index.
//! - [`build_index_from_stored`]: builds an index from pre-stored documents
//!   (used by the static [`super::SPECIFICATIONS`] registry).
//!
//! [`StoredResource`] wraps a [`Cow<Value>`](std::borrow::Cow) so the registry holds
//! both borrowed (externally-owned, zero-copy) and owned (retrieved) documents uniformly.

use std::{borrow::Cow, collections::VecDeque, num::NonZeroUsize, sync::Arc};

use ahash::{AHashMap, AHashSet};
use fluent_uri::Uri;
use serde_json::Value;

use crate::{
    cache::UriCache,
    meta::metas_for_draft,
    path::JsonPointerSegment,
    pointer::{pointer, ParsedPointer, ParsedPointerSegment},
    uri, Draft, Error, JsonPointerNode, Retrieve,
};

use super::{index::Index, input::PendingResource};

/// A schema document stored in the registry, either borrowed from the caller or owned.
#[derive(Debug)]
pub(super) struct StoredResource<'a> {
    value: Cow<'a, Value>,
    draft: Draft,
}

impl<'a> StoredResource<'a> {
    #[inline]
    pub(super) fn owned(value: Value, draft: Draft) -> Self {
        Self {
            value: Cow::Owned(value),
            draft,
        }
    }

    #[inline]
    pub(super) fn borrowed(value: &'a Value, draft: Draft) -> Self {
        Self {
            value: Cow::Borrowed(value),
            draft,
        }
    }

    #[inline]
    pub(super) fn contents(&self) -> &Value {
        &self.value
    }

    #[inline]
    pub(super) fn borrowed_contents(&self) -> Option<&'a Value> {
        match &self.value {
            Cow::Borrowed(value) => Some(value),
            Cow::Owned(_) => None,
        }
    }

    #[inline]
    pub(super) fn draft(&self) -> Draft {
        self.draft
    }
}

pub(super) type ResourceStore<'a> = AHashMap<Arc<Uri<String>>, Arc<StoredResource<'a>>>;
pub(super) type KnownResources = AHashSet<Uri<String>>;
type VisitedLocalRefs<'a> = AHashSet<(NonZeroUsize, &'a str)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ReferenceKind {
    DollarRef,
    DollarSchema,
}

/// An entry in the processing queue.
///
/// `pointer_path` is a JSON Pointer relative to the document root (`""` means root).
/// Local `$ref`s are always resolved against the document root.
struct CrawlTask {
    base_uri: Arc<Uri<String>>,
    document_root_uri: Arc<Uri<String>>,
    pointer_path: String,
    draft: Draft,
}

/// A deferred local `$ref` target.
///
/// Like [`CrawlTask`] but carries the pre-resolved value address (`schema_ptr`) obtained
/// for free during the `pointer()` call at push time. Used in [`process_deferred_refs`] to
/// skip already-visited targets without a second `pointer()` traversal.
struct PendingLocalRef {
    document_root_uri: Arc<Uri<String>>,
    pointer: String,
    schema_ptr: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct VisitedSchemaContext {
    schema_ptr: NonZeroUsize,
    base_ptr: NonZeroUsize,
    draft: Draft,
}

type DeferredTarget<'a> = (Arc<Uri<String>>, Draft, &'a Value);

/// Lifetime-free traversal state passed to external-resource collection helpers.
struct CrawlState {
    external: AHashSet<(String, Uri<String>, ReferenceKind)>,
    uri_scratch: String,
    found_metaschema_ref: bool,
    /// Tracks schema/base/draft traversal contexts we have already processed.
    visited_schemas: AHashSet<VisitedSchemaContext>,
    deferred_refs: Vec<PendingLocalRef>,
}

impl CrawlState {
    fn new() -> Self {
        Self {
            external: AHashSet::new(),
            uri_scratch: String::new(),
            found_metaschema_ref: false,
            visited_schemas: AHashSet::new(),
            deferred_refs: Vec::new(),
        }
    }
}

struct BuildState<'a> {
    queue: VecDeque<CrawlTask>,
    custom_metaschemas: Vec<String>,
    index: Index<'a>,
    crawl: CrawlState,
}

impl BuildState<'_> {
    fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            custom_metaschemas: Vec::new(),
            index: Index::default(),
            crawl: CrawlState::new(),
        }
    }
}

/// Result of resolving a `$id` against the current base URI.
struct ResolvedId {
    base: Arc<Uri<String>>,
    /// True when the resolved URI differs from the input base URI.
    uri_changed: bool,
}

pub(super) fn index_resources<'a>(
    pairs: impl IntoIterator<Item = (Uri<String>, PendingResource<'a>)>,
    retriever: &dyn Retrieve,
    documents: &mut ResourceStore<'a>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    draft_override: Option<Draft>,
) -> Result<(Vec<String>, Index<'a>), Error> {
    let mut state = BuildState::new();
    enqueue_resources(
        pairs,
        documents,
        known_resources,
        &mut state,
        draft_override,
    );
    fetch_and_index(
        &mut state,
        documents,
        known_resources,
        resolution_cache,
        draft_override.unwrap_or_default(),
        retriever,
    )?;
    Ok((state.custom_metaschemas, state.index))
}

#[cfg(feature = "retrieve-async")]
pub(super) async fn index_resources_async<'a>(
    pairs: impl IntoIterator<Item = (Uri<String>, PendingResource<'a>)>,
    retriever: &dyn crate::AsyncRetrieve,
    documents: &mut ResourceStore<'a>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    draft_override: Option<Draft>,
) -> Result<(Vec<String>, Index<'a>), Error> {
    let mut state = BuildState::new();
    enqueue_resources(
        pairs,
        documents,
        known_resources,
        &mut state,
        draft_override,
    );
    fetch_and_index_async(
        &mut state,
        documents,
        known_resources,
        resolution_cache,
        draft_override.unwrap_or_default(),
        retriever,
    )
    .await?;
    Ok((state.custom_metaschemas, state.index))
}

/// Build prepared local index data for all documents already in `documents`.
pub(super) fn build_index_from_stored<'a>(
    documents: &ResourceStore<'a>,
    resolution_cache: &mut UriCache,
) -> Result<Index<'a>, Error> {
    let mut state = BuildState::new();
    let mut known_resources = KnownResources::default();

    for (doc_uri, document) in documents {
        known_resources.insert((**doc_uri).clone());
        state.index.register_document(doc_uri, document);
    }

    for (doc_uri, document) in documents {
        let document_root = document
            .borrowed_contents()
            .expect("build_index_from_stored is only called with borrowed documents");
        let mut visited_local_refs = VisitedLocalRefs::new();
        crawl_borrowed_at(
            Arc::clone(doc_uri),
            doc_uri,
            document_root,
            "",
            document.draft(),
            &mut state,
            &mut known_resources,
            resolution_cache,
            &mut visited_local_refs,
        )?;
    }
    Ok(state.index)
}

pub(super) fn validate_custom_metaschemas(
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

/// Shared sync processing loop used during registry preparation. After the
/// initial input has been ingested into `state`, this function drives the
/// BFS-fetch cycle until all reachable external resources have been retrieved,
/// then handles meta-schema injection and runs a final queue pass.
fn fetch_and_index<'a>(
    state: &mut BuildState<'a>,
    documents: &mut ResourceStore<'a>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    default_draft: Draft,
    retriever: &dyn Retrieve,
) -> Result<(), Error> {
    while !(state.queue.is_empty() && state.crawl.external.is_empty()) {
        flush_pending(state, documents, known_resources, resolution_cache)?;
        fetch_external_resources(state, documents, known_resources, default_draft, retriever)?;
    }

    finalize_index(
        state,
        documents,
        known_resources,
        resolution_cache,
        default_draft,
    )
}

/// Shared async processing loop used during registry preparation. Batches
/// concurrent external retrievals with `join_all` and otherwise mirrors
/// [`fetch_and_index`].
#[cfg(feature = "retrieve-async")]
async fn fetch_and_index_async<'a>(
    state: &mut BuildState<'a>,
    documents: &mut ResourceStore<'a>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    default_draft: Draft,
    retriever: &dyn crate::AsyncRetrieve,
) -> Result<(), Error> {
    while !(state.queue.is_empty() && state.crawl.external.is_empty()) {
        flush_pending(state, documents, known_resources, resolution_cache)?;
        fetch_external_resources_async(state, documents, known_resources, default_draft, retriever)
            .await?;
    }

    finalize_index(
        state,
        documents,
        known_resources,
        resolution_cache,
        default_draft,
    )
}

/// Convert resources into stored documents, register them with the
/// index, and enqueue them as the starting set for BFS traversal.
///
/// `draft` forces a specific draft for all resources; `None` means auto-detect per resource.
/// Resources are added to `known_resources` here so the retriever does not re-fetch them
/// during the BFS loop.
fn enqueue_resources<'a>(
    pairs: impl IntoIterator<Item = (Uri<String>, PendingResource<'a>)>,
    documents: &mut ResourceStore<'a>,
    known_resources: &mut KnownResources,
    state: &mut BuildState<'a>,
    draft: Option<Draft>,
) {
    for (uri, resource) in pairs {
        let key = Arc::new(uri);
        let (draft, document) = match resource {
            PendingResource::Value(value) => {
                let draft = draft.unwrap_or_else(|| Draft::default().detect(&value));
                (draft, StoredResource::owned(value, draft))
            }
            PendingResource::ValueRef(value) => {
                let draft = draft.unwrap_or_else(|| Draft::default().detect(value));
                (draft, StoredResource::borrowed(value, draft))
            }
            PendingResource::Resource(resource) => {
                let (draft, contents) = resource.into_inner();
                (draft, StoredResource::owned(contents, draft))
            }
            PendingResource::ResourceRef(resource) => {
                let draft = resource.draft();
                (draft, StoredResource::borrowed(resource.contents(), draft))
            }
        };
        let document = Arc::new(document);

        documents.insert(Arc::clone(&key), Arc::clone(&document));
        known_resources.insert((*key).clone());
        state.index.register_document(&key, &document);

        // Draft::Unknown means the resource declared a custom $schema; collect its URI
        // for post-build validation that a matching meta-schema was registered.
        if draft == Draft::Unknown {
            if let Some(meta) = document
                .contents()
                .as_object()
                .and_then(|obj| obj.get("$schema"))
                .and_then(|schema| schema.as_str())
            {
                state.custom_metaschemas.push(meta.to_string());
            }
        }

        state.queue.push_back(CrawlTask {
            base_uri: Arc::clone(&key),
            document_root_uri: key,
            pointer_path: String::new(),
            draft,
        });
    }
}

fn drain_queue<'r>(
    state: &mut BuildState<'r>,
    documents: &ResourceStore<'r>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
) -> Result<(), Error> {
    while let Some(CrawlTask {
        base_uri: base,
        document_root_uri: root_uri,
        pointer_path,
        draft,
    }) = state.queue.pop_front()
    {
        let Some(document) = documents.get(&root_uri) else {
            continue;
        };
        if let Some(document_root) = document.borrowed_contents() {
            let mut visited_local_refs = VisitedLocalRefs::new();
            crawl_borrowed_at(
                base,
                &root_uri,
                document_root,
                &pointer_path,
                draft,
                state,
                known_resources,
                resolution_cache,
                &mut visited_local_refs,
            )?;
            continue;
        }
        crawl_owned_at(
            base,
            &root_uri,
            document,
            &pointer_path,
            draft,
            state,
            known_resources,
            resolution_cache,
        )?;
    }
    Ok(())
}

fn flush_pending<'a>(
    state: &mut BuildState<'a>,
    documents: &ResourceStore<'a>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
) -> Result<(), Error> {
    let mut visited_local_refs = VisitedLocalRefs::new();
    drain_queue(state, documents, known_resources, resolution_cache)?;
    process_deferred_refs(state, documents, resolution_cache, &mut visited_local_refs)?;
    Ok(())
}

/// Process deferred local-ref targets collected during the main traversal.
///
/// Called after `drain_queue` finishes so that all subresource nodes are already in
/// `visited_schemas`. Targets that were visited by the main BFS (e.g. `#/definitions/Foo`
/// under a JSON Schema keyword) are skipped in O(1) via the pre-stored value address,
/// avoiding a redundant `pointer()` traversal. Non-subresource targets
/// (e.g. `#/components/schemas/Foo`) are still fully traversed. New deferred entries
/// added during traversal are also processed iteratively until none remain.
fn process_deferred_refs<'a>(
    state: &mut BuildState<'_>,
    documents: &'a ResourceStore<'a>,
    resolution_cache: &mut UriCache,
    visited_local_refs: &mut VisitedLocalRefs<'a>,
) -> Result<(), Error> {
    while !state.crawl.deferred_refs.is_empty() {
        let batch = std::mem::take(&mut state.crawl.deferred_refs);
        for PendingLocalRef {
            document_root_uri: doc_key,
            pointer: pointer_path,
            schema_ptr,
        } in batch
        {
            let Some(document) = documents.get(&doc_key) else {
                continue;
            };
            let root = document.contents();
            let Some((effective_base_uri, draft, contents)) = resolve_deferred_target(
                &doc_key,
                root,
                &pointer_path,
                document.draft(),
                resolution_cache,
            )?
            else {
                continue;
            };

            // Fast path: if this target was already visited by the main BFS traversal
            // (e.g. a `#/definitions/Foo` that `walk_children_with_path` descended into),
            // all its subresources were processed and `record_refs_from_object` was already
            // called on each — skip without a redundant `pointer()` traversal.
            if state
                .crawl
                .visited_schemas
                .contains(&visited_schema_context_from_ptr(
                    &effective_base_uri,
                    draft,
                    schema_ptr,
                ))
            {
                continue;
            }
            record_refs_recursive(
                &effective_base_uri,
                root,
                contents,
                &mut state.crawl,
                resolution_cache,
                draft,
                &doc_key,
                visited_local_refs,
            )?;
        }
    }
    Ok(())
}

/// Inject meta-schemas if referenced, then run a final queue pass to index them.
fn finalize_index<'a>(
    state: &mut BuildState<'a>,
    documents: &mut ResourceStore<'a>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    default_draft: Draft,
) -> Result<(), Error> {
    inject_metaschemas(
        state.crawl.found_metaschema_ref,
        documents,
        known_resources,
        default_draft,
        state,
    )?;

    if !state.queue.is_empty() {
        let mut visited_local_refs = VisitedLocalRefs::new();
        drain_queue(state, documents, known_resources, resolution_cache)?;
        process_deferred_refs(state, documents, resolution_cache, &mut visited_local_refs)?;
    }

    Ok(())
}

/// Fetch all pending external resources synchronously and enqueue the results.
fn fetch_external_resources<'a>(
    state: &mut BuildState<'a>,
    documents: &mut ResourceStore<'a>,
    known_resources: &mut KnownResources,
    default_draft: Draft,
    retriever: &dyn Retrieve,
) -> Result<(), Error> {
    for (original, uri, kind) in state.crawl.external.drain() {
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
            let (key, draft) = store_retrieved(
                retrieved,
                fragmentless,
                default_draft,
                documents,
                known_resources,
                &mut state.index,
                &mut state.custom_metaschemas,
            );
            enqueue_fragment_entry(&uri, &key, default_draft, documents, &mut state.queue);
            state.queue.push_back(CrawlTask {
                base_uri: Arc::clone(&key),
                document_root_uri: key,
                pointer_path: String::new(),
                draft,
            });
        }
    }
    Ok(())
}

/// Fetch all pending external resources concurrently and enqueue the results.
/// Groups requests by base URI and issues them in a single `join_all` batch.
#[cfg(feature = "retrieve-async")]
async fn fetch_external_resources_async<'a>(
    state: &mut BuildState<'a>,
    documents: &mut ResourceStore<'a>,
    known_resources: &mut KnownResources,
    default_draft: Draft,
    retriever: &dyn crate::AsyncRetrieve,
) -> Result<(), Error> {
    type ExternalRefsByBase = AHashMap<Uri<String>, Vec<(String, Uri<String>, ReferenceKind)>>;

    if state.crawl.external.is_empty() {
        return Ok(());
    }

    let mut grouped = ExternalRefsByBase::new();
    for (original, uri, kind) in state.crawl.external.drain() {
        let mut fragmentless = uri.clone();
        fragmentless.set_fragment(None);
        if !known_resources.contains(&fragmentless) {
            grouped
                .entry(fragmentless)
                .or_default()
                .push((original, uri, kind));
        }
    }

    // Use grouped.keys() for futures (borrows) then grouped.into_iter() for results (consumes).
    // The map is not mutated between the two iterations, so zip order is stable.
    let results = futures::future::join_all(grouped.keys().map(|u| retriever.retrieve(u))).await;

    for ((fragmentless, refs), result) in grouped.into_iter().zip(results) {
        let retrieved = match result {
            Ok(retrieved) => retrieved,
            Err(error) => {
                if let Some((original, uri, kind)) = refs.into_iter().next() {
                    handle_retrieve_error(&uri, &original, &fragmentless, error, kind)?;
                }
                continue;
            }
        };
        let (key, draft) = store_retrieved(
            retrieved,
            fragmentless,
            default_draft,
            documents,
            known_resources,
            &mut state.index,
            &mut state.custom_metaschemas,
        );
        for (_, uri, _) in &refs {
            enqueue_fragment_entry(uri, &key, default_draft, documents, &mut state.queue);
        }
        state.queue.push_back(CrawlTask {
            base_uri: Arc::clone(&key),
            document_root_uri: key,
            pointer_path: String::new(),
            draft,
        });
    }
    Ok(())
}

fn crawl_borrowed_at<'r>(
    current_base_uri: Arc<Uri<String>>,
    document_root_uri: &Arc<Uri<String>>,
    document_root: &'r Value,
    pointer_path: &str,
    draft: Draft,
    state: &mut BuildState<'r>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    visited: &mut VisitedLocalRefs<'r>,
) -> Result<(), Error> {
    let Some(subschema) = (if pointer_path.is_empty() {
        Some(document_root)
    } else {
        pointer(document_root, pointer_path)
    }) else {
        return Ok(());
    };

    crawl_schema(
        current_base_uri,
        document_root,
        subschema,
        draft,
        pointer_path.is_empty(),
        document_root_uri,
        state,
        known_resources,
        resolution_cache,
        visited,
    )
}

fn crawl_schema<'v>(
    mut base: Arc<Uri<String>>,
    document_root: &'v Value,
    subschema: &'v Value,
    draft: Draft,
    is_document_root: bool,
    document_root_uri: &Arc<Uri<String>>,
    state: &mut BuildState<'v>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    visited_local_refs: &mut VisitedLocalRefs<'v>,
) -> Result<(), Error> {
    let Some(object) = subschema.as_object() else {
        return Ok(());
    };
    let analysis = draft.analyze_object(object);

    if let Some(id) = analysis.id {
        let ResolvedId {
            base: new_base,
            uri_changed,
        } = resolve_subresource_id(&base, id, known_resources, resolution_cache)?;
        base = new_base;
        if !(is_document_root && base.as_ref() == document_root_uri.as_ref()) {
            state
                .index
                .register_borrowed_subresource(&base, draft, uri_changed, subschema);
        }
    } else if analysis.has_anchor && !is_document_root {
        state
            .index
            .register_borrowed_subresource(&base, draft, false, subschema);
    }

    if state
        .crawl
        .visited_schemas
        .insert(visited_schema_context(&base, draft, subschema))
        && (analysis.dollar_ref.is_some() || analysis.meta_schema.is_some())
    {
        record_refs(
            &base,
            document_root,
            analysis.dollar_ref,
            analysis.meta_schema,
            &mut state.crawl,
            resolution_cache,
            draft,
            document_root_uri,
            visited_local_refs,
        )?;
    }

    draft.walk_children(object, &mut |_key, _sub, child, child_draft| {
        crawl_schema(
            Arc::clone(&base),
            document_root,
            child,
            child_draft,
            false,
            document_root_uri,
            state,
            known_resources,
            resolution_cache,
            visited_local_refs,
        )
    })
}

fn crawl_owned_at<'r>(
    current_base_uri: Arc<Uri<String>>,
    document_root_uri: &Arc<Uri<String>>,
    document: &Arc<StoredResource<'r>>,
    pointer_path: &str,
    draft: Draft,
    state: &mut BuildState<'r>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
) -> Result<(), Error> {
    let document_root = document.contents();
    let subschema = if pointer_path.is_empty() {
        Some(document_root)
    } else {
        pointer(document_root, pointer_path)
    };
    let Some(subschema) = subschema else {
        return Ok(());
    };
    let parsed_pointer = ParsedPointer::from_json_pointer(pointer_path);
    let mut visited_local_refs = VisitedLocalRefs::new();

    with_pointer_node_from_parsed(parsed_pointer.as_ref(), |path| {
        crawl_owned_schema(
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
            &mut visited_local_refs,
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

fn crawl_owned_schema<'r, 'doc>(
    mut base: Arc<Uri<String>>,
    document_root: &'doc Value,
    subschema: &'doc Value,
    draft: Draft,
    is_document_root: bool,
    path: &JsonPointerNode<'_, '_>,
    document_root_uri: &Arc<Uri<String>>,
    document: &Arc<StoredResource<'r>>,
    state: &mut BuildState<'r>,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
    visited_local_refs: &mut VisitedLocalRefs<'doc>,
) -> Result<(), Error> {
    let Some(object) = subschema.as_object() else {
        return Ok(());
    };
    let analysis = draft.analyze_object(object);

    if let Some(id) = analysis.id {
        let ResolvedId {
            base: new_base,
            uri_changed,
        } = resolve_subresource_id(&base, id, known_resources, resolution_cache)?;
        base = new_base;
        if !(is_document_root && base.as_ref() == document_root_uri.as_ref())
            && (uri_changed || analysis.has_anchor)
        {
            let pointer = ParsedPointer::from_pointer_node(path);
            state.index.register_owned_subresource(
                &base,
                document,
                &pointer,
                draft,
                uri_changed,
                subschema,
            );
        }
    } else if analysis.has_anchor && !is_document_root {
        let pointer = ParsedPointer::from_pointer_node(path);
        state
            .index
            .register_owned_subresource(&base, document, &pointer, draft, false, subschema);
    }

    if state
        .crawl
        .visited_schemas
        .insert(visited_schema_context(&base, draft, subschema))
        && (analysis.dollar_ref.is_some() || analysis.meta_schema.is_some())
    {
        record_refs(
            &base,
            document_root,
            analysis.dollar_ref,
            analysis.meta_schema,
            &mut state.crawl,
            resolution_cache,
            draft,
            document_root_uri,
            visited_local_refs,
        )?;
    }

    draft.walk_children(object, &mut |key, sub, child_value, child_draft| {
        with_owned_child_path(path, key, sub.as_ref(), |child_path| {
            crawl_owned_schema(
                Arc::clone(&base),
                document_root,
                child_value,
                child_draft,
                false,
                child_path,
                document_root_uri,
                document,
                state,
                known_resources,
                resolution_cache,
                visited_local_refs,
            )
        })
    })
}

fn with_owned_child_path<R>(
    path: &JsonPointerNode<'_, '_>,
    key: &str,
    sub: Option<&JsonPointerSegment<'_>>,
    f: impl FnOnce(&JsonPointerNode<'_, '_>) -> R,
) -> R {
    let first = path.push(key);
    match sub {
        Some(JsonPointerSegment::Key(k)) => {
            let second = first.push(k.as_ref());
            f(&second)
        }
        Some(JsonPointerSegment::Index(i)) => {
            let second = first.push(*i);
            f(&second)
        }
        None => f(&first),
    }
}

fn record_refs<'doc>(
    base: &Arc<Uri<String>>,
    root: &'doc Value,
    dollar_ref: Option<&'doc str>,
    meta_schema: Option<&'doc str>,
    crawl: &mut CrawlState,
    resolution_cache: &mut UriCache,
    _draft: Draft,
    doc_key: &Arc<Uri<String>>,
    visited: &mut VisitedLocalRefs<'doc>,
) -> Result<(), Error> {
    for (reference, key) in [(dollar_ref, "$ref"), (meta_schema, "$schema")] {
        let Some(reference) = reference else {
            continue;
        };
        if reference.starts_with("https://json-schema.org/draft/")
            || reference.starts_with("http://json-schema.org/draft-")
            || base.as_str().starts_with("https://json-schema.org/draft/")
        {
            if key == "$ref" {
                crawl.found_metaschema_ref = true;
            }
            continue;
        }
        if reference == "#" {
            continue;
        }
        if reference.starts_with('#') {
            if record_local_ref_visit(visited, base, reference) {
                let pointer_path = reference.trim_start_matches('#');
                if let Some(referenced) = pointer(root, pointer_path) {
                    let schema_ptr = std::ptr::from_ref::<Value>(referenced) as usize;
                    crawl.deferred_refs.push(PendingLocalRef {
                        document_root_uri: Arc::clone(doc_key),
                        pointer: pointer_path.to_string(),
                        schema_ptr,
                    });
                }
            }
            continue;
        }
        let resolved = if base.has_fragment() {
            let mut base_without_fragment = base.as_ref().clone();
            base_without_fragment.set_fragment(None);

            let (path, fragment) = match reference.split_once('#') {
                Some((path, fragment)) => (path, Some(fragment)),
                None => (reference, None),
            };

            let mut resolved =
                (*resolution_cache.resolve_against(&base_without_fragment.borrow(), path)?).clone();
            if let Some(fragment) = fragment {
                if let Some(encoded) = uri::EncodedString::new(fragment) {
                    resolved = resolved.with_fragment(Some(encoded));
                } else {
                    uri::encode_to(fragment, &mut crawl.uri_scratch);
                    resolved = resolved
                        .with_fragment(Some(uri::EncodedString::new_or_panic(&crawl.uri_scratch)));
                    crawl.uri_scratch.clear();
                }
            }
            resolved
        } else {
            (*resolution_cache.resolve_against(&base.borrow(), reference)?).clone()
        };

        let kind = if key == "$schema" {
            ReferenceKind::DollarSchema
        } else {
            ReferenceKind::DollarRef
        };
        crawl
            .external
            .insert((reference.to_string(), resolved, kind));
    }
    Ok(())
}

fn record_refs_from_object<'doc>(
    base: &Arc<Uri<String>>,
    root: &'doc Value,
    contents: &'doc Value,
    crawl: &mut CrawlState,
    resolution_cache: &mut UriCache,
    draft: Draft,
    doc_key: &Arc<Uri<String>>,
    visited: &mut VisitedLocalRefs<'doc>,
) -> Result<(), Error> {
    if base.scheme().as_str() == "urn" {
        return Ok(());
    }
    if let Some(object) = contents.as_object() {
        let dollar_ref = object.get("$ref").and_then(Value::as_str);
        let meta_schema = object.get("$schema").and_then(Value::as_str);
        if dollar_ref.is_some() || meta_schema.is_some() {
            record_refs(
                base,
                root,
                dollar_ref,
                meta_schema,
                crawl,
                resolution_cache,
                draft,
                doc_key,
                visited,
            )?;
        }
    }
    Ok(())
}

fn record_refs_recursive<'doc>(
    base: &Arc<Uri<String>>,
    root: &'doc Value,
    contents: &'doc Value,
    crawl: &mut CrawlState,
    resolution_cache: &mut UriCache,
    draft: Draft,
    doc_key: &Arc<Uri<String>>,
    visited_refs: &mut VisitedLocalRefs<'doc>,
) -> Result<(), Error> {
    let current_base = match draft.id_of(contents) {
        Some(id) => resolve_id(base, id, resolution_cache)?,
        None => Arc::clone(base),
    };

    if !crawl
        .visited_schemas
        .insert(visited_schema_context(&current_base, draft, contents))
    {
        return Ok(());
    }

    record_refs_from_object(
        &current_base,
        root,
        contents,
        crawl,
        resolution_cache,
        draft,
        doc_key,
        visited_refs,
    )?;

    for subresource in draft.subresources_of(contents) {
        let subresource_draft = draft.detect(subresource);
        record_refs_recursive(
            &current_base,
            root,
            subresource,
            crawl,
            resolution_cache,
            subresource_draft,
            doc_key,
            visited_refs,
        )?;
    }
    Ok(())
}

fn enqueue_fragment_entry(
    uri: &Uri<String>,
    key: &Arc<Uri<String>>,
    default_draft: Draft,
    documents: &ResourceStore<'_>,
    queue: &mut VecDeque<CrawlTask>,
) {
    if let Some(fragment) = uri.fragment() {
        let Some(document) = documents.get(key) else {
            return;
        };
        if let Some(resolved) = pointer(document.contents(), fragment.as_str()) {
            let fragment_draft = default_draft.detect(resolved);
            queue.push_back(CrawlTask {
                base_uri: Arc::clone(key),
                document_root_uri: Arc::clone(key),
                pointer_path: fragment.as_str().to_string(),
                draft: fragment_draft,
            });
        }
    }
}

fn inject_metaschemas<'a>(
    found_metaschema_ref: bool,
    documents: &mut ResourceStore<'a>,
    known_resources: &mut KnownResources,
    draft_version: Draft,
    state: &mut BuildState<'a>,
) -> Result<(), Error> {
    if !found_metaschema_ref {
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
            Arc::new(StoredResource::borrowed(schema, draft)),
        );
        known_resources.insert((*key).clone());
        state.index.register_document(
            &key,
            documents
                .get(&key)
                .expect("meta-schema document was just inserted into the store"),
        );
        state.queue.push_back(CrawlTask {
            base_uri: Arc::clone(&key),
            document_root_uri: Arc::clone(&key),
            pointer_path: String::new(),
            draft,
        });
    }
    Ok(())
}

fn store_retrieved<'a>(
    retrieved: Value,
    fragmentless: Uri<String>,
    default_draft: Draft,
    documents: &mut ResourceStore<'a>,
    known_resources: &mut KnownResources,
    index: &mut Index<'a>,
    custom_metaschemas: &mut Vec<String>,
) -> (Arc<Uri<String>>, Draft) {
    let draft = default_draft.detect(&retrieved);
    let key = Arc::new(fragmentless);
    documents.insert(
        Arc::clone(&key),
        Arc::new(StoredResource::owned(retrieved, draft)),
    );

    let contents = documents
        .get(&key)
        .expect("document was just inserted")
        .contents();
    known_resources.insert((*key).clone());
    index.register_document(
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

fn record_local_ref_visit<'a>(
    visited_local_refs: &mut VisitedLocalRefs<'a>,
    base: &Arc<Uri<String>>,
    reference: &'a str,
) -> bool {
    visited_local_refs.insert((base_uri_ptr(base), reference))
}

fn resolve_deferred_target<'a>(
    document_root_uri: &Arc<Uri<String>>,
    document_root: &'a Value,
    pointer_path: &str,
    document_draft: Draft,
    resolution_cache: &mut UriCache,
) -> Result<Option<DeferredTarget<'a>>, Error> {
    let mut current = document_root;
    let mut current_draft = document_draft;
    let mut current_base = Arc::clone(document_root_uri);

    if let Some(id) = current_draft.id_of(current) {
        current_base = resolve_id(&current_base, id, resolution_cache)?;
    }

    let pointer = ParsedPointer::from_json_pointer(pointer_path);
    let Some(pointer) = pointer.as_ref() else {
        return Ok(Some((current_base, current_draft, current)));
    };

    for segment in &pointer.segments {
        let Some(next) = (match segment {
            ParsedPointerSegment::Key(key) => current
                .as_object()
                .and_then(|object| object.get(key.as_ref())),
            ParsedPointerSegment::Index(index) => {
                current.as_array().and_then(|array| array.get(*index))
            }
        }) else {
            return Ok(None);
        };
        current = next;
        current_draft = current_draft.detect(current);
        if let Some(id) = current_draft.id_of(current) {
            current_base = resolve_id(&current_base, id, resolution_cache)?;
        }
    }

    Ok(Some((current_base, current_draft, current)))
}

fn visited_schema_context(
    base: &Arc<Uri<String>>,
    draft: Draft,
    schema: &Value,
) -> VisitedSchemaContext {
    VisitedSchemaContext {
        schema_ptr: schema_ptr(schema),
        base_ptr: base_uri_ptr(base),
        draft,
    }
}

fn visited_schema_context_from_ptr(
    base: &Arc<Uri<String>>,
    draft: Draft,
    schema_ptr: usize,
) -> VisitedSchemaContext {
    VisitedSchemaContext {
        schema_ptr: NonZeroUsize::new(schema_ptr).expect("Value pointer should never be null"),
        base_ptr: base_uri_ptr(base),
        draft,
    }
}

fn base_uri_ptr(base: &Arc<Uri<String>>) -> NonZeroUsize {
    NonZeroUsize::new(Arc::as_ptr(base) as usize).expect("Arc pointer should never be null")
}

fn schema_ptr(schema: &Value) -> NonZeroUsize {
    NonZeroUsize::new(std::ptr::from_ref::<Value>(schema) as usize)
        .expect("Value pointer should never be null")
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
        ReferenceKind::DollarSchema => Ok(()),
        ReferenceKind::DollarRef => {
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

fn resolve_id(
    base: &Arc<Uri<String>>,
    id: &str,
    resolution_cache: &mut UriCache,
) -> Result<Arc<Uri<String>>, Error> {
    if id.starts_with('#') {
        return Ok(Arc::clone(base));
    }
    let normalized = id.strip_suffix('#').unwrap_or(id);
    resolution_cache.resolve_against(&base.borrow(), normalized)
}

fn resolve_subresource_id(
    current_base_uri: &Arc<Uri<String>>,
    id: &str,
    known_resources: &mut KnownResources,
    resolution_cache: &mut UriCache,
) -> Result<ResolvedId, Error> {
    let base = resolve_id(current_base_uri, id, resolution_cache)?;
    let uri_changed = base != *current_base_uri;
    known_resources.insert((*base).clone());
    Ok(ResolvedId { base, uri_changed })
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use ahash::AHashMap;
    use fluent_uri::Uri;
    use serde_json::{json, Value};

    use crate::{
        registry::SPECIFICATIONS, uri::from_str, Anchor, Draft, Registry, Resource, Retrieve,
    };

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
        assert!(registry.contains_resource("urn:root"));
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
    fn test_prepare_collects_relative_refs_for_each_borrowed_alias() {
        let schema = json!({"$ref": "remote.json"});
        let registry = Registry::new()
            .retriever(create_test_retriever(&[
                ("http://a/remote.json", json!({"type": "string"})),
                ("http://b/remote.json", json!({"type": "integer"})),
            ]))
            .add("http://a/root.json", &schema)
            .expect("First alias should be accepted")
            .add("http://b/root.json", &schema)
            .expect("Second alias should be accepted")
            .prepare()
            .expect("Registry should prepare");

        assert!(registry.contains_resource("http://a/remote.json"));
        assert!(registry.contains_resource("http://b/remote.json"));

        let resolver = registry.resolver(from_str("http://b/root.json").expect("Invalid root URI"));
        assert_eq!(
            resolver
                .lookup("remote.json")
                .expect("Borrowed alias should resolve its own relative ref")
                .contents(),
            &json!({"type": "integer"})
        );
    }

    #[test]
    fn test_prepare_collects_deferred_relative_refs_for_each_borrowed_alias() {
        let schema = json!({
            "$defs": {
                "remote": { "$ref": "remote.json" }
            },
            "$ref": "#/$defs/remote"
        });
        let registry = Registry::new()
            .retriever(create_test_retriever(&[
                ("http://a/remote.json", json!({"type": "string"})),
                ("http://b/remote.json", json!({"type": "integer"})),
            ]))
            .add("http://a/root.json", &schema)
            .expect("First alias should be accepted")
            .add("http://b/root.json", &schema)
            .expect("Second alias should be accepted")
            .prepare()
            .expect("Registry should prepare");

        assert!(registry.contains_resource("http://a/remote.json"));
        assert!(registry.contains_resource("http://b/remote.json"));
    }

    #[test]
    fn test_prepare_collects_deferred_refs_using_inherited_base_uri() {
        let schema = json!({
            "$id": "http://localhost:1234/scope_change_defs2.json",
            "type": "object",
            "properties": {
                "list": {"$ref": "#/definitions/baz/definitions/bar"}
            },
            "definitions": {
                "baz": {
                    "$id": "baseUriChangeFolderInSubschema/",
                    "definitions": {
                        "bar": {
                            "type": "array",
                            "items": {"$ref": "folderInteger.json"}
                        }
                    }
                }
            }
        });

        let registry = Registry::new()
            .retriever(create_test_retriever(&[(
                "http://localhost:1234/baseUriChangeFolderInSubschema/folderInteger.json",
                json!({"type": "integer"}),
            )]))
            .add("http://localhost:1234/scope_change_defs2.json", &schema)
            .expect("Schema should be accepted")
            .prepare()
            .expect("Registry should prepare");

        assert!(registry.contains_resource(
            "http://localhost:1234/baseUriChangeFolderInSubschema/folderInteger.json"
        ));
        assert!(!registry.contains_resource("http://localhost:1234/folderInteger.json"));
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

        assert!(registry.contains_resource("http://example.com/remote"));
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

        assert!(original.contains_resource("urn:one"));
        assert!(!original.contains_resource("urn:two"));
        assert!(registry.contains_resource("urn:one"));
        assert!(registry.contains_resource("urn:two"));
    }

    #[test]
    fn test_prepared_registry_can_be_extended_via_extend() {
        let original = Registry::new()
            .add("urn:one", json!({"type": "string"}))
            .expect("Resource should be accepted")
            .prepare()
            .expect("Registry should prepare");

        let registry = original
            .extend([
                ("urn:two", json!({"type": "integer"})),
                ("urn:three", json!({"type": "boolean"})),
            ])
            .expect("Resources should be accepted")
            .prepare()
            .expect("Registry should prepare");

        assert!(original.contains_resource("urn:one"));
        assert!(!original.contains_resource("urn:two"));
        assert!(!original.contains_resource("urn:three"));
        assert!(registry.contains_resource("urn:one"));
        assert!(registry.contains_resource("urn:two"));
        assert!(registry.contains_resource("urn:three"));
    }

    #[test]
    fn test_registry_builder_accepts_borrowed_values() {
        let schema = json!({"type": "string"});
        let registry = Registry::new()
            .add("urn:test", &schema)
            .expect("Resource should be accepted")
            .prepare()
            .expect("Registry should prepare");

        assert!(registry.contains_resource("urn:test"));
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
    fn test_invalid_reference() {
        let resource = Draft::Draft202012.create_resource(json!({"$schema": "$##"}));
        let _ = Registry::new()
            .add("http://#/", resource)
            .and_then(crate::registry::RegistryBuilder::prepare);
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
