//! Shared state for one canonicalization run: draft, pattern engine, and a compiled-regex cache.
use std::{cell::RefCell, collections::HashMap, sync::Arc};

use referencing::Draft;

use crate::options::PatternEngineOptions;

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
    /// `None` caches a rejected pattern so callers don't recompile it.
    regex_cache: RefCell<HashMap<Arc<str>, Option<Arc<CompiledMatcher>>>>,
}

impl CanonicalizationContext {
    pub(crate) fn new(draft: Draft, pattern_options: PatternEngineOptions) -> Self {
        Self {
            draft,
            pattern_options,
            regex_cache: RefCell::new(HashMap::new()),
        }
    }

    pub(crate) fn draft(&self) -> Draft {
        self.draft
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
