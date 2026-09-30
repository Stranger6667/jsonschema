use crate::{
    content_encoding::{ContentEncodingCheckType, ContentEncodingConverterType},
    content_media_type::ContentMediaTypeCheckType,
    keywords::{
        self,
        custom::{CustomKeyword, KeywordFactory},
        format::Format,
        unevaluated_items::PendingItemsValidators,
        unevaluated_properties::PendingPropertyValidators,
        BoxedValidator, BuiltinKeyword, Keyword,
    },
    node::{PendingSchemaNode, SchemaNode},
    options::{PatternEngineOptions, ValidationOptions},
    paths::{Location, LocationSegment},
    types::{JsonType, JsonTypeSet},
    validator::Validate,
    Json, LazyInstance, SerdeJson, ValidationError, Validator, ValidatorMap,
};
use ahash::{AHashMap, AHashSet};
use referencing::{
    uri, write_escaped_str, Draft, List, Registry, Resolved, Resolver, ResourceRef, Uri,
    Vocabulary, VocabularySet,
};
use serde_json::{Map, Value};
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::VecDeque,
    fmt,
    rc::Rc,
    sync::{Arc, LazyLock},
};

pub(crate) const DEFAULT_SCHEME: &str = "json-schema";
pub(crate) const DEFAULT_BASE_URI: &str = "json-schema:///";

pub(crate) const fn formats_are_assertions_by_default(draft: Draft) -> bool {
    matches!(draft, Draft::Draft4 | Draft::Draft6 | Draft::Draft7)
}

/// Type alias for shared cache maps in compiler state.
type SharedCache<K, V> = RefCell<AHashMap<K, V>>;
/// Type alias for shared sets in compiler state.
type SharedSet<T> = RefCell<AHashSet<T>>;

pub(crate) trait CompilationOptions<F: Json> {
    fn validate_formats(&self) -> Option<bool>;
    fn declares_vocabulary(&self, uri: &str) -> bool;
    fn are_unknown_formats_ignored(&self) -> bool;
    fn get_content_media_type_check(&self, media_type: &str) -> Option<ContentMediaTypeCheckType>;
    fn content_encoding_check(&self, content_encoding: &str) -> Option<ContentEncodingCheckType>;
    fn get_content_encoding_convert(
        &self,
        content_encoding: &str,
    ) -> Option<ContentEncodingConverterType>;
    fn get_keyword_factory(&self, name: &str) -> Option<&Arc<dyn KeywordFactory<F>>>;
    fn get_format(&self, format: &str) -> Option<(&String, &Arc<dyn Format>)>;
    fn pattern_options(&self) -> PatternEngineOptions;
    fn email_options(&self) -> Option<&email_address::Options>;
}

impl<R, F: Json> CompilationOptions<F> for ValidationOptions<'_, R, F> {
    fn validate_formats(&self) -> Option<bool> {
        ValidationOptions::validate_formats(self)
    }

    fn declares_vocabulary(&self, uri: &str) -> bool {
        ValidationOptions::declares_vocabulary(self, uri)
    }

    fn are_unknown_formats_ignored(&self) -> bool {
        ValidationOptions::are_unknown_formats_ignored(self)
    }

    fn get_content_media_type_check(&self, media_type: &str) -> Option<ContentMediaTypeCheckType> {
        ValidationOptions::get_content_media_type_check(self, media_type)
    }

    fn content_encoding_check(&self, content_encoding: &str) -> Option<ContentEncodingCheckType> {
        ValidationOptions::content_encoding_check(self, content_encoding)
    }

    fn get_content_encoding_convert(
        &self,
        content_encoding: &str,
    ) -> Option<ContentEncodingConverterType> {
        ValidationOptions::get_content_encoding_convert(self, content_encoding)
    }

    fn get_keyword_factory(&self, name: &str) -> Option<&Arc<dyn KeywordFactory<F>>> {
        ValidationOptions::get_keyword_factory(self, name)
    }

    fn get_format(&self, format: &str) -> Option<(&String, &Arc<dyn Format>)> {
        ValidationOptions::get_format(self, format)
    }

    fn pattern_options(&self) -> PatternEngineOptions {
        ValidationOptions::compiler_pattern_options(self)
    }

    fn email_options(&self) -> Option<&email_address::Options> {
        ValidationOptions::compiler_email_options(self)
    }
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub(crate) struct LocationCacheKey {
    pub(crate) base_uri: Arc<Uri<String>>,
    location: Arc<str>,
    dynamic_scope: List<Uri<String>>,
}

/// Named anchors can share a base URI and output location without naming the same schema.
/// The node cache lives only for one root compilation: input/registry values stay borrowed
/// and immovable until that cache is dropped. This pointer is never persisted or dereferenced.
#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub(crate) struct NodeCacheKey {
    location: LocationCacheKey,
    schema_ptr: usize,
}

#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
struct PropertyValidatorsPendingKey {
    schema_ptr: usize,
}

impl PropertyValidatorsPendingKey {
    fn new(schema: &Map<String, Value>) -> Self {
        Self {
            schema_ptr: std::ptr::from_ref(schema) as usize,
        }
    }
}

#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
struct ItemsValidatorsPendingKey {
    schema_ptr: usize,
}

impl ItemsValidatorsPendingKey {
    fn new(schema: &Map<String, Value>) -> Self {
        Self {
            schema_ptr: std::ptr::from_ref(schema) as usize,
        }
    }
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
pub(crate) struct AliasCacheKey {
    uri: Arc<Uri<String>>,
    dynamic_scope: List<Uri<String>>,
}

/// A `$ref`'s resolved URI and the location its target starts at.
type RefTarget = (Arc<Uri<String>>, Location);

/// Base URIs are interned, so identity keys them without hashing the whole URI. Holding the `Arc`
/// keeps the address from being reused.
#[derive(Clone)]
struct BaseUriKey(Arc<Uri<String>>);

impl PartialEq for BaseUriKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for BaseUriKey {}

impl std::hash::Hash for BaseUriKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::ptr::addr_of!(*self.0).hash(state);
    }
}

/// Nested `$ref` targets compiled on the call stack before the next goes to the worklist; each
/// takes a few KiB of stack.
const MAX_NESTED_REF_COMPILATIONS: usize = 8;

/// A `$ref` target left for the worklist.
struct DeferredTarget<'a, F: Json> {
    contents: &'a Value,
    resolver: Resolver<'a>,
    draft: Draft,
    resource_base: Location,
    alias: Arc<Uri<String>>,
    key: AliasCacheKey,
    placeholder: PendingSchemaNode<F>,
    /// Placeholders in progress when this target was deferred, restored while it compiles so cycles
    /// through them close as on the call stack.
    in_progress: AHashMap<Arc<Uri<String>>, PendingSchemaNode<F>>,
}

/// A cached node, the round that compiled it, and whether it is settled.
///
/// Round 0 is the schema, each later round one deferred target. A settled node reaches only
/// compiled nodes and no deferred target: no `$ref` cycle passes through it, and owning it keeps
/// no later round alive, so any round may own it. An unsettled node is owned only from its own
/// round, and a deferred target only from earlier ones, so `Arc` edges form no cycle.
struct CachedNode<F: Json> {
    node: SchemaNode<F>,
    round: usize,
    settled: bool,
}

impl<F: Json> Clone for CachedNode<F> {
    fn clone(&self) -> Self {
        CachedNode {
            node: self.node.clone(),
            round: self.round,
            settled: self.settled,
        }
    }
}

/// A node found for a `$ref` target and whether its referrer may own it.
pub(crate) enum TargetNode<F: Json> {
    /// Compiled in the current round, or settled.
    Owned(SchemaNode<F>),
    /// An unsettled node from an earlier round; the tree owns it.
    Shared(SchemaNode<F>),
}

impl<F: Json> TargetNode<F> {
    /// The node, however it was found.
    pub(crate) fn into_node(self) -> SchemaNode<F> {
        match self {
            TargetNode::Owned(node) | TargetNode::Shared(node) => node,
        }
    }

    pub(crate) fn into_validator(self) -> Box<dyn Validate<F>> {
        match self {
            TargetNode::Owned(node) => Box::new(node),
            TargetNode::Shared(node) => Box::new(PendingSchemaNode::pointing_at(&node)),
        }
    }
}

/// Keyed by dynamic scope, since `$dynamicRef` resolves per scope.
fn deferred_key(resolved: &Resolved<'_>, alias: &Arc<Uri<String>>) -> AliasCacheKey {
    AliasCacheKey {
        uri: Arc::clone(alias),
        dynamic_scope: resolved.resolver().dynamic_scope(),
    }
}

/// Shared caches reused across every `Context` derived from a schema root.
struct SharedContextState<'a, F: Json = SerdeJson> {
    seen: SharedSet<Arc<Uri<String>>>,
    location_nodes: SharedCache<NodeCacheKey, CachedNode<F>>,
    alias_nodes: SharedCache<AliasCacheKey, CachedNode<F>>,
    alias_placeholders: SharedCache<Arc<Uri<String>>, PendingSchemaNode<F>>,
    /// Deferred targets whose round has not started, one per dynamic scope.
    deferred_placeholders: SharedCache<AliasCacheKey, PendingSchemaNode<F>>,
    pending_property_validators_by_schema:
        SharedCache<PropertyValidatorsPendingKey, PendingPropertyValidators<F>>,
    pending_items_validators_by_schema:
        SharedCache<ItemsValidatorsPendingKey, PendingItemsValidators<F>>,
    pattern_cache: SharedCache<Arc<str>, PatternCacheEntry>,
    ref_targets: SharedCache<BaseUriKey, AHashMap<Box<str>, RefTarget>>,
    /// Locations of anchor-bearing schemas, keyed by their resource root's address and then
    /// by their own. Roots stay borrowed until this cache is dropped.
    anchor_locations: SharedCache<usize, AHashMap<usize, Location>>,
    uri_buffer: RefCell<uri::EncodedBuffer>,
    /// `$ref` targets compiling on the call stack.
    nested_ref_compilations: Cell<usize>,
    /// Deferred targets, in the order found.
    deferred_targets: RefCell<VecDeque<DeferredTarget<'a, F>>>,
    /// The current round.
    round: Cell<usize>,
    /// The node compiling now reaches a node not yet compiled or a deferred target.
    reaches_unsettled: Cell<bool>,
}

impl<F: Json> fmt::Debug for SharedContextState<'_, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedContextState").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
struct PatternCacheEntry {
    translated: Arc<str>,
    fancy: Option<Arc<fancy_regex::Regex>>,
    standard: Option<Arc<regex::Regex>>,
}

impl<F: Json> SharedContextState<'_, F> {
    /// `capacity` pre-sizes the per-location node cache to avoid rehashing during a build.
    fn new(capacity: usize) -> Self {
        Self {
            seen: RefCell::new(AHashSet::new()),
            location_nodes: RefCell::new(AHashMap::with_capacity(capacity)),
            alias_nodes: RefCell::new(AHashMap::new()),
            alias_placeholders: RefCell::new(AHashMap::new()),
            deferred_placeholders: RefCell::new(AHashMap::new()),
            pending_property_validators_by_schema: RefCell::new(AHashMap::new()),
            pending_items_validators_by_schema: RefCell::new(AHashMap::new()),
            pattern_cache: RefCell::new(AHashMap::new()),
            ref_targets: RefCell::new(AHashMap::new()),
            anchor_locations: RefCell::new(AHashMap::new()),
            uri_buffer: RefCell::new(uri::EncodedBuffer::new()),
            nested_ref_compilations: Cell::new(0),
            deferred_targets: RefCell::new(VecDeque::new()),
            round: Cell::new(0),
            reaches_unsettled: Cell::new(false),
        }
    }
}

/// Per-location view used while compiling schemas into validators.
pub(crate) struct Context<'a, F: Json = SerdeJson> {
    config: &'a dyn CompilationOptions<F>,
    resolver: Resolver<'a>,
    vocabularies: VocabularySet,
    location: Location,
    /// The location where the current resource starts.
    ///
    /// When compiling a schema reached via `$ref`, this is set to the `$ref` target location.
    /// Used to compute the "suffix" (path relative to resource root) for evaluation paths.
    ///
    /// # Example
    ///
    /// ```text
    /// Schema: { "$ref": "#/$defs/Item", "$defs": { "Item": { "type": "string" } } }
    ///
    /// When compiling the "type" keyword inside "Item":
    ///   location      = /$defs/Item/type
    ///   resource_base = /$defs/Item
    ///   suffix()      = /type
    /// ```
    resource_base: Location,
    /// Length of the `location` prefix outside the resource the base URI names.
    ///
    /// Absolute locations count their JSON Pointer from that resource's root, which a
    /// subschema declaring its own `$id` moves below the document root.
    resource_start: usize,
    pub(crate) draft: Draft,
    shared: Rc<SharedContextState<'a, F>>,
}

impl<F: Json> Clone for Context<'_, F> {
    fn clone(&self) -> Self {
        Context {
            config: self.config,
            resolver: self.resolver.clone(),
            vocabularies: self.vocabularies.clone(),
            location: self.location.clone(),
            resource_base: self.resource_base.clone(),
            resource_start: self.resource_start,
            draft: self.draft,
            shared: Rc::clone(&self.shared),
        }
    }
}

impl<'a, F: Json> Context<'a, F> {
    pub(crate) fn new(
        config: &'a dyn CompilationOptions<F>,
        resolver: Resolver<'a>,
        vocabularies: VocabularySet,
        draft: Draft,
        location: Location,
        capacity: usize,
    ) -> Self {
        Context {
            config,
            resolver,
            resource_base: location.clone(),
            resource_start: 0,
            location,
            vocabularies,
            draft,
            shared: Rc::new(SharedContextState::new(capacity)),
        }
    }
    pub(crate) fn draft(&self) -> Draft {
        self.draft
    }
    pub(crate) fn resolver(&self) -> &Resolver<'a> {
        &self.resolver
    }
    pub(crate) fn config(&self) -> &dyn CompilationOptions<F> {
        self.config
    }

    /// Create a context for this schema.
    pub(crate) fn in_subresource(
        &self,
        resource: ResourceRef<'_>,
    ) -> Result<Context<'a, F>, referencing::Error> {
        let resolver = self.resolver.in_subresource(resource)?;
        let draft = resource.draft();
        // A `$schema` below the root switches the draft, and its vocabularies with it: Draft 4-7
        // roots have none, so a nested Draft 2019-09+ subschema would compile no keywords.
        let vocabularies = if draft == self.draft {
            self.vocabularies.clone()
        } else {
            resolver.find_vocabularies(draft, resource.contents())
        };
        let resource_start = if resource.id().is_some() {
            self.location.as_str().len()
        } else {
            self.resource_start
        };
        Ok(Context {
            config: self.config,
            resolver,
            vocabularies,
            draft,
            resource_base: self.resource_base.clone(),
            resource_start,
            location: self.location.clone(),
            shared: Rc::clone(&self.shared),
        })
    }
    pub(crate) fn as_resource_ref<'r>(&self, contents: &'r Value) -> ResourceRef<'r> {
        self.draft.detect(contents).create_resource_ref(contents)
    }

    #[inline]
    pub(crate) fn new_at_location<'s>(&self, chunk: impl Into<LocationSegment<'s>>) -> Self {
        let location = self.location.join(chunk);
        Context {
            config: self.config,
            resolver: self.resolver.clone(),
            vocabularies: self.vocabularies.clone(),
            resource_base: self.resource_base.clone(),
            resource_start: self.resource_start,
            location,
            draft: self.draft,
            shared: Rc::clone(&self.shared),
        }
    }
    pub(crate) fn lookup(&self, reference: &str) -> Result<Resolved<'a>, referencing::Error> {
        self.resolver.lookup(reference)
    }

    pub(crate) fn location_cache_key(&self) -> LocationCacheKey {
        LocationCacheKey {
            base_uri: self.resolver.base_uri(),
            location: self.location.as_arc(),
            dynamic_scope: self.resolver.dynamic_scope(),
        }
    }

    fn alias_cache_key(&self, alias: Arc<Uri<String>>) -> AliasCacheKey {
        AliasCacheKey {
            uri: alias,
            dynamic_scope: self.resolver.dynamic_scope(),
        }
    }

    pub(crate) fn base_uri(&self) -> Option<Arc<Uri<String>>> {
        let base_uri = self.resolver.base_uri();
        if base_uri.scheme().as_str() == DEFAULT_SCHEME {
            None
        } else {
            Some(base_uri)
        }
    }

    pub(crate) fn absolute_location(&self, location: &Location) -> Option<Arc<Uri<String>>> {
        let base = self.base_uri()?;
        let mut buffer = self.shared.uri_buffer.borrow_mut();
        buffer.clear();
        buffer.encode_str::<uri::Path>(&location.as_str()[self.resource_start..]);
        let resolved = base.with_fragment(Some(buffer.as_estr()));
        buffer.clear();
        Some(Arc::new(resolved))
    }

    fn translated_pattern(&self, pattern: &str) -> Result<Arc<str>, ()> {
        if let Some(entry) = self.shared.pattern_cache.borrow().get(pattern) {
            return Ok(Arc::clone(&entry.translated));
        }
        let translated = Arc::<str>::from(jsonschema_regex::to_rust_regex(pattern)?);
        self.shared.pattern_cache.borrow_mut().insert(
            Arc::from(pattern),
            PatternCacheEntry {
                translated: Arc::clone(&translated),
                fancy: None,
                standard: None,
            },
        );
        Ok(translated)
    }

    fn is_known_keyword(&self, keyword: &str) -> bool {
        self.draft.is_known_keyword(keyword)
    }
    pub(crate) fn supports_adjacent_validation(&self) -> bool {
        !matches!(self.draft, Draft::Draft4 | Draft::Draft6 | Draft::Draft7)
    }
    pub(crate) fn supports_integer_valued_numbers(&self) -> bool {
        !matches!(self.draft, Draft::Draft4)
    }
    pub(crate) fn validates_formats_by_default(&self) -> bool {
        self.config.validate_formats().unwrap_or_else(|| {
            self.asserts_formats_by_dialect() || formats_are_assertions_by_default(self.draft)
        })
    }
    pub(crate) fn are_unknown_formats_ignored(&self) -> bool {
        !self.asserts_formats_by_dialect() && self.config.are_unknown_formats_ignored()
    }
    /// The meta-schema requires `format` to be an assertion.
    pub(crate) fn asserts_formats_by_dialect(&self) -> bool {
        // Not `has_vocabulary`: that reports every vocabulary as present below Draft 2019-09.
        // Draft 2019-09 writes the requirement as its Format vocabulary declared `true`.
        self.vocabularies.contains(&Vocabulary::FormatAssertion)
            || self.vocabularies.contains(&Vocabulary::Format)
    }
    /// Enter a referenced resource, which may follow a meta-schema of its own.
    ///
    /// # Errors
    ///
    /// That meta-schema requires a vocabulary this crate does not implement.
    pub(crate) fn with_resolver_and_draft(
        &self,
        resolver: Resolver<'a>,
        draft: Draft,
        vocabularies: VocabularySet,
        resource_base: Location,
    ) -> Result<Context<'a, F>, ValidationError<'static>> {
        ensure_vocabularies_supported(self.config, &vocabularies)?;
        Ok(Context {
            config: self.config,
            resolver,
            draft,
            vocabularies,
            location: resource_base.clone(),
            resource_base,
            resource_start: 0,
            shared: Rc::clone(&self.shared),
        })
    }
    /// Count absolute locations from `start` bytes into `location`, where a `$id` subschema begins.
    #[must_use]
    pub(crate) fn with_resource_start(mut self, start: usize) -> Self {
        self.resource_start = start;
        self
    }
    pub(crate) fn get_content_media_type_check(
        &self,
        media_type: &str,
    ) -> Option<ContentMediaTypeCheckType> {
        self.config.get_content_media_type_check(media_type)
    }
    pub(crate) fn get_content_encoding_check(
        &self,
        content_encoding: &str,
    ) -> Option<ContentEncodingCheckType> {
        self.config.content_encoding_check(content_encoding)
    }

    pub(crate) fn get_content_encoding_convert(
        &self,
        content_encoding: &str,
    ) -> Option<ContentEncodingConverterType> {
        self.config.get_content_encoding_convert(content_encoding)
    }
    pub(crate) fn get_keyword_factory(&self, name: &str) -> Option<&Arc<dyn KeywordFactory<F>>> {
        self.config.get_keyword_factory(name)
    }
    /// Whether a custom keyword takes over this keyword.
    pub(crate) fn is_keyword_overridden(&self, name: &str) -> bool {
        self.get_keyword_factory(name).is_some()
    }
    pub(crate) fn get_format(&self, format: &str) -> Option<(&String, &Arc<dyn Format>)> {
        self.config.get_format(format)
    }
    pub(crate) fn is_circular_reference(
        &self,
        reference: &str,
    ) -> Result<bool, referencing::Error> {
        let uri = self
            .resolver
            .resolve_uri(&self.resolver.base_uri().borrow(), reference)?;
        Ok(self.shared.seen.borrow().contains(&*uri))
    }
    pub(crate) fn mark_seen(&self, reference: &str) -> Result<(), referencing::Error> {
        let uri = self
            .resolver
            .resolve_uri(&self.resolver.base_uri().borrow(), reference)?;
        self.shared.seen.borrow_mut().insert(uri);
        Ok(())
    }

    pub(crate) fn lookup_recursive_reference(&self) -> Result<Resolved<'a>, referencing::Error> {
        self.resolver.lookup_recursive_ref()
    }
    pub(crate) fn resolve_reference_uri(
        &self,
        reference: &str,
    ) -> Result<Arc<Uri<String>>, referencing::Error> {
        self.resolver
            .resolve_uri(&self.resolver.base_uri().borrow(), reference)
    }

    /// Location of `target` within the resource rooted at `root`, if `target` declares an anchor.
    pub(crate) fn anchor_location(&self, root: &Value, target: &Value) -> Option<Location> {
        self.shared
            .anchor_locations
            .borrow_mut()
            .entry(std::ptr::from_ref(root) as usize)
            .or_insert_with(|| {
                let mut index = AHashMap::new();
                index_anchors(root, &mut Vec::new(), &mut index);
                index
            })
            .get(&(std::ptr::from_ref(target) as usize))
            .cloned()
    }

    /// The resolved URI of `reference` and the location its target starts at.
    ///
    /// Schemas reuse a handful of targets across many `$ref` sites, so this is derived once per
    /// base URI and reference rather than per site. `target_base` runs only on a miss.
    pub(crate) fn ref_target(
        &self,
        reference: &str,
        target_base: impl FnOnce(&Uri<String>) -> Location,
    ) -> Result<RefTarget, referencing::Error> {
        let base_uri = BaseUriKey(self.resolver.base_uri());
        if let Some(target) = self
            .shared
            .ref_targets
            .borrow()
            .get(&base_uri)
            .and_then(|by_reference| by_reference.get(reference))
        {
            return Ok(target.clone());
        }
        let alias = self.resolve_reference_uri(reference)?;
        let target = (Arc::clone(&alias), target_base(&alias));
        self.shared
            .ref_targets
            .borrow_mut()
            .entry(base_uri)
            .or_default()
            .insert(reference.into(), target.clone());
        Ok(target)
    }

    /// Record that the node compiling now reaches an unsettled node.
    fn reach_unsettled(&self) {
        self.shared.reaches_unsettled.set(true);
    }

    /// Start tracking what a new node reaches; returns the state to pass to `finish_node`.
    fn start_node(&self) -> bool {
        self.shared.reaches_unsettled.replace(false)
    }

    /// Whether the node started with `outer` is settled; the enclosing node reaches what it does.
    fn finish_node(&self, outer: bool) -> bool {
        let reaches_unsettled = self.shared.reaches_unsettled.get();
        self.shared
            .reaches_unsettled
            .set(outer || reaches_unsettled);
        !reaches_unsettled
    }

    /// Whether the node compiling now may own `cached`.
    fn may_own(&self, cached: &CachedNode<F>) -> bool {
        cached.settled || cached.round == self.shared.round.get()
    }

    /// Record the edge to `cached` the caller is about to add.
    fn reuse(&self, cached: CachedNode<F>) -> TargetNode<F> {
        if !cached.settled {
            self.reach_unsettled();
        }
        if self.may_own(&cached) {
            TargetNode::Owned(cached.node)
        } else {
            TargetNode::Shared(cached.node)
        }
    }

    fn cached_location_node(&self, key: &NodeCacheKey) -> Option<CachedNode<F>> {
        self.shared.location_nodes.borrow().get(key).cloned()
    }

    fn cache_location_node(&self, key: NodeCacheKey, node: CachedNode<F>) {
        self.shared.location_nodes.borrow_mut().insert(key, node);
    }

    fn cached_alias_node(&self, key: &AliasCacheKey) -> Option<CachedNode<F>> {
        self.shared.alias_nodes.borrow().get(key).cloned()
    }

    fn cache_alias_node(&self, key: AliasCacheKey, node: CachedNode<F>) {
        self.shared.alias_nodes.borrow_mut().insert(key, node);
    }

    fn property_schema_key(schema: &Map<String, Value>) -> PropertyValidatorsPendingKey {
        PropertyValidatorsPendingKey::new(schema)
    }

    pub(crate) fn get_pending_property_validators_for_schema(
        &self,
        schema: &Map<String, Value>,
    ) -> Option<PendingPropertyValidators<F>> {
        let key = Self::property_schema_key(schema);
        let pending = self
            .shared
            .pending_property_validators_by_schema
            .borrow()
            .get(&key)
            .cloned();
        self.reach_pending(pending)
    }

    pub(crate) fn cache_pending_property_validators_for_schema(
        &self,
        schema: &Map<String, Value>,
        pending: PendingPropertyValidators<F>,
    ) {
        let key = Self::property_schema_key(schema);
        self.shared
            .pending_property_validators_by_schema
            .borrow_mut()
            .insert(key, pending);
    }

    pub(crate) fn remove_pending_property_validators_for_schema(
        &self,
        schema: &Map<String, Value>,
    ) {
        let key = Self::property_schema_key(schema);
        self.shared
            .pending_property_validators_by_schema
            .borrow_mut()
            .remove(&key);
    }

    fn items_schema_key(schema: &Map<String, Value>) -> ItemsValidatorsPendingKey {
        ItemsValidatorsPendingKey::new(schema)
    }

    pub(crate) fn get_pending_items_validators_for_schema(
        &self,
        schema: &Map<String, Value>,
    ) -> Option<PendingItemsValidators<F>> {
        let key = Self::items_schema_key(schema);
        let pending = self
            .shared
            .pending_items_validators_by_schema
            .borrow()
            .get(&key)
            .cloned();
        self.reach_pending(pending)
    }

    pub(crate) fn cache_pending_items_validators_for_schema(
        &self,
        schema: &Map<String, Value>,
        pending: PendingItemsValidators<F>,
    ) {
        let key = Self::items_schema_key(schema);
        self.shared
            .pending_items_validators_by_schema
            .borrow_mut()
            .insert(key, pending);
    }

    pub(crate) fn remove_pending_items_validators_for_schema(&self, schema: &Map<String, Value>) {
        let key = Self::items_schema_key(schema);
        self.shared
            .pending_items_validators_by_schema
            .borrow_mut()
            .remove(&key);
    }

    /// Validators found here are still compiling.
    fn reach_pending<T>(&self, pending: Option<T>) -> Option<T> {
        if pending.is_some() {
            self.reach_unsettled();
        }
        pending
    }

    pub(crate) fn cached_alias_placeholder(
        &self,
        alias: &Arc<Uri<String>>,
    ) -> Option<PendingSchemaNode<F>> {
        self.shared.alias_placeholders.borrow().get(alias).cloned()
    }

    pub(crate) fn set_alias_placeholder(
        &self,
        alias: Arc<Uri<String>>,
        node: PendingSchemaNode<F>,
    ) {
        self.shared
            .alias_placeholders
            .borrow_mut()
            .insert(alias, node);
    }

    pub(crate) fn remove_alias_placeholder(&self, alias: &Arc<Uri<String>>) {
        self.shared.alias_placeholders.borrow_mut().remove(alias);
    }

    /// Get a cached compiled regex, or compile and cache it if not present.
    pub(crate) fn get_or_compile_regex(
        &self,
        pattern: &str,
    ) -> Result<Arc<fancy_regex::Regex>, ()> {
        let translated = self.translated_pattern(pattern)?;
        {
            let cache = self.shared.pattern_cache.borrow();
            if let Some(entry) = cache.get(pattern) {
                if let Some(regex) = &entry.fancy {
                    return Ok(Arc::clone(regex));
                }
            }
        }

        let (backtrack_limit, size_limit, dfa_size_limit) = match self.config.pattern_options() {
            PatternEngineOptions::FancyRegex {
                backtrack_limit,
                size_limit,
                dfa_size_limit,
            } => (backtrack_limit, size_limit, dfa_size_limit),
            PatternEngineOptions::Regex { .. } => (None, None, None),
        };

        let regex = Arc::new(crate::regex::build_fancy_regex(
            translated.as_ref(),
            backtrack_limit,
            size_limit,
            dfa_size_limit,
        )?);

        if let Some(entry) = self.shared.pattern_cache.borrow_mut().get_mut(pattern) {
            entry.fancy = Some(Arc::clone(&regex));
        }

        Ok(regex)
    }

    /// Get a cached compiled standard regex, or compile and cache it if not present.
    pub(crate) fn get_or_compile_standard_regex(
        &self,
        pattern: &str,
    ) -> Result<Arc<regex::Regex>, ()> {
        let translated = self.translated_pattern(pattern)?;
        {
            let cache = self.shared.pattern_cache.borrow();
            if let Some(entry) = cache.get(pattern) {
                if let Some(regex) = &entry.standard {
                    return Ok(Arc::clone(regex));
                }
            }
        }

        let (size_limit, dfa_size_limit) = match self.config.pattern_options() {
            PatternEngineOptions::Regex {
                size_limit,
                dfa_size_limit,
            } => (size_limit, dfa_size_limit),
            PatternEngineOptions::FancyRegex { .. } => (None, None),
        };

        let regex = Arc::new(crate::regex::build_standard_regex(
            translated.as_ref(),
            size_limit,
            dfa_size_limit,
        )?);

        if let Some(entry) = self.shared.pattern_cache.borrow_mut().get_mut(pattern) {
            entry.standard = Some(Arc::clone(&regex));
        }

        Ok(regex)
    }

    /// Lookup a reference that is potentially recursive and return already
    /// compiled nodes when available.
    ///
    /// `target` is the resolver the reference resolved to: the node is cached under the dynamic
    /// scope it was compiled in, which is the target's, not this context's.
    pub(crate) fn lookup_maybe_recursive(
        &self,
        reference: &str,
        target: &Resolver<'_>,
    ) -> Result<Option<Box<dyn Validate<F>>>, ValidationError<'static>> {
        if self.is_circular_reference(reference)? {
            let uri = self
                .resolve_reference_uri(reference)
                .map_err(ValidationError::from)?;
            let key = AliasCacheKey {
                uri: Arc::clone(&uri),
                dynamic_scope: target.dynamic_scope(),
            };
            if let Some(node) = self.cached_alias_node(&key) {
                return Ok(Some(self.reuse(node).into_validator()));
            }
            if let Some(node) = self.cached_alias_placeholder(&uri) {
                self.reach_unsettled();
                return Ok(Some(Box::new(node)));
            }
        }
        Ok(None)
    }

    /// Whether the target of `alias` goes to the worklist: the call stack holds too many `$ref`
    /// targets, or the target is deferred already.
    pub(crate) fn defers_ref_target(
        &self,
        resolved: &Resolved<'a>,
        alias: &Arc<Uri<String>>,
    ) -> bool {
        if self.shared.nested_ref_compilations.get() >= MAX_NESTED_REF_COMPILATIONS {
            return true;
        }
        let deferred = self.shared.deferred_placeholders.borrow();
        !deferred.is_empty() && deferred.contains_key(&deferred_key(resolved, alias))
    }

    /// Run `compile` for a `$ref` target on the call stack.
    pub(crate) fn compile_ref_target<T>(&self, compile: impl FnOnce() -> T) -> T {
        let nested = &self.shared.nested_ref_compilations;
        nested.set(nested.get() + 1);
        let result = compile();
        nested.set(nested.get() - 1);
        result
    }

    /// Defer the target of `alias`; later `$ref`s to it get the returned placeholder.
    pub(crate) fn defer_ref_target(
        &self,
        resolved: Resolved<'a>,
        resource_base: Location,
        alias: Arc<Uri<String>>,
    ) -> PendingSchemaNode<F> {
        self.reach_unsettled();
        let key = deferred_key(&resolved, &alias);
        let existing = self
            .shared
            .deferred_placeholders
            .borrow()
            .get(&key)
            .cloned();
        if let Some(placeholder) = existing {
            return placeholder;
        }
        let (contents, resolver, draft) = resolved.into_inner();
        let placeholder = PendingSchemaNode::new();
        self.shared
            .deferred_placeholders
            .borrow_mut()
            .insert(key.clone(), placeholder.clone());
        let in_progress = self.shared.alias_placeholders.borrow().clone();
        self.shared
            .deferred_targets
            .borrow_mut()
            .push_back(DeferredTarget {
                contents,
                resolver,
                draft,
                resource_base,
                alias,
                key,
                placeholder: placeholder.clone(),
                in_progress,
            });
        placeholder
    }

    /// Start the next deferred target's round and drop its placeholder, so only earlier rounds
    /// own it.
    fn next_deferred_target(&self) -> Option<DeferredTarget<'a, F>> {
        let target = self.shared.deferred_targets.borrow_mut().pop_front()?;
        self.shared
            .deferred_placeholders
            .borrow_mut()
            .remove(&target.key);
        let round = &self.shared.round;
        round.set(round.get() + 1);
        Some(target)
    }

    fn replace_alias_placeholders(
        &self,
        placeholders: AHashMap<Arc<Uri<String>>, PendingSchemaNode<F>>,
    ) {
        *self.shared.alias_placeholders.borrow_mut() = placeholders;
    }

    pub(crate) fn location(&self) -> &Location {
        &self.location
    }

    /// Returns the current location relative to the resource base.
    ///
    /// This "suffix" is used for evaluation path computation. When an error occurs
    /// inside a `$ref` target, we combine the `$ref` traversal chain (prefix) with
    /// this suffix to form the complete evaluation path.
    ///
    /// # Example
    ///
    /// ```text
    /// Schema:
    /// {
    ///   "properties": {
    ///     "user": { "$ref": "#/$defs/Person" }
    ///   },
    ///   "$defs": {
    ///     "Person": {
    ///       "properties": {
    ///         "age": { "type": "integer" }
    ///       }
    ///     }
    ///   }
    /// }
    ///
    /// When compiling the "type" keyword inside "Person":
    ///   location()      = /$defs/Person/properties/age/type
    ///   resource_base   = /$defs/Person
    ///   suffix()        = /properties/age/type
    ///
    /// At validation time, if reached via /properties/user/$ref:
    ///   tracker = /properties/user/$ref + /properties/age/type
    ///                   = /properties/user/$ref/properties/age/type
    /// ```
    pub(crate) fn suffix(&self) -> Location {
        let suffix = self
            .location
            .as_str()
            .strip_prefix(self.resource_base.as_str())
            .expect("location must start with resource_base");
        Location::from_escaped(suffix)
    }

    pub(crate) fn has_vocabulary(&self, vocabulary: &Vocabulary) -> bool {
        if self.draft() < Draft::Draft201909 || vocabulary == &Vocabulary::Core {
            true
        } else {
            self.vocabularies.contains(vocabulary)
        }
    }
}

pub(crate) fn build_registry<'a, F: Json>(
    config: &'a ValidationOptions<'a, Arc<dyn referencing::Retrieve>, F>,
    draft: Draft,
    resource: ResourceRef<'a>,
    schema_id: Option<&'a str>,
) -> Result<(referencing::Registry<'a>, referencing::Uri<String>), referencing::Error> {
    let base_uri = resolve_base_uri(config.base_uri.as_ref(), schema_id)?;
    let registry = referencing::Registry::new()
        .retriever(config.retriever.clone())
        .draft(draft)
        .add(base_uri.as_str(), resource)?
        .prepare()?;
    Ok((registry, base_uri))
}

/// Compile `schema` into a validator over representation `F`, honoring `config`.
pub(crate) fn build_validator<F: Json>(
    config: &ValidationOptions<'_, Arc<dyn referencing::Retrieve>, F>,
    schema: &Value,
) -> Result<Validator<F>, ValidationError<'static>> {
    let draft = config.draft_for(schema)?;
    let resource = draft.create_resource_ref(schema);

    if config.validate_schema {
        validate_schema(draft, schema)?;
    }

    if let Some(registry) = config.registry {
        let base_uri = resolve_base_uri(config.base_uri.as_ref(), resource.id())?;
        let registry = registry
            .add(base_uri.as_str(), resource)?
            .retriever(config.retriever.clone())
            .draft(draft)
            .prepare()?;
        return build_validator_with_registry(config, schema, draft, resource, &registry);
    }
    let (registry, _) = build_registry(config, draft, resource, resource.id())?;
    build_validator_with_registry(config, schema, draft, resource, &registry)
}

#[cfg(feature = "resolve-async")]
pub(crate) async fn build_registry_async<'a, F: Json>(
    config: &'a ValidationOptions<'a, Arc<dyn referencing::AsyncRetrieve>, F>,
    draft: Draft,
    resource: ResourceRef<'a>,
    schema_id: Option<&'a str>,
) -> Result<(referencing::Registry<'a>, referencing::Uri<String>), referencing::Error> {
    let base_uri = resolve_base_uri(config.base_uri.as_ref(), schema_id)?;
    let registry = referencing::Registry::new()
        .async_retriever(config.retriever.clone())
        .draft(draft)
        .add(base_uri.as_str(), resource)?
        .async_prepare()
        .await?;
    Ok((registry, base_uri))
}

#[cfg(feature = "resolve-async")]
pub(crate) async fn build_validator_async<F: Json>(
    config: &ValidationOptions<'_, Arc<dyn referencing::AsyncRetrieve>, F>,
    schema: &Value,
) -> Result<Validator<F>, ValidationError<'static>> {
    let draft = config.draft_for(schema).await?;
    let resource_ref = draft.create_resource_ref(schema); // single computation

    if config.validate_schema {
        validate_schema(draft, schema)?;
    }

    if let Some(registry) = config.registry {
        let base_uri = resolve_base_uri(config.base_uri.as_ref(), resource_ref.id())?;
        let registry = registry
            .add(base_uri.as_str(), resource_ref)?
            .async_retriever(config.retriever.clone())
            .draft(draft)
            .async_prepare()
            .await?;
        return build_validator_with_registry(config, schema, draft, resource_ref, &registry);
    }

    let (registry, _) =
        build_registry_async(config, draft, resource_ref, resource_ref.id()).await?;
    build_validator_with_registry(config, schema, draft, resource_ref, &registry)
}

/// Upper-bound object-node count in `schema`, used to pre-size the per-location node cache.
///
/// Over-counts (data objects in `enum`/`const` are not subschemas), but it only sizes a transient
/// build cache, so over-provisioning is cheap and under-provisioning would reintroduce rehashing.
fn estimate_subschema_count(schema: &Value) -> usize {
    match schema {
        Value::Object(map) => 1 + map.values().map(estimate_subschema_count).sum::<usize>(),
        Value::Array(items) => items.iter().map(estimate_subschema_count).sum(),
        _ => 0,
    }
}

/// A meta-schema that requires a vocabulary this crate does not implement cannot be honored,
/// so the schemas written against it are refused.
fn ensure_vocabularies_supported<F: Json>(
    config: &dyn CompilationOptions<F>,
    vocabularies: &VocabularySet,
) -> Result<(), ValidationError<'static>> {
    for uri in vocabularies.custom() {
        if !config.declares_vocabulary(uri) {
            return Err(ValidationError::compile_error(
                Location::new(),
                Location::new(),
                Location::new(),
                LazyInstance::Ready(Cow::Owned(Value::Null)),
                format!("Unknown vocabulary: '{uri}' is required by the meta-schema. Adjust configuration to declare support for it"),
            ));
        }
    }
    Ok(())
}

fn build_validator_with_registry<R, F: Json>(
    config: &ValidationOptions<'_, R, F>,
    schema: &Value,
    draft: Draft,
    resource: ResourceRef<'_>,
    registry: &Registry<'_>,
) -> Result<Validator<F>, ValidationError<'static>> {
    let requested_base_uri = resolve_base_uri(config.base_uri.as_ref(), resource.id())?;
    let base_uri = normalize_base_uri(registry, &requested_base_uri);
    let vocabularies = registry.find_vocabularies(draft, schema);
    ensure_vocabularies_supported(config, &vocabularies)?;
    let resolver = registry.resolver(base_uri);
    let capacity = estimate_subschema_count(schema);
    let ctx: Context<'_, F> = Context::new(
        config,
        resolver,
        vocabularies,
        draft,
        Location::new(),
        capacity,
    );
    compile_validator(&ctx, resource, config.draft())
}

/// Compile `resource`, then the `$ref` targets it deferred.
///
/// Deferred targets compile from the top of the stack, so long `$ref` chains keep the stack flat.
fn compile_validator<F: Json>(
    ctx: &Context<'_, F>,
    resource: ResourceRef<'_>,
    draft: Draft,
) -> Result<Validator<F>, ValidationError<'static>> {
    let root = compile(ctx, resource).map_err(ValidationError::into_build_error)?;
    let targets = compile_deferred_targets(ctx)?;
    Ok(Validator {
        root,
        targets,
        draft,
    })
}

/// Drain the worklist and return the targets it compiled.
///
/// Out of line to keep its frame off the stack while the schema compiles.
#[inline(never)]
fn compile_deferred_targets<F: Json>(
    ctx: &Context<'_, F>,
) -> Result<Vec<SchemaNode<F>>, ValidationError<'static>> {
    let mut targets = Vec::new();
    while let Some(target) = ctx.next_deferred_target() {
        ctx.replace_alias_placeholders(target.in_progress);
        let vocabularies = target
            .resolver
            .find_vocabularies(target.draft, target.contents);
        let resource = target.draft.create_resource_ref(target.contents);
        let target_ctx = ctx
            .with_resolver_and_draft(
                target.resolver,
                resource.draft(),
                vocabularies,
                target.resource_base,
            )
            .map_err(ValidationError::into_build_error)?;
        match compile_with_alias(&target_ctx, resource, target.alias)
            .map_err(ValidationError::into_build_error)?
        {
            TargetNode::Owned(node) => {
                target.placeholder.initialize_owned(node.clone());
                targets.push(node);
            }
            TargetNode::Shared(node) => target.placeholder.initialize(&node),
        }
        ctx.replace_alias_placeholders(AHashMap::new());
    }
    Ok(targets)
}

pub(crate) fn normalize_base_uri(registry: &Registry<'_>, base_uri: &Uri<String>) -> Uri<String> {
    if registry.contains_resource(base_uri.as_str()) {
        return base_uri.clone();
    }

    if base_uri
        .fragment()
        .is_some_and(|fragment| fragment.as_str().is_empty())
    {
        let mut normalized = base_uri.clone();
        normalized.set_fragment(None);
        if registry.contains_resource(normalized.as_str()) {
            return normalized;
        }
    }

    panic!("generated registry is missing root URI '{base_uri}'");
}

/// Parsed once: a document naming no base and carrying no `$id` takes this, and re-parsing the
/// same constant is work every document would otherwise repeat.
static DEFAULT_BASE: LazyLock<Uri<String>> =
    LazyLock::new(|| uri::from_str(DEFAULT_BASE_URI).expect("the default base URI is valid"));

pub(crate) fn resolve_base_uri(
    base_uri: Option<&String>,
    schema_id: Option<&str>,
) -> Result<Uri<String>, referencing::Error> {
    match (base_uri, schema_id) {
        (Some(base_uri), _) => uri::from_str(base_uri),
        (None, Some(schema_id)) => uri::from_str(schema_id),
        (None, None) => Ok(DEFAULT_BASE.clone()),
    }
}

pub(crate) fn validate_schema(
    draft: Draft,
    schema: &Value,
) -> Result<(), ValidationError<'static>> {
    // Boolean schemas are always valid per the spec, skip validation
    if schema.is_boolean() {
        return Ok(());
    }

    // For objects, we can skip validation if they're empty (always valid)
    if let Some(obj) = schema.as_object() {
        if obj.is_empty() {
            return Ok(());
        }
    }

    let mut embedded = Vec::new();
    if declares_nested_schema(schema) {
        collect_embedded_resources(draft, draft, schema, &mut embedded);
    }
    let validator = crate::meta::validator_for_draft(draft);
    if embedded.is_empty() {
        return validator
            .validate(schema)
            .map_err(ValidationError::to_owned);
    }
    // Each embedded resource with its own dialect is validated against its own meta-schema
    // (JSON Schema 2020-12 Core, Section 9.3.3), so the enclosing one sees it as `{}`.
    let mut resources = Vec::with_capacity(embedded.len());
    let enclosing = without_embedded_resources(schema, &embedded, &Location::new(), &mut resources);
    validator
        .validate(&enclosing)
        .map_err(ValidationError::to_owned)?;
    for (location, draft, contents) in resources {
        validate_schema(draft, contents)
            .map_err(|error| error.with_instance_path_prefix(&location))?;
    }
    Ok(())
}

/// Whether `$schema` appears anywhere below the root. This structural scan is several times
/// cheaper than the schema-aware traversal, which a single-dialect document never needs.
fn declares_nested_schema(schema: &Value) -> bool {
    fn contains_schema_keyword(value: &Value) -> bool {
        match value {
            Value::Object(map) => map
                .iter()
                .any(|(key, child)| key == "$schema" || contains_schema_keyword(child)),
            Value::Array(items) => items.iter().any(contains_schema_keyword),
            _ => false,
        }
    }
    schema
        .as_object()
        .is_some_and(|map| map.values().any(contains_schema_keyword))
}

/// Embedded resources below `schema` whose `$schema` names a draft other than `meta_draft`, the
/// draft whose meta-schema the enclosing resource is validated against.
///
/// Only a resource root may declare `$schema` (2019-09 and 2020-12 Core, Section 8.1.1), so a
/// subschema without an identifier stays under the enclosing meta-schema.
fn collect_embedded_resources<'a>(
    draft: Draft,
    meta_draft: Draft,
    schema: &'a Value,
    embedded: &mut Vec<(&'a Value, Draft)>,
) {
    for subresource in draft.subresources_of(schema) {
        let subresource_draft = draft.detect(subresource);
        let own_dialect = subresource_draft != meta_draft && subresource_draft != Draft::Unknown;
        // A resource written against an older draft may name itself with that draft's `id`.
        if own_dialect
            && (draft.create_resource_ref(subresource).id().is_some()
                || subresource_draft
                    .create_resource_ref(subresource)
                    .id()
                    .is_some())
        {
            embedded.push((subresource, subresource_draft));
        } else {
            collect_embedded_resources(subresource_draft, meta_draft, subresource, embedded);
        }
    }
}

/// Copy of `value` with every `embedded` resource replaced by `{}`, which every draft accepts.
/// The replaced resources are recorded with their location.
fn without_embedded_resources<'a>(
    value: &'a Value,
    embedded: &[(&'a Value, Draft)],
    location: &Location,
    replaced: &mut Vec<(Location, Draft, &'a Value)>,
) -> Value {
    if let Some((_, draft)) = embedded
        .iter()
        .find(|(resource, _)| std::ptr::eq(*resource, value))
    {
        replaced.push((location.clone(), *draft, value));
        return Value::Object(Map::new());
    }
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, child)| {
                    let child =
                        without_embedded_resources(child, embedded, &location.join(key), replaced);
                    (key.clone(), child)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .enumerate()
                .map(|(idx, child)| {
                    without_embedded_resources(child, embedded, &location.join(idx), replaced)
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// Keywords that can name the schema holding them, across all drafts.
const ANCHOR_KEYWORDS: [&str; 4] = ["$anchor", "$dynamicAnchor", "$id", "id"];

fn index_anchors<'v>(
    value: &'v Value,
    path: &mut Vec<LocationSegment<'v>>,
    index: &mut AHashMap<usize, Location>,
) {
    match value {
        Value::Object(map) => {
            if ANCHOR_KEYWORDS
                .iter()
                .any(|keyword| map.contains_key(*keyword))
            {
                let location = path.iter().fold(Location::new(), |location, segment| {
                    location.join(segment.clone())
                });
                index.insert(std::ptr::from_ref(value) as usize, location);
            }
            for (key, child) in map {
                path.push(key.into());
                index_anchors(child, path, index);
                path.pop();
            }
        }
        Value::Array(items) => {
            for (idx, child) in items.iter().enumerate() {
                path.push(idx.into());
                index_anchors(child, path, index);
                path.pop();
            }
        }
        _ => {}
    }
}

/// Compile a JSON Schema instance to a tree of nodes.
pub(crate) fn compile<'a, F: Json>(
    ctx: &Context<F>,
    resource: ResourceRef<'a>,
) -> Result<SchemaNode<F>, ValidationError<'a>> {
    let ctx = ctx.in_subresource(resource)?;
    compile_with_internal(&ctx, resource, None).map(TargetNode::into_node)
}

/// Compile the target of `alias`, or reuse the compiled one.
pub(crate) fn compile_with_alias<'a, F: Json>(
    ctx: &Context<F>,
    resource: ResourceRef<'a>,
    alias: Arc<Uri<String>>,
) -> Result<TargetNode<F>, ValidationError<'a>> {
    compile_with_internal(ctx, resource, Some(alias))
}

/// The cached node for `resource`, or the key to cache a new one under.
///
/// `$ref` targets already compiled under their alias are found before compilation reaches here.
fn find_compiled<F: Json>(
    ctx: &Context<F>,
    resource: ResourceRef<'_>,
    is_ref_target: bool,
) -> Result<TargetNode<F>, NodeCacheKey> {
    let key = NodeCacheKey {
        location: ctx.location_cache_key(),
        schema_ptr: std::ptr::from_ref(resource.contents()) as usize,
    };
    if let Some(existing) = ctx.cached_location_node(&key) {
        // Outside a `$ref`, a node this round may not own compiles again instead of being shared.
        if is_ref_target || ctx.may_own(&existing) {
            return Ok(ctx.reuse(existing));
        }
    }
    Err(key)
}

/// The placeholder `$ref` cycles to `alias` use until its node compiles.
fn start_compiling<F: Json>(ctx: &Context<F>, alias: &Arc<Uri<String>>) -> PendingSchemaNode<F> {
    let placeholder = PendingSchemaNode::new();
    ctx.set_alias_placeholder(Arc::clone(alias), placeholder.clone());
    placeholder
}

/// Point the placeholder at the compiled `node` and cache it.
fn finish_compiling<F: Json>(
    ctx: &Context<F>,
    key: NodeCacheKey,
    target: Option<(&Arc<Uri<String>>, PendingSchemaNode<F>)>,
    node: &SchemaNode<F>,
    settled: bool,
) {
    let cached = CachedNode {
        node: node.clone(),
        round: ctx.shared.round.get(),
        settled,
    };
    if let Some((alias, placeholder)) = target {
        placeholder.initialize(node);
        ctx.remove_alias_placeholder(alias);
        ctx.cache_alias_node(ctx.alias_cache_key(Arc::clone(alias)), cached.clone());
    }
    ctx.cache_location_node(key, cached);
}

/// Lookups and bookkeeping live in separate functions to keep each nested frame small.
#[allow(clippy::needless_pass_by_value)]
fn compile_with_internal<'a, F: Json>(
    ctx: &Context<F>,
    resource: ResourceRef<'a>,
    alias: Option<Arc<Uri<String>>>,
) -> Result<TargetNode<F>, ValidationError<'a>> {
    let key = match find_compiled(ctx, resource, alias.is_some()) {
        Ok(existing) => return Ok(existing),
        Err(key) => key,
    };
    let outer = ctx.start_node();
    let target = alias
        .as_ref()
        .map(|alias| (alias, start_compiling(ctx, alias)));
    let node = compile_without_cache(ctx, resource);
    let settled = ctx.finish_node(outer);
    let node = node?;
    finish_compiling(ctx, key, target, &node, settled);
    Ok(TargetNode::Owned(node))
}

fn compile_without_cache<'a, F: Json>(
    ctx: &Context<F>,
    resource: ResourceRef<'a>,
) -> Result<SchemaNode<F>, ValidationError<'a>> {
    match resource.contents() {
        Value::Bool(value) => match value {
            true => Ok(SchemaNode::from_boolean(ctx, None)),
            false => Ok(SchemaNode::from_boolean(
                ctx,
                Some(
                    keywords::boolean::FalseValidator::compile(ctx.location().clone())
                        .expect("Should always compile"),
                ),
            )),
        },
        Value::Object(schema) => {
            if !ctx.supports_adjacent_validation() {
                // Drafts 4-7 ignore every `$ref` sibling, annotations included
                if let Some(reference) = schema.get("$ref") {
                    return if let Some(validator) =
                        keywords::ref_::compile_ref(ctx, schema, reference)
                    {
                        let validators = vec![(BuiltinKeyword::Ref.into(), validator?)];
                        Ok(SchemaNode::from_keywords(ctx, validators, None))
                    } else {
                        // Infinite reference to the same location
                        Ok(SchemaNode::from_boolean(ctx, None))
                    };
                }
            }

            let mut validators = Vec::with_capacity(schema.len());
            let mut annotations = Map::new();
            for (keyword, value) in schema {
                // Check if this keyword is overridden, then check the standard definitions
                if let Some(factory) = ctx.get_keyword_factory(keyword) {
                    let path = ctx.location().join(keyword);
                    let validator = CustomKeyword::new(
                        factory.init(schema, value, path.clone(), keyword)?,
                        path,
                        keyword.clone(),
                    );
                    let validator: BoxedValidator<F> = Box::new(validator);
                    validators.push((Keyword::custom(keyword), validator));
                } else if let Some((keyword, validator)) = keywords::get_for_draft(ctx, keyword)
                    .and_then(|(keyword, f)| f(ctx, schema, value).map(|v| (keyword, v)))
                {
                    validators.push((keyword, validator.map_err(ValidationError::to_owned)?));
                } else if !ctx.is_known_keyword(keyword) {
                    // Treat all non-validation keywords as annotations
                    annotations.insert(keyword.clone(), value.clone());
                }
            }
            let annotations = if annotations.is_empty() {
                None
            } else {
                Some(Arc::new(Value::Object(annotations)))
            };
            Ok(SchemaNode::from_keywords(ctx, validators, annotations))
        }
        _ => {
            let location = ctx.location().clone();
            Err(ValidationError::multiple_type_error(
                location.clone(),
                location,
                Location::new(),
                LazyInstance::Ready(Cow::Borrowed(resource.contents())),
                JsonTypeSet::from(JsonType::Boolean).insert(JsonType::Object),
            ))
        }
    }
}

/// Iteratively traverse a schema document and compile a [`Validator`] for every
/// reachable subschema, keyed by URI-fragment JSON pointer.
///
/// Each subschema is compiled with its own fresh [`Context`] so that caches from
/// sibling compilations do not interfere. Nodes that fail to compile (e.g.
/// unresolvable `$ref`) are silently skipped.
fn collect_validators<'a, F: Json>(
    config: &'a dyn CompilationOptions<F>,
    resolver: &Resolver<'a>,
    vocabularies: &VocabularySet,
    schema: &'a Value,
    draft: Draft,
) -> AHashMap<String, Validator<F>> {
    let mut validators: AHashMap<String, Validator<F>> = AHashMap::new();
    let mut stack: Vec<(&'a Value, String)> = vec![(schema, "#".to_string())];
    while let Some((current, pointer)) = stack.pop() {
        if matches!(current, Value::Object(_) | Value::Bool(_)) {
            let ctx: Context<'_, F> = Context::new(
                config,
                resolver.clone(),
                vocabularies.clone(),
                draft,
                Location::new(),
                estimate_subschema_count(current),
            );
            let resource_ref = ctx.as_resource_ref(current);
            if let Ok(validator) = compile_validator(&ctx, resource_ref, draft) {
                validators.insert(pointer.clone(), validator);
            }
        }
        match current {
            Value::Object(obj) => {
                for (key, value) in obj {
                    let mut escaped = String::new();
                    write_escaped_str(&mut escaped, key);
                    stack.push((value, format!("{pointer}/{escaped}")));
                }
            }
            Value::Array(arr) => {
                for (idx, item) in arr.iter().enumerate() {
                    stack.push((item, format!("{pointer}/{idx}")));
                }
            }
            _ => {}
        }
    }
    validators
}

fn build_validator_map_with_registry<R, F: Json>(
    config: &ValidationOptions<'_, R, F>,
    schema: &Value,
    draft: Draft,
    resource: ResourceRef<'_>,
    registry: &Registry<'_>,
) -> Result<ValidatorMap<F>, ValidationError<'static>> {
    let requested_base_uri = resolve_base_uri(config.base_uri.as_ref(), resource.id())?;
    let base_uri = normalize_base_uri(registry, &requested_base_uri);
    let vocabularies = registry.find_vocabularies(draft, schema);
    ensure_vocabularies_supported(config, &vocabularies)?;
    let resolver = registry.resolver(base_uri);
    let validators = collect_validators::<F>(config, &resolver, &vocabularies, schema, draft);
    Ok(ValidatorMap { validators })
}

/// Compile every reachable subschema into a validator over representation `F`, honoring `config`.
pub(crate) fn build_validator_map<F: Json>(
    config: &ValidationOptions<'_, Arc<dyn referencing::Retrieve>, F>,
    schema: &Value,
) -> Result<ValidatorMap<F>, ValidationError<'static>> {
    let draft = config.draft_for(schema)?;
    let resource = draft.create_resource_ref(schema);
    validate_schema(draft, schema)?;

    if let Some(registry) = config.registry {
        let base_uri = resolve_base_uri(config.base_uri.as_ref(), resource.id())?;
        let registry = registry
            .add(base_uri.as_str(), resource)?
            .retriever(config.retriever.clone())
            .draft(draft)
            .prepare()?;
        return build_validator_map_with_registry(config, schema, draft, resource, &registry);
    }

    let (registry, _) = build_registry(config, draft, resource, resource.id())?;
    build_validator_map_with_registry(config, schema, draft, resource, &registry)
}

#[cfg(feature = "resolve-async")]
pub(crate) async fn build_validator_map_async<F: Json>(
    config: &ValidationOptions<'_, Arc<dyn referencing::AsyncRetrieve>, F>,
    schema: &Value,
) -> Result<ValidatorMap<F>, ValidationError<'static>> {
    let draft = config.draft_for(schema).await?;
    let resource = draft.create_resource_ref(schema);

    validate_schema(draft, schema)?;

    if let Some(registry) = config.registry {
        let base_uri = resolve_base_uri(config.base_uri.as_ref(), resource.id())?;
        let registry = registry
            .add(base_uri.as_str(), resource)?
            .async_retriever(config.retriever.clone())
            .draft(draft)
            .async_prepare()
            .await?;
        return build_validator_map_with_registry(config, schema, draft, resource, &registry);
    }

    let (registry, _) = build_registry_async(config, draft, resource, resource.id()).await?;
    build_validator_map_with_registry(config, schema, draft, resource, &registry)
}
