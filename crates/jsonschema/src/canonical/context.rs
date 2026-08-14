//! Shared state for one canonicalization run: draft, pattern engine, and a compiled-regex cache.
use std::{
    cell::{Cell, RefCell},
    cmp::Ordering,
    collections::BTreeSet,
    sync::Arc,
};

use ahash::AHashMap;

use referencing::Draft;

use crate::{
    canonical::{ir::Schema, schema::DefinitionMap},
    options::PatternEngineOptions,
};

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
    intersections: RefCell<AHashMap<(Schema, Schema), Remembered>>,
    /// An intersection reached during this run that the canonical form cannot express exactly.
    /// Nodes built around it may already be wrong, so the whole run is discarded rather than the site.
    inexact_intersection: Cell<bool>,
    /// The targets an intersection may read through. Absent while a document is being canonicalized,
    /// where bodies are still arriving.
    definitions: Option<Arc<DefinitionMap>>,
    /// The targets that lie on a reference cycle, which no walk reads through: it would not
    /// terminate. Every other target of the same map is read through as usual.
    cyclic: BTreeSet<Arc<str>>,
    /// Intersections this run may still take before giving up. A conjunction over unions multiplies them.
    intersections_left: Cell<u64>,
}

/// Intersections one run may take before giving up and leaving the document `Raw`. Above what the
/// most demanding document written in earnest needs; a row of conjunctions over unions passes it in
/// a fraction of a second.
const INTERSECTION_BUDGET: u64 = 1_000_000;

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
            inexact_intersection: Cell::new(false),
            intersections_left: Cell::new(INTERSECTION_BUDGET),
            definitions: None,
            cyclic: BTreeSet::new(),
        }
    }

    /// The same context, reading intersections through `definitions`. The caller passes a map only
    /// when it is complete, and names the targets on a cycle, which stay unread.
    pub(crate) fn resolving(
        mut self,
        definitions: Arc<DefinitionMap>,
        cyclic: BTreeSet<Arc<str>>,
    ) -> Self {
        self.definitions = Some(definitions);
        self.cyclic = cyclic;
        self
    }

    /// The same context, allowed `budget` intersections rather than a whole document's worth.
    pub(crate) fn within(mut self, budget: u64) -> Self {
        self.intersections_left = Cell::new(budget);
        self
    }

    /// Point this run at a map a settling pass has moved on. What it already remembers stays: a
    /// pass settling a body only after every body it reads met those bodies in their final form.
    pub(crate) fn read_targets(&mut self, definitions: Arc<DefinitionMap>) {
        self.definitions = Some(definitions);
    }

    pub(crate) fn pattern_options(&self) -> PatternEngineOptions {
        self.pattern_options
    }

    /// The body `uri` names, or `None` where this run reads no targets and where reading through
    /// this one would not terminate.
    pub(crate) fn definition(&self, uri: &str) -> Option<&Schema> {
        if self.cyclic.contains(uri) {
            return None;
        }
        self.definitions.as_ref()?.get(uri)
    }

    pub(crate) fn draft(&self) -> Draft {
        self.draft
    }

    pub(crate) fn record_inexact_intersection(&self) {
        self.inexact_intersection.set(true);
    }

    pub(crate) fn saw_inexact_intersection(&self) -> bool {
        self.inexact_intersection.get()
    }

    /// Run `probe`, reporting whether it reached an intersection the canonical form cannot express
    /// exactly. The flag is left as it was, so what a probe reaches decides nothing beyond its own
    /// answer. The intersection budget is not restored: it bounds the work one run may do, and
    /// speculative work is work.
    pub(crate) fn probe<T>(&self, probe: impl FnOnce() -> T) -> (T, bool) {
        let before = self.inexact_intersection.replace(false);
        let answer = probe();
        let inexact = self.inexact_intersection.replace(before);
        (answer, inexact)
    }

    /// Take one intersection from what this run may still spend, reporting whether it had any. Once
    /// it runs out every later intersection is refused too and the walk unwinds.
    pub(crate) fn take_intersection(&self) -> bool {
        let left = self.intersections_left.get();
        if left == 0 {
            return false;
        }
        self.intersections_left.set(left - 1);
        true
    }

    /// Run `work` against at most `cap` intersections, reporting whether it ran out. What it spends
    /// counts against the run's budget, so a bounded side question cannot exhaust the main answer.
    pub(crate) fn capped<T>(&self, cap: u64, work: impl FnOnce() -> T) -> (T, bool) {
        let before = self.intersections_left.get();
        let allowance = before.min(cap);
        self.intersections_left.set(allowance);
        let answer = work();
        let left = self.intersections_left.get();
        self.intersections_left.set(before - (allowance - left));
        (answer, left == 0)
    }

    pub(crate) fn outgrew_distribution(&self) -> bool {
        self.intersections_left.get() == 0
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

    /// The intersection of these two, from an earlier run of the same pair. One the form could only
    /// approximate is recorded again here: a walk reading it is as approximate as the one that
    /// first took it.
    pub(crate) fn recall_intersection(&self, left: &Schema, right: &Schema) -> Option<Schema> {
        let key = intersection_key(left.clone(), right.clone());
        let remembered = self.intersections.borrow().get(&key).cloned()?;
        if remembered.inexact {
            self.record_inexact_intersection();
        }
        Some(remembered.result)
    }

    /// Remember this pair's intersection, and whether taking it approximated.
    pub(crate) fn remember_intersection(
        &self,
        left: Schema,
        right: Schema,
        result: &Schema,
        inexact: bool,
    ) {
        let mut intersections = self.intersections.borrow_mut();
        if intersections.len() < INTERSECTION_CACHE_CAPACITY {
            intersections.insert(
                intersection_key(left, right),
                Remembered {
                    result: result.clone(),
                    inexact,
                },
            );
        }
    }
}

/// One pair's intersection, beside whether the form could only approximate it.
#[derive(Clone)]
struct Remembered {
    result: Schema,
    inexact: bool,
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
