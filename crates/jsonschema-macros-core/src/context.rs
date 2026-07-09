use std::{
    collections::{BTreeSet, HashMap, HashSet},
    ops::{Deref, DerefMut},
    sync::Arc,
};

use indexmap::IndexMap;
use proc_macro2::TokenStream;
use referencing::{write_escaped_str, Draft, Registry, Uri, VocabularySet};

/// `Deref`/`DerefMut` to `CompileContext` for a scope guard holding `ctx: &mut CompileContext`.
macro_rules! impl_scope_guard {
    ($guard:ident) => {
        impl<'cfg> Deref for $guard<'_, 'cfg> {
            type Target = CompileContext<'cfg>;
            fn deref(&self) -> &Self::Target {
                self.ctx
            }
        }
        impl DerefMut for $guard<'_, '_> {
            fn deref_mut(&mut self) -> &mut Self::Target {
                self.ctx
            }
        }
    };
}

/// Generated functions of one category, interned by schema location to
/// deduplicate repeated subschemas and break reference cycles.
pub(crate) struct FnTable {
    /// Maps a schema location to the generated function's name.
    location_to_name: HashMap<String, String>,
    /// Maps a generated function name to its compiled body.
    bodies: IndexMap<String, TokenStream>,
    /// Optional `validate` bodies for functions that have one.
    validate_bodies: IndexMap<String, TokenStream>,
    /// Optional `collect` bodies for functions that have one.
    collect_bodies: IndexMap<String, TokenStream>,
    /// Locations currently being compiled (cycle guard).
    in_progress: HashSet<String>,
    counter: usize,
    prefix: &'static str,
}

impl FnTable {
    pub(crate) fn new(prefix: &'static str) -> Self {
        Self {
            location_to_name: HashMap::new(),
            bodies: IndexMap::new(),
            validate_bodies: IndexMap::new(),
            collect_bodies: IndexMap::new(),
            in_progress: HashSet::new(),
            counter: 0,
            prefix,
        }
    }

    /// Returns the name previously assigned to `location`, if any.
    pub(crate) fn get_name(&self, location: &str) -> Option<&String> {
        self.location_to_name.get(location)
    }

    /// Allocates the next unique name for `location`, registers it, and returns it.
    pub(crate) fn alloc_name(&mut self, location: &str) -> String {
        let name = format!("{}_{}", self.prefix, self.counter);
        self.counter += 1;
        self.location_to_name
            .insert(location.to_string(), name.clone());
        name
    }

    /// Stores the compiled body for a previously allocated name.
    pub(crate) fn set_body(&mut self, name: &str, body: TokenStream) {
        self.bodies.insert(name.to_string(), body);
    }

    /// Stores the `validate` body alongside the `is_valid` body.
    pub(crate) fn set_validate_body(&mut self, name: &str, validate: TokenStream) {
        self.validate_bodies.insert(name.to_string(), validate);
    }

    /// Returns the `validate` body for a function, if available.
    pub(crate) fn get_validate_body(&self, name: &str) -> Option<&TokenStream> {
        self.validate_bodies.get(name)
    }

    /// Stores the `collect` body alongside the `is_valid` body.
    pub(crate) fn set_collect_body(&mut self, name: &str, collect: TokenStream) {
        self.collect_bodies.insert(name.to_string(), collect);
    }

    /// Returns the `collect` body for a function, if available.
    pub(crate) fn get_collect_body(&self, name: &str) -> Option<&TokenStream> {
        self.collect_bodies.get(name)
    }

    /// Iterates over all (name, body) pairs in insertion order.
    pub(crate) fn iter_bodies(&self) -> impl Iterator<Item = (&str, &TokenStream)> {
        self.bodies.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Marks `location` as currently being compiled (enter cycle guard).
    pub(crate) fn begin_compiling(&mut self, location: &str) {
        self.in_progress.insert(location.to_string());
    }

    /// Clears the in-progress mark for `location` (exit cycle guard).
    pub(crate) fn finish_compiling(&mut self, location: &str) {
        self.in_progress.remove(location);
    }

    /// Returns `true` if `location` is currently being compiled.
    pub(crate) fn is_compiling(&self, location: &str) -> bool {
        self.in_progress.contains(location)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PatternEngineConfig {
    FancyRegex {
        backtrack_limit: Option<usize>,
        size_limit: Option<usize>,
        dfa_size_limit: Option<usize>,
    },
    Regex {
        size_limit: Option<usize>,
        dfa_size_limit: Option<usize>,
    },
}

impl Default for PatternEngineConfig {
    fn default() -> Self {
        Self::FancyRegex {
            backtrack_limit: None,
            size_limit: None,
            dfa_size_limit: None,
        }
    }
}

/// URI/IRI `format` checks memoized per validation, since their `fluent-uri` parse is expensive and
/// schema documents repeat the same URIs many times.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum UriFormatCache {
    Uri,
    UriReference,
    Iri,
    IriReference,
}

impl UriFormatCache {
    pub(crate) fn for_format(format_name: &str) -> Option<Self> {
        Some(match format_name {
            "uri" => Self::Uri,
            "uri-reference" => Self::UriReference,
            "iri" => Self::Iri,
            "iri-reference" => Self::IriReference,
            _ => return None,
        })
    }

    pub(crate) fn base(self) -> &'static str {
        match self {
            Self::Uri => "uri",
            Self::UriReference => "uri_reference",
            Self::Iri => "iri",
            Self::IriReference => "iri_reference",
        }
    }

    pub(crate) fn format_fn(self) -> &'static str {
        match self {
            Self::Uri => "is_valid_uri",
            Self::UriReference => "is_valid_uri_reference",
            Self::Iri => "is_valid_iri",
            Self::IriReference => "is_valid_iri_reference",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct EmailOptionsConfig {
    pub(crate) minimum_sub_domains: Option<usize>,
    pub(crate) no_minimum_sub_domains: bool,
    pub(crate) required_tld: bool,
    pub(crate) allow_domain_literal: Option<bool>,
    pub(crate) allow_display_text: Option<bool>,
}

/// Immutable configuration built from macro attributes.
pub(crate) struct CodegenConfig {
    pub(crate) schema: serde_json::Value,
    pub(crate) registry: Registry<'static>,
    pub(crate) base_uri: Arc<Uri<String>>,
    pub(crate) draft: Draft,
    pub(crate) runtime_crate_alias: Option<TokenStream>,
    pub(crate) validate_formats: Option<bool>,
    pub(crate) custom_formats: HashMap<String, TokenStream>,
    pub(crate) custom_keywords: HashMap<String, TokenStream>,
    pub(crate) content_media_types: HashMap<String, TokenStream>,
    pub(crate) content_encodings: HashMap<String, (TokenStream, TokenStream)>,
    pub(crate) ignore_unknown_formats: bool,
    pub(crate) email_options: Option<EmailOptionsConfig>,
    pub(crate) pattern_options: PatternEngineConfig,
}

/// Mutable compilation state threaded through all `compile_*` calls.
pub(crate) struct CompileContext<'cfg> {
    pub(crate) config: &'cfg CodegenConfig,
    pub(crate) draft: Draft,
    pub(crate) vocabularies: VocabularySet,
    pub(crate) current_base_uri: Arc<Uri<String>>,
    /// Per-subschema validity checks: `fn(&Value) -> bool` (plus a `validate` variant).
    pub(crate) is_valid_fns: FnTable,
    /// Per-subschema `unevaluatedProperties` checks: is a given property key evaluated?
    pub(crate) key_eval_fns: FnTable,
    /// Per-subschema `unevaluatedItems` checks: is a given array index evaluated?
    pub(crate) item_eval_fns: FnTable,
    pub(crate) dynamic_anchor_bindings_cache:
        HashMap<String, Vec<crate::codegen::DynamicAnchorBinding>>,
    pub(crate) dynamic_anchor_bindings_being_compiled: HashSet<String>,
    pub(crate) regex_to_helper: HashMap<String, String>,
    pub(crate) translated_regex_cache: HashMap<String, String>,
    pub(crate) regex_helpers: Vec<(String, String)>,
    pub(crate) regex_counter: usize,
    pub(crate) custom_keyword_counter: usize,
    pub(crate) compiling_stack: Vec<String>,
    pub(crate) compiling_instance_depths: Vec<usize>,
    pub(crate) schema_depth: usize,
    pub(crate) instance_depth: usize,
    pub(crate) helper_root_depths: Vec<usize>,
    pub(crate) uses_recursive_ref: bool,
    pub(crate) uses_dynamic_ref: bool,
    pub(crate) uses_ref_cycle: bool,
    pub(crate) uri_format_caches: BTreeSet<UriFormatCache>,
    /// JSON Pointer tracking the current position in the schema document, embedded into the error
    /// constructors generated by `validate()`.
    pub(crate) schema_path: String,
}

impl<'cfg> CompileContext<'cfg> {
    pub(crate) fn new(config: &'cfg CodegenConfig) -> Self {
        Self {
            current_base_uri: config.base_uri.clone(),
            draft: config.draft,
            vocabularies: config
                .registry
                .find_vocabularies(config.draft, &config.schema),
            config,
            is_valid_fns: FnTable::new("validate_ref"),
            key_eval_fns: FnTable::new("eval_ref"),
            item_eval_fns: FnTable::new("eval_items_ref"),
            dynamic_anchor_bindings_cache: HashMap::new(),
            dynamic_anchor_bindings_being_compiled: HashSet::new(),
            regex_to_helper: HashMap::new(),
            translated_regex_cache: HashMap::new(),
            regex_helpers: Vec::new(),
            regex_counter: 0,
            custom_keyword_counter: 0,
            compiling_stack: Vec::new(),
            compiling_instance_depths: Vec::new(),
            schema_depth: 0,
            instance_depth: 0,
            helper_root_depths: Vec::new(),
            uses_recursive_ref: false,
            uses_dynamic_ref: false,
            uses_ref_cycle: false,
            uri_format_caches: BTreeSet::new(),
            schema_path: String::new(),
        }
    }

    pub(crate) fn with_schema_scope<T>(
        &mut self,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        self.schema_depth += 1;
        let mut scope = SchemaDepthGuard { ctx: self };
        f(&mut scope)
    }

    pub(crate) fn with_base_uri_scope<T>(
        &mut self,
        base_uri: Arc<Uri<String>>,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        let prev_base_uri = std::mem::replace(&mut self.current_base_uri, base_uri);
        let mut scope = BaseUriGuard {
            ctx: self,
            prev_base_uri,
        };
        f(&mut scope)
    }

    pub(crate) fn with_schema_env<T>(
        &mut self,
        schema: &serde_json::Value,
        schema_base_uri: Arc<Uri<String>>,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        let prev_base_uri = std::mem::replace(&mut self.current_base_uri, schema_base_uri);
        let prev_draft = self.draft;
        let prev_vocabularies = self.vocabularies.clone();

        if let Some(schema_uri) = schema.get("$schema").and_then(|v| v.as_str()) {
            self.draft = self
                .draft
                .detect(&serde_json::json!({ "$schema": schema_uri }));
        }
        self.vocabularies = self.config.registry.find_vocabularies(self.draft, schema);

        let mut scope = SchemaEnvGuard {
            ctx: self,
            prev_base_uri,
            prev_draft,
            prev_vocabularies,
        };
        f(&mut scope)
    }

    pub(crate) fn with_helper_root_scope<T>(
        &mut self,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        self.helper_root_depths.push(self.schema_depth);
        let mut scope = HelperRootDepthGuard { ctx: self };
        f(&mut scope)
    }

    pub(crate) fn with_is_valid_scope<T>(
        &mut self,
        location: &str,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        self.is_valid_fns.begin_compiling(location);
        self.compiling_stack.push(location.to_string());
        self.compiling_instance_depths.push(self.instance_depth);
        let mut scope = ValidateScopeGuard {
            ctx: self,
            location: location.to_string(),
        };
        f(&mut scope)
    }

    pub(crate) fn with_instance_scope<T>(
        &mut self,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        self.instance_depth += 1;
        let mut scope = InstanceDepthGuard { ctx: self };
        f(&mut scope)
    }

    pub(crate) fn closes_same_instance_cycle(&self, location: &str) -> bool {
        self.compiling_stack
            .iter()
            .position(|loc| loc == location)
            .map(|idx| self.compiling_instance_depths[idx])
            .is_some_and(|entry_depth| entry_depth == self.instance_depth)
    }

    pub(crate) fn with_key_eval_scope<T>(
        &mut self,
        location: &str,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        self.key_eval_fns.begin_compiling(location);
        let mut scope = KeyEvalScopeGuard {
            ctx: self,
            location: location.to_string(),
        };
        f(&mut scope)
    }

    pub(crate) fn with_item_eval_scope<T>(
        &mut self,
        location: &str,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        self.item_eval_fns.begin_compiling(location);
        let mut scope = ItemEvalScopeGuard {
            ctx: self,
            location: location.to_string(),
        };
        f(&mut scope)
    }

    pub(crate) fn with_dynamic_anchor_bindings_scope<T>(
        &mut self,
        cache_key: String,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> Option<T> {
        if !self
            .dynamic_anchor_bindings_being_compiled
            .insert(cache_key.clone())
        {
            return None;
        }
        let mut scope = DynamicAnchorBindingsScopeGuard {
            ctx: self,
            cache_key,
        };
        Some(f(&mut scope))
    }

    /// Runs `f` with `schema_path` replaced wholesale (not appended).
    pub(crate) fn with_schema_path_swap<T>(
        &mut self,
        path: String,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        let prev_path = std::mem::replace(&mut self.schema_path, path);
        let mut scope = SchemaPathSwapGuard {
            ctx: self,
            prev_path,
        };
        f(&mut scope)
    }

    pub(crate) fn with_schema_path_segment<T>(
        &mut self,
        segment: &str,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        let prev_len = self.schema_path.len();
        self.schema_path.push('/');
        write_escaped_str(&mut self.schema_path, segment);
        let mut scope = SchemaPathScopeGuard {
            ctx: self,
            prev_len,
        };
        f(&mut scope)
    }

    /// Returns the current schema path as a JSON Pointer string.
    pub(crate) fn current_schema_path(&self) -> &str {
        &self.schema_path
    }

    /// Returns the path to `keyword` relative to the current schema position.
    pub(crate) fn schema_path_for_keyword(&self, keyword: &str) -> String {
        let mut path = self.schema_path.clone();
        path.push('/');
        write_escaped_str(&mut path, keyword);
        path
    }
}

pub(crate) struct SchemaDepthGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
}

impl_scope_guard!(SchemaDepthGuard);

impl Drop for SchemaDepthGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx.schema_depth = self.ctx.schema_depth.saturating_sub(1);
    }
}

pub(crate) struct BaseUriGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
    prev_base_uri: Arc<Uri<String>>,
}

impl_scope_guard!(BaseUriGuard);

impl Drop for BaseUriGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx.current_base_uri = self.prev_base_uri.clone();
    }
}

pub(crate) struct SchemaEnvGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
    prev_base_uri: Arc<Uri<String>>,
    prev_draft: Draft,
    prev_vocabularies: VocabularySet,
}

impl_scope_guard!(SchemaEnvGuard);

impl Drop for SchemaEnvGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx.current_base_uri = self.prev_base_uri.clone();
        self.ctx.draft = self.prev_draft;
        self.ctx.vocabularies = self.prev_vocabularies.clone();
    }
}

pub(crate) struct HelperRootDepthGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
}

impl_scope_guard!(HelperRootDepthGuard);

impl Drop for HelperRootDepthGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx.helper_root_depths.pop();
    }
}

pub(crate) struct ValidateScopeGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
    location: String,
}

impl_scope_guard!(ValidateScopeGuard);

impl Drop for ValidateScopeGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx.is_valid_fns.finish_compiling(&self.location);
        let popped = self.ctx.compiling_stack.pop();
        self.ctx.compiling_instance_depths.pop();
        debug_assert_eq!(popped.as_deref(), Some(self.location.as_str()));
    }
}

pub(crate) struct InstanceDepthGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
}

impl_scope_guard!(InstanceDepthGuard);

impl Drop for InstanceDepthGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx.instance_depth = self.ctx.instance_depth.saturating_sub(1);
    }
}

pub(crate) struct KeyEvalScopeGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
    location: String,
}

impl_scope_guard!(KeyEvalScopeGuard);

impl Drop for KeyEvalScopeGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx.key_eval_fns.finish_compiling(&self.location);
    }
}

pub(crate) struct ItemEvalScopeGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
    location: String,
}

impl_scope_guard!(ItemEvalScopeGuard);

impl Drop for ItemEvalScopeGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx.item_eval_fns.finish_compiling(&self.location);
    }
}

pub(crate) struct DynamicAnchorBindingsScopeGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
    cache_key: String,
}

impl_scope_guard!(DynamicAnchorBindingsScopeGuard);

impl Drop for DynamicAnchorBindingsScopeGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx
            .dynamic_anchor_bindings_being_compiled
            .remove(&self.cache_key);
    }
}

pub(crate) struct SchemaPathSwapGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
    prev_path: String,
}

impl_scope_guard!(SchemaPathSwapGuard);

impl Drop for SchemaPathSwapGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx.schema_path = std::mem::take(&mut self.prev_path);
    }
}

pub(crate) struct SchemaPathScopeGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
    prev_len: usize,
}

impl_scope_guard!(SchemaPathScopeGuard);

impl Drop for SchemaPathScopeGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx.schema_path.truncate(self.prev_len);
    }
}
