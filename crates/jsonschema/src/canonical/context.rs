//! Shared state for one canonicalization run: draft, pattern engine, and a compiled-regex cache.
use std::{
    cell::{Cell, RefCell},
    cmp::Ordering,
    sync::Arc,
};

use ahash::AHashMap;

use referencing::Draft;

use crate::{canonical::ir::Schema, options::PatternEngineOptions};

/// Past this many remembered pairs a run keeps recomputing rather than grow without end.
const INTERSECTION_CACHE_CAPACITY: usize = 1 << 20;

pub(crate) enum CompiledMatcher {
    Regex(regex::Regex),
    FancyRegex(fancy_regex::Regex),
}

impl CompiledMatcher {
    /// A match error (e.g. `fancy_regex` hitting its backtrack limit) counts as no match, matching
    /// the runtime `pattern` validator's `is_valid`.
    pub(crate) fn is_match(&self, text: &str) -> bool {
        match self {
            Self::Regex(regex) => regex.is_match(text),
            Self::FancyRegex(regex) => regex.is_match(text).unwrap_or(false),
        }
    }
}

pub(crate) struct CanonicalizationContext {
    draft: Draft,
    pattern_options: PatternEngineOptions,
    /// When false `format` is an annotation, constrains nothing, and is dropped.
    validate_formats: bool,
    /// `None` caches a rejected pattern so callers don't recompile it.
    regex_cache: RefCell<AHashMap<Arc<str>, Option<Arc<CompiledMatcher>>>>,
    /// A conjunction over unions takes the product of their branches, which reaches the same pair
    /// of nodes over and over - on a schema of five such conjunctions, 431 times per distinct pair.
    intersections: RefCell<AHashMap<(Schema, Schema), Schema>>,
    /// A meet reached during this run that the canonical form has no exact spelling for. Nodes
    /// built around it may already be wrong, so the whole run is discarded rather than the site.
    unspellable_meet: Cell<bool>,
}

impl CanonicalizationContext {
    pub(crate) fn new(
        draft: Draft,
        pattern_options: PatternEngineOptions,
        validate_formats: bool,
    ) -> Self {
        Self {
            draft,
            pattern_options,
            validate_formats,
            regex_cache: RefCell::new(AHashMap::new()),
            intersections: RefCell::new(AHashMap::new()),
            unspellable_meet: Cell::new(false),
        }
    }

    pub(crate) fn draft(&self) -> Draft {
        self.draft
    }

    pub(crate) fn record_unspellable_meet(&self) {
        self.unspellable_meet.set(true);
    }

    pub(crate) fn saw_unspellable_meet(&self) -> bool {
        self.unspellable_meet.get()
    }

    pub(crate) fn validate_formats(&self) -> bool {
        self.validate_formats
    }

    /// The pattern compiled under the configured engine, or `None` if the engine rejects it. Compiled
    /// once per run and cached, so parse-time validation and membership share the same matcher.
    pub(crate) fn compile_regex(&self, pattern: &Arc<str>) -> Option<Arc<CompiledMatcher>> {
        if let Some(cached) = self.regex_cache.borrow().get(pattern) {
            return cached.clone();
        }
        let compiled = compile(self.pattern_options, pattern).map(Arc::new);
        self.regex_cache
            .borrow_mut()
            .insert(Arc::clone(pattern), compiled.clone());
        compiled
    }

    /// The intersection of these two, from an earlier run of the same pair.
    pub(crate) fn recall_intersection(&self, left: &Schema, right: &Schema) -> Option<Schema> {
        let key = intersection_key(left.clone(), right.clone());
        self.intersections.borrow().get(&key).cloned()
    }

    pub(crate) fn remember_intersection(&self, left: Schema, right: Schema, result: &Schema) {
        let mut intersections = self.intersections.borrow_mut();
        if intersections.len() < INTERSECTION_CACHE_CAPACITY {
            intersections.insert(intersection_key(left, right), result.clone());
        }
    }
}

fn intersection_key(left: Schema, right: Schema) -> (Schema, Schema) {
    match left.cached_hash().cmp(&right.cached_hash()) {
        Ordering::Greater => (right, left),
        Ordering::Equal if left > right => (right, left),
        Ordering::Less | Ordering::Equal => (left, right),
    }
}

fn compile(options: PatternEngineOptions, pattern: &str) -> Option<CompiledMatcher> {
    let translated = jsonschema_regex::to_rust_regex(pattern).ok()?;
    match options {
        PatternEngineOptions::Regex {
            size_limit,
            dfa_size_limit,
        } => crate::regex::build_standard_regex(&translated, size_limit, dfa_size_limit)
            .ok()
            .map(CompiledMatcher::Regex),
        PatternEngineOptions::FancyRegex {
            backtrack_limit,
            size_limit,
            dfa_size_limit,
        } => crate::regex::build_fancy_regex(
            &translated,
            backtrack_limit,
            size_limit,
            dfa_size_limit,
        )
        .ok()
        .map(CompiledMatcher::FancyRegex),
    }
}
