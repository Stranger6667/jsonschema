//! Builder for constructing Registry instances with borrowed or owned documents.

use crate::{registry::DocumentVec, uri, Draft, Error, IntoDocument, Retrieve};
use ahash::AHashMap;
use fluent_uri::Uri;
use serde_json::Value;
use std::{borrow::Cow, sync::Arc};

/// Builder for creating a [`Registry`](crate::Registry).
///
/// The builder pattern ensures that all documents are collected before crawling
/// external references, and provides a clean API for both borrowed and owned schemas.
///
/// # Examples
///
/// ```rust
/// use referencing::Registry;
/// use serde_json::json;
///
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let schema1 = json!({"type": "string"});
/// let schema2 = json!({"type": "number"});
///
/// // Borrowed schemas (zero-copy)
/// let registry = Registry::builder()
///     .with_document("https://example.com/schema1", &schema1)?
///     .with_document("https://example.com/schema2", &schema2)?
///     .build()?;
/// # Ok(())
/// # }
/// ```
pub struct RegistryBuilder<'doc> {
    documents: AHashMap<Arc<Uri<String>>, (Cow<'doc, Value>, Draft)>,
    retriever: Option<Arc<dyn Retrieve>>,
}

impl<'doc> RegistryBuilder<'doc> {
    /// Create a new empty builder.
    #[must_use]
    pub fn new() -> RegistryBuilder<'doc> {
        RegistryBuilder {
            documents: AHashMap::new(),
            retriever: None,
        }
    }

    /// Add a document to the registry.
    ///
    /// This method accepts any type implementing [`IntoDocument`], which includes:
    /// - `&'doc Value` - borrowed schema (auto-detect draft)
    /// - `Value` - owned schema (auto-detect draft)
    /// - `(&'doc Value, Draft)` - borrowed with explicit draft
    /// - `(Value, Draft)` - owned with explicit draft
    /// - `Resource` - existing resource type
    ///
    /// # Examples
    ///
    /// ```rust
    /// use referencing::{Registry, Draft};
    /// use serde_json::json;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let schema = json!({"type": "string"});
    ///
    /// let registry = Registry::builder()
    ///     // Borrowed, auto-detect
    ///     .with_document("https://example.com/a", &schema)?
    ///     // Owned, auto-detect
    ///     .with_document("https://example.com/b", json!({"type": "number"}))?
    ///     // Borrowed, explicit draft
    ///     .with_document("https://example.com/c", (&schema, Draft::Draft7))?
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if the URI is invalid.
    pub fn with_document(
        mut self,
        uri: &str,
        doc: impl IntoDocument<'doc>,
    ) -> Result<RegistryBuilder<'doc>, Error> {
        let (cow, draft) = doc.into_document();
        let parsed_uri = uri::from_str(uri.trim_end_matches('#'))?;
        self.documents.insert(Arc::new(parsed_uri), (cow, draft));
        Ok(self)
    }

    /// Set the retriever for fetching external references.
    ///
    /// If a retriever is provided, the builder will recursively fetch all
    /// `$ref` references found in the documents during `build()`.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use referencing::{Registry, DefaultRetriever};
    /// use serde_json::json;
    ///
    /// # fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let schema = json!({
    ///     "properties": {
    ///         "name": {"$ref": "https://example.com/name-schema.json"}
    ///     }
    /// });
    ///
    /// let registry = Registry::builder()
    ///     .with_document("https://example.com/root", &schema)?
    ///     .with_retriever(DefaultRetriever)
    ///     .build()?; // Will fetch name-schema.json
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_retriever(mut self, retriever: impl crate::IntoRetriever) -> RegistryBuilder<'doc> {
        self.retriever = Some(retriever.into_retriever());
        self
    }

    /// Build the registry, fetching any external references if a retriever was provided.
    ///
    /// This consumes the builder and returns a fully constructed [`Registry`](crate::Registry).
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Any external reference cannot be retrieved
    /// - Any URI is invalid
    /// - Circular references are detected
    pub fn build(self) -> Result<crate::Registry<'doc>, Error> {
        use crate::DefaultRetriever;

        // Convert documents to the format expected by process_builder_documents
        let pairs: Vec<_> = self.documents.into_iter().collect();

        let draft = Draft::default(); // TODO: Allow configuring default draft
        let retriever = self.retriever.unwrap_or_else(|| Arc::new(DefaultRetriever));
        build_registry_with_retriever(pairs, retriever, draft)
    }
}

impl Default for RegistryBuilder<'_> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "retrieve-async")]
impl<'doc> RegistryBuilder<'doc> {
    /// Set an async retriever for fetching external references.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use referencing::{Registry, Resource};
    /// use serde_json::json;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let schema = json!({
    ///     "properties": {
    ///         "name": {"$ref": "https://example.com/name-schema.json"}
    ///     }
    /// });
    ///
    /// struct MyRetriever;
    ///
    /// #[cfg_attr(target_family = "wasm", async_trait::async_trait(?Send))]
    /// #[cfg_attr(not(target_family = "wasm"), async_trait::async_trait)]
    /// impl referencing::AsyncRetrieve for MyRetriever {
    ///     async fn retrieve(
    ///         &self,
    ///         _uri: &referencing::Uri<String>,
    ///     ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    ///         Ok(json!({"type": "string"}))
    ///     }
    /// }
    ///
    /// let registry = Registry::builder()
    ///     .with_document("https://example.com/root", &schema)?
    ///     .with_async_retriever(MyRetriever)
    ///     .build_async().await?;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn with_async_retriever(
        self,
        retriever: impl crate::IntoAsyncRetriever,
    ) -> AsyncRegistryBuilder<'doc> {
        AsyncRegistryBuilder {
            documents: self.documents,
            retriever: retriever.into_retriever(),
        }
    }
}

/// Builder for creating a [`Registry`](crate::Registry) with async retrieval.
#[cfg(feature = "retrieve-async")]
pub struct AsyncRegistryBuilder<'doc> {
    documents: AHashMap<Arc<Uri<String>>, (Cow<'doc, Value>, Draft)>,
    retriever: Arc<dyn crate::AsyncRetrieve>,
}

#[cfg(feature = "retrieve-async")]
impl<'doc> AsyncRegistryBuilder<'doc> {
    /// Add a document to the registry.
    ///
    /// See [`RegistryBuilder::with_document`] for details.
    pub fn with_document(mut self, uri: &str, doc: impl IntoDocument<'doc>) -> Result<Self, Error> {
        let (cow, draft) = doc.into_document();
        let parsed_uri = uri::from_str(uri.trim_end_matches('#'))?;
        self.documents.insert(Arc::new(parsed_uri), (cow, draft));
        Ok(self)
    }

    /// Build the registry asynchronously, fetching any external references.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Any external reference cannot be retrieved
    /// - Any URI is invalid
    /// - Circular references are detected
    pub async fn build_async(self) -> Result<crate::Registry<'doc>, Error> {
        let pairs: Vec<_> = self.documents.into_iter().collect();
        build_registry_with_async_retriever(pairs, self.retriever, Draft::default()).await
    }
}

/// Helper function to build a registry from documents with a retriever.
///
/// This function:
/// 1. Stores initial documents in the registry
/// 2. Uses temporary resource/anchor maps to discover external references
/// 3. Recursively fetches all external references
/// 4. Stores all fetched documents in the registry
/// 5. Returns a Registry containing ONLY documents (resources/anchors computed later by `ResolutionContext`)
pub(crate) fn build_registry_with_retriever(
    documents: DocumentVec<'_>,
    retriever: Arc<dyn Retrieve>,
    default_draft: Draft,
) -> Result<crate::Registry<'_>, Error> {
    use crate::{
        cache::UriCache,
        registry::{
            create_resource, handle_fragment, handle_retrieve_error, ProcessingState, ResourceMap,
        },
        resource::InnerResourcePtr,
    };
    use ahash::AHashMap;

    let mut doc_store = AHashMap::new();
    let mut resolution_cache = UriCache::new();

    // Temporary maps used ONLY for discovering what to fetch
    // These will NOT be stored in the Registry - ResolutionContext will rebuild them
    let mut resources = ResourceMap::new();
    let mut anchors = AHashMap::new();
    let mut state = ProcessingState::new();

    // PHASE 1: Insert all initial documents into doc_store first
    // We must complete all insertions BEFORE creating any pointers, otherwise
    // HashMap reallocation will invalidate pointers
    let mut initial_uris = Vec::new();

    // Start with SPECIFICATIONS meta-schemas
    for (uri, (cow, draft)) in crate::SPECIFICATIONS.documents() {
        doc_store.insert(uri.clone(), (Cow::Borrowed(cow.as_ref()), *draft));
        initial_uris.push((uri.clone(), *draft));
    }

    // Add user-provided documents
    for (uri, (cow, draft)) in documents {
        use std::collections::hash_map::Entry;

        // Only insert if URI doesn't already exist (keep first occurrence)
        if let Entry::Vacant(entry) = doc_store.entry(uri.clone()) {
            entry.insert((cow, draft));
            initial_uris.push((uri, draft));
        }
        // Skip duplicate - keep the first one (SPECIFICATIONS takes precedence)
    }

    // PHASE 2: Now that doc_store is stable, create resource pointers
    for (uri, draft) in initial_uris {
        // Create temporary resource pointer for discovery
        let stored_value = match &doc_store[&uri].0 {
            std::borrow::Cow::Owned(v) => v as *const Value,
            std::borrow::Cow::Borrowed(v) => *v as *const Value,
        };

        let resource_ptr = InnerResourcePtr::new(stored_value, draft);
        resources.insert(uri.clone(), resource_ptr.clone());

        // Track custom metaschemas
        if draft == Draft::Unknown {
            state.custom_metaschemas.push(uri.clone());
        }

        state.queue.push_back((uri, resource_ptr));
    }

    // Process queue and fetch external references
    loop {
        if state.queue.is_empty() && state.external.is_empty() {
            break;
        }

        // Process queue - this discovers external refs
        crate::registry::process_queue(
            &mut state,
            &mut resources,
            &mut anchors,
            &mut resolution_cache,
        )?;

        // Retrieve external resources
        for (original, uri, kind, ref_draft) in state.external.drain() {
            let mut fragmentless = uri.clone();
            fragmentless.set_fragment(None);
            if !resources.contains_key(&fragmentless) {
                let retrieved = match retriever.retrieve(&fragmentless) {
                    Ok(retrieved) => retrieved,
                    Err(error) => {
                        handle_retrieve_error(&uri, &original, &fragmentless, error, kind)?;
                        continue;
                    }
                };

                // Use the referencing document's draft for fetched resources
                // This ensures remote documents are processed with the correct draft
                let (key, resource) = create_resource(
                    retrieved,
                    fragmentless,
                    ref_draft,
                    &mut doc_store,
                    &mut resources,
                    &mut state.custom_metaschemas,
                );
                handle_fragment(&uri, &resource, &key, ref_draft, &mut state.queue);
                state.queue.push_back((key, resource));
            }
        }
    }

    // Validate custom metaschemas
    crate::registry::validate_custom_metaschemas(&state.custom_metaschemas, &resources)?;

    // Return registry with ONLY documents
    // Resources and anchors are temporary and discarded
    // ResolutionContext will recompute them from documents
    Ok(crate::Registry {
        documents: doc_store,
        resolution_cache: resolution_cache.into_shared(),
        retriever,
        #[cfg(feature = "retrieve-async")]
        async_retriever: None,
        draft: default_draft,
    })
}

#[cfg(feature = "retrieve-async")]
#[allow(clippy::elidable_lifetime_names)]
pub(crate) async fn build_registry_with_async_retriever<'doc>(
    documents: DocumentVec<'doc>,
    retriever: Arc<dyn crate::AsyncRetrieve>,
    default_draft: Draft,
) -> Result<crate::Registry<'doc>, Error> {
    use crate::{
        cache::UriCache,
        registry::{
            create_resource, handle_fragment, handle_retrieve_error, ProcessingState, ResourceMap,
        },
        resource::InnerResourcePtr,
    };
    use ahash::AHashMap;

    let mut doc_store = AHashMap::new();
    let mut resolution_cache = UriCache::new();

    let mut resources = ResourceMap::new();
    let mut anchors = AHashMap::new();
    let mut state = ProcessingState::new();
    let mut initial_uris = Vec::new();

    for (uri, (cow, draft)) in crate::SPECIFICATIONS.documents() {
        doc_store.insert(uri.clone(), (Cow::Borrowed(cow.as_ref()), *draft));
        initial_uris.push((uri.clone(), *draft));
    }

    for (uri, (cow, draft)) in documents {
        use std::collections::hash_map::Entry;
        if let Entry::Vacant(entry) = doc_store.entry(uri.clone()) {
            entry.insert((cow, draft));
            initial_uris.push((uri, draft));
        }
    }

    for (uri, draft) in initial_uris {
        let stored_value = match &doc_store[&uri].0 {
            std::borrow::Cow::Owned(v) => v as *const Value,
            std::borrow::Cow::Borrowed(v) => *v as *const Value,
        };

        let resource_ptr = InnerResourcePtr::new(stored_value, draft);
        resources.insert(uri.clone(), resource_ptr.clone());

        if draft == Draft::Unknown {
            state.custom_metaschemas.push(uri.clone());
        }

        state.queue.push_back((uri, resource_ptr));
    }

    loop {
        if state.queue.is_empty() && state.external.is_empty() {
            break;
        }

        crate::registry::process_queue(
            &mut state,
            &mut resources,
            &mut anchors,
            &mut resolution_cache,
        )?;

        for (original, uri, kind, ref_draft) in state.external.drain() {
            let mut fragmentless = uri.clone();
            fragmentless.set_fragment(None);
            if resources.contains_key(&fragmentless) {
                continue;
            }

            let retrieved = match retriever.retrieve(&fragmentless).await {
                Ok(retrieved) => retrieved,
                Err(error) => {
                    handle_retrieve_error(&uri, &original, &fragmentless, error, kind)?;
                    continue;
                }
            };

            let (key, resource) = create_resource(
                retrieved,
                fragmentless,
                ref_draft,
                &mut doc_store,
                &mut resources,
                &mut state.custom_metaschemas,
            );
            handle_fragment(&uri, &resource, &key, ref_draft, &mut state.queue);
            state.queue.push_back((key, resource));
        }
    }

    crate::registry::validate_custom_metaschemas(&state.custom_metaschemas, &resources)?;

    Ok(crate::Registry {
        documents: doc_store,
        resolution_cache: resolution_cache.into_shared(),
        retriever: Arc::new(crate::DefaultRetriever),
        #[cfg(feature = "retrieve-async")]
        async_retriever: Some(retriever),
        draft: default_draft,
    })
}
