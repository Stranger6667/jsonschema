use std::{
    cell::Cell,
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
    sync::Arc,
};

use referencing::Draft;
use serde_json::{Number, Value};
use strum::{IntoStaticStr, VariantArray};

use crate::{
    canonical::{
        algebra,
        context::CanonicalizationContext,
        emit, emptiness,
        error::OperandMismatch,
        ir::{
            BoundCardinality, BoundInteger, BoundNumber, Distinctness, ObjectLeaf, Schema,
            SchemaKind, UncheckableFacet, Verdict,
        },
        negate, oracle, parse, rename, CanonicalizationError, ROOT_DEFINITION_KEY,
    },
    options::PatternEngineOptions,
    JsonType,
};

pub(crate) type DefinitionMap = BTreeMap<Arc<str>, Schema>;

/// Whether one schema admits every value another does, as [`CanonicalSchema::covers`] reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
pub enum Containment {
    /// Every value the argument admits, the receiver admits too.
    Yes,
    /// Some value the argument admits, the receiver rejects.
    No,
    /// Neither was established.
    Unknown,
}

impl Containment {
    /// Stable `snake_case` label of this answer (e.g. `"yes"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Whether any value satisfies a schema, as [`CanonicalSchema::satisfiability`] answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "snake_case")]
pub enum Satisfiability {
    /// Some value satisfies the schema.
    Yes,
    /// No value does.
    No,
    /// Neither was established.
    Unknown,
}

impl Satisfiability {
    /// Stable `snake_case` label of this answer (e.g. `"yes"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Every pointer `schema` reads, its own and those of the targets it reaches. `#` stands for the
/// document root, which is not a key of the map, so it appears only when a node refers to it.
fn pointers_read(schema: &Schema, definitions: &DefinitionMap) -> BTreeSet<Arc<str>> {
    // The root is not followed: whether `#` is read is the question the callers ask, and a walk
    // reading through it would answer that the same way for every node of a recursive document.
    let reached = emptiness::reachable_definition_keys(schema, None, definitions);
    let mut pointers: BTreeSet<Arc<str>> = reached.iter().map(Arc::clone).collect();
    // `#` is no entry of the map, and neither is a pointer naming no body, so the reachability
    // walk skips both; the nodes it reached are asked which references they contain.
    let bodies = reached
        .iter()
        .filter_map(|uri| definitions.get(uri.as_ref() as &str));
    for node in std::iter::once(schema).chain(bodies) {
        let mut referenced = Vec::new();
        emptiness::collect_classified_references(
            node,
            emptiness::Position::InPlace,
            &mut referenced,
        );
        pointers.extend(referenced.drain(..).map(|(uri, _)| Arc::clone(uri)));
    }
    pointers
}

/// How far a candidate walks into a node before giving up.
const CANDIDATE_DEPTH: u32 = 6;

/// The longest instance worth building. A bound past this is a bound no instance is built for: the
/// answer is left undecided rather than spending the memory a schema keyword asked for.
const CANDIDATE_LENGTH: u64 = 64;

/// The key a filler entry is given, where an object needs more of them than it requires.
const FILLER_KEY: &str = "a";

/// How many instances one walk may build. Depth and width bound a candidate on their own, but a
/// node wide at every level multiplies them - 64 keys six levels down is a number of values no
/// machine holds - so the walk also spends from one count and gives up where it runs out.
const CANDIDATE_NODES: u32 = 4_096;

/// Intersections the difference behind a coverage question may take. It is the last resort, after
/// the oracle and a value scan, and a caller comparing two wide documents wants an answer back.
const COVERS_DIFFERENCE_BUDGET: u64 = 20_000;

/// Names tried per seed before the next one. A leaf wanting more is answered by later seeds, or
/// left undecided.
const CANDIDATE_KEYS_PER_SEED: usize = 8;

/// The length a leaf demands, or `None` where no instance that long is worth building.
fn demanded_length(minimum: Option<&BoundCardinality>) -> Option<usize> {
    let Some(minimum) = minimum else {
        return Some(0);
    };
    minimum
        .to_usize()
        .filter(|length| *length as u64 <= CANDIDATE_LENGTH)
}

/// Instances worth trying against `node`, shortest first. A candidate proves nothing on its own -
/// `algebra::admits_value` decides - so the list only has to be short.
fn candidate_instances(
    node: &Schema,
    document: &Document,
    depth: u32,
    budget: &Cell<u32>,
    ctx: &CanonicalizationContext,
) -> Vec<Value> {
    if depth == 0 || budget.get() == 0 {
        return Vec::new();
    }
    budget.set(budget.get() - 1);
    match node.kind() {
        // A pointer accepts what its target does, so the instances worth trying are the target's.
        SchemaKind::Reference(uri) => document
            .target(uri)
            .map(|target| candidate_instances(target, document, depth - 1, budget, ctx))
            .unwrap_or_default(),
        SchemaKind::True => vec![Value::Null],
        SchemaKind::Const(value) => vec![value.as_value().clone()],
        SchemaKind::Enum(values) => values
            .as_slice()
            .iter()
            .map(|value| value.as_value().clone())
            .collect(),
        SchemaKind::MultiType(set) => set.iter().map(shortest_instance).collect(),
        SchemaKind::TypedGroup { body, .. } => {
            candidate_instances(body, document, depth - 1, budget, ctx)
        }
        SchemaKind::String(leaf) => {
            let Some(length) = demanded_length(leaf.get().lengths.minimum.as_ref()) else {
                return Vec::new();
            };
            vec![Value::String("a".repeat(length))]
        }
        SchemaKind::Integer(leaf) => {
            // An integer bound is stored as the first integer it admits.
            let bounds = &leaf.get().bounds;
            let end = |bound: &BoundInteger| WindowEnd {
                limit: bound.to_number(),
                admitted: true,
            };
            whole_number_candidates(
                bounds.minimum.as_ref().map(end),
                bounds.maximum.as_ref().map(end),
            )
        }
        SchemaKind::Number(leaf) => {
            let leaf = leaf.get();
            let end = |bound: &BoundNumber| WindowEnd {
                limit: bound.to_number(),
                admitted: bound.is_inclusive(),
            };
            whole_number_candidates(
                leaf.minimum.as_ref().map(end),
                leaf.maximum.as_ref().map(end),
            )
        }
        SchemaKind::Array(leaf) => {
            let leaf = leaf.get();
            let Some(length) = demanded_length(leaf.lengths.minimum.as_ref()) else {
                return Vec::new();
            };
            // A `contains` demand needs an element meeting it, which the floor alone never asks
            // for; each demand contributes one, and the check below turns down what does not hold.
            let demanded: Option<Vec<Value>> = leaf
                .contains
                .iter()
                .map(|facet| {
                    candidate_instances(&facet.schema, document, depth - 1, budget, ctx)
                        .into_iter()
                        .next()
                })
                .collect();
            let Some(demanded) = demanded else {
                return Vec::new();
            };
            let element = |index: usize| {
                let schema = leaf.prefix.get(index).or(leaf.items.as_ref());
                schema.map_or(Some(Value::Null), |schema| {
                    candidate_instances(schema, document, depth - 1, budget, ctx)
                        .into_iter()
                        .next()
                })
            };
            // A prefix schema governs the index it names, so demanded elements sit past the whole
            // prefix rather than at the head.
            let mut items = Vec::new();
            for index in 0..leaf.prefix.len() {
                let Some(value) = element(index) else {
                    return Vec::new();
                };
                items.push(value);
            }
            items.extend(demanded);
            // One candidate per index promises no distinctness; the check below turns that down.
            for index in items.len()..length {
                let Some(value) = element(index) else {
                    return Vec::new();
                };
                items.push(value);
            }
            vec![Value::Array(items)]
        }
        SchemaKind::Object(leaf) => {
            let leaf = leaf.get();
            let Some(size) = demanded_length(leaf.sizes.minimum.as_ref()) else {
                return Vec::new();
            };
            let mut object = serde_json::Map::new();
            for key in candidate_keys(leaf, size, ctx) {
                // A key answers to the entry declaring it, then a pattern entry it matches, then
                // the shield. Where several govern one key, the check below turns down a value that
                // satisfies only the one taken here.
                let governing = leaf
                    .properties
                    .get(key.as_str())
                    .or_else(|| {
                        leaf.pattern_properties
                            .iter()
                            .find(|(pattern, _)| algebra::matches_key(pattern, &key, ctx))
                            .map(|(_, schema)| schema)
                    })
                    .or(leaf.additional.as_ref());
                let Some(value) = governing.map_or(Some(Value::Null), |schema| {
                    candidate_instances(schema, document, depth - 1, budget, ctx)
                        .into_iter()
                        .next()
                }) else {
                    return Vec::new();
                };
                object.insert(key, value);
            }
            vec![Value::Object(object)]
        }
        // A branch's values are worth trying against the whole: `admits_value` decides.
        SchemaKind::AllOf(branches) | SchemaKind::AnyOf(branches) => branches
            .as_slice()
            .iter()
            .flat_map(|branch| candidate_instances(branch, document, depth - 1, budget, ctx))
            .collect(),
        SchemaKind::OneOf(branches) => branches
            .iter()
            .flat_map(|branch| candidate_instances(branch, document, depth - 1, budget, ctx))
            .collect(),
        SchemaKind::False | SchemaKind::Not(_) | SchemaKind::Raw(_) => Vec::new(),
    }
}

/// Keys to build a candidate object out of: the ones the leaf demands, then names its constraints
/// admit, up to the size floor.
///
/// A key constraint can turn down every name of the plain filler sequence, so each pattern seeds
/// names of its own. A name already demanded is never offered twice.
fn candidate_keys(leaf: &ObjectLeaf, size: usize, ctx: &CanonicalizationContext) -> Vec<String> {
    let mut keys: Vec<String> = leaf.required.iter().map(ToString::to_string).collect();
    let wanted = size.max(keys.len());
    if keys.len() >= wanted {
        return keys;
    }
    let admitted = |key: &str| {
        leaf.property_names.as_ref().is_none_or(|names| {
            algebra::admits_value(
                names,
                &Value::String(key.to_string()),
                UncheckableFacet::Undecided,
                ctx,
            ) == Verdict::Admits
        })
    };
    let seeds = leaf
        .properties
        .keys()
        .map(ToString::to_string)
        .chain(
            leaf.pattern_properties
                .keys()
                .map(|pattern| literal_prefix(pattern)),
        )
        .chain(std::iter::once(FILLER_KEY.to_string()));
    for seed in seeds {
        for index in 0..CANDIDATE_KEYS_PER_SEED {
            if keys.len() >= wanted {
                return keys;
            }
            let key = format!("{seed}{index}");
            if !keys.contains(&key) && admitted(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

/// The literal head of a key pattern, which every key an anchored one matches carries. Empty where
/// the pattern spells nothing a name can be built from.
fn literal_prefix(pattern: &str) -> String {
    pattern
        .strip_prefix('^')
        .unwrap_or(pattern)
        .chars()
        .take_while(|character| character.is_alphanumeric() || *character == '_')
        .collect()
}

/// Zero, which every divisor takes, and the ends of the window.
fn whole_number_candidates(minimum: Option<WindowEnd>, maximum: Option<WindowEnd>) -> Vec<Value> {
    // A point between the two ends, or one step in from the only end there is.
    let interior = interior_point(
        minimum.as_ref().map(|end| &end.limit),
        maximum.as_ref().map(|end| &end.limit),
    );
    // Admitted ends first: a caller taking one candidate needs one the window holds, and an
    // excluded end is no value of it.
    let mut candidates: Vec<Value> = [minimum, maximum]
        .into_iter()
        .flatten()
        .filter(|end| end.admitted)
        .map(|end| Value::Number(end.limit))
        .collect();
    candidates.extend(interior);
    // Zero, which every divisor takes.
    candidates.push(Value::Number(0.into()));
    candidates
}

/// One end of a numeric window, and whether the window admits it.
struct WindowEnd {
    limit: Number,
    admitted: bool,
}

/// A number strictly inside the window, where one can be spelled. A window narrower than the gap
/// between two neighbouring floats has none.
fn interior_point(minimum: Option<&Number>, maximum: Option<&Number>) -> Option<Value> {
    let point = match (
        minimum.and_then(Number::as_f64),
        maximum.and_then(Number::as_f64),
    ) {
        (Some(low), Some(high)) => low + (high - low) / 2.0,
        (Some(low), None) => low + 1.0,
        (None, Some(high)) => high - 1.0,
        (None, None) => return None,
    };
    Number::from_f64(point).map(Value::Number)
}

/// The shortest instance of a type.
fn shortest_instance(ty: JsonType) -> Value {
    match ty {
        JsonType::Null => Value::Null,
        JsonType::Boolean => Value::Bool(false),
        JsonType::String => Value::String(String::new()),
        JsonType::Integer | JsonType::Number => Value::Number(0.into()),
        JsonType::Array => Value::Array(Vec::new()),
        JsonType::Object => Value::Object(serde_json::Map::new()),
    }
}

/// `Yes` when the leaf's own size or value window is enough to pick a value out of, `Unknown` when
/// something else narrows which values are left. `Unknown` is not `No`: such a leaf usually does
/// hold values, but saying so needs a value nothing here found.
fn holds_a_value(window_is_enough: bool) -> Satisfiability {
    if window_is_enough {
        Satisfiability::Yes
    } else {
        Satisfiability::Unknown
    }
}

/// Whether `#` is reachable from `schema`, named there or in a target it reads. Such a node reads
/// the document root, so which document it belongs to decides what it accepts.
pub(crate) fn reads_document_root(schema: &Schema, definitions: &DefinitionMap) -> bool {
    pointers_read(schema, definitions).contains(ROOT_DEFINITION_KEY)
}

/// What a node can read of its document: the bodies it reaches, and the root when it names `#`.
/// Two nodes written the same way differ only where these differ; the rest of the document is
/// bookkeeping the node cannot observe.
fn document_slice(schema: &Schema, document: &Document) -> (Option<Schema>, DefinitionMap) {
    let mut pointers = pointers_read(schema, &document.definitions);
    // `#` names the root, and reading the root is reading everything the root reads.
    let root = pointers.contains(ROOT_DEFINITION_KEY).then(|| {
        pointers.extend(pointers_read(&document.root, &document.definitions));
        document.root.clone()
    });
    let targets = pointers
        .into_iter()
        .filter_map(|uri| {
            let body = document.definitions.get(uri.as_ref() as &str)?;
            Some((uri, body.clone()))
        })
        .collect();
    (root, targets)
}

/// What a handle's pointers name: `#` its root, every other pointer an entry of its map. The two
/// travel together, or a handle read on its own binds `#` to itself.
#[derive(Clone, Debug, Eq)]
struct Document {
    root: Schema,
    definitions: Arc<DefinitionMap>,
    /// The definition keys naming a body written inside this document. Renaming one apart when two
    /// documents merge keeps both meanings; renaming a retrieved resource's key would instead hide
    /// that the two documents read that resource differently.
    local: Arc<BTreeSet<Arc<str>>>,
}

impl Document {
    /// The schema `uri` names: `#` the root, every other pointer an entry of the map.
    fn target(&self, uri: &str) -> Option<&Schema> {
        if uri == ROOT_DEFINITION_KEY {
            return Some(&self.root);
        }
        self.definitions.get(uri)
    }
}

impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root
            && (Arc::ptr_eq(&self.definitions, &other.definitions)
                || self.definitions == other.definitions)
    }
}

impl Ord for Document {
    fn cmp(&self, other: &Self) -> Ordering {
        self.definitions
            .cmp(&other.definitions)
            .then_with(|| self.root.cmp(&other.root))
    }
}

impl PartialOrd for Document {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Canonical JSON Schema IR handle.
#[derive(Clone, Debug)]
pub struct CanonicalSchema {
    inner: Schema,
    draft: Draft,
    pattern_options: PatternEngineOptions,
    validate_formats: bool,
    /// Shared with every child handle.
    document: Document,
}

// Draft, format-assertion policy and pattern engine are part of a schema's identity, not just its
// IR: the operations refuse to combine operands that disagree on any of them.
impl PartialEq for CanonicalSchema {
    fn eq(&self, other: &Self) -> bool {
        self.draft == other.draft
            && self.validate_formats == other.validate_formats
            && self.pattern_options == other.pattern_options
            && self.inner == other.inner
            && self.reads_same_document_as(other)
    }
}

impl Eq for CanonicalSchema {}

impl PartialOrd for CanonicalSchema {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CanonicalSchema {
    fn cmp(&self, other: &Self) -> Ordering {
        self.inner
            .cmp(&other.inner)
            .then_with(|| self.draft.cmp(&other.draft))
            .then_with(|| self.validate_formats.cmp(&other.validate_formats))
            .then_with(|| self.pattern_options.cmp(&other.pattern_options))
            // A node is told apart by the part of its document it can read, so two written the same
            // way and reading the same bodies are one schema whichever documents they came out of.
            .then_with(|| {
                if self.document == other.document || self.reads_no_document() {
                    return Ordering::Equal;
                }
                document_slice(&self.inner, &self.document)
                    .cmp(&document_slice(&other.inner, &other.document))
            })
    }
}

// The definition map is left out so a handle does not hash its whole document; handles differing
// only in their targets collide.
impl Hash for CanonicalSchema {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.hash(state);
        self.draft.hash(state);
        self.validate_formats.hash(state);
        self.pattern_options.hash(state);
    }
}

impl CanonicalSchema {
    /// A handle whose node is the document root, so `#` names that node.
    pub(crate) fn new(
        inner: Schema,
        draft: Draft,
        pattern_options: PatternEngineOptions,
        validate_formats: bool,
        definitions: Arc<DefinitionMap>,
        local: Arc<BTreeSet<Arc<str>>>,
    ) -> Self {
        let document = Document {
            root: inner.clone(),
            definitions,
            local,
        };
        Self {
            inner,
            draft,
            pattern_options,
            validate_formats,
            document,
        }
    }

    /// A handle for a node read against `document`, which keeps naming whatever it named there.
    fn within(
        inner: Schema,
        draft: Draft,
        pattern_options: PatternEngineOptions,
        validate_formats: bool,
        document: Document,
    ) -> Self {
        Self {
            inner,
            draft,
            pattern_options,
            validate_formats,
            document,
        }
    }

    /// Emit this canonical schema back to JSON Schema.
    #[must_use]
    pub fn to_json_schema(&self) -> Value {
        if self.inner == self.document.root {
            // Parsing already drops what the document never names, so the whole map belongs here.
            return emit::to_json_schema(&self.inner, self.draft, &self.document.definitions);
        }
        let definitions = emit::reachable_definitions(
            &self.inner,
            &self.document.root,
            &self.document.definitions,
        );
        let value = emit::to_json_schema(&self.inner, self.draft, &definitions);
        emit::rebind_document_root(value, &self.document.root, self.draft)
    }

    /// Whether any instance satisfies this schema.
    #[must_use]
    pub fn satisfiability(&self) -> Satisfiability {
        // A pointer and the schema it names accept the same values, so the answer is the target's.
        let mut node = &self.inner;
        let mut walked: Vec<&Arc<str>> = Vec::new();
        while let SchemaKind::Reference(uri) = node.kind() {
            // A chain returning to a pointer it followed would not terminate.
            if walked.contains(&uri) {
                return Satisfiability::Unknown;
            }
            let target = if uri.as_ref() == ROOT_DEFINITION_KEY {
                &self.document.root
            } else {
                match self.document.definitions.get(uri.as_ref() as &str) {
                    Some(target) => target,
                    None => return Satisfiability::Unknown,
                }
            };
            walked.push(uri);
            node = target;
        }
        // Canonicalization reduces what it proves empty to `False`. Every other answer here names a
        // value the schema takes: a member it lists, or the shortest value of its type.
        // TODO(canonical): the composite forms wait on an oracle that finds a value for them.
        let answer = match node.kind() {
            SchemaKind::False => Satisfiability::No,
            // `Enum` carries at least two members by construction, so it names one outright, and a
            // type set constrains nothing beyond the type, which has values of its own.
            SchemaKind::True
            | SchemaKind::Const(_)
            | SchemaKind::Enum(_)
            | SchemaKind::MultiType(_) => Satisfiability::Yes,
            // A group carries the value set its constructor packed, whose members are all of the
            // group's type; one that kept none of them folded to `false` instead.
            SchemaKind::TypedGroup { ty, body } => {
                debug_assert!(
                    body.kind()
                        .finite_values()
                        .is_some_and(|values| values.iter().any(|value| value.json_type() == *ty)),
                    "a typed group holds a value of its own type"
                );
                Satisfiability::Yes
            }
            // A leaf whose window is non-empty by construction holds a value of its own type, as
            // long as nothing beyond that window narrows which values those are.
            SchemaKind::String(leaf) => {
                let leaf = leaf.get();
                holds_a_value(
                    leaf.patterns.is_empty()
                        && leaf.excluded_patterns.is_empty()
                        && leaf.formats.is_empty()
                        && leaf.excluded_formats.is_empty()
                        && leaf.content_media_types.is_empty()
                        && leaf.content_encodings.is_empty()
                        && leaf.excluded.is_empty(),
                )
            }

            // The empty array and the empty object are the instances to reach for, where the leaf
            // admits a value that short and demands nothing of what it would have to hold.
            SchemaKind::Array(leaf) => {
                let leaf = leaf.get();
                holds_a_value(
                    leaf.lengths.contains(&BoundCardinality::from(0))
                        && leaf.contains.is_empty()
                        && leaf.distinctness != Distinctness::SomeRepeated,
                )
            }
            SchemaKind::Object(leaf) => {
                let leaf = leaf.get();
                holds_a_value(
                    leaf.sizes.contains(&BoundCardinality::from(0))
                        && leaf.required.is_empty()
                        && leaf.violations.is_empty(),
                )
            }
            // A numeric window is over the reals, where non-empty does not mean a value can be
            // written down: nothing lies between `1` and its own successor. These and the composite
            // forms are answered by the candidate walk below, which exhibits one.
            SchemaKind::Integer(_)
            | SchemaKind::Number(_)
            | SchemaKind::Not(_)
            | SchemaKind::AllOf(_)
            | SchemaKind::AnyOf(_)
            | SchemaKind::OneOf(_)
            | SchemaKind::Reference(_)
            | SchemaKind::Raw(_) => Satisfiability::Unknown,
        };
        if answer != Satisfiability::Unknown {
            return answer;
        }
        // No value found is not the same as none existing: a required key or a divisor needs one
        // the arms above do not build. Build it and check it, on a walk that stayed exact.
        let context = self.context_reading(
            &[&self.inner],
            &[&self.document.root],
            &self.document.definitions,
        );
        for candidate in candidate_instances(
            node,
            &self.document,
            CANDIDATE_DEPTH,
            &Cell::new(CANDIDATE_NODES),
            &context,
        ) {
            let (verdict, inexact) = context.probe(|| {
                algebra::admits_value(node, &candidate, UncheckableFacet::Undecided, &context)
            });
            if verdict == Verdict::Admits && !inexact && !context.outgrew_distribution() {
                return Satisfiability::Yes;
            }
        }
        Satisfiability::Unknown
    }

    /// Whether this node contains no `$ref` at all, so nothing of its document can be seen through
    /// it.
    /// The common case, and answered by a scan of the node rather than a walk of the map.
    fn reads_no_document(&self) -> bool {
        !algebra::contains_reference(&self.inner)
    }

    /// Whether the two nodes read the same part of their documents, which is all either can observe
    /// of the document it came out of.
    fn reads_same_document_as(&self, other: &Self) -> bool {
        if self.document == other.document {
            return true;
        }
        // The two nodes are equal, so they contain the same references - none, in this case.
        if self.reads_no_document() {
            return true;
        }
        // The two nodes are equal, so they contain the same references and one walk covers both.
        let mut pointers = pointers_read(&self.inner, &self.document.definitions);
        if pointers.contains(ROOT_DEFINITION_KEY) {
            if self.document.root != other.document.root {
                return false;
            }
            pointers.extend(pointers_read(
                &self.document.root,
                &self.document.definitions,
            ));
        }
        pointers.iter().all(|uri| {
            self.document.definitions.get(uri.as_ref() as &str)
                == other.document.definitions.get(uri.as_ref() as &str)
        })
    }

    /// Borrow the internal canonical IR kind.
    #[must_use]
    pub(crate) fn schema_kind(&self) -> &SchemaKind {
        self.inner.kind()
    }

    #[must_use]
    pub fn draft(&self) -> Draft {
        self.draft
    }

    /// Wrap a child IR node in a handle sharing this schema's draft, options, and document.
    pub(crate) fn wrap_child(&self, child: &Schema) -> Self {
        Self::within(
            child.clone(),
            self.draft,
            self.pattern_options,
            self.validate_formats,
            self.document.clone(),
        )
    }

    /// The reference target registered under `uri`, or the document itself under `#`.
    ///
    /// The target keeps the document it was written in, so a `#` inside it names that document and
    /// not the target standing in for it.
    #[must_use]
    pub fn definition(&self, uri: &str) -> Option<CanonicalSchema> {
        if uri == ROOT_DEFINITION_KEY {
            return Some(self.wrap_child(&self.document.root));
        }
        self.document
            .definitions
            .get(uri)
            .map(|body| self.wrap_child(body))
    }

    /// Every reachable reference target known to this document, keyed by its URI.
    #[must_use]
    pub fn definitions(&self) -> impl ExactSizeIterator<Item = (String, CanonicalSchema)> + '_ {
        self.document
            .definitions
            .iter()
            .map(|(uri, body)| (uri.to_string(), self.wrap_child(body)))
    }

    /// Every value both schemas admit.
    ///
    /// # Errors
    ///
    /// [`CanonicalizationError::IncompatibleOperands`] when the operands cannot be combined,
    /// [`CanonicalizationError::UnsupportedOperand`] when either side is unsupported, and
    /// [`CanonicalizationError::UnsupportedResult`] when the canonical form cannot express their
    /// intersection exactly.
    pub fn intersect(&self, other: &Self) -> Result<Self, CanonicalizationError> {
        self.combine(other, |left, right, ctx, _| {
            // The identities, answered on the operands as handed in: both are canonical already, so
            // reading their pointers through would fold them further and `a & a == a` would fail.
            match (left.kind(), right.kind()) {
                _ if left == right => return Some(left.clone()),
                (SchemaKind::True, _) | (_, SchemaKind::False) => return Some(right.clone()),
                (SchemaKind::False, _) | (_, SchemaKind::True) => return Some(left.clone()),
                _ => {}
            }
            Some(algebra::intersect(left.clone(), right.clone(), ctx))
        })
    }

    /// Every value either schema admits.
    ///
    /// # Errors
    ///
    /// [`CanonicalizationError::IncompatibleOperands`] when the operands cannot be combined,
    /// [`CanonicalizationError::UnsupportedOperand`] when either side is unsupported, and
    /// [`CanonicalizationError::UnsupportedResult`] when the canonical form cannot express their
    /// union exactly.
    pub fn union(&self, other: &Self) -> Result<Self, CanonicalizationError> {
        self.combine(other, |left, right, ctx, _| {
            // The identities, kept in the form handed in for the same reason.
            match (left.kind(), right.kind()) {
                _ if left == right => return Some(left.clone()),
                (SchemaKind::False, _) | (_, SchemaKind::True) => return Some(right.clone()),
                (SchemaKind::True, _) | (_, SchemaKind::False) => return Some(left.clone()),
                _ => {}
            }
            Some(algebra::union(vec![left.clone(), right.clone()], ctx))
        })
    }

    /// Every value this schema admits and `other` rejects (`self \ other`).
    ///
    /// The difference is what a change to a schema stops admitting: `old.subtract(new)` accepts
    /// exactly the values `old` took and `new` turns away, and is empty iff `new` still takes
    /// everything `old` did.
    ///
    /// # Errors
    ///
    /// [`CanonicalizationError::IncompatibleOperands`] when the operands cannot be combined,
    /// [`CanonicalizationError::UnsupportedOperand`] when either side is unsupported, and
    /// [`CanonicalizationError::UnsupportedResult`] when the canonical form cannot express `other`'s
    /// complement, or the intersection with it, exactly.
    pub fn subtract(&self, other: &Self) -> Result<Self, CanonicalizationError> {
        self.combine(other, |left, right, ctx, definitions| {
            self.difference(left, right, ctx, definitions)
        })
    }

    /// `self \ other` against a context and map the caller already built, or `None` where the
    /// canonical form cannot express it. The body of [`Self::subtract`], reached directly by the
    /// operations that have the frame in hand and would otherwise build a second one.
    fn difference(
        &self,
        taken: &Schema,
        removed: &Schema,
        ctx: &CanonicalizationContext,
        definitions: &DefinitionMap,
    ) -> Option<Schema> {
        // A degenerate difference is one of the operands or empty, so it needs no complement -
        // and asking for one would decline over a schema whose complement is inexpressible even
        // where the difference itself is free.
        match (taken.kind(), removed.kind()) {
            _ if taken == removed => return Some(Schema::falsy()),
            (SchemaKind::False, _) | (_, SchemaKind::True) => return Some(Schema::falsy()),
            (_, SchemaKind::False) => return Some(taken.clone()),
            _ => {}
        }
        // Operands that share no value, and one that contains the other, have a difference the form
        // gives directly.
        // Under a probe, so an approximated intersection falls through to the complement below.
        let (met, inexact) = ctx.probe(|| algebra::intersect(taken.clone(), removed.clone(), ctx));
        if !inexact && matches!(met.kind(), SchemaKind::False) {
            return Some(taken.clone());
        }
        // Everything `taken` admits `removed` admits too, so the difference is empty. Asked through
        // the oracle, which turns down an equality resting on a facet no checker covers and compares
        // through the targets its pointers name.
        if oracle::covers(removed, taken, ctx) == Verdict::Admits {
            return Some(Schema::falsy());
        }
        // A complement over a facet no checker covers bars the values the algebra reads as meeting
        // it, so the difference built around it is not the one that checker takes - unless `taken`
        // demands the same facets, where both sides carry them.
        if !algebra::uncheckable_string_facets(removed, ctx)
            .is_subset(&algebra::uncheckable_string_facets(taken, ctx))
        {
            return None;
        }
        // On the operation's own context, so the complement reads the same targets; the probe keeps
        // what it approximated deciding only itself. Sharing a document keeps `#` naming the root
        // both operands were read against.
        let same_document = self.document.definitions.as_ref() == definitions;
        let (complement, inexact) =
            ctx.probe(|| negate::negate_in_place(removed, definitions, ctx));
        let complement = complement?;
        // Across documents, a complement naming `#` would name the wrong root on the side that did
        // not supply it.
        if !same_document && reads_document_root(&complement, definitions) {
            return None;
        }
        // A complement built around an intersection the form could only approximate, or around
        // whatever a walk out of intersections had reached, is no complement to subtract with.
        if inexact || ctx.outgrew_distribution() {
            return None;
        }
        Some(algebra::intersect(taken.clone(), complement, ctx))
    }

    /// The frame every set operation shares: combinable operands, one context, and the document the
    /// result belongs to. `op` declines what the canonical form cannot express exactly.
    fn combine(
        &self,
        other: &Self,
        op: impl FnOnce(&Schema, &Schema, &CanonicalizationContext, &DefinitionMap) -> Option<Schema>,
    ) -> Result<Self, CanonicalizationError> {
        self.check_operands(other)?;
        let merged = self.merged_definitions(other)?;
        let definitions = &merged.definitions;
        Self::check_document_roots(&merged, definitions)?;
        let context = self.context_reading(
            &[&merged.left, &merged.right],
            &[&merged.left_root, &merged.right_root],
            definitions,
        );
        let inner = op(&merged.left, &merged.right, &context, definitions)
            .ok_or(CanonicalizationError::UnsupportedResult)?;
        // Running out of intersections leaves whatever the walk had reached, and an approximated
        // one the result was built around is no result at all.
        if context.outgrew_distribution() || context.saw_inexact_intersection() {
            return Err(CanonicalizationError::UnsupportedResult);
        }
        let document = Self::combined_document(&merged, &inner, definitions);
        Ok(Self::within(
            inner,
            self.draft,
            self.pattern_options,
            self.validate_formats,
            document,
        ))
    }

    /// Whether this schema admits every value `other` admits.
    ///
    /// # Errors
    ///
    /// [`CanonicalizationError::IncompatibleOperands`] when the operands cannot be combined, and
    /// [`CanonicalizationError::UnsupportedOperand`] when either side is unsupported.
    pub fn covers(&self, other: &Self) -> Result<Containment, CanonicalizationError> {
        self.check_operands(other)?;
        // References resolve through one map, so distinct maps make the two sides incomparable for
        // the same reason they cannot be intersected.
        let merged = self.merged_definitions(other)?;
        let definitions = &merged.definitions;
        Self::check_document_roots(&merged, definitions)?;
        let context = self.context_reading(
            &[&merged.left, &merged.right],
            &[&merged.left_root, &merged.right_root],
            definitions,
        );
        // A walk that approximated, or that ran out of intersections and left whatever it had
        // reached, carries no verdict - each answer below is read only where its own walk finished.
        let decided = |verdict: Verdict, decided: Containment| {
            if context.saw_inexact_intersection() || context.outgrew_distribution() {
                return Some(Containment::Unknown);
            }
            (verdict == Verdict::Admits).then_some(decided)
        };
        if let Some(answer) = decided(
            oracle::covers(&merged.left, &merged.right, &context),
            Containment::Yes,
        ) {
            return Ok(answer);
        }
        // Only a value `other` admits and this schema rejects refutes the coverage, so the
        // refutation rests on such a value rather than on the proof above having failed. A form
        // listing its members hands them over; every other one leaves the difference to say whether
        // any value is left over at all.
        if let Some(values) = merged.right.kind().finite_values() {
            // Only a member whose whole class this schema shares nothing of is refused: under
            // Draft 4 the intersection narrows `1` to the integer type, which still takes `1`.
            // Probed, so what the scan approximated does not decide the answers after it.
            let (refuted, inexact) =
                context.probe(|| {
                    Verdict::from_bool(values.iter().any(|value| {
                        algebra::rejects_value(&merged.left, value.as_value(), &context)
                    }))
                });
            let refuted = if inexact { Verdict::Unknown } else { refuted };
            if let Some(answer) = decided(refuted, Containment::No) {
                return Ok(answer);
            }
        }
        // On the frame already built here: `subtract` would merge the maps and re-walk the graph.
        // The difference the other way round: `other \ self`, on the frame already built here.
        let flipped = Combined {
            definitions: Arc::clone(definitions),
            left: merged.right.clone(),
            right: merged.left.clone(),
            left_root: merged.right_root.clone(),
            right_root: merged.left_root.clone(),
            local: Arc::clone(&merged.local),
        };
        // Bounded: the difference is a side question, and one over two wide schemas costs more than
        // the answer is worth. Out of allowance it says `Unknown`, where the long walk almost always
        // lands anyway.
        let ((difference, inexact), outgrew) = context.capped(COVERS_DIFFERENCE_BUDGET, || {
            context.probe(|| self.difference(&merged.right, &merged.left, &context, definitions))
        });
        let left_over = if inexact || outgrew || context.outgrew_distribution() {
            Satisfiability::Unknown
        } else {
            difference.map_or(Satisfiability::Unknown, |difference| {
                let document = Self::combined_document(&flipped, &difference, definitions);
                Self::within(
                    difference,
                    self.draft,
                    self.pattern_options,
                    self.validate_formats,
                    document,
                )
                .satisfiability()
            })
        };
        Ok(match left_over {
            Satisfiability::Yes => Containment::No,
            // Nothing left over is the coverage itself, proven the other way round.
            Satisfiability::No => Containment::Yes,
            Satisfiability::Unknown => Containment::Unknown,
        })
    }

    /// Every value this schema rejects.
    ///
    /// # Errors
    ///
    /// [`CanonicalizationError::UnsupportedOperand`] when this schema is unsupported, and
    /// [`CanonicalizationError::UnsupportedResult`] where the canonical form cannot express the
    /// complement exactly. The complement of a schema admitting nothing is every value, which is a
    /// result like any other - it is returned as `true`, never an error.
    pub fn negate(&self) -> Result<Self, CanonicalizationError> {
        // A `Raw` operand is unsupported whichever operation reaches it, so a unary one reports it the
        // same way the binary ones do rather than as a complement it could not express.
        if matches!(self.schema_kind(), SchemaKind::Raw(_)) {
            return Err(CanonicalizationError::UnsupportedOperand);
        }
        let context = self.context_reading(
            &[&self.inner],
            &[&self.document.root],
            &self.document.definitions,
        );
        let inner =
            negate::negate_with_definitions(&self.inner, &self.document.definitions, &context)
                .ok_or(CanonicalizationError::UnsupportedResult)?;
        // A complement built around an intersection the form could only approximate, or around
        // whatever a walk out of intersections had reached, is a decline and not a result.
        if context.outgrew_distribution() || context.saw_inexact_intersection() {
            return Err(CanonicalizationError::UnsupportedResult);
        }
        // `negate_with_definitions` declines a complement naming the root, so this one is a root of
        // its own.
        // A resolved complement may name fewer targets than the source; carrying the dead ones
        // would emit unreferenced definitions and block combination with other documents.
        let mut definitions = (*self.document.definitions).clone();
        parse::prune_unreachable_definitions(&inner, &mut definitions);
        let local = narrowed(&self.document.local, &definitions);
        Ok(Self::new(
            inner,
            self.draft,
            self.pattern_options,
            self.validate_formats,
            Arc::new(definitions),
            local,
        ))
    }

    /// The document the result of combining these two belongs to. Two nodes of one document combine
    /// inside it. Where the documents differ, `#` on one side keeps naming what it named there, so
    /// the result stays in that document; a result naming no root at all becomes a root of its own.
    fn combined_document(
        other: &Combined,
        inner: &Schema,
        definitions: &Arc<DefinitionMap>,
    ) -> Document {
        // Both maps, pruned to what the result still names - or `definitions()` would answer about
        // targets nothing emits. The root is read off the *result*: one that folded its pointer
        // away names none, and keeping one would carry a closure the result cannot reach.
        let root = if !reads_document_root(inner, definitions) {
            inner.clone()
        } else if reads_document_root(&other.left, definitions) {
            other.left_root.clone()
        } else {
            other.right_root.clone()
        };
        // A kept root is reachable in its own right - `definition("#")` returns it - so what it
        // names is kept too.
        let mut retained = emit::reachable_definitions(inner, &root, definitions).into_owned();
        if root != *inner {
            retained.extend(
                emit::reachable_definitions(&root, &root, definitions)
                    .iter()
                    .map(|(uri, body)| (Arc::clone(uri), body.clone())),
            );
        }
        Document {
            local: narrowed(&other.local, &retained),
            definitions: Arc::new(retained),
            root,
        }
    }

    /// `#` names the document a node was read against, so two nodes of different documents are
    /// comparable only when at most one of them reads it.
    fn check_document_roots(
        other: &Combined,
        definitions: &Arc<DefinitionMap>,
    ) -> Result<(), CanonicalizationError> {
        // The maps are merged; only the root cannot be. Two roots bind `#` to the same thing when
        // they are written the same way *and* read the same bodies.
        if other.left_root != other.right_root
            && reads_document_root(&other.left, definitions)
            && reads_document_root(&other.right, definitions)
        {
            return Err(CanonicalizationError::IncompatibleOperands(
                OperandMismatch::DocumentRoots,
            ));
        }
        Ok(())
    }

    /// A context for one operation, reading through every target of `definitions` a walk can
    /// finish. A document being canonicalized cannot do this: its bodies are still arriving.
    fn context_reading(
        &self,
        nodes: &[&Schema],
        roots: &[&Schema],
        definitions: &Arc<DefinitionMap>,
    ) -> CanonicalizationContext {
        // Only the reachable targets are walked for cycles: a leaf pays for what it names, not for
        // the document it came out of.
        let mut reachable = BTreeSet::new();
        for node in nodes {
            reachable.extend(pointers_read(node, definitions));
        }
        // Through the operands' own roots: `self`'s spells the names it had before the merge
        // renamed them apart, which are no longer keys of this map.
        if reachable.contains(ROOT_DEFINITION_KEY) {
            for root in roots {
                reachable.extend(pointers_read(root, definitions));
            }
        }
        CanonicalizationContext::new(self.draft, self.pattern_options, self.validate_formats)
            .resolving(
                Arc::clone(definitions),
                emptiness::cyclic_definition_keys(definitions, &reachable),
            )
    }

    fn check_operands(&self, other: &Self) -> Result<(), CanonicalizationError> {
        // An unknown `$schema` canonicalizes to `Raw` under `Draft::Unknown`, so the modeling check
        // comes first or that document reports a draft mismatch instead.
        if matches!(self.schema_kind(), SchemaKind::Raw(_))
            || matches!(other.schema_kind(), SchemaKind::Raw(_))
        {
            return Err(CanonicalizationError::UnsupportedOperand);
        }
        let mismatch = if self.draft != other.draft {
            OperandMismatch::Drafts {
                left: self.draft,
                right: other.draft,
            }
        } else if self.validate_formats != other.validate_formats {
            OperandMismatch::FormatAssertions
        } else if self.pattern_options != other.pattern_options {
            OperandMismatch::PatternEngine
        } else {
            return Ok(());
        };
        Err(CanonicalizationError::IncompatibleOperands(mismatch))
    }

    /// The map the result resolves references through.
    fn merged_definitions(&self, other: &Self) -> Result<Combined, CanonicalizationError> {
        let ours = &self.document.definitions;
        let theirs = &other.document.definitions;
        let mut combined = Combined {
            definitions: Arc::clone(ours),
            left: self.inner.clone(),
            right: other.inner.clone(),
            left_root: self.document.root.clone(),
            right_root: other.document.root.clone(),
            local: Arc::clone(&self.document.local),
        };
        // Both sides' private names travel with the result whichever map it keeps, or the operand
        // order would decide which of them a later operation may rename apart.
        if Arc::ptr_eq(ours, theirs) || theirs.is_empty() {
            combined.local = united(&self.document.local, &other.document.local);
            return Ok(combined);
        }
        if ours.is_empty() {
            combined.definitions = Arc::clone(theirs);
            combined.local = united(&self.document.local, &other.document.local);
            return Ok(combined);
        }
        // A key both maps hold under a different body cannot resolve two ways, so one side takes a
        // fresh name for it and its references follow. Every shared key counts, not just those an
        // operand names: a body is reached through targets too.
        let Some(renames) =
            rename::reconcile(ours, theirs, &self.document.local, &other.document.local)
        else {
            return Err(CanonicalizationError::IncompatibleOperands(
                OperandMismatch::Definitions,
            ));
        };
        let (ours, theirs) = if renames.is_empty() {
            (Arc::clone(ours), Arc::clone(theirs))
        } else {
            combined.left = rename::rename_references(&self.inner, &renames.left);
            combined.right = rename::rename_references(&other.inner, &renames.right);
            combined.left_root = rename::rename_references(&self.document.root, &renames.left);
            combined.right_root = rename::rename_references(&other.document.root, &renames.right);
            combined.local = Arc::new(rename::rename_keys(
                &self.document.local,
                &renames.left,
                &other.document.local,
                &renames.right,
            ));
            (
                Arc::new(rename::rename_definitions(ours, &renames.left)),
                Arc::new(rename::rename_definitions(theirs, &renames.right)),
            )
        };
        if renames.is_empty() {
            combined.local = united(&self.document.local, &other.document.local);
        }
        combined.definitions = if theirs.keys().all(|uri| ours.contains_key(uri)) {
            ours
        } else {
            let mut merged = (*ours).clone();
            merged.extend(
                theirs
                    .iter()
                    .map(|(uri, body)| (Arc::clone(uri), body.clone())),
            );
            Arc::new(merged)
        };
        Ok(combined)
    }
}

/// The private names a pruned map still holds. A marker outliving its body would let a later
/// operation adopt a retrieved resource under that name and rename it apart, where the two
/// documents reading that resource differently must refuse instead.
fn narrowed(
    local: &Arc<BTreeSet<Arc<str>>>,
    definitions: &DefinitionMap,
) -> Arc<BTreeSet<Arc<str>>> {
    if local.iter().all(|uri| definitions.contains_key(uri)) {
        return Arc::clone(local);
    }
    Arc::new(
        local
            .iter()
            .filter(|uri| definitions.contains_key(&***uri))
            .map(Arc::clone)
            .collect(),
    )
}

/// Both sets of private names.
fn united(
    ours: &Arc<BTreeSet<Arc<str>>>,
    theirs: &Arc<BTreeSet<Arc<str>>>,
) -> Arc<BTreeSet<Arc<str>>> {
    if Arc::ptr_eq(ours, theirs) {
        return Arc::clone(ours);
    }
    Arc::new(ours.union(theirs).map(Arc::clone).collect())
}

/// What both operands resolve through, and the two nodes as the merge left them: a `$defs` key the
/// two documents bound differently is renamed apart on one side, and that side's node follows.
struct Combined {
    definitions: Arc<DefinitionMap>,
    left: Schema,
    right: Schema,
    left_root: Schema,
    right_root: Schema,
    local: Arc<BTreeSet<Arc<str>>>,
}
