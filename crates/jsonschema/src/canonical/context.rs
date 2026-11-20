//! Shared state for one canonicalization run.
use std::{
    borrow::Cow,
    cell::{Cell, OnceCell, RefCell},
    cmp::Ordering,
    collections::BTreeSet,
    mem,
    sync::{Arc, Mutex},
};

use ahash::{AHashMap, AHashSet};

use referencing::Draft;

use crate::{
    canonical::{
        emptiness::{self, Position},
        ir::{Schema, StringFormat},
        schema::DefinitionMap,
        ROOT_DEFINITION_KEY,
    },
    keywords::format::BuiltinFormat,
    options::{PatternEngineOptions, PatternSyntax},
    regex::{Compiled, RegexEngine},
};

/// Containment pairs the proofs of one run may assume at once. Each nested pair opens two more
/// bodies one level deeper on the stack, and a proof needing more settles nothing.
pub(crate) const ASSUMED_PAIRS: usize = 64;

/// Two definition keys, `(inner, outer)`: a target and one a containment proof holds it inside.
type TargetPair = (Arc<str>, Arc<str>);

/// Past this many remembered pairs a run keeps recomputing rather than grow without end.
const INTERSECTION_CACHE_CAPACITY: usize = 1 << 20;

/// Past this many remembered nodes a run walks them again rather than grow without end.
const FACET_CACHE_CAPACITY: usize = 1 << 16;

/// Patterns compiled under one document's engine, shared by every run over it.
pub(crate) type SharedRegexes = Arc<Mutex<AHashMap<Arc<str>, Option<Arc<CompiledMatcher>>>>>;

pub(crate) enum CompiledMatcher {
    Regex(Compiled<regex::Regex>),
    FancyRegex(Compiled<fancy_regex::Regex>),
}

impl CompiledMatcher {
    /// `None` where the engine cannot finish the match, e.g. on reaching the backtrack limit.
    fn is_match(&self, text: &str) -> Option<bool> {
        match self {
            Self::Regex(regex) => regex.is_match(text).ok(),
            Self::FancyRegex(regex) => regex.is_match(text).ok(),
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
    /// The compiled patterns of the document this run reads, which outlive it: every run over that
    /// document translates and compiles the same texts under the same engine.
    shared_regexes: Option<SharedRegexes>,
    /// An `allOf` over unions takes the product of their branches, which reaches the same pair
    /// of nodes many times.
    intersections: RefCell<AHashMap<(Schema, Schema), Remembered>>,
    /// Every containment query reads the facets of the node it asks about, and the object walks ask
    /// about the same handful of nodes once per piece they cut out.
    uncheckable_facets: RefCell<AHashMap<Schema, Arc<BTreeSet<Arc<str>>>>>,
    /// An intersection reached during this run that the canonical form cannot express exactly.
    /// Nodes built around it may already be wrong, so the whole run is discarded rather than the site.
    inexact_intersection: Cell<bool>,
    /// The targets an intersection may read through. Absent while a document is being canonicalized,
    /// where bodies are still arriving.
    definitions: Option<Arc<DefinitionMap>>,
    /// The targets that lie on a reference cycle, which no walk reads through: it would not
    /// terminate. Every other target of the same map is read through as usual.
    cyclic: BTreeSet<Arc<str>>,
    /// The targets a containment proof may assume one inside another. Found over `definitions` on
    /// the first pair a proof asks about.
    guarded: OnceCell<AHashSet<Arc<str>>>,
    /// The pairs the containment proofs in progress assume, outermost first.
    assumed: RefCell<Vec<TargetPair>>,
    /// The pairs a proof started with nothing assumed settled, and whether each held. Those outcomes
    /// rest on no assumption, so they hold for the whole run. A list: a run starts a handful of such
    /// proofs, and creating a hashed map costs every run that starts none.
    settled: RefCell<Vec<(TargetPair, bool)>>,
    /// The pairs a nested proof was started for under the assumptions now in force.
    attempted: RefCell<Vec<TargetPair>>,
    /// Intersections this run may still take before giving up. An `allOf` over unions multiplies them.
    intersections_left: Cell<u64>,
    /// Variants the conditional splits of this run may still produce. Nesting multiplies them.
    variants_left: Cell<u64>,
    /// Intersections this run has taken, which a run that gives up early leaves short of the budget.
    #[cfg(test)]
    intersections_taken: Cell<u64>,
    /// Set where a conditional split asked for more cases than were left.
    outgrew_cases: Cell<bool>,
    /// Address of the first subschema whose parse declined, for the pointer naming it. Compared
    /// against the document's nodes, never read through.
    declined_at: Cell<Option<usize>>,
}

/// Intersections one run may take before giving up and leaving the document `Raw`; a row of
/// `allOf`s over unions exceeds it.
const INTERSECTION_BUDGET: u64 = 1_000_000;

/// Variants the conditional splits of one run may produce before the document stays `Raw`.
const VARIANT_BUDGET: u64 = 4096;

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
            shared_regexes: None,
            intersections: RefCell::new(AHashMap::new()),
            uncheckable_facets: RefCell::new(AHashMap::new()),
            inexact_intersection: Cell::new(false),
            intersections_left: Cell::new(INTERSECTION_BUDGET),
            variants_left: Cell::new(VARIANT_BUDGET),
            #[cfg(test)]
            intersections_taken: Cell::new(0),
            outgrew_cases: Cell::new(false),
            declined_at: Cell::new(None),
            definitions: None,
            cyclic: BTreeSet::new(),
            guarded: OnceCell::new(),
            assumed: RefCell::new(Vec::new()),
            settled: RefCell::new(Vec::new()),
            attempted: RefCell::new(Vec::new()),
        }
    }

    /// The same context, keeping what it compiles for the next run over the same document.
    pub(crate) fn sharing_regexes(mut self, regexes: SharedRegexes) -> Self {
        self.shared_regexes = Some(regexes);
        self
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

    /// The targets this run reads through.
    pub(crate) fn targets(&self) -> &DefinitionMap {
        self.definitions
            .as_deref()
            .expect("a settling run reads targets")
    }

    /// The targets, for a settling pass to move on. What the run already remembers stays: a pass
    /// settles a body only after every body it reads met those bodies in their final form.
    pub(crate) fn targets_mut(&mut self) -> &mut DefinitionMap {
        let definitions = self
            .definitions
            .as_mut()
            .expect("a settling run reads targets");
        // Held here alone, so the edit lands in place instead of copying the map per body.
        debug_assert_eq!(
            Arc::strong_count(definitions),
            1,
            "a settling run holds its targets alone"
        );
        // Found over the bodies, so an edited map is asked again.
        self.guarded.take();
        self.settled.get_mut().clear();
        Arc::make_mut(definitions)
    }

    pub(crate) fn into_targets(self) -> DefinitionMap {
        Arc::unwrap_or_clone(self.definitions.expect("a settling run reads targets"))
    }

    pub(crate) fn pattern_options(&self) -> PatternEngineOptions {
        self.pattern_options
    }

    /// Whether `text` satisfies `format`, or `None` when the format is not one of the built-ins.
    pub(crate) fn format_accepts(&self, format: &StringFormat, text: &str) -> Option<bool> {
        if self.pattern_options.syntax() == PatternSyntax::Engine
            && matches!(format, StringFormat::Builtin(BuiltinFormat::Regex))
        {
            return Some(crate::keywords::format::engine_accepts_regex(
                self.pattern_options,
                text,
            ));
        }
        format.is_valid(text)
    }

    /// The targets a set operation reads through; absent while a document is being canonicalized.
    pub(crate) fn definitions(&self) -> Option<&DefinitionMap> {
        self.definitions.as_deref()
    }

    /// The body `uri` names, or `None` where this run reads no targets and where reading through
    /// this one would not terminate.
    pub(crate) fn definition(&self, uri: &str) -> Option<&Schema> {
        if self.cyclic.contains(uri) {
            return None;
        }
        self.definitions.as_ref()?.get(uri)
    }

    /// The body `uri` names where a containment proof may assume it inside another target, cycle
    /// or not: a proof opens it once per assumption and reads the pointer inside as assumed.
    pub(crate) fn guarded_target(&self, uri: &str) -> Option<&Schema> {
        if !self.guards(uri) {
            return None;
        }
        self.definitions.as_ref()?.get(uri)
    }

    /// Runs `proof` with target `inner` assumed inside `outer`; the assumption is gone when
    /// it returns. `None` where either target is not guarded, where an assumption in force already
    /// holds `inner` inside or `outer` around something, or past [`ASSUMED_PAIRS`].
    ///
    /// The intersections taken meanwhile rest on the assumption, so they are remembered only while
    /// it holds.
    pub(crate) fn assuming_contained<T>(
        &self,
        inner: &Arc<str>,
        outer: &Arc<str>,
        proof: impl FnOnce() -> T,
    ) -> Option<T> {
        // Sound by induction on the value. Every cycle through a guarded target passes a property
        // value, an array element or a property name, so a proof reaches an assumed pair again on a
        // proper part of the value it started from, or in place on a pair of targets these reach
        // in place - never on these two themselves. A cycle closing in place would reach the pair on
        // the same value, where the assumption would only justify itself.
        if !self.guards(inner) || !self.guards(outer) {
            return None;
        }
        let depth = {
            let assumed = self.assumed.borrow();
            // Each target enters each side once, so nesting stays within the targets a parse
            // already walked; otherwise every pair of targets a union reaches nests another level.
            if assumed.len() >= ASSUMED_PAIRS
                || assumed.iter().any(|(assumed_inner, assumed_outer)| {
                    assumed_inner == inner || assumed_outer == outer
                })
            {
                return None;
            }
            assumed.len()
        };
        self.assumed
            .borrow_mut()
            .push((Arc::clone(inner), Arc::clone(outer)));
        let _held = Held {
            ctx: self,
            depth,
            outside: self.intersections.take(),
            attempted: self.attempted.take(),
        };
        Some(proof())
    }

    /// Whether a proof of `inner` inside `outer` started with nothing assumed held, where this run
    /// has one. Asked with assumptions in force there is none: an outcome under them is theirs.
    pub(crate) fn settled_containment(&self, inner: &Arc<str>, outer: &Arc<str>) -> Option<bool> {
        if self.assumes_any() {
            return None;
        }
        self.settled
            .borrow()
            .iter()
            .find(|((settled_inner, settled_outer), _)| {
                settled_inner == inner && settled_outer == outer
            })
            .map(|(_, held)| *held)
    }

    /// Record how a proof of `inner` inside `outer` ended, kept only where it started with nothing
    /// assumed.
    pub(crate) fn settle_containment(&self, inner: &Arc<str>, outer: &Arc<str>, held: bool) {
        if self.assumes_any() {
            return;
        }
        debug_assert!(
            self.settled_containment(inner, outer).is_none(),
            "a settled pair is read back, not proven again"
        );
        self.settled
            .borrow_mut()
            .push(((Arc::clone(inner), Arc::clone(outer)), held));
    }

    /// Note that a nested proof of `inner` inside `outer` starts under the assumptions in force.
    pub(crate) fn note_attempt(&self, inner: &Arc<str>, outer: &Arc<str>) {
        debug_assert!(self.assumes_any(), "nested proofs start inside a proof");
        if !self.attempted(inner, outer) {
            self.attempted
                .borrow_mut()
                .push((Arc::clone(inner), Arc::clone(outer)));
        }
    }

    /// Whether a nested proof of `inner` inside `outer` already ran under the assumptions in force.
    pub(crate) fn attempted(&self, inner: &Arc<str>, outer: &Arc<str>) -> bool {
        self.assumes_any()
            && self
                .attempted
                .borrow()
                .iter()
                .any(|(attempted_inner, attempted_outer)| {
                    attempted_inner == inner && attempted_outer == outer
                })
    }

    /// Whether a containment proof in progress assumes `inner` inside `outer`.
    pub(crate) fn assumes_contained(&self, inner: &str, outer: &str) -> bool {
        self.assumed
            .borrow()
            .iter()
            .any(|(assumed_inner, assumed_outer)| {
                debug_assert!(self.guards(assumed_inner), "an assumed target is guarded");
                debug_assert!(self.guards(assumed_outer), "an assumed target is guarded");
                **assumed_inner == *inner && **assumed_outer == *outer
            })
    }

    /// Whether a containment proof is in progress.
    pub(crate) fn assumes_any(&self) -> bool {
        !self.assumed.borrow().is_empty()
    }

    fn guards(&self, uri: &str) -> bool {
        let Some(definitions) = self.definitions.as_deref() else {
            return false;
        };
        self.guarded
            .get_or_init(|| guarded_targets(definitions))
            .contains(uri)
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

    /// Whether `matcher` matches `text`. A match the engine cannot finish may go either way, so the
    /// run is discarded as an approximated one is, and the `false` returned decides nothing.
    pub(crate) fn matches(&self, matcher: &CompiledMatcher, text: &str) -> bool {
        matcher.is_match(text).unwrap_or_else(|| {
            self.record_inexact_intersection();
            false
        })
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
        #[cfg(test)]
        self.intersections_taken
            .set(self.intersections_taken.get() + 1);
        true
    }

    /// Whether a product of unions with `width` branches fits in what this run may still spend. One
    /// that does not would exhaust the allowance partway through, so it spends what is left now and
    /// the run gives up exactly as it would had it taken the product's intersections one by one.
    pub(crate) fn take_product(&self, width: u64) -> bool {
        if width <= self.intersections_left.get() {
            return true;
        }
        self.intersections_left.set(0);
        false
    }

    /// Take `count` conditional-split variants from what this run may still spend, reporting
    /// whether it had them.
    pub(crate) fn take_variants(&self, count: u64) -> bool {
        let left = self.variants_left.get();
        if left < count {
            self.note_outgrew_cases();
            return false;
        }
        self.variants_left.set(left - count);
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

    /// Run `work` without letting it change the run: the budget it spends is given back and the
    /// inexactness it reaches is forgotten. A recording parse must spend exactly what the plain
    /// parse spends. What it remembers stays: a pair the probe could only approximate had less
    /// budget than any later computation of the same pair, so the run reads the same answer.
    pub(crate) fn speculate<T>(&self, work: impl FnOnce() -> T) -> T {
        let budget = self.intersections_left.get();
        let (answer, _) = self.probe(work);
        self.intersections_left.set(budget);
        answer
    }

    #[cfg(test)]
    pub(crate) fn intersections_taken(&self) -> u64 {
        self.intersections_taken.get()
    }

    pub(crate) fn outgrew_distribution(&self) -> bool {
        self.intersections_left.get() == 0
    }

    pub(crate) fn note_outgrew_cases(&self) {
        self.outgrew_cases.set(true);
    }

    pub(crate) fn outgrew_cases(&self) -> bool {
        self.outgrew_cases.get()
    }

    /// Remember the subschema whose parse declined, unless one is already remembered: the walk is
    /// depth first and every caller passes a decline on, so the first is the one that caused it.
    pub(crate) fn note_declined(&self, address: usize) {
        if self.declined_at.get().is_none() {
            self.declined_at.set(Some(address));
        }
    }

    /// Forget the decline of an earlier parse attempt, whose nodes this one re-reads.
    pub(crate) fn forget_decline(&self) {
        self.declined_at.set(None);
    }

    pub(crate) fn declined_at(&self) -> Option<usize> {
        self.declined_at.get()
    }

    /// Parse a schema this run wrote itself, keeping what it declines on off the record: those
    /// nodes are not the document's, and the node standing in for them is the one being rewritten.
    pub(crate) fn over_rewritten<T>(&self, parse: impl FnOnce() -> T) -> T {
        let before = self.declined_at.replace(None);
        let parsed = parse();
        self.declined_at.set(before);
        parsed
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
        // Reached once per pattern per run, so the shared map is locked that often rather than
        // once per use. The document fixes the engine, so its text alone names the matcher.
        let shared = self.shared_regexes.as_ref();
        let held = shared.and_then(|shared| {
            let shared = shared.lock().ok()?;
            shared.get(pattern).cloned()
        });
        let compiled = if let Some(compiled) = held {
            compiled
        } else {
            // Compiled outside the lock: holding it across a translation would serialize every
            // other run over the document, and a panic there would poison the cache.
            let compiled = compile(self.pattern_options, pattern).map(Arc::new);
            if let Some(mut shared) = shared.and_then(|shared| shared.lock().ok()) {
                shared.insert(Arc::clone(pattern), compiled.clone());
            }
            compiled
        };
        self.regex_cache
            .borrow_mut()
            .insert(Arc::clone(pattern), compiled.clone());
        compiled
    }

    /// The facets of `schema` no checker covers, walking the node the first time it is asked about.
    pub(crate) fn uncheckable_facets(
        &self,
        schema: &Schema,
        walk: impl FnOnce() -> BTreeSet<Arc<str>>,
    ) -> Arc<BTreeSet<Arc<str>>> {
        if let Some(cached) = self.uncheckable_facets.borrow().get(schema) {
            return Arc::clone(cached);
        }
        let found = Arc::new(walk());
        let mut cache = self.uncheckable_facets.borrow_mut();
        if cache.len() < FACET_CACHE_CAPACITY {
            cache.insert(schema.clone(), Arc::clone(&found));
        }
        found
    }

    /// The intersection of these two, from an earlier computation of the same pair. One the form
    /// could only approximate is recorded again here: a walk reading it is as approximate as the
    /// one that first took it.
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

/// An assumption in force. Dropping it releases the assumption and what was remembered under it, on
/// every way out of the proof.
struct Held<'a> {
    ctx: &'a CanonicalizationContext,
    /// Assumptions in force before this one.
    depth: usize,
    /// What the run remembered before this assumption, which holds without it.
    outside: AHashMap<(Schema, Schema), Remembered>,
    /// The nested proofs started before this assumption.
    attempted: Vec<TargetPair>,
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        let mut assumed = self.ctx.assumed.borrow_mut();
        debug_assert_eq!(
            assumed.len(),
            self.depth + 1,
            "an assumption is released after every one made inside its proof"
        );
        assumed.truncate(self.depth);
        *self.ctx.intersections.borrow_mut() = mem::take(&mut self.outside);
        *self.ctx.attempted.borrow_mut() = mem::take(&mut self.attempted);
    }
}

/// The targets of `definitions` every cycle through which passes a consuming position.
///
/// `#` names a root the map does not hold, so a cycle through it is invisible here. The root is
/// taken to name every target in place, which leaves out any target reaching `#` in place.
fn guarded_targets(definitions: &DefinitionMap) -> AHashSet<Arc<str>> {
    let mut edges = emptiness::reference_edges(&Schema::truthy(), definitions);
    edges
        .get_mut(ROOT_DEFINITION_KEY)
        .expect("the graph holds the root")
        .extend(
            definitions
                .keys()
                .map(|uri| (Arc::clone(uri), Position::InPlace)),
        );
    let mut guarded = emptiness::guarded_members(&edges);
    guarded.remove(ROOT_DEFINITION_KEY);
    guarded
}

fn intersection_key(left: Schema, right: Schema) -> (Schema, Schema) {
    match left.cached_hash().cmp(&right.cached_hash()) {
        Ordering::Greater => (right, left),
        Ordering::Equal if left > right => (right, left),
        Ordering::Less | Ordering::Equal => (left, right),
    }
}

fn compile(options: PatternEngineOptions, pattern: &str) -> Option<CompiledMatcher> {
    let syntax = options.syntax();
    let translated = match syntax {
        PatternSyntax::Ecma262 => jsonschema_regex::to_rust_regex(pattern).ok()?,
        PatternSyntax::Engine => Cow::Borrowed(pattern),
    };
    match options {
        PatternEngineOptions::Regex {
            size_limit,
            dfa_size_limit,
            ..
        } => crate::regex::build_standard_regex(
            pattern,
            &translated,
            size_limit,
            dfa_size_limit,
            syntax,
        )
        .ok()
        .map(CompiledMatcher::Regex),
        PatternEngineOptions::FancyRegex {
            backtrack_limit,
            size_limit,
            dfa_size_limit,
            ..
        } => crate::regex::build_fancy_regex(
            pattern,
            &translated,
            backtrack_limit,
            size_limit,
            dfa_size_limit,
            syntax,
        )
        .ok()
        .map(CompiledMatcher::FancyRegex),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use test_case::test_case;

    use super::*;
    use crate::canonical::parse;

    fn guarded(schema: &Value) -> Vec<String> {
        let registry = referencing::Registry::new()
            .add(
                "json-schema:///",
                Draft::Draft202012.create_resource_ref(schema),
            )
            .expect("the fixture is a resource")
            .draft(Draft::Draft202012)
            .prepare()
            .expect("the registry prepares");
        let resolver = registry
            .resolver(referencing::uri::from_str("json-schema:///").expect("a valid base URI"));
        let ctx = CanonicalizationContext::new(
            Draft::Draft202012,
            PatternEngineOptions::default(),
            false,
        );
        let parsed = parse::parse(schema, &ctx, &resolver, None)
            .expect("the fixture parses")
            .0
            .expect("the fixture is modeled");
        let mut names: Vec<String> = guarded_targets(&parsed.definitions)
            .iter()
            .map(ToString::to_string)
            .collect();
        names.sort();
        names
    }

    #[test_case(
        &json!({"$ref": "#/$defs/t", "$defs": {"t": {"type": "object", "properties": {
            "next": {"$ref": "#/$defs/t"},
            "root": {"$ref": "#"}
        }}}}),
        &["#/$defs/t"];
        "the root named under a property"
    )]
    #[test_case(
        &json!({"$ref": "#/$defs/t", "$defs": {"t": {"anyOf": [
            {"$ref": "#"},
            {"type": "object", "properties": {"next": {"$ref": "#/$defs/t"}}}
        ]}}}),
        &[];
        "the root named in place"
    )]
    fn a_target_naming_the_root_in_place_is_not_guarded(schema: &Value, expected: &[&str]) {
        assert_eq!(guarded(schema), expected);
    }
}
