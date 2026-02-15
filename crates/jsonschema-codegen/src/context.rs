use std::{
    collections::{HashMap, HashSet},
    ops::{Deref, DerefMut},
    sync::Arc,
};

use crate::codegen::backend::BackendKind;
use proc_macro2::TokenStream;
use referencing::{Draft, Registry, Uri, VocabularySet};

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
    pub(crate) registry: Registry,
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
    pub(crate) location_to_function: HashMap<String, String>,
    pub(crate) location_to_eval_function: HashMap<String, String>,
    pub(crate) location_to_item_eval_function: HashMap<String, String>,
    pub(crate) is_valid_bodies: HashMap<String, TokenStream>,
    pub(crate) eval_bodies: HashMap<String, TokenStream>,
    pub(crate) item_eval_bodies: HashMap<String, TokenStream>,
    pub(crate) dynamic_anchor_bindings_cache:
        HashMap<String, Vec<(String, String, String, String)>>,
    pub(crate) dynamic_anchor_bindings_in_progress: HashSet<String>,
    pub(crate) regex_to_helper: HashMap<String, String>,
    pub(crate) regex_helpers: Vec<(String, String)>,
    pub(crate) ref_counter: usize,
    pub(crate) eval_counter: usize,
    pub(crate) item_eval_counter: usize,
    pub(crate) regex_counter: usize,
    pub(crate) seen: HashSet<String>,
    pub(crate) eval_seen: HashSet<String>,
    pub(crate) item_eval_seen: HashSet<String>,
    pub(crate) compiling_stack: Vec<String>,
    pub(crate) schema_depth: usize,
    pub(crate) helper_root_depths: Vec<usize>,
    pub(crate) uses_recursive_ref: bool,
    pub(crate) uses_dynamic_ref: bool,
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
            location_to_function: HashMap::new(),
            location_to_eval_function: HashMap::new(),
            location_to_item_eval_function: HashMap::new(),
            is_valid_bodies: HashMap::new(),
            eval_bodies: HashMap::new(),
            item_eval_bodies: HashMap::new(),
            dynamic_anchor_bindings_cache: HashMap::new(),
            dynamic_anchor_bindings_in_progress: HashSet::new(),
            regex_to_helper: HashMap::new(),
            regex_helpers: Vec::new(),
            ref_counter: 0,
            eval_counter: 0,
            item_eval_counter: 0,
            regex_counter: 0,
            seen: HashSet::new(),
            eval_seen: HashSet::new(),
            item_eval_seen: HashSet::new(),
            compiling_stack: Vec::new(),
            schema_depth: 0,
            helper_root_depths: Vec::new(),
            uses_recursive_ref: matches!(config.draft, Draft::Draft201909),
            uses_dynamic_ref: false,
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

    pub(crate) fn enter_ref_compilation_scope<'a>(
        &'a mut self,
        location: &str,
    ) -> RefCompilationGuard<'a, 'cfg> {
        self.seen.insert(location.to_string());
        self.compiling_stack.push(location.to_string());
        RefCompilationGuard {
            ctx: self,
            location: location.to_string(),
        }
    }

    pub(crate) fn with_ref_compilation_scope<T>(
        &mut self,
        location: &str,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        let mut scope = self.enter_ref_compilation_scope(location);
        f(&mut scope)
    }

    pub(crate) fn enter_eval_compilation_scope<'a>(
        &'a mut self,
        location: &str,
    ) -> EvalCompilationGuard<'a, 'cfg> {
        self.eval_seen.insert(location.to_string());
        EvalCompilationGuard {
            ctx: self,
            location: location.to_string(),
        }
    }

    pub(crate) fn with_eval_compilation_scope<T>(
        &mut self,
        location: &str,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        let mut scope = self.enter_eval_compilation_scope(location);
        f(&mut scope)
    }

    pub(crate) fn enter_item_eval_compilation_scope<'a>(
        &'a mut self,
        location: &str,
    ) -> ItemEvalCompilationGuard<'a, 'cfg> {
        self.item_eval_seen.insert(location.to_string());
        ItemEvalCompilationGuard {
            ctx: self,
            location: location.to_string(),
        }
    }

    pub(crate) fn with_item_eval_compilation_scope<T>(
        &mut self,
        location: &str,
        f: impl FnOnce(&mut CompileContext<'cfg>) -> T,
    ) -> T {
        let mut scope = self.enter_item_eval_compilation_scope(location);
        f(&mut scope)
    }

    pub(crate) fn enter_dynamic_anchor_bindings_scope<'a>(
        &'a mut self,
        cache_key: String,
    ) -> Option<DynamicAnchorBindingsScopeGuard<'a, 'cfg>> {
        if !self
            .dynamic_anchor_bindings_in_progress
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
}

pub(crate) struct SchemaDepthGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
}

impl<'cfg> Deref for SchemaDepthGuard<'_, 'cfg> {
    type Target = CompileContext<'cfg>;

    fn deref(&self) -> &Self::Target {
        self.ctx
    }
}

impl DerefMut for SchemaDepthGuard<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ctx
    }
}

impl Drop for SchemaDepthGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx.schema_depth = self.ctx.schema_depth.saturating_sub(1);
    }
}

pub(crate) struct BaseUriGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
    prev_base_uri: Arc<Uri<String>>,
}

impl<'cfg> Deref for BaseUriGuard<'_, 'cfg> {
    type Target = CompileContext<'cfg>;

    fn deref(&self) -> &Self::Target {
        self.ctx
    }
}

impl DerefMut for BaseUriGuard<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ctx
    }
}

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

impl<'cfg> Deref for SchemaEnvGuard<'_, 'cfg> {
    type Target = CompileContext<'cfg>;

    fn deref(&self) -> &Self::Target {
        self.ctx
    }
}

impl DerefMut for SchemaEnvGuard<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ctx
    }
}

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

impl<'cfg> Deref for HelperRootDepthGuard<'_, 'cfg> {
    type Target = CompileContext<'cfg>;

    fn deref(&self) -> &Self::Target {
        self.ctx
    }
}

impl DerefMut for HelperRootDepthGuard<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ctx
    }
}

impl Drop for HelperRootDepthGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx.helper_root_depths.pop();
    }
}

pub(crate) struct RefCompilationGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
    location: String,
}

impl<'cfg> Deref for RefCompilationGuard<'_, 'cfg> {
    type Target = CompileContext<'cfg>;

    fn deref(&self) -> &Self::Target {
        self.ctx
    }
}

impl DerefMut for RefCompilationGuard<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ctx
    }
}

impl Drop for RefCompilationGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx.seen.remove(&self.location);
        debug_assert_eq!(
            self.ctx.compiling_stack.pop().as_deref(),
            Some(self.location.as_str())
        );
    }
}

pub(crate) struct EvalCompilationGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
    location: String,
}

impl<'cfg> Deref for EvalCompilationGuard<'_, 'cfg> {
    type Target = CompileContext<'cfg>;

    fn deref(&self) -> &Self::Target {
        self.ctx
    }
}

impl DerefMut for EvalCompilationGuard<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ctx
    }
}

impl Drop for EvalCompilationGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx.eval_seen.remove(&self.location);
    }
}

pub(crate) struct ItemEvalCompilationGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
    location: String,
}

impl<'cfg> Deref for ItemEvalCompilationGuard<'_, 'cfg> {
    type Target = CompileContext<'cfg>;

    fn deref(&self) -> &Self::Target {
        self.ctx
    }
}

impl DerefMut for ItemEvalCompilationGuard<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ctx
    }
}

impl Drop for ItemEvalCompilationGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx.item_eval_seen.remove(&self.location);
    }
}

pub(crate) struct DynamicAnchorBindingsScopeGuard<'a, 'cfg> {
    ctx: &'a mut CompileContext<'cfg>,
    cache_key: String,
}

impl<'cfg> Deref for DynamicAnchorBindingsScopeGuard<'_, 'cfg> {
    type Target = CompileContext<'cfg>;

    fn deref(&self) -> &Self::Target {
        self.ctx
    }
}

impl DerefMut for DynamicAnchorBindingsScopeGuard<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.ctx
    }
}

impl Drop for DynamicAnchorBindingsScopeGuard<'_, '_> {
    fn drop(&mut self) {
        self.ctx
            .dynamic_anchor_bindings_in_progress
            .remove(&self.cache_key);
    }
}
