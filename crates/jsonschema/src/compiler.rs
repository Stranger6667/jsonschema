use crate::{
    content_encoding::{ContentEncodingCheckType, ContentEncodingConverterType},
    content_media_type::ContentMediaTypeCheckType,
    ecma,
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
    ValidationError, Validator, ValidatorMap,
};
use ahash::{AHashMap, AHashSet};
use referencing::{
    uri, write_escaped_str, Draft, List, Registry, Resolved, Resolver, ResourceRef, Uri,
    Vocabulary, VocabularySet,
};
use serde_json::{Map, Value};
use std::{cell::RefCell, rc::Rc, sync::Arc};

const DEFAULT_SCHEME: &str = "json-schema";
pub(crate) const DEFAULT_BASE_URI: &str = "json-schema:///";

/// Type alias for shared cache maps in compiler state.
type SharedCache<K, V> = Rc<RefCell<AHashMap<K, V>>>;
/// Type alias for shared sets in compiler state.
type SharedSet<T> = Rc<RefCell<AHashSet<T>>>;

pub(crate) trait CompilationOptions {
    fn validate_formats(&self) -> Option<bool>;
    fn are_unknown_formats_ignored(&self) -> bool;
    fn get_content_media_type_check(&self, media_type: &str) -> Option<ContentMediaTypeCheckType>;
    fn content_encoding_check(&self, content_encoding: &str) -> Option<ContentEncodingCheckType>;
    fn get_content_encoding_convert(
        &self,
        content_encoding: &str,
    ) -> Option<ContentEncodingConverterType>;
    fn get_keyword_factory(&self, name: &str) -> Option<&Arc<dyn KeywordFactory>>;
    fn get_format(&self, format: &str) -> Option<(&String, &Arc<dyn Format>)>;
    fn pattern_options(&self) -> PatternEngineOptions;
    fn email_options(&self) -> Option<&email_address::Options>;
}

impl<R> CompilationOptions for ValidationOptions<'_, R> {
    fn validate_formats(&self) -> Option<bool> {
        ValidationOptions::validate_formats(self)
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

    fn get_keyword_factory(&self, name: &str) -> Option<&Arc<dyn KeywordFactory>> {
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

/// Shared caches reused across every `Context` derived from a schema root.
#[derive(Debug, Clone)]
struct SharedContextState {
    seen: SharedSet<Arc<Uri<String>>>,
    location_nodes: SharedCache<LocationCacheKey, SchemaNode>,
    alias_nodes: SharedCache<AliasCacheKey, SchemaNode>,
    pending_nodes: SharedCache<LocationCacheKey, PendingSchemaNode>,
    alias_placeholders: SharedCache<Arc<Uri<String>>, PendingSchemaNode>,
    pending_property_validators: SharedCache<LocationCacheKey, PendingPropertyValidators>,
    pending_property_validators_by_schema:
        SharedCache<PropertyValidatorsPendingKey, PendingPropertyValidators>,
    pending_items_validators: SharedCache<LocationCacheKey, PendingItemsValidators>,
    pending_items_validators_by_schema:
        SharedCache<ItemsValidatorsPendingKey, PendingItemsValidators>,
    pattern_cache: SharedCache<Arc<str>, PatternCacheEntry>,
    uri_buffer: Rc<RefCell<String>>,
}

#[derive(Debug, Clone)]
struct PatternCacheEntry {
    translated: Arc<str>,
    fancy: Option<Arc<fancy_regex::Regex>>,
    standard: Option<Arc<regex::Regex>>,
}

impl SharedContextState {
    fn new() -> Self {
        Self {
            seen: Rc::new(RefCell::new(AHashSet::new())),
            location_nodes: Rc::new(RefCell::new(AHashMap::new())),
            alias_nodes: Rc::new(RefCell::new(AHashMap::new())),
            pending_nodes: Rc::new(RefCell::new(AHashMap::new())),
            alias_placeholders: Rc::new(RefCell::new(AHashMap::new())),
            pending_property_validators: Rc::new(RefCell::new(AHashMap::new())),
            pending_property_validators_by_schema: Rc::new(RefCell::new(AHashMap::new())),
            pending_items_validators: Rc::new(RefCell::new(AHashMap::new())),
            pending_items_validators_by_schema: Rc::new(RefCell::new(AHashMap::new())),
            pattern_cache: Rc::new(RefCell::new(AHashMap::new())),
            uri_buffer: Rc::new(RefCell::new(String::new())),
        }
    }
}

/// Per-location view used while compiling schemas into validators.
#[derive(Clone)]
pub(crate) struct Context<'a> {
    config: &'a dyn CompilationOptions,
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
    pub(crate) draft: Draft,
    shared: SharedContextState,
}

impl<'a> Context<'a> {
    pub(crate) fn new(
        config: &'a dyn CompilationOptions,
        resolver: Resolver<'a>,
        vocabularies: VocabularySet,
        draft: Draft,
        location: Location,
    ) -> Self {
        Context {
            config,
            resolver,
            resource_base: location.clone(),
            location,
            vocabularies,
            draft,
            shared: SharedContextState::new(),
        }
    }
    pub(crate) fn draft(&self) -> Draft {
        self.draft
    }
    pub(crate) fn config(&self) -> &dyn CompilationOptions {
        self.config
    }

    /// Create a context for this schema.
    pub(crate) fn in_subresource(
        &'a self,
        resource: ResourceRef<'_>,
    ) -> Result<Context<'a>, referencing::Error> {
        let resolver = self.resolver.in_subresource(resource)?;
        Ok(Context {
            config: self.config,
            resolver,
            vocabularies: self.vocabularies.clone(),
            draft: resource.draft(),
            resource_base: self.resource_base.clone(),
            location: self.location.clone(),
            shared: self.shared.clone(),
        })
    }
    pub(crate) fn as_resource_ref<'r>(&'a self, contents: &'r Value) -> ResourceRef<'r> {
        self.draft.detect(contents).create_resource_ref(contents)
    }

    #[inline]
    pub(crate) fn new_at_location(&'a self, chunk: impl Into<LocationSegment<'a>>) -> Self {
        let location = self.location.join(chunk);
        Context {
            config: self.config,
            resolver: self.resolver.clone(),
            vocabularies: self.vocabularies.clone(),
            resource_base: self.resource_base.clone(),
            location,
            draft: self.draft,
            shared: self.shared.clone(),
        }
    }
    pub(crate) fn lookup(&'a self, reference: &str) -> Result<Resolved<'a>, referencing::Error> {
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
        uri::encode_to(location.as_str(), &mut buffer);
        let resolved = base.with_fragment(Some(uri::EncodedString::new_or_panic(&buffer)));
        buffer.clear();
        Some(Arc::new(resolved))
    }

    fn translated_pattern(&self, pattern: &str) -> Result<Arc<str>, ()> {
        if let Some(entry) = self.shared.pattern_cache.borrow().get(pattern) {
            return Ok(Arc::clone(&entry.translated));
        }
        let translated = Arc::<str>::from(ecma::to_rust_regex(pattern)?);
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
    pub(crate) fn find_vocabularies(&self, draft: Draft, contents: &Value) -> VocabularySet {
        self.resolver.find_vocabularies(draft, contents)
    }
    pub(crate) fn validates_formats_by_default(&self) -> bool {
        self.config.validate_formats().unwrap_or(matches!(
            self.draft,
            Draft::Draft4 | Draft::Draft6 | Draft::Draft7
        ))
    }
    pub(crate) fn are_unknown_formats_ignored(&self) -> bool {
        self.config.are_unknown_formats_ignored()
    }
    pub(crate) fn with_resolver_and_draft(
        &'a self,
        resolver: Resolver<'a>,
        draft: Draft,
        vocabularies: VocabularySet,
        resource_base: Location,
    ) -> Context<'a> {
        Context {
            config: self.config,
            resolver,
            draft,
            vocabularies,
            location: resource_base.clone(),
            resource_base,
            shared: self.shared.clone(),
        }
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
    pub(crate) fn get_keyword_factory(&self, name: &str) -> Option<&Arc<dyn KeywordFactory>> {
        self.config.get_keyword_factory(name)
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

    pub(crate) fn lookup_recursive_reference(&self) -> Result<Resolved<'_>, referencing::Error> {
        self.resolver.lookup_recursive_ref()
    }
    pub(crate) fn absolute_location_uri(&self) -> Result<Arc<Uri<String>>, referencing::Error> {
        // Reuse the shared buffer to avoid allocations
        let mut buffer = self.shared.uri_buffer.borrow_mut();
        buffer.clear();
        buffer.push('#');
        if !self.location.as_str().is_empty() {
            uri::encode_to(self.location.as_str(), &mut buffer);
        }
        let result = self
            .resolver
            .resolve_uri(&self.resolver.base_uri().borrow(), &buffer);
        buffer.clear();
        result
    }

    pub(crate) fn resolve_reference_uri(
        &self,
        reference: &str,
    ) -> Result<Arc<Uri<String>>, referencing::Error> {
        self.resolver
            .resolve_uri(&self.resolver.base_uri().borrow(), reference)
    }

    pub(crate) fn cached_location_node(&self, key: &LocationCacheKey) -> Option<SchemaNode> {
        self.shared.location_nodes.borrow().get(key).cloned()
    }

    pub(crate) fn cache_location_node(&self, key: LocationCacheKey, node: SchemaNode) {
        self.shared.location_nodes.borrow_mut().insert(key, node);
    }

    pub(crate) fn cached_alias_node(&self, key: &AliasCacheKey) -> Option<SchemaNode> {
        self.shared.alias_nodes.borrow().get(key).cloned()
    }

    pub(crate) fn cache_alias_node(&self, key: AliasCacheKey, node: SchemaNode) {
        self.shared.alias_nodes.borrow_mut().insert(key, node);
    }

    pub(crate) fn cached_pending_location_node(
        &self,
        key: &LocationCacheKey,
    ) -> Option<PendingSchemaNode> {
        self.shared.pending_nodes.borrow().get(key).cloned()
    }

    pub(crate) fn cache_pending_location_node(
        &self,
        key: LocationCacheKey,
        node: PendingSchemaNode,
    ) {
        self.shared.pending_nodes.borrow_mut().insert(key, node);
    }

    pub(crate) fn remove_pending_location_node(&self, key: &LocationCacheKey) {
        self.shared.pending_nodes.borrow_mut().remove(key);
    }

    pub(crate) fn get_pending_property_validators(
        &self,
        key: &LocationCacheKey,
    ) -> Option<PendingPropertyValidators> {
        self.shared
            .pending_property_validators
            .borrow()
            .get(key)
            .cloned()
    }

    pub(crate) fn cache_pending_property_validators(
        &self,
        key: LocationCacheKey,
        pending: PendingPropertyValidators,
    ) {
        self.shared
            .pending_property_validators
            .borrow_mut()
            .insert(key, pending);
    }

    pub(crate) fn remove_pending_property_validators(&self, key: &LocationCacheKey) {
        self.shared
            .pending_property_validators
            .borrow_mut()
            .remove(key);
    }

    fn property_schema_key(schema: &Map<String, Value>) -> PropertyValidatorsPendingKey {
        PropertyValidatorsPendingKey::new(schema)
    }

    pub(crate) fn get_pending_property_validators_for_schema(
        &self,
        schema: &Map<String, Value>,
    ) -> Option<PendingPropertyValidators> {
        let key = Self::property_schema_key(schema);
        self.shared
            .pending_property_validators_by_schema
            .borrow()
            .get(&key)
            .cloned()
    }

    pub(crate) fn cache_pending_property_validators_for_schema(
        &self,
        schema: &Map<String, Value>,
        pending: PendingPropertyValidators,
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
    ) -> Option<PendingItemsValidators> {
        let key = Self::items_schema_key(schema);
        self.shared
            .pending_items_validators_by_schema
            .borrow()
            .get(&key)
            .cloned()
    }

    pub(crate) fn get_pending_items_validators(
        &self,
        key: &LocationCacheKey,
    ) -> Option<PendingItemsValidators> {
        self.shared
            .pending_items_validators
            .borrow()
            .get(key)
            .cloned()
    }

    pub(crate) fn cached_alias_placeholder(
        &self,
        alias: &Arc<Uri<String>>,
    ) -> Option<PendingSchemaNode> {
        self.shared.alias_placeholders.borrow().get(alias).cloned()
    }

    pub(crate) fn set_alias_placeholder(&self, alias: Arc<Uri<String>>, node: PendingSchemaNode) {
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

        let mut builder = fancy_regex::RegexBuilder::new(translated.as_ref());
        if let Some(limit) = backtrack_limit {
            builder.backtrack_limit(limit);
        }
        if let Some(limit) = size_limit {
            builder.delegate_size_limit(limit);
        }
        if let Some(limit) = dfa_size_limit {
            builder.delegate_dfa_size_limit(limit);
        }
        let regex = Arc::new(builder.build().map_err(|_| ())?);

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

        let mut builder = regex::RegexBuilder::new(translated.as_ref());
        if let Some(limit) = size_limit {
            builder.size_limit(limit);
        }
        if let Some(limit) = dfa_size_limit {
            builder.dfa_size_limit(limit);
        }
        let regex = Arc::new(builder.build().map_err(|_| ())?);

        if let Some(entry) = self.shared.pattern_cache.borrow_mut().get_mut(pattern) {
            entry.standard = Some(Arc::clone(&regex));
        }

        Ok(regex)
    }

    /// Lookup a reference that is potentially recursive and return already
    /// compiled nodes when available.
    pub(crate) fn lookup_maybe_recursive(
        &self,
        reference: &str,
    ) -> Result<Option<Box<dyn Validate>>, ValidationError<'static>> {
        if self.is_circular_reference(reference)? {
            let uri = self
                .resolve_reference_uri(reference)
                .map_err(ValidationError::from)?;
            let key = self.alias_cache_key(Arc::clone(&uri));
            if let Some(node) = self.cached_alias_node(&key) {
                return Ok(Some(Box::new(node)));
            }
            if let Some(node) = self.cached_alias_placeholder(&uri) {
                return Ok(Some(Box::new(node)));
            }
        }
        Ok(None)
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

pub(crate) fn build_registry<'a>(
    config: &'a ValidationOptions<'a>,
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

pub(crate) fn build_validator(
    config: &ValidationOptions<'_>,
    schema: &Value,
) -> Result<Validator, ValidationError<'static>> {
    let draft = config.draft_for(schema)?;
    let resource = draft.create_resource_ref(schema);

    // Validate the schema itself
    if config.validate_schema {
        validate_schema(draft, schema)?;
    }

    if let Some(registry) = config.registry {
        let base_uri = resolve_base_uri(config.base_uri.as_ref(), resource.id())?;
        let overlay = registry
            .add(base_uri.as_str(), resource)?
            .retriever(config.retriever.clone())
            .draft(draft)
            .prepare()?;
        return build_validator_with_registry(config, schema, draft, resource, &overlay);
    }

    let (registry, _) = build_registry(config, draft, resource, resource.id())?;
    build_validator_with_registry(config, schema, draft, resource, &registry)
}

#[cfg(feature = "resolve-async")]
pub(crate) async fn build_registry_async<'a>(
    config: &'a ValidationOptions<'a, Arc<dyn referencing::AsyncRetrieve>>,
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
pub(crate) async fn build_validator_async(
    config: &ValidationOptions<'_, Arc<dyn referencing::AsyncRetrieve>>,
    schema: &Value,
) -> Result<Validator, ValidationError<'static>> {
    let draft = config.draft_for(schema).await?;
    let resource_ref = draft.create_resource_ref(schema); // single computation

    if config.validate_schema {
        validate_schema(draft, schema)?;
    }

    if let Some(registry) = config.registry {
        let base_uri = resolve_base_uri(config.base_uri.as_ref(), resource_ref.id())?;
        let overlay = registry
            .add(base_uri.as_str(), resource_ref)?
            .async_retriever(config.retriever.clone())
            .draft(draft)
            .async_prepare()
            .await?;
        return build_validator_with_registry(config, schema, draft, resource_ref, &overlay);
    }

    let (registry, _) =
        build_registry_async(config, draft, resource_ref, resource_ref.id()).await?;
    build_validator_with_registry(config, schema, draft, resource_ref, &registry)
}

fn build_validator_with_registry<R>(
    config: &ValidationOptions<'_, R>,
    schema: &Value,
    draft: Draft,
    resource: ResourceRef<'_>,
    registry: &Registry<'_>,
) -> Result<Validator, ValidationError<'static>> {
    let requested_base_uri = resolve_base_uri(config.base_uri.as_ref(), resource.id())?;
    let base_uri = normalize_base_uri(registry, &requested_base_uri);
    let vocabularies = registry.find_vocabularies(draft, schema);
    let resolver = registry.resolver(base_uri);
    let ctx = Context::new(config, resolver, vocabularies, draft, Location::new());
    let root = compile(&ctx, resource).map_err(ValidationError::to_owned)?;
    let draft = config.draft();
    Ok(Validator { root, draft })
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

pub(crate) fn resolve_base_uri(
    base_uri: Option<&String>,
    schema_id: Option<&str>,
) -> Result<Uri<String>, referencing::Error> {
    if let Some(base_uri) = base_uri {
        uri::from_str(base_uri)
    } else {
        uri::from_str(schema_id.unwrap_or(DEFAULT_BASE_URI))
    }
}

fn annotations_to_value(annotations: AHashMap<String, Value>) -> Arc<Value> {
    let mut object = Map::with_capacity(annotations.len());
    for (key, value) in annotations {
        object.insert(key, value);
    }
    Arc::new(Value::Object(object))
}

fn validate_schema(draft: Draft, schema: &Value) -> Result<(), ValidationError<'static>> {
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

    let validator = crate::meta::validator_for_draft(draft);
    if let Err(error) = validator.validate(schema) {
        return Err(error.to_owned());
    }
    Ok(())
}

/// Compile a JSON Schema instance to a tree of nodes.
pub(crate) fn compile<'a>(
    ctx: &Context,
    resource: ResourceRef<'a>,
) -> Result<SchemaNode, ValidationError<'a>> {
    let ctx = ctx.in_subresource(resource)?;
    compile_with_internal(&ctx, resource, None)
}

pub(crate) fn compile_with_alias<'a>(
    ctx: &Context,
    resource: ResourceRef<'a>,
    alias: Arc<Uri<String>>,
) -> Result<SchemaNode, ValidationError<'a>> {
    compile_with_internal(ctx, resource, Some(alias))
}

#[allow(clippy::needless_pass_by_value)]
fn compile_with_internal<'a>(
    ctx: &Context,
    resource: ResourceRef<'a>,
    alias: Option<Arc<Uri<String>>>,
) -> Result<SchemaNode, ValidationError<'a>> {
    // Check if this alias already has a cached node
    if let Some(alias_key) = alias.as_ref() {
        let scoped_key = ctx.alias_cache_key(Arc::clone(alias_key));
        if let Some(existing_alias) = ctx.cached_alias_node(&scoped_key) {
            return Ok(existing_alias);
        }
    }

    // Check location-based cache
    let key = ctx.location_cache_key();
    if let Some(existing) = ctx.cached_location_node(&key) {
        return Ok(existing);
    }

    // Check if there's a pending node (circular reference being compiled)
    if let Some(pending) = ctx.cached_pending_location_node(&key) {
        // If the node has already been initialized, reuse it. Otherwise, we rely on the
        // in-flight compilation to finish initialization and continue compiling here.
        if let Some(node) = pending.get() {
            return Ok(node.clone());
        }
    }

    // Create placeholder for circular reference detection
    let placeholder = PendingSchemaNode::new();
    ctx.cache_pending_location_node(key.clone(), placeholder.clone());
    if let Some(alias_key) = alias.as_ref() {
        ctx.set_alias_placeholder(Arc::clone(alias_key), placeholder.clone());
    }

    // Compile the schema
    match compile_without_cache(ctx, resource) {
        Ok(node) => {
            // Initialize the placeholder with the compiled node
            placeholder.initialize(&node);

            // Remove from pending cache and add to final cache
            ctx.remove_pending_location_node(&key);
            ctx.cache_location_node(key.clone(), node.clone());

            if let Some(alias_key) = alias.as_ref() {
                ctx.remove_alias_placeholder(alias_key);
                let scoped_key = ctx.alias_cache_key(Arc::clone(alias_key));
                ctx.cache_alias_node(scoped_key, node.clone());
            }
            Ok(node)
        }
        Err(err) => Err(err),
    }
}

fn compile_without_cache<'a>(
    ctx: &Context,
    resource: ResourceRef<'a>,
) -> Result<SchemaNode, ValidationError<'a>> {
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
            // A schema could contain validation keywords along with annotations and we need to
            // collect annotations separately
            if !ctx.supports_adjacent_validation() {
                // Older drafts ignore all other keywords if `$ref` is present
                if let Some(reference) = schema.get("$ref") {
                    // Treat all keywords other than `$ref` as annotations
                    let annotations: AHashMap<String, Value> = schema
                        .iter()
                        .filter_map(|(k, v)| {
                            if k.as_str() == "$ref" {
                                None
                            } else {
                                Some((k.clone(), v.clone()))
                            }
                        })
                        .collect();
                    let annotations = if annotations.is_empty() {
                        None
                    } else {
                        Some(annotations_to_value(annotations))
                    };
                    return if let Some(validator) =
                        keywords::ref_::compile_ref(ctx, schema, reference)
                    {
                        let validators = vec![(BuiltinKeyword::Ref.into(), validator?)];
                        Ok(SchemaNode::from_keywords(ctx, validators, annotations))
                    } else {
                        // Infinite reference to the same location
                        Ok(SchemaNode::from_boolean(ctx, None))
                    };
                }
            }

            let mut validators = Vec::with_capacity(schema.len());
            let mut annotations = AHashMap::new();
            for (keyword, value) in schema {
                // Check if this keyword is overridden, then check the standard definitions
                if let Some(factory) = ctx.get_keyword_factory(keyword) {
                    let path = ctx.location().join(keyword);
                    let validator = CustomKeyword::new(
                        factory.init(schema, value, path.clone(), keyword)?,
                        path,
                        keyword.clone(),
                    );
                    let validator: BoxedValidator = Box::new(validator);
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
                Some(annotations_to_value(annotations))
            };
            Ok(SchemaNode::from_keywords(ctx, validators, annotations))
        }
        _ => {
            let location = ctx.location().clone();
            Err(ValidationError::multiple_type_error(
                location.clone(),
                location,
                Location::new(),
                resource.contents(),
                JsonTypeSet::empty()
                    .insert(JsonType::Boolean)
                    .insert(JsonType::Object),
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
fn collect_validators<'a>(
    config: &'a dyn CompilationOptions,
    resolver: &Resolver<'a>,
    vocabularies: &VocabularySet,
    schema: &'a Value,
    draft: Draft,
) -> AHashMap<String, Validator> {
    let mut validators: AHashMap<String, Validator> = AHashMap::new();
    let mut stack: Vec<(&'a Value, String)> = vec![(schema, "#".to_string())];

    while let Some((current, pointer)) = stack.pop() {
        // Only Object and Bool are valid JSON schemas — skip compilation for everything else
        if matches!(current, Value::Object(_) | Value::Bool(_)) {
            let ctx = Context::new(
                config,
                resolver.clone(),
                vocabularies.clone(),
                draft,
                Location::new(),
            );
            let resource_ref = ctx.as_resource_ref(current);
            if let Ok(node) = compile(&ctx, resource_ref) {
                // Store each sub-validator with the same draft as the top-level schema.
                // Per-subschema draft detection is not performed here; subschemas that
                // declare their own `$schema` will still compile correctly since the
                // registry handles resolution, but their Validator::draft() will reflect
                // the top-level draft.
                validators.insert(pointer.clone(), Validator { root: node, draft });
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

fn build_validator_map_with_registry<R>(
    config: &ValidationOptions<'_, R>,
    schema: &Value,
    draft: Draft,
    resource: ResourceRef<'_>,
    registry: &Registry<'_>,
) -> Result<ValidatorMap, ValidationError<'static>> {
    let requested_base_uri = resolve_base_uri(config.base_uri.as_ref(), resource.id())?;
    let base_uri = normalize_base_uri(registry, &requested_base_uri);
    let vocabularies = registry.find_vocabularies(draft, schema);
    let resolver = registry.resolver(base_uri);
    let validators = collect_validators(config, &resolver, &vocabularies, schema, draft);
    Ok(ValidatorMap { validators })
}

pub(crate) fn build_validator_map(
    config: &ValidationOptions<'_>,
    schema: &Value,
) -> Result<ValidatorMap, ValidationError<'static>> {
    let draft = config.draft_for(schema)?;
    let resource = draft.create_resource_ref(schema);

    if config.validate_schema {
        validate_schema(draft, schema)?;
    }

    if let Some(registry) = config.registry {
        let base_uri = resolve_base_uri(config.base_uri.as_ref(), resource.id())?;
        let overlay = registry
            .add(base_uri.as_str(), resource)?
            .retriever(config.retriever.clone())
            .draft(draft)
            .prepare()?;
        return build_validator_map_with_registry(config, schema, draft, resource, &overlay);
    }

    let (registry, _) = build_registry(config, draft, resource, resource.id())?;
    build_validator_map_with_registry(config, schema, draft, resource, &registry)
}

#[cfg(feature = "resolve-async")]
pub(crate) async fn build_validator_map_async(
    config: &ValidationOptions<'_, Arc<dyn referencing::AsyncRetrieve>>,
    schema: &Value,
) -> Result<ValidatorMap, ValidationError<'static>> {
    let draft = config.draft_for(schema).await?;
    let resource_ref = draft.create_resource_ref(schema);

    if config.validate_schema {
        validate_schema(draft, schema)?;
    }

    if let Some(registry) = config.registry {
        let base_uri = resolve_base_uri(config.base_uri.as_ref(), resource_ref.id())?;
        let overlay = registry
            .add(base_uri.as_str(), resource_ref)?
            .async_retriever(config.retriever.clone())
            .draft(draft)
            .async_prepare()
            .await?;
        return build_validator_map_with_registry(config, schema, draft, resource_ref, &overlay);
    }

    let (registry, _) =
        build_registry_async(config, draft, resource_ref, resource_ref.id()).await?;
    build_validator_map_with_registry(config, schema, draft, resource_ref, &registry)
}
