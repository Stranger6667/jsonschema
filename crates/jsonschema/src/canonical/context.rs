use std::{
    cell::{Cell, RefCell},
    sync::Arc,
};

use ahash::AHashMap;
use referencing::Draft;
use serde_json::{Number, Value};

use crate::{
    canonical::ir::{CanonicalJson, SchemaNode, SharedSchema},
    compiler::formats_are_assertions_by_default,
    options::PatternEngineOptions,
    JsonType,
};

pub(crate) enum CompiledMatcher {
    Regex(regex::Regex),
    FancyRegex(fancy_regex::Regex),
}

impl CompiledMatcher {
    /// A regex engine error (e.g. `fancy_regex` hitting its backtrack limit) counts as "no match".
    pub(crate) fn is_match(&self, text: &str) -> bool {
        match self {
            Self::Regex(r) => r.is_match(text),
            Self::FancyRegex(r) => r.is_match(text).unwrap_or(false),
        }
    }
}

/// Which canonicalization pass a `walk_memo` entry came from. Part of the memo key so passes visiting the same node
/// don't read each other's cached results.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum WalkStage {
    Numeric,
    Array,
    Object,
    Leaves,
    Collapse,
    ConstEnum,
    Intern,
    OneOf,
    IfThenElse,
    NotElim,
    TypePartition,
    Negate,
    NegateWithoutContains,
}

// Both memos key on `*const SchemaNode` for cheap identity hashing; each entry holds its input `Arc`(s) so the
// address can't be reused for an unrelated schema while the entry lives.

type WalkMemoKey = (WalkStage, *const SchemaNode);
type WalkMemoEntry = (SharedSchema, SharedSchema);
type IntersectMemoKey = (*const SchemaNode, *const SchemaNode);
type IntersectMemoEntry = (SharedSchema, SharedSchema, SharedSchema);

/// Cap on partition steps the union-coverage search may take per outermost `covers` query: enough for its target
/// shapes, small enough that re-entrant partitioning through `intersect_canonical` stays cheap.
const PARTITION_FUEL: u32 = 64;

pub(crate) struct CanonicalizationContext {
    draft: Draft,
    pattern_options: PatternEngineOptions,
    validate_formats: bool,
    /// `None` caches a compile failure so callsites don't retry.
    regex_cache: RefCell<AHashMap<Arc<str>, Option<Arc<CompiledMatcher>>>>,
    /// Memo for [`pattern_is_extended_with`] under this context's options; the coverage oracle
    /// probes the same pattern repeatedly inside the canonicalization fixpoint loop.
    extended_pattern_cache: RefCell<AHashMap<Arc<str>, bool>>,
    /// Per-stage walk memo. `one_of::linear_encode` reuses one `Arc` across many positions; without this each
    /// occurrence re-walks independently.
    walk_memo: RefCell<AHashMap<WalkMemoKey, WalkMemoEntry>>,
    /// Memo for `intersect_canonical` over `linear_encode`'s shared-`Arc` graphs. Keys are pointer-sorted so `(A, B)`
    /// and `(B, A)` share an entry.
    intersect_memo: RefCell<AHashMap<IntersectMemoKey, IntersectMemoEntry>>,
    canonical_value_cache: RefCell<AHashMap<Arc<str>, Arc<Value>>>,
    /// Memo for `covers`; the union-coverage partition search probes the same `(big, small)` pairs repeatedly.
    covers_memo: RefCell<AHashMap<(SharedSchema, SharedSchema), bool>>,
    /// Budget for union-coverage partition steps, bounding the `covers` <-> `intersect_canonical` recursion. Refilled
    /// per outermost `covers` query, never shared: else a result would depend on fuel earlier queries spent, breaking
    /// idempotence.
    partition_fuel: Cell<u32>,
    /// Depth of nested `covers` queries; the 0 -> 1 transition refills `partition_fuel`.
    covers_query_depth: Cell<u32>,
    /// Dedup table: structurally-equal schemas share one `Arc`, so pointer equality means structural equality. Keyed
    /// by structural hash, same-hash collisions resolved by deep equality. See [`Self::intern`].
    intern_table: RefCell<AHashMap<u64, Vec<SharedSchema>>>,
}

impl Default for CanonicalizationContext {
    fn default() -> Self {
        Self::with_pattern_options(PatternEngineOptions::default())
    }
}

impl CanonicalizationContext {
    #[must_use]
    pub(crate) fn with_draft(mut self, draft: Draft) -> Self {
        self.draft = draft;
        self.validate_formats = formats_are_assertions_by_default(draft);
        self
    }

    #[must_use]
    pub(crate) fn with_format_assertions(mut self, yes: bool) -> Self {
        self.validate_formats = yes;
        self
    }

    pub(crate) fn validates_formats(&self) -> bool {
        self.validate_formats
    }

    pub(crate) fn draft(&self) -> Draft {
        self.draft
    }

    pub(crate) fn pattern_options(&self) -> PatternEngineOptions {
        self.pattern_options
    }

    #[must_use]
    pub(crate) fn with_pattern_options(pattern_options: PatternEngineOptions) -> Self {
        Self {
            draft: Draft::default(),
            pattern_options,
            validate_formats: formats_are_assertions_by_default(Draft::default()),
            regex_cache: RefCell::new(AHashMap::new()),
            extended_pattern_cache: RefCell::new(AHashMap::new()),
            walk_memo: RefCell::new(AHashMap::new()),
            intersect_memo: RefCell::new(AHashMap::new()),
            canonical_value_cache: RefCell::new(AHashMap::new()),
            covers_memo: RefCell::new(AHashMap::new()),
            partition_fuel: Cell::new(PARTITION_FUEL),
            covers_query_depth: Cell::new(0),
            intern_table: RefCell::new(AHashMap::new()),
        }
    }

    /// `covers` result memo. Cleared implicitly by the context's lifetime (one per canonicalize / `is_subschema_of`).
    pub(crate) fn covers_memo(&self) -> &RefCell<AHashMap<(SharedSchema, SharedSchema), bool>> {
        &self.covers_memo
    }

    /// Re-entrancy guard for the `covers` union-coverage partition (see field docs). Takes one unit of partition
    /// budget; `false` when exhausted (the caller stops partitioning).
    pub(crate) fn consume_partition_fuel(&self) -> bool {
        let remaining = self.partition_fuel.get();
        if remaining == 0 {
            return false;
        }
        self.partition_fuel.set(remaining - 1);
        true
    }

    pub(crate) fn partition_fuel_remaining(&self) -> u32 {
        self.partition_fuel.get()
    }

    /// Mark a `covers` query entry; refills the partition budget on the outermost query so nested queries (via
    /// `intersect_canonical` on partition remainders) share it.
    pub(crate) fn enter_covers_query(&self) {
        let depth = self.covers_query_depth.get();
        if depth == 0 {
            self.partition_fuel.set(PARTITION_FUEL);
        }
        self.covers_query_depth.set(depth + 1);
    }

    pub(crate) fn exit_covers_query(&self) {
        self.covers_query_depth
            .set(self.covers_query_depth.get() - 1);
    }

    /// The canonical `Arc` for this node's structure; inserts on first sight. Children must already be interned for
    /// pointer equality to mean structural equality tree-wide.
    pub(crate) fn intern(&self, node: SharedSchema) -> SharedSchema {
        let mut table = self.intern_table.borrow_mut();
        let bucket = table.entry(node.hash).or_default();
        if let Some(existing) = bucket
            .iter()
            .find(|existing| Arc::ptr_eq(existing, &node) || ***existing == *node)
        {
            return Arc::clone(existing);
        }
        bucket.push(Arc::clone(&node));
        node
    }

    /// Draft-aware integer check: Draft 4 rejects floats without fractional parts (e.g. `1.0`); later drafts accept.
    pub(crate) fn is_integer(&self, number: &Number) -> bool {
        if matches!(self.draft, Draft::Draft4) {
            crate::keywords::legacy::type_draft_4::is_integer(number)
        } else {
            crate::keywords::type_::is_integer(number)
        }
    }

    /// Memoized [`pattern_is_extended_with`] under this context's options.
    pub(crate) fn pattern_is_extended(&self, pattern: &Arc<str>) -> bool {
        if let Some(&extended) = self.extended_pattern_cache.borrow().get(pattern) {
            return extended;
        }
        let extended = pattern_is_extended_with(self.pattern_options, pattern);
        self.extended_pattern_cache
            .borrow_mut()
            .insert(Arc::clone(pattern), extended);
        extended
    }

    /// `None` means the engine rejected the pattern.
    pub(crate) fn compile_regex(&self, pattern: &Arc<str>) -> Option<Arc<CompiledMatcher>> {
        if let Some(cached) = self.regex_cache.borrow().get(pattern) {
            return cached.clone();
        }
        let compiled = compile_with(self.pattern_options, pattern).map(Arc::new);
        self.regex_cache
            .borrow_mut()
            .insert(Arc::clone(pattern), compiled.clone());
        compiled
    }

    /// Return `stage`'s cached output for `input`, else compute via `body` and cache.
    pub(crate) fn with_walk_memo<F>(
        &self,
        stage: WalkStage,
        input: &SharedSchema,
        body: F,
    ) -> SharedSchema
    where
        F: FnOnce() -> SharedSchema,
    {
        let key = Arc::as_ptr(input);
        if let Some(cached) = self.walk_memo.borrow().get(&(stage, key)) {
            return Arc::clone(&cached.1);
        }
        let result = body();
        self.walk_memo
            .borrow_mut()
            .insert((stage, key), (Arc::clone(input), Arc::clone(&result)));
        result
    }

    /// Return the cached intersection of `left` and `right`, else compute via `body` and cache. Order-independent.
    pub(crate) fn with_intersect_memo<F>(
        &self,
        left: &SharedSchema,
        right: &SharedSchema,
        body: F,
    ) -> SharedSchema
    where
        F: FnOnce() -> SharedSchema,
    {
        let left_ptr = Arc::as_ptr(left);
        let right_ptr = Arc::as_ptr(right);
        let (key, key_left, key_right) = if left_ptr <= right_ptr {
            ((left_ptr, right_ptr), left, right)
        } else {
            ((right_ptr, left_ptr), right, left)
        };
        if let Some(cached) = self.intersect_memo.borrow().get(&key) {
            return Arc::clone(&cached.2);
        }
        let result = body();
        self.intersect_memo.borrow_mut().insert(
            key,
            (
                Arc::clone(key_left),
                Arc::clone(key_right),
                Arc::clone(&result),
            ),
        );
        result
    }

    pub(crate) fn parse_canonical(&self, canonical: &CanonicalJson) -> Arc<Value> {
        let key = canonical.as_arc();
        if let Some(cached) = self.canonical_value_cache.borrow().get(key) {
            return Arc::clone(cached);
        }
        let parsed = serde_json::from_str::<Value>(canonical.as_str())
            .expect("CanonicalJson holds valid JSON");
        let parsed = Arc::new(parsed);
        self.canonical_value_cache
            .borrow_mut()
            .insert(Arc::clone(key), Arc::clone(&parsed));
        parsed
    }
}

/// Under Draft 4 a fractionless float (e.g. `1.0`) satisfies `integer`, so collapsing an integer schema to a bare
/// `const`/value set would start rejecting it. Later drafts treat `1.0` as a number, making the guard unnecessary.
pub(crate) fn keeps_draft4_integer_guard(kind: JsonType, draft: Draft) -> bool {
    kind == JsonType::Integer && draft == Draft::Draft4
}

/// True for patterns the standard `regex` engine can't compile under `options`' limits (lookaround,
/// backreferences, size caps). Patterns that fail ECMA translation are flagged extended; the real
/// compile error surfaces later when the validator runs.
pub(crate) fn pattern_is_extended_with(options: PatternEngineOptions, pattern: &str) -> bool {
    match jsonschema_regex::to_rust_regex(pattern) {
        Ok(translated) => {
            let (PatternEngineOptions::Regex {
                size_limit,
                dfa_size_limit,
            }
            | PatternEngineOptions::FancyRegex {
                size_limit,
                dfa_size_limit,
                ..
            }) = options;
            crate::regex::build_standard_regex(&translated, size_limit, dfa_size_limit).is_err()
        }
        Err(()) => true,
    }
}

pub(crate) fn compile_with(
    options: PatternEngineOptions,
    pattern: &str,
) -> Option<CompiledMatcher> {
    let translated = jsonschema_regex::to_rust_regex(pattern).ok()?;
    let pattern = translated.as_ref();
    match options {
        PatternEngineOptions::Regex {
            size_limit,
            dfa_size_limit,
        } => crate::regex::build_standard_regex(pattern, size_limit, dfa_size_limit)
            .ok()
            .map(CompiledMatcher::Regex),
        PatternEngineOptions::FancyRegex {
            backtrack_limit,
            size_limit,
            dfa_size_limit,
        } => crate::regex::build_fancy_regex(pattern, backtrack_limit, size_limit, dfa_size_limit)
            .ok()
            .map(CompiledMatcher::FancyRegex),
    }
}
