use std::{
    collections::{BTreeSet, HashMap, HashSet},
    ops::{Deref, DerefMut},
    sync::Arc,
};

use indexmap::IndexMap;
use proc_macro2::{Ident, Span, TokenStream};
use quote::format_ident;
use referencing::{write_escaped_str, Draft, Registry, Uri, VocabularySet};

use crate::codegen::emit::ValueEmitter;

/// `Deref`/`DerefMut` to `CompileContext` for a scope guard holding `ctx: &mut CompileContext`.
macro_rules! impl_scope_guard {
    ($guard:ident) => {
        impl<'cfg, E: ValueEmitter> Deref for $guard<'_, 'cfg, E> {
            type Target = CompileContext<'cfg, E>;
            fn deref(&self) -> &Self::Target {
                self.ctx
            }
        }
        impl<E: ValueEmitter> DerefMut for $guard<'_, '_, E> {
            fn deref_mut(&mut self) -> &mut Self::Target {
                self.ctx
            }
        }
    };
}

/// A generated function's error reporting: `entry` calls the function holding `body`.
pub(crate) struct ErrorsFn {
    pub(crate) entry: TokenStream,
    pub(crate) body: TokenStream,
}

/// Generated functions of one category, interned by schema location to
/// deduplicate repeated subschemas and break reference cycles.
pub(crate) struct FnTable {
    location_to_name: HashMap<String, String>,
    /// Maps a generated function name to its compiled body.
    bodies: IndexMap<String, TokenStream>,
    /// Optional error-reporting bodies for functions that have one.
    errors_bodies: IndexMap<String, ErrorsFn>,
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
            errors_bodies: IndexMap::new(),
            in_progress: HashSet::new(),
            counter: 0,
            prefix,
        }
    }

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

    /// Stores the error-reporting body alongside the `is_valid` body.
    pub(crate) fn set_errors_body(&mut self, name: &str, errors: ErrorsFn) {
        self.errors_bodies.insert(name.to_string(), errors);
    }

    pub(crate) fn get_errors_body(&self, name: &str) -> Option<&ErrorsFn> {
        self.errors_bodies.get(name)
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum PatternSyntaxConfig {
    #[default]
    Ecma262,
    Engine,
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

/// URI/IRI `format` results memoized per validation run, since instances repeat the same URIs.
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

/// Which public methods the validator gets.
// Independent switches, one per method.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct MethodGates {
    pub(crate) is_valid: bool,
    pub(crate) validate: bool,
    pub(crate) iter_errors: bool,
    pub(crate) evaluate: bool,
}

impl Default for MethodGates {
    fn default() -> Self {
        Self {
            is_valid: true,
            validate: true,
            iter_errors: true,
            evaluate: true,
        }
    }
}

/// Immutable configuration built from macro attributes.
pub(crate) struct CodegenConfig {
    pub(crate) schema: serde_json::Value,
    pub(crate) registry: Registry<'static>,
    pub(crate) base_uri: Arc<Uri<String>>,
    pub(crate) draft: Draft,
    /// Vocabularies the caller declares support for, normalized.
    pub(crate) vocabularies: Vec<String>,
    pub(crate) runtime_crate_alias: Option<TokenStream>,
    pub(crate) validate_formats: Option<bool>,
    pub(crate) custom_formats: HashMap<String, TokenStream>,
    /// Formats registered with `number(...)`, which read numbers.
    pub(crate) custom_number_formats: HashMap<String, TokenStream>,
    pub(crate) custom_keywords: HashMap<String, TokenStream>,
    pub(crate) content_media_types: HashMap<String, TokenStream>,
    pub(crate) content_encodings: HashMap<String, (TokenStream, TokenStream)>,
    pub(crate) ignore_unknown_formats: bool,
    pub(crate) email_options: Option<EmailOptionsConfig>,
    pub(crate) pattern_options: PatternEngineConfig,
    pub(crate) pattern_syntax: PatternSyntaxConfig,
    pub(crate) methods: MethodGates,
}

/// The key memoized `$ref` cycle results are stored under.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefResultsKey {
    /// The target and the instance.
    Instance,
    /// Also the dynamic scope, which `$recursiveRef` and `$dynamicRef` make a result depend on.
    InstanceAndScope {
        /// How many generated calls keep their results.
        calls: usize,
    },
}

/// Mutable compilation state threaded through all `compile_*` calls.
pub(crate) struct CompileContext<'cfg, E: ValueEmitter> {
    backend: std::marker::PhantomData<E>,
    pub(crate) config: &'cfg CodegenConfig,
    pub(crate) draft: Draft,
    pub(crate) vocabularies: VocabularySet,
    /// The first vocabulary an entered meta-schema requires that is neither implemented nor declared.
    pub(crate) undeclared_vocabulary: Option<String>,
    pub(crate) current_base_uri: Arc<Uri<String>>,
    /// Per-subschema validity functions returning `bool`, plus an error-reporting variant.
    pub(crate) is_valid_fns: FnTable,
    /// Per-subschema `unevaluatedProperties` checks: is a given property key evaluated?
    pub(crate) key_eval_fns: FnTable,
    /// Per-subschema `unevaluatedItems` checks: is a given array index evaluated?
    pub(crate) item_eval_fns: FnTable,
    /// Applicator branches emitted once as paired validity and error-collection helpers.
    pub(crate) branch_helpers: Vec<(TokenStream, TokenStream)>,
    /// Index into `branch_helpers`, keyed by the helper's code.
    branch_helper_ids: HashMap<String, usize>,
    /// Error reporting for `type: array` with every item checked by one referenced schema, keyed
    /// by that schema's `is_valid` function.
    pub(crate) array_errors_helpers: IndexMap<String, (Ident, TokenStream)>,
    /// Labels handed out by [`Self::next_loop_label`].
    loop_labels: usize,
    discriminator_assumption: Option<DiscriminatorAssumption>,
    pub(crate) dynamic_anchor_bindings_cache:
        HashMap<String, Vec<crate::codegen::DynamicAnchorBinding>>,
    pub(crate) dynamic_anchor_bindings_being_compiled: HashSet<String>,
    pub(crate) regex_to_helper: HashMap<String, String>,
    /// JSON Pointers of anchor-bearing schemas, keyed by their resource root's address and then
    /// by their own. Roots live in the registry for the whole expansion.
    pub(crate) anchor_pointers: HashMap<usize, HashMap<usize, String>>,
    pub(crate) translated_regex_cache: HashMap<String, String>,
    /// Translated patterns matched by counting characters, see [`jsonschema_regex::RepeatedClass`].
    pub(crate) repeated_classes: HashSet<String>,
    pub(crate) regex_helpers: Vec<(String, String)>,
    pub(crate) regex_counter: usize,
    pub(crate) custom_keyword_counter: usize,
    pub(crate) compiling_stack: Vec<String>,
    pub(crate) schema_depth: usize,
    pub(crate) helper_root_depths: Vec<usize>,
    pub(crate) uses_recursive_ref: bool,
    pub(crate) uses_dynamic_ref: bool,
    pub(crate) uses_ref_cycle: bool,
    /// `$ref` targets in cycles one validation can take through the same instance more than once.
    pub(crate) revisited_ref_targets: HashSet<String>,
    /// For each target location, the target whose code first compiled it; `None` when the root's
    /// did.
    pub(crate) compiled_within: HashMap<String, Option<String>>,
    /// Reference targets a reference reaches while their own code compiles.
    pub(crate) reentered_targets: HashSet<String>,
    /// Reference-target locations of each schema, keyed by its address.
    pub(crate) target_locations: HashMap<usize, Vec<String>>,
    /// `$ref` targets in cycles that come back to the instance they hold.
    pub(crate) same_instance_ref_targets: HashSet<String>,
    /// The location of the `$ref` target the root's code is, when a cycle through the root comes
    /// back to the instance it holds.
    pub(crate) root_target: Option<String>,
    /// The key memoized `$ref` cycle results are stored under.
    pub(crate) ref_results_key: RefResultsKey,
    /// Names of the dynamic-scope stacks that generated references look up.
    pub(crate) looked_up_stacks: BTreeSet<String>,
    pub(crate) uri_format_caches: BTreeSet<UriFormatCache>,
    /// JSON Pointer tracking the current position in the schema document, embedded into the
    /// generated error constructors.
    pub(crate) schema_path: String,
}

/// The draft a schema is written in and the vocabularies in effect for it.
#[derive(Clone)]
pub(crate) struct Dialect {
    pub(crate) draft: Draft,
    pub(crate) vocabularies: VocabularySet,
}

/// The vocabularies of `schema`, and the first custom one it requires that the configured
/// `vocabularies` do not declare.
fn find_vocabularies(
    config: &CodegenConfig,
    draft: Draft,
    schema: &serde_json::Value,
) -> (VocabularySet, Option<String>) {
    let vocabularies = config.registry.find_vocabularies(draft, schema);
    let undeclared = undeclared_vocabulary(config, &vocabularies);
    (vocabularies, undeclared)
}

/// The first vocabulary in `vocabularies` that nobody implements.
fn undeclared_vocabulary(config: &CodegenConfig, vocabularies: &VocabularySet) -> Option<String> {
    vocabularies
        .custom()
        .find(|uri| !config.vocabularies.iter().any(|declared| declared == uri))
        .map(str::to_owned)
}

impl<'cfg, E: ValueEmitter> CompileContext<'cfg, E> {
    /// Whether the generated code keeps the results of `$ref` cycles for the run.
    pub(crate) fn keeps_ref_results(&self) -> bool {
        match self.ref_results_key {
            RefResultsKey::Instance => !self.revisited_ref_targets.is_empty(),
            RefResultsKey::InstanceAndScope { calls } => calls > 0,
        }
    }

    pub(crate) fn new(config: &'cfg CodegenConfig) -> Self {
        let (vocabularies, undeclared_vocabulary) =
            find_vocabularies(config, config.draft, &config.schema);
        Self {
            backend: std::marker::PhantomData,
            current_base_uri: config.base_uri.clone(),
            draft: config.draft,
            vocabularies,
            undeclared_vocabulary,
            config,
            is_valid_fns: FnTable::new("validate_ref"),
            key_eval_fns: FnTable::new("eval_ref"),
            item_eval_fns: FnTable::new("eval_items_ref"),
            branch_helpers: Vec::new(),
            branch_helper_ids: HashMap::new(),
            array_errors_helpers: IndexMap::new(),
            loop_labels: 0,
            discriminator_assumption: None,
            dynamic_anchor_bindings_cache: HashMap::new(),
            dynamic_anchor_bindings_being_compiled: HashSet::new(),
            regex_to_helper: HashMap::new(),
            anchor_pointers: HashMap::new(),
            translated_regex_cache: HashMap::new(),
            repeated_classes: HashSet::new(),
            regex_helpers: Vec::new(),
            regex_counter: 0,
            custom_keyword_counter: 0,
            compiling_stack: Vec::new(),
            schema_depth: 0,
            helper_root_depths: Vec::new(),
            uses_recursive_ref: false,
            uses_dynamic_ref: false,
            uses_ref_cycle: false,
            revisited_ref_targets: HashSet::new(),
            compiled_within: HashMap::new(),
            reentered_targets: HashSet::new(),
            target_locations: HashMap::new(),
            same_instance_ref_targets: HashSet::new(),
            root_target: None,
            ref_results_key: RefResultsKey::Instance,
            looked_up_stacks: BTreeSet::new(),
            uri_format_caches: BTreeSet::new(),
            schema_path: String::new(),
        }
    }

    /// The helper running `is_valid` and `errors` for one branch, shared by every branch
    /// compiling to the same code: `evaluate` compiles each branch again for its validity, which
    /// would otherwise copy every helper below it once per enclosing branch.
    pub(crate) fn register_branch_helper(
        &mut self,
        is_valid: TokenStream,
        errors: TokenStream,
    ) -> usize {
        let key = format!("{is_valid}\u{0}{errors}");
        let next = self.branch_helpers.len();
        let idx = *self.branch_helper_ids.entry(key).or_insert(next);
        if idx == next {
            self.branch_helpers.push((is_valid, errors));
        }
        idx
    }

    /// The helper reporting errors for arrays of the schema behind `ref_name`, with `body` building
    /// it the first time; the type error lands at `__type_path`.
    pub(crate) fn array_errors_helper(
        &mut self,
        ref_name: String,
        body: impl FnOnce() -> TokenStream,
    ) -> Ident {
        let entry = self.array_errors_helpers.entry(ref_name);
        let idx = entry.index();
        entry
            .or_insert_with(|| (format_ident!("array_errors_{}", idx), body()))
            .0
            .clone()
    }

    /// A loop label unique within the expansion: nested loops sharing a label would trigger
    /// rustc's label-shadowing warning, which no `allow` silences.
    pub(crate) fn next_loop_label(&mut self) -> syn::Lifetime {
        let label = syn::Lifetime::new(&format!("'__all_{}", self.loop_labels), Span::call_site());
        self.loop_labels += 1;
        label
    }

    pub(crate) fn with_discriminator_assumption<T>(
        &mut self,
        property_name: &str,
        reduced_property: serde_json::Value,
        f: impl FnOnce(&mut CompileContext<'cfg, E>) -> T,
    ) -> T {
        let assumption = DiscriminatorAssumption {
            schema_path: self.schema_path.clone(),
            property_name: property_name.to_owned(),
            reduced_property,
        };
        let previous = self.discriminator_assumption.replace(assumption);
        let mut scope = DiscriminatorAssumptionGuard {
            ctx: self,
            previous,
        };
        f(&mut scope)
    }

    pub(crate) fn discriminator_assumption(&self) -> Option<(&str, &serde_json::Value)> {
        self.discriminator_assumption
            .as_ref()
            .filter(|assumption| assumption.schema_path == self.schema_path)
            .map(|assumption| {
                (
                    assumption.property_name.as_str(),
                    &assumption.reduced_property,
                )
            })
    }

    pub(crate) fn with_schema_scope<T>(
        &mut self,
        f: impl FnOnce(&mut CompileContext<'cfg, E>) -> T,
    ) -> T {
        self.schema_depth += 1;
        let mut scope = SchemaDepthGuard { ctx: self };
        f(&mut scope)
    }

    pub(crate) fn with_base_uri_scope<T>(
        &mut self,
        base_uri: Arc<Uri<String>>,
        f: impl FnOnce(&mut CompileContext<'cfg, E>) -> T,
    ) -> T {
        let prev_base_uri = std::mem::replace(&mut self.current_base_uri, base_uri);
        let mut scope = BaseUriGuard {
            ctx: self,
            prev_base_uri,
        };
        f(&mut scope)
    }

    /// `f` in the resource at `schema_base_uri` and the `dialect` its schema is written in.
    pub(crate) fn with_schema_env<T>(
        &mut self,
        schema_base_uri: Arc<Uri<String>>,
        dialect: Dialect,
        f: impl FnOnce(&mut CompileContext<'cfg, E>) -> T,
    ) -> T {
        if self.undeclared_vocabulary.is_none() {
            self.undeclared_vocabulary = undeclared_vocabulary(self.config, &dialect.vocabularies);
        }
        let prev_base_uri = std::mem::replace(&mut self.current_base_uri, schema_base_uri);
        let prev_draft = std::mem::replace(&mut self.draft, dialect.draft);
        let prev_vocabularies = std::mem::replace(&mut self.vocabularies, dialect.vocabularies);

        let mut scope = SchemaEnvGuard {
            ctx: self,
            prev_base_uri,
            prev_draft,
            prev_vocabularies,
        };
        f(&mut scope)
    }

    /// The dialect in effect here.
    pub(crate) fn dialect(&self) -> Dialect {
        Dialect {
            draft: self.draft,
            vocabularies: self.vocabularies.clone(),
        }
    }

    /// The dialect `schema` is written in: the one its `$schema` names, or the one in effect here.
    pub(crate) fn dialect_of(&self, schema: &serde_json::Value) -> Dialect {
        if schema
            .get("$schema")
            .and_then(serde_json::Value::as_str)
            .is_none()
        {
            return self.dialect();
        }
        let draft = self.draft.detect(schema);
        Dialect {
            draft,
            vocabularies: self.config.registry.find_vocabularies(draft, schema),
        }
    }

    /// `f` in the draft and vocabularies `schema`'s `$schema` names.
    pub(crate) fn with_dialect_of<T>(
        &mut self,
        schema: &serde_json::Map<String, serde_json::Value>,
        f: impl FnOnce(&mut CompileContext<'cfg, E>) -> T,
    ) -> T {
        let Some(dialect) = schema.get("$schema").and_then(serde_json::Value::as_str) else {
            return f(self);
        };
        let draft = Draft::from_schema_uri(dialect);
        if draft == self.draft {
            return f(self);
        }
        let (vocabularies, undeclared_vocabulary) = find_vocabularies(
            self.config,
            draft,
            &serde_json::json!({ "$schema": dialect }),
        );
        if self.undeclared_vocabulary.is_none() {
            self.undeclared_vocabulary = undeclared_vocabulary;
        }
        let prev_draft = std::mem::replace(&mut self.draft, draft);
        let prev_vocabularies = std::mem::replace(&mut self.vocabularies, vocabularies);
        let mut scope = DialectGuard {
            ctx: self,
            prev_draft,
            prev_vocabularies,
        };
        f(&mut scope)
    }

    pub(crate) fn with_helper_root_scope<T>(
        &mut self,
        f: impl FnOnce(&mut CompileContext<'cfg, E>) -> T,
    ) -> T {
        self.helper_root_depths.push(self.schema_depth);
        let mut scope = HelperRootDepthGuard { ctx: self };
        f(&mut scope)
    }

    pub(crate) fn with_is_valid_scope<T>(
        &mut self,
        location: &str,
        f: impl FnOnce(&mut CompileContext<'cfg, E>) -> T,
    ) -> T {
        self.is_valid_fns.begin_compiling(location);
        self.compiling_stack.push(location.to_string());
        let mut scope = ValidateScopeGuard {
            ctx: self,
            location: location.to_string(),
        };
        f(&mut scope)
    }

    /// Runs `f` as if inside the code of each reference target in `locations`, entered on the same
    /// instance.
    pub(crate) fn within_targets<T>(
        &mut self,
        locations: &[String],
        f: impl FnOnce(&mut CompileContext<'cfg, E>) -> T,
    ) -> T {
        match locations.split_first() {
            Some((location, rest)) => {
                self.with_is_valid_scope(location, |ctx| ctx.within_targets(rest, f))
            }
            None => f(self),
        }
    }

    /// Whether a `$ref` to `location`, a target still compiling, stops when it re-enters an
    /// instance the target holds already, as the runtime's guard on the target does. A target on
    /// a cycle back to its own instance takes the guard at every `$ref` that closes a cycle, the
    /// ones passing it a nested instance included: that instance comes back through the cycle too.
    pub(crate) fn closes_same_instance_cycle(&self, location: &str) -> bool {
        self.same_instance_ref_targets.contains(location)
    }

    pub(crate) fn with_key_eval_scope<T>(
        &mut self,
        location: &str,
        f: impl FnOnce(&mut CompileContext<'cfg, E>) -> T,
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
        f: impl FnOnce(&mut CompileContext<'cfg, E>) -> T,
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
        f: impl FnOnce(&mut CompileContext<'cfg, E>) -> T,
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
        f: impl FnOnce(&mut CompileContext<'cfg, E>) -> T,
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
        f: impl FnOnce(&mut CompileContext<'cfg, E>) -> T,
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

struct DiscriminatorAssumption {
    schema_path: String,
    property_name: String,
    reduced_property: serde_json::Value,
}

pub(crate) struct SchemaDepthGuard<'a, 'cfg, E: ValueEmitter> {
    ctx: &'a mut CompileContext<'cfg, E>,
}

impl_scope_guard!(SchemaDepthGuard);

impl<E: ValueEmitter> Drop for SchemaDepthGuard<'_, '_, E> {
    fn drop(&mut self) {
        self.ctx.schema_depth = self.ctx.schema_depth.saturating_sub(1);
    }
}

pub(crate) struct BaseUriGuard<'a, 'cfg, E: ValueEmitter> {
    ctx: &'a mut CompileContext<'cfg, E>,
    prev_base_uri: Arc<Uri<String>>,
}

pub(crate) struct DiscriminatorAssumptionGuard<'a, 'cfg, E: ValueEmitter> {
    ctx: &'a mut CompileContext<'cfg, E>,
    previous: Option<DiscriminatorAssumption>,
}

impl_scope_guard!(DiscriminatorAssumptionGuard);

impl<E: ValueEmitter> Drop for DiscriminatorAssumptionGuard<'_, '_, E> {
    fn drop(&mut self) {
        self.ctx.discriminator_assumption = self.previous.take();
    }
}

impl_scope_guard!(BaseUriGuard);

impl<E: ValueEmitter> Drop for BaseUriGuard<'_, '_, E> {
    fn drop(&mut self) {
        self.ctx.current_base_uri = self.prev_base_uri.clone();
    }
}

pub(crate) struct SchemaEnvGuard<'a, 'cfg, E: ValueEmitter> {
    ctx: &'a mut CompileContext<'cfg, E>,
    prev_base_uri: Arc<Uri<String>>,
    prev_draft: Draft,
    prev_vocabularies: VocabularySet,
}

impl_scope_guard!(SchemaEnvGuard);

impl<E: ValueEmitter> Drop for SchemaEnvGuard<'_, '_, E> {
    fn drop(&mut self) {
        self.ctx.current_base_uri = self.prev_base_uri.clone();
        self.ctx.draft = self.prev_draft;
        self.ctx.vocabularies = self.prev_vocabularies.clone();
    }
}

pub(crate) struct DialectGuard<'a, 'cfg, E: ValueEmitter> {
    ctx: &'a mut CompileContext<'cfg, E>,
    prev_draft: Draft,
    prev_vocabularies: VocabularySet,
}

impl_scope_guard!(DialectGuard);

impl<E: ValueEmitter> Drop for DialectGuard<'_, '_, E> {
    fn drop(&mut self) {
        self.ctx.draft = self.prev_draft;
        self.ctx.vocabularies = std::mem::take(&mut self.prev_vocabularies);
    }
}

pub(crate) struct HelperRootDepthGuard<'a, 'cfg, E: ValueEmitter> {
    ctx: &'a mut CompileContext<'cfg, E>,
}

impl_scope_guard!(HelperRootDepthGuard);

impl<E: ValueEmitter> Drop for HelperRootDepthGuard<'_, '_, E> {
    fn drop(&mut self) {
        self.ctx.helper_root_depths.pop();
    }
}

pub(crate) struct ValidateScopeGuard<'a, 'cfg, E: ValueEmitter> {
    ctx: &'a mut CompileContext<'cfg, E>,
    location: String,
}

impl_scope_guard!(ValidateScopeGuard);

impl<E: ValueEmitter> Drop for ValidateScopeGuard<'_, '_, E> {
    fn drop(&mut self) {
        self.ctx.is_valid_fns.finish_compiling(&self.location);
        let popped = self.ctx.compiling_stack.pop();
        debug_assert_eq!(popped.as_deref(), Some(self.location.as_str()));
    }
}

pub(crate) struct KeyEvalScopeGuard<'a, 'cfg, E: ValueEmitter> {
    ctx: &'a mut CompileContext<'cfg, E>,
    location: String,
}

impl_scope_guard!(KeyEvalScopeGuard);

impl<E: ValueEmitter> Drop for KeyEvalScopeGuard<'_, '_, E> {
    fn drop(&mut self) {
        self.ctx.key_eval_fns.finish_compiling(&self.location);
    }
}

pub(crate) struct ItemEvalScopeGuard<'a, 'cfg, E: ValueEmitter> {
    ctx: &'a mut CompileContext<'cfg, E>,
    location: String,
}

impl_scope_guard!(ItemEvalScopeGuard);

impl<E: ValueEmitter> Drop for ItemEvalScopeGuard<'_, '_, E> {
    fn drop(&mut self) {
        self.ctx.item_eval_fns.finish_compiling(&self.location);
    }
}

pub(crate) struct DynamicAnchorBindingsScopeGuard<'a, 'cfg, E: ValueEmitter> {
    ctx: &'a mut CompileContext<'cfg, E>,
    cache_key: String,
}

impl_scope_guard!(DynamicAnchorBindingsScopeGuard);

impl<E: ValueEmitter> Drop for DynamicAnchorBindingsScopeGuard<'_, '_, E> {
    fn drop(&mut self) {
        self.ctx
            .dynamic_anchor_bindings_being_compiled
            .remove(&self.cache_key);
    }
}

pub(crate) struct SchemaPathSwapGuard<'a, 'cfg, E: ValueEmitter> {
    ctx: &'a mut CompileContext<'cfg, E>,
    prev_path: String,
}

impl_scope_guard!(SchemaPathSwapGuard);

impl<E: ValueEmitter> Drop for SchemaPathSwapGuard<'_, '_, E> {
    fn drop(&mut self) {
        self.ctx.schema_path = std::mem::take(&mut self.prev_path);
    }
}

pub(crate) struct SchemaPathScopeGuard<'a, 'cfg, E: ValueEmitter> {
    ctx: &'a mut CompileContext<'cfg, E>,
    prev_len: usize,
}

impl_scope_guard!(SchemaPathScopeGuard);

impl<E: ValueEmitter> Drop for SchemaPathScopeGuard<'_, '_, E> {
    fn drop(&mut self) {
        self.ctx.schema_path.truncate(self.prev_len);
    }
}
