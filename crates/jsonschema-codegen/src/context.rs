use std::{
    collections::{HashMap, HashSet},
    ops::{Deref, DerefMut},
    sync::Arc,
};

/// Generates the `Deref` and `DerefMut` impls for a scope guard whose `ctx`
/// field is `&'a mut CompileContext<'cfg>`.  Every guard uses the same two
/// impl bodies, so there is no point repeating them eight times.
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

use crate::codegen::backend::BackendKind;
use proc_macro2::TokenStream;
use referencing::{write_escaped_str, Draft, Registry, Uri, VocabularySet};

/// Tracks helper functions for one compilation mode (`is_valid` / `key_eval` / `item_eval`).
///
/// Groups together the location→name map, the name→body map, and the in-progress
/// set that guards against infinite recursion — eliminating the nine scattered
/// fields that previously lived directly on `CompileContext`.
pub(crate) struct Helpers {
    /// Maps schema location strings to the generated helper function name.
    location_to_name: HashMap<String, String>,
    /// Maps generated helper function names to their compiled bodies.
    bodies: HashMap<String, TokenStream>,
    /// Optional `validate`/`iter_errors` bodies for helper functions that support them.
    validate_iter_bodies: HashMap<String, (TokenStream, TokenStream)>,
    /// Locations currently being compiled (cycle guard).
    in_progress: HashSet<String>,
    counter: usize,
    prefix: &'static str,
}

impl Helpers {
    pub(crate) fn new(prefix: &'static str) -> Self {
        Self {
            location_to_name: HashMap::new(),
            bodies: HashMap::new(),
            validate_iter_bodies: HashMap::new(),
            in_progress: HashSet::new(),
            counter: 0,
            prefix,
        }
    }

    /// Returns the previously assigned helper name for `location`, if any.
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

    /// Stores `validate` and `iter_errors` bodies alongside the `is_valid` body.
    pub(crate) fn set_validate_iter_bodies(
        &mut self,
        name: &str,
        validate: TokenStream,
        iter_errors: TokenStream,
    ) {
        self.validate_iter_bodies
            .insert(name.to_string(), (validate, iter_errors));
    }

    /// Returns `validate`/`iter_errors` bodies for a helper, if available.
    pub(crate) fn get_validate_iter_bodies(
        &self,
        name: &str,
    ) -> Option<&(TokenStream, TokenStream)> {
        self.validate_iter_bodies.get(name)
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
    pub(crate) ignore_unknown_formats: bool,
    pub(crate) email_options: Option<EmailOptionsConfig>,
    pub(crate) pattern_options: PatternEngineConfig,
    pub(crate) backend: BackendKind,
}

/// Mutable compilation state threaded through all `compile_*` calls.
pub(crate) struct CompileContext<'cfg> {
    pub(crate) config: &'cfg CodegenConfig,
    pub(crate) draft: Draft,
    pub(crate) vocabularies: VocabularySet,
    pub(crate) current_base_uri: Arc<Uri<String>>,
    /// Helper functions for the `is_valid` mode.
    pub(crate) is_valid_helpers: Helpers,
    /// Helper functions for the key-evaluation (unevaluatedProperties) mode.
    pub(crate) key_eval_helpers: Helpers,
    /// Helper functions for the index-evaluation (unevaluatedItems) mode.
    pub(crate) item_eval_helpers: Helpers,
    pub(crate) dynamic_anchor_bindings_cache:
        HashMap<String, Vec<crate::codegen::DynamicAnchorBinding>>,
    pub(crate) dynamic_anchor_bindings_being_compiled: HashSet<String>,
    pub(crate) regex_to_helper: HashMap<String, String>,
    pub(crate) regex_helpers: Vec<(String, String)>,
    pub(crate) regex_counter: usize,
    pub(crate) compiling_stack: Vec<String>,
    pub(crate) schema_depth: usize,
    pub(crate) helper_root_depths: Vec<usize>,
    pub(crate) uses_recursive_ref: bool,
    pub(crate) uses_dynamic_ref: bool,
    /// Owned JSON Pointer string tracking the current position in the schema
    /// document during compilation. Used to embed accurate schema paths in error
    /// constructor calls generated by `validate()` / `iter_errors()`.
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
            is_valid_helpers: Helpers::new("validate_ref"),
            key_eval_helpers: Helpers::new("eval_ref"),
            item_eval_helpers: Helpers::new("eval_items_ref"),
            dynamic_anchor_bindings_cache: HashMap::new(),
            dynamic_anchor_bindings_being_compiled: HashSet::new(),
            regex_to_helper: HashMap::new(),
            regex_helpers: Vec::new(),
            regex_counter: 0,
            compiling_stack: Vec::new(),
            schema_depth: 0,
            helper_root_depths: Vec::new(),
            uses_recursive_ref: false,
            uses_dynamic_ref: false,
            schema_path: String::new(),
        }
    }

    pub(crate) fn enter_schema_scope<'a>(&'a mut self) -> SchemaDepthGuard<'a, 'cfg> {
        self.schema_depth += 1;
        SchemaDepthGuard { ctx: self }
    }

    pub(crate) fn with_schema_scope<T>(
        &mut self,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        let mut scope = self.enter_schema_scope();
        f(&mut scope)
    }

    pub(crate) fn enter_base_uri_scope<'a>(
        &'a mut self,
        base_uri: Arc<Uri<String>>,
    ) -> BaseUriGuard<'a, 'cfg> {
        let prev_base_uri = self.current_base_uri.clone();
        self.current_base_uri = base_uri;
        BaseUriGuard {
            ctx: self,
            prev_base_uri,
        }
    }

    pub(crate) fn with_base_uri_scope<T>(
        &mut self,
        base_uri: Arc<Uri<String>>,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        let mut scope = self.enter_base_uri_scope(base_uri);
        f(&mut scope)
    }

    pub(crate) fn enter_schema_env_scope<'a>(
        &'a mut self,
        schema: &serde_json::Value,
        schema_base_uri: Arc<Uri<String>>,
    ) -> SchemaEnvGuard<'a, 'cfg> {
        let prev_base_uri = self.current_base_uri.clone();
        let prev_draft = self.draft;
        let prev_vocabularies = self.vocabularies.clone();

        self.current_base_uri = schema_base_uri;
        if let Some(schema_uri) = schema.get("$schema").and_then(|v| v.as_str()) {
            self.draft = self
                .draft
                .detect(&serde_json::json!({ "$schema": schema_uri }));
        }
        self.vocabularies = self.config.registry.find_vocabularies(self.draft, schema);

        SchemaEnvGuard {
            ctx: self,
            prev_base_uri,
            prev_draft,
            prev_vocabularies,
        }
    }

    pub(crate) fn with_schema_env<T>(
        &mut self,
        schema: &serde_json::Value,
        schema_base_uri: Arc<Uri<String>>,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        let mut scope = self.enter_schema_env_scope(schema, schema_base_uri);
        f(&mut scope)
    }

    pub(crate) fn enter_helper_root_scope<'a>(&'a mut self) -> HelperRootDepthGuard<'a, 'cfg> {
        self.helper_root_depths.push(self.schema_depth);
        HelperRootDepthGuard { ctx: self }
    }

    pub(crate) fn with_helper_root_scope<T>(
        &mut self,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        let mut scope = self.enter_helper_root_scope();
        f(&mut scope)
    }

    pub(crate) fn enter_validate_scope<'a>(
        &'a mut self,
        location: &str,
    ) -> ValidateScopeGuard<'a, 'cfg> {
        self.is_valid_helpers.begin_compiling(location);
        self.compiling_stack.push(location.to_string());
        ValidateScopeGuard {
            ctx: self,
            location: location.to_string(),
        }
    }

    pub(crate) fn with_is_valid_scope<T>(
        &mut self,
        location: &str,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        let mut scope = self.enter_validate_scope(location);
        f(&mut scope)
    }

    pub(crate) fn enter_key_eval_scope<'a>(
        &'a mut self,
        location: &str,
    ) -> KeyEvalScopeGuard<'a, 'cfg> {
        self.key_eval_helpers.begin_compiling(location);
        KeyEvalScopeGuard {
            ctx: self,
            location: location.to_string(),
        }
    }

    pub(crate) fn with_key_eval_scope<T>(
        &mut self,
        location: &str,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        let mut scope = self.enter_key_eval_scope(location);
        f(&mut scope)
    }

    pub(crate) fn enter_item_eval_scope<'a>(
        &'a mut self,
        location: &str,
    ) -> ItemEvalScopeGuard<'a, 'cfg> {
        self.item_eval_helpers.begin_compiling(location);
        ItemEvalScopeGuard {
            ctx: self,
            location: location.to_string(),
        }
    }

    pub(crate) fn with_item_eval_scope<T>(
        &mut self,
        location: &str,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        let mut scope = self.enter_item_eval_scope(location);
        f(&mut scope)
    }

    pub(crate) fn enter_dynamic_anchor_bindings_scope<'a>(
        &'a mut self,
        cache_key: String,
    ) -> Option<DynamicAnchorBindingsScopeGuard<'a, 'cfg>> {
        if !self
            .dynamic_anchor_bindings_being_compiled
            .insert(cache_key.clone())
        {
            return None;
        }
        Some(DynamicAnchorBindingsScopeGuard {
            ctx: self,
            cache_key,
        })
    }

    pub(crate) fn with_dynamic_anchor_bindings_scope<T>(
        &mut self,
        cache_key: String,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> Option<T> {
        let mut scope = self.enter_dynamic_anchor_bindings_scope(cache_key)?;
        Some(f(&mut scope))
    }

    pub(crate) fn enter_schema_path_scope<'a>(
        &'a mut self,
        segment: &str,
    ) -> SchemaPathScopeGuard<'a, 'cfg> {
        let prev_len = self.schema_path.len();
        self.schema_path.push('/');
        write_escaped_str(&mut self.schema_path, segment);
        SchemaPathScopeGuard {
            ctx: self,
            prev_len,
        }
    }

    pub(crate) fn with_schema_path_segment<T>(
        &mut self,
        segment: &str,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        let mut scope = self.enter_schema_path_scope(segment);
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
        self.ctx.is_valid_helpers.finish_compiling(&self.location);
        debug_assert_eq!(
            self.ctx.compiling_stack.pop().as_deref(),
            Some(self.location.as_str())
        );
    }
}

pub(crate) struct KeyEvalScopeGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
    location: String,
}

impl_scope_guard!(KeyEvalScopeGuard);

impl Drop for KeyEvalScopeGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx.key_eval_helpers.finish_compiling(&self.location);
    }
}

pub(crate) struct ItemEvalScopeGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
    location: String,
}

impl_scope_guard!(ItemEvalScopeGuard);

impl Drop for ItemEvalScopeGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx.item_eval_helpers.finish_compiling(&self.location);
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
