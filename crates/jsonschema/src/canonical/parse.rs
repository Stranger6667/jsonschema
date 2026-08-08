//! Parsing schema documents into structural IR; anything not modeled stays `Raw`.
use std::{borrow::Cow, sync::Arc};

use ahash::{AHashMap, AHashSet};

use referencing::{Draft, Resolver};
use serde_json::Value;

use crate::{
    canonical::{
        algebra,
        context::CanonicalizationContext,
        emptiness,
        ir::{
            canonicalize_value_set, type_set_schema, typed_group, ArrayLeaf, BoundCardinality,
            BoundNumber, BoundRational, CanonicalJson, ContainsFacet, Distinctness, Divisors,
            ExcludedDivisors, IntegerLeaf, LengthBounds, NumberLeaf, ObjectLeaf, PropertyMap,
            Schema, SchemaKind, Side, StringFormat, StringLeaf,
        },
        negate, CanonicalizationError, DefinitionMap, CANONICAL_REFERENCE_PREFIX,
        ROOT_DEFINITION_KEY,
    },
    JsonType, JsonTypeSet,
};

/// Root IR plus every symbolic `$ref` target parsed during canonicalization.
pub(crate) struct ParseOutput {
    pub(crate) root: Schema,
    pub(crate) definitions: DefinitionMap,
    /// Whether any `$ref` resolved during this parse. `reference_to_definition` is the only
    /// producer of `SchemaKind::Reference`, so `false` means the emptiness pass has no graph to
    /// build.
    pub(crate) has_references: bool,
    /// The choices between pointers left undecided for want of a body still being parsed.
    pub(crate) pending_choices: Vec<Vec<Schema>>,
}

/// Parse a document into structural IR when every construct is modeled; `Ok(None)` keeps it `Raw`.
/// Keywords the draft does not define are annotations the validator ignores, so they never block
/// modeling - except an unknown `$schema`, whose dialect semantics are unknowable.
pub(crate) fn parse<'a>(
    value: &'a Value,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'a>,
) -> Result<Option<ParseOutput>, CanonicalizationError> {
    parse_inner(
        value,
        ctx,
        resolver,
        &Assumptions::default(),
        Pruning::Prune,
    )
}

/// Targets a parse resolves to a fixed body rather than to a symbolic `Reference`.
#[derive(Default, Clone)]
pub(crate) struct Assumptions {
    /// Resolved as `false`.
    pub(crate) empty: AHashSet<Arc<str>>,
    /// Resolved as `true`.
    pub(crate) admits_all: AHashSet<Arc<str>>,
    /// Bodies a finished round produced, for the decisions a target still being parsed cannot
    /// answer. A body only ever narrows between rounds, so a decision this map settles stays
    /// settled the same way.
    pub(crate) finished: DefinitionMap,
}

/// [`parse`] under `assumptions`.
///
/// One parse applies a whole round's hypothesis, so every body comes back canonicalized under it.
pub(crate) fn parse_with<'a>(
    value: &'a Value,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'a>,
    assumptions: &Assumptions,
) -> Result<Option<ParseOutput>, CanonicalizationError> {
    parse_inner(value, ctx, resolver, assumptions, Pruning::Prune)
}

/// [`parse_with`] keeping every body, including those the hypothesis made unreachable.
///
/// Folding every reference to a key is what makes it unreachable, and the fixpoint reads that body
/// to decide whether the assumption held - so pruning here would delete the evidence.
pub(crate) fn parse_hypothesis<'a>(
    value: &'a Value,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'a>,
    assumptions: &Assumptions,
) -> Result<Option<ParseOutput>, CanonicalizationError> {
    parse_inner(value, ctx, resolver, assumptions, Pruning::Keep)
}

/// Whether to drop the definitions the emitted IR no longer references.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pruning {
    Prune,
    Keep,
}

fn parse_inner<'a>(
    value: &'a Value,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'a>,
    assumptions: &Assumptions,
    pruning: Pruning,
) -> Result<Option<ParseOutput>, CanonicalizationError> {
    // A body that came out `false` denotes the empty set, so every reference to it folds as well -
    // but `resolve_reference` can only fold a target whose body already finished parsing, which
    // makes that dependent on the order definitions were registered in. Re-parsing with the folded
    // keys added, until none are new, settles it: the result no longer depends on the order.
    let mut folded = assumptions.clone();
    let mut tracks = false;
    let mut reparsed_for_bodies = false;
    loop {
        let attempt = parse_once(value, ctx, resolver, &folded, pruning, tracks)?;
        if attempt.needs_dynamic_scope {
            debug_assert!(!tracks, "a tracked parse never requests tracking");
            // A referenced resource spells a dynamic reference the root document does not, so the
            // definitions minted so far were keyed without the environment; reparse with it
            // tracked. Untracked keys carry an empty digest, the same string a tracked parse
            // mints for them, so the folded set stays valid.
            tracks = true;
            continue;
        }
        let Some(parsed) = attempt.output else {
            return Ok(None);
        };
        let mut grew = false;
        for (key, body) in &parsed.definitions {
            if matches!(body.kind(), SchemaKind::False) {
                grew |= folded.empty.insert(Arc::clone(key));
            }
        }
        if !grew {
            // A choice between pointers reads the bodies they name, and one still being parsed has
            // none to read - which would make the form depend on the order the targets registered
            // in. One re-parse with this round's bodies known settles every such choice: what the
            // choice reads off a body is which types it admits, and folding one cannot change that.
            let settles = !reparsed_for_bodies
                && parsed
                    .pending_choices
                    .iter()
                    .any(|branches| algebra::choice_folds(branches, &parsed.definitions, ctx));
            if settles {
                reparsed_for_bodies = true;
                folded.finished = parsed.definitions.clone();
                continue;
            }
            return Ok(Some(parsed));
        }
    }
}

/// One parse attempt plus whether a lazily-resolved target turned out to need the dynamic scope.
struct DocumentParse {
    output: Option<ParseOutput>,
    needs_dynamic_scope: bool,
}

fn parse_once<'a>(
    value: &'a Value,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'a>,
    assumptions: &'a Assumptions,
    pruning: Pruning,
    tracks_dynamic_scope: bool,
) -> Result<DocumentParse, CanonicalizationError> {
    let mut state = ParseState::new(value, resolver.base_uri().as_str(), assumptions);
    if !tracks_dynamic_scope {
        state.dynamic_scope = DynamicScope::Untracked {
            needs_tracking: false,
        };
    }
    let parsed = parse_schema_in_scope(value, ctx, true, resolver, &mut state)?;
    // An in-between object meet the IR cannot spell may have produced nodes already, so discard
    // the whole document rather than just that pairing site.
    if ctx.saw_unspellable_meet() {
        return Ok(DocumentParse {
            output: None,
            needs_dynamic_scope: state.dynamic_scope.needs_tracking(),
        });
    }
    let Some(root) = parsed else {
        return Ok(DocumentParse {
            output: None,
            needs_dynamic_scope: state.dynamic_scope.needs_tracking(),
        });
    };
    let needs_dynamic_scope = state.dynamic_scope.needs_tracking();
    if pruning == Pruning::Prune {
        prune_unreachable_definitions(&root, &mut state.definitions);
    }
    Ok(DocumentParse {
        output: Some(ParseOutput {
            root,
            definitions: state.definitions,
            has_references: state.facts.has_references,
            pending_choices: state.facts.pending_choices,
        }),
        needs_dynamic_scope,
    })
}

/// Canonical reference graph plus facts whose interaction is decided only after parsing the root.
struct ParseState<'a> {
    root: &'a Value,
    root_base_uri: Arc<str>,
    /// Whole-document facts whose interaction is only decided once the root is complete.
    facts: DocumentFacts,
    definitions: DefinitionMap,
    in_progress: AHashSet<Arc<str>>,
    /// The target each definition key was minted for, so a key cannot be reused for another one.
    sources: AHashMap<Arc<str>, &'a Value>,
    /// Empty on every parse outside the definition fixpoint.
    assumptions: &'a Assumptions,
    dynamic_scope: DynamicScope,
}

/// Flags set anywhere in the document and read after the whole parse.
///
/// Independent observations rather than a state machine, so the usual "replace bools with an enum"
/// advice does not apply - any combination of them is reachable.
#[derive(Default)]
#[allow(clippy::struct_excessive_bools)]
struct DocumentFacts {
    /// `reference_to_definition` is the only producer of `Reference`, so this gates the emptiness
    /// pass.
    has_references: bool,
    /// The choices between pointers left undecided for want of a body still being parsed.
    pending_choices: Vec<Vec<Schema>>,
}

/// How a parse attempt treats the dynamic scope.
enum DynamicScope {
    /// A dynamic reference is spelled: digests are computed and definition keys specialized.
    Tracked,
    /// No dynamic reference has been reached, so keys stay unspecialized until one does.
    Untracked { needs_tracking: bool },
}

impl DynamicScope {
    fn tracked(&self) -> bool {
        matches!(self, DynamicScope::Tracked)
    }

    fn needs_tracking(&self) -> bool {
        matches!(
            self,
            DynamicScope::Untracked {
                needs_tracking: true,
                ..
            }
        )
    }

    fn request_tracking(&mut self) {
        if let Self::Untracked { needs_tracking } = self {
            *needs_tracking = true;
        }
    }
}

impl<'a> ParseState<'a> {
    fn new(root: &'a Value, root_base_uri: &str, assumptions: &'a Assumptions) -> Self {
        Self {
            root,
            root_base_uri: Arc::from(root_base_uri),
            facts: DocumentFacts::default(),
            definitions: DefinitionMap::new(),
            in_progress: AHashSet::new(),
            sources: AHashMap::default(),
            assumptions,
            dynamic_scope: DynamicScope::Tracked,
        }
    }

    /// Whether `key` is being resolved as `false` for this parse.
    fn assumes_empty(&self, key: &str) -> bool {
        self.assumptions.empty.contains(key)
    }

    /// Whether `key` is being resolved as `true` for this parse.
    fn assumes_admits_all(&self, key: &str) -> bool {
        self.assumptions.admits_all.contains(key)
    }
}

fn parse_schema<'a>(
    value: &Value,
    ctx: &CanonicalizationContext,
    is_root: bool,
    resolver: &Resolver<'a>,
    state: &mut ParseState<'a>,
) -> Result<Option<Schema>, CanonicalizationError> {
    let resolver = resolver.in_subresource(ctx.draft().create_resource_ref(value))?;
    parse_schema_in_scope(value, ctx, is_root, &resolver, state)
}

/// The dynamic-scope facts a target's parse can observe, derived from its resolver: for each
/// dynamic-anchor name, the outermost scope resource overriding its lexical resolution, and under
/// 2019-09 the resource the contiguous `$recursiveAnchor` chain lands on. Equal digests resolve
/// every dynamic reference in the target identically, so the digest disambiguates definition keys.
/// Sorted by name; the value set per name is finite, so recursion revisits a key and
/// [`ParseState::in_progress`] closes the cycle.
type DynamicEnv = Arc<[(Arc<str>, Arc<str>)]>;

/// The 2019-09 spelling binds here. `anchorString` cannot start with `$`, so no collision.
const RECURSIVE_ANCHOR_NAME: &str = "$recursiveAnchor";

fn empty_environment() -> DynamicEnv {
    Arc::from(Vec::new())
}

fn dynamic_scope_digest(
    resolver: &Resolver<'_>,
    draft: Draft,
) -> Result<DynamicEnv, CanonicalizationError> {
    if matches!(draft, Draft::Draft201909) {
        return recursive_chain_digest(resolver);
    }
    dynamic_anchor_digest(resolver)
}

/// For each dynamic-anchor name, the outermost scope resource declaring it - the resource the
/// resolver's overwrite walk would land on when overriding a lexical resolution.
fn dynamic_anchor_digest(resolver: &Resolver<'_>) -> Result<DynamicEnv, CanonicalizationError> {
    let mut bindings: Vec<(Arc<str>, Arc<str>)> = Vec::new();
    // Innermost to outermost, overwriting on every match, exactly like the resolver's walk.
    for uri in &resolver.dynamic_scope() {
        let (contents, _, resource_draft) = resolver.lookup(uri.as_str())?.into_inner();
        let mut names = Vec::new();
        resource_dynamic_anchor_names(contents, resource_draft, &mut names);
        for name in names {
            let resource: Arc<str> = Arc::from(uri.as_str());
            match bindings.iter_mut().find(|(bound, _)| *bound == name) {
                Some(binding) => binding.1 = resource,
                None => bindings.push((name, resource)),
            }
        }
    }
    if bindings.is_empty() {
        return Ok(empty_environment());
    }
    bindings.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(Arc::from(bindings))
}

/// Every `$dynamicAnchor` name the registry attributes to this resource: its own and those of
/// subresources, stopping where an identifier starts an embedded resource of its own.
fn resource_dynamic_anchor_names(contents: &Value, draft: Draft, names: &mut Vec<Arc<str>>) {
    if let Some(name) = contents
        .as_object()
        .and_then(|map| map.get("$dynamicAnchor"))
        .and_then(Value::as_str)
    {
        names.push(Arc::from(name));
    }
    for subresource in draft.subresources_of(contents) {
        if draft.create_resource_ref(subresource).id().is_none() {
            resource_dynamic_anchor_names(subresource, draft, names);
        }
    }
}

/// The resource a `$recursiveRef` walk lands on: the outermost entry of the contiguous
/// `$recursiveAnchor: true` chain at the inner end of the scope, mirroring
/// [`Resolver::lookup_recursive_ref`].
fn recursive_chain_digest(resolver: &Resolver<'_>) -> Result<DynamicEnv, CanonicalizationError> {
    let mut landing: Option<Arc<str>> = None;
    for uri in &resolver.dynamic_scope() {
        let (contents, _, _) = resolver.lookup(uri.as_str())?.into_inner();
        if resource_root_has_recursive_anchor(contents) {
            landing = Some(Arc::from(uri.as_str()));
        } else {
            break;
        }
    }
    match landing {
        Some(resource) => Ok(Arc::from(vec![(
            Arc::from(RECURSIVE_ANCHOR_NAME),
            resource,
        )])),
        None => Ok(empty_environment()),
    }
}

fn resource_root_has_recursive_anchor(contents: &Value) -> bool {
    contents
        .as_object()
        .and_then(|map| map.get("$recursiveAnchor"))
        .and_then(Value::as_bool)
        == Some(true)
}

/// Whether this schema object carries a reference whose resolver consults the dynamic scope.
fn has_dynamic_reference(map: &serde_json::Map<String, Value>, draft: Draft) -> bool {
    (draft.is_known_keyword("$dynamicRef") && map.get("$dynamicRef").is_some_and(Value::is_string))
        || (matches!(draft, Draft::Draft201909)
            && map.get("$recursiveRef").is_some_and(Value::is_string))
}

/// The definition key `key` takes under `env`.
///
/// Any target reached under a binding is specialized, whether or not it spells a dynamic reference
/// itself: it may only *reach* one through a `$ref`, and its canonical form still differs per
/// binding. Filtering on the target's own text instead would let a plain-keyed intermediary be
/// cached from the first path and wrongly reused for the second.
fn specialized_key(key: &Arc<str>, env: &DynamicEnv) -> Arc<str> {
    if env.is_empty() {
        return Arc::clone(key);
    }
    let decoded = match key.strip_prefix(CANONICAL_REFERENCE_PREFIX) {
        Some(encoded) => percent_encoding::percent_decode_str(encoded)
            .decode_utf8_lossy()
            .into_owned(),
        None => key.to_string(),
    };
    // Anchor names from externally registered resources bypass `anchorString` validation, so a
    // component may spell the join delimiters; escaping keeps `(key, env) -> string` injective.
    let mut spelled = match Cow::from(percent_encoding::utf8_percent_encode(
        &decoded,
        SPECIALIZATION_COMPONENT,
    )) {
        Cow::Borrowed(_) => decoded,
        Cow::Owned(escaped) => escaped,
    };
    for (name, resource) in env.iter() {
        spelled.push_str("|dyn=");
        spelled.extend(percent_encoding::utf8_percent_encode(
            name,
            SPECIALIZATION_COMPONENT,
        ));
        spelled.push('@');
        spelled.extend(percent_encoding::utf8_percent_encode(
            resource,
            SPECIALIZATION_COMPONENT,
        ));
    }
    let spelled =
        percent_encoding::utf8_percent_encode(&spelled, percent_encoding::NON_ALPHANUMERIC);
    let uri = format!("{CANONICAL_REFERENCE_PREFIX}{spelled}");
    let uri = referencing::uri::from_str(&uri).expect("a percent-encoded canonical URI is valid");
    Arc::from(uri.as_str())
}

/// The join delimiters, and `%` so the escaping is itself injective.
const SPECIALIZATION_COMPONENT: &percent_encoding::AsciiSet =
    &percent_encoding::CONTROLS.add(b'%').add(b'|').add(b'@');

/// Parse a root or resolved target whose resolver already carries that resource's base URI.
fn parse_schema_in_scope<'a>(
    value: &Value,
    ctx: &CanonicalizationContext,
    is_root: bool,
    resolver: &Resolver<'a>,
    state: &mut ParseState<'a>,
) -> Result<Option<Schema>, CanonicalizationError> {
    let map = match value {
        Value::Bool(true) => return Ok(Some(Schema::truthy())),
        Value::Bool(false) => return Ok(Some(Schema::falsy())),
        Value::Object(map) => map,
        // Not a schema document; the root is rejected earlier, a nested one keeps the document raw.
        Value::Null | Value::Number(_) | Value::String(_) | Value::Array(_) => return Ok(None),
    };

    // Runs before the `$ref` split below, which would hide a `$ref` from the applicator check.
    if has_unevaluated(map, ctx.draft()) {
        let Some(degraded) = degrade_unevaluated(map, ctx.draft(), ctx, resolver)? else {
            return Ok(None);
        };
        return parse_schema_in_scope(&degraded, ctx, is_root, resolver, state);
    }

    // An untracked attempt stops before resolving a dynamic reference. Re-running it with the
    // scope tracked gives every reachable target the environment-specialized key it requires.
    if !state.dynamic_scope.tracked() && has_dynamic_reference(map, ctx.draft()) {
        state.dynamic_scope.request_tracking();
        return Ok(None);
    }

    // The reference keywords are independent and may be spelled together, so each contributes a
    // conjunct.
    let mut references: Vec<Schema> = Vec::new();
    if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
        let Some(schema) = resolve_reference(reference, ctx, resolver, state)? else {
            return Ok(None);
        };
        references.push(schema);
    }
    if ctx.draft().is_known_keyword("$dynamicRef") {
        if let Some(reference) = map.get("$dynamicRef").and_then(Value::as_str) {
            debug_assert!(
                state.dynamic_scope.tracked(),
                "a dynamic reference is only resolved with the dynamic scope tracked"
            );
            // [`Resolver::lookup`] implements the dynamic rule already: a fragment naming a
            // `$dynamicAnchor` resolves against the outermost resource in scope declaring it, one
            // that does not resolves as `$ref`.
            let Some(schema) = resolve_reference(reference, ctx, resolver, state)? else {
                return Ok(None);
            };
            references.push(schema);
        }
    }
    // 2020-12 still spells `$recursiveRef` in its metaschema as a deprecated compatibility entry,
    // so `is_known_keyword` is true there - but the validator only compiles it under 2019-09.
    if matches!(ctx.draft(), Draft::Draft201909) {
        if let Some(reference) = map.get("$recursiveRef").and_then(Value::as_str) {
            debug_assert!(
                state.dynamic_scope.tracked(),
                "a recursive reference is only resolved with the dynamic scope tracked"
            );
            // 2019-09 constrains the value to `#`; anything else is not a recursive reference.
            if reference != "#" {
                return Ok(None);
            }
            let Some(schema) = resolve_recursive_reference(ctx, resolver, state)? else {
                return Ok(None);
            };
            references.push(schema);
        }
    }
    if let Some(combined) = combine_references(references, ctx) {
        if matches!(ctx.draft(), Draft::Draft4 | Draft::Draft6 | Draft::Draft7) {
            return Ok(Some(combined));
        }
        if !ref_has_assertion_siblings(map, ctx.draft()) {
            return Ok(Some(combined));
        }
        let mut siblings = map.clone();
        siblings.remove("$ref");
        siblings.remove("$dynamicRef");
        siblings.remove("$recursiveRef");
        // The resolver already entered this resource; a surviving relative identifier would shift
        // the base a second time when the clone re-enters below.
        siblings.remove("$id");
        siblings.remove("id");
        return Ok(
            parse_schema(&Value::Object(siblings), ctx, is_root, resolver, state)?
                .map(|siblings| algebra::intersect(combined, siblings, ctx)),
        );
    }

    let mut type_set = None;
    let mut enum_values = None;
    let mut const_value = None;
    let mut min_length: Option<BoundCardinality> = None;
    let mut max_length: Option<BoundCardinality> = None;
    let mut distinctness = Distinctness::Unconstrained;
    let mut min_items: Option<BoundCardinality> = None;
    let mut max_items: Option<BoundCardinality> = None;
    let mut items: Option<Schema> = None;
    let mut contains_schema: Option<Schema> = None;
    let mut min_contains: Option<BoundCardinality> = None;
    let mut max_contains: Option<BoundCardinality> = None;
    let mut item_prefix: Option<Vec<Schema>> = None;
    let mut additional_items: Option<&Value> = None;
    let mut required: Vec<Arc<str>> = Vec::new();
    let mut property_names: Option<Schema> = None;
    let mut properties = PropertyMap::default();
    let mut pattern_properties = PropertyMap::default();
    let mut forbid_unmatched_keys = false;
    let mut additional_schema: Option<Schema> = None;
    let mut min_properties: Option<BoundCardinality> = None;
    let mut max_properties: Option<BoundCardinality> = None;
    let mut patterns: Vec<Arc<str>> = Vec::new();
    let mut formats: Vec<StringFormat> = Vec::new();
    let mut content_media_types: Vec<Arc<str>> = Vec::new();
    let mut content_encodings: Vec<Arc<str>> = Vec::new();
    let mut multiple_of = Divisors::default();
    // The number domain keeps each end as written: on the reals an excluded bound has no successor
    // to fold it into, unlike the integer path below.
    let mut real_minimum: Option<BoundNumber> = None;
    let mut real_maximum: Option<BoundNumber> = None;
    // Draft 4 spells exclusivity as a boolean modifier on `minimum`/`maximum`, which may be read
    // before the bound it modifies, so it is applied once the whole object has been read.
    let mut draft4_exclusive_minimum = false;
    let mut draft4_exclusive_maximum = false;
    let mut if_schema: Option<Schema> = None;
    let mut then_schema: Option<Schema> = None;
    let mut else_schema: Option<Schema> = None;
    let mut conjuncts: Vec<Schema> = Vec::new();
    for (key, entry) in map {
        match (key.as_str(), entry) {
            ("$schema", Value::String(uri)) => {
                let declared = Draft::from_schema_uri(uri);
                // An unknown dialect has unknowable semantics. A nested one naming the dialect
                // already in force is inert - every bundled `meta/*.json` spells it that way.
                //
                // TODO(canonical): not modeled yet - a nested `$schema` that switches dialect.
                if matches!(declared, Draft::Unknown) || (!is_root && declared != ctx.draft()) {
                    return Ok(None);
                }
            }
            // A dynamic anchor names a resource for a later `$dynamicRef`; it admits every value.
            // Matched against its declared value shape, so a mistyped one still keeps the doc raw.
            // A string `$recursiveRef` only reaches this loop under a draft whose validator never
            // compiles it (2019-09 consumes it as a reference above), so it asserts nothing.
            ("$id" | "id" | "$anchor" | "$dynamicAnchor" | "$recursiveRef", Value::String(_))
            | ("$recursiveAnchor", Value::Bool(_))
            | ("$defs" | "definitions", Value::Object(_)) => {}
            ("allOf", Value::Array(branches)) => {
                for branch in branches {
                    match parse_schema(branch, ctx, false, resolver, state)? {
                        Some(schema) => conjuncts.push(schema),
                        None => return Ok(None),
                    }
                }
            }
            ("anyOf", Value::Array(items)) => {
                let mut branches = Vec::new();
                for branch in items {
                    match parse_schema(branch, ctx, false, resolver, state)? {
                        Some(schema) => branches.push(schema),
                        None => return Ok(None),
                    }
                }
                conjuncts.push(algebra::union(branches, ctx));
            }
            ("oneOf", Value::Array(items)) => {
                let mut branches = Vec::new();
                for branch in items {
                    match parse_schema(branch, ctx, false, resolver, state)? {
                        Some(schema) => branches.push(schema),
                        None => return Ok(None),
                    }
                }
                match algebra::one_of(
                    branches,
                    &state.definitions,
                    &state.assumptions.finished,
                    &mut state.facts.pending_choices,
                    ctx,
                ) {
                    Some(schema) => conjuncts.push(schema),
                    None => return Ok(None),
                }
            }
            ("type", value) => match parse_type_set(value) {
                Some(set) => type_set = Some(set),
                None => return Ok(None),
            },
            // A `const`/`enum` number too large to expand into a plain decimal spelling has no
            // exact runtime comparison, so such a document stays raw. Only `arbitrary-precision`
            // reaches this: the cap is a million digits, past every other build's numeric range.
            ("enum", Value::Array(values)) if ctx.draft().is_known_keyword("enum") => {
                if !values.iter().all(finite_value_spelling_is_exact) {
                    return Ok(None);
                }
                enum_values = Some(values);
            }
            ("const", value) if ctx.draft().is_known_keyword("const") => {
                if !finite_value_spelling_is_exact(value) {
                    return Ok(None);
                }
                const_value = Some(value);
            }
            // In the default build a length bound past `u64` has no modeled form; keep the document raw.
            ("minLength", Value::Number(number)) if ctx.draft().is_known_keyword("minLength") => {
                match BoundCardinality::from_number(number) {
                    Some(bound) => min_length = Some(bound),
                    None => return Ok(None),
                }
            }
            ("maxLength", Value::Number(number)) if ctx.draft().is_known_keyword("maxLength") => {
                match BoundCardinality::from_number(number) {
                    Some(bound) => max_length = Some(bound),
                    None => return Ok(None),
                }
            }
            ("uniqueItems", Value::Bool(flag)) if ctx.draft().is_known_keyword("uniqueItems") => {
                distinctness = if *flag {
                    Distinctness::AllDistinct
                } else {
                    Distinctness::Unconstrained
                };
            }
            ("minItems", Value::Number(number)) if ctx.draft().is_known_keyword("minItems") => {
                match BoundCardinality::from_number(number) {
                    Some(bound) => min_items = Some(bound),
                    None => return Ok(None),
                }
            }
            ("maxItems", Value::Number(number)) if ctx.draft().is_known_keyword("maxItems") => {
                match BoundCardinality::from_number(number) {
                    Some(bound) => max_items = Some(bound),
                    None => return Ok(None),
                }
            }
            // The uniform schema form constrains every element.
            ("items", value @ (Value::Object(_) | Value::Bool(_)))
                if ctx.draft().is_known_keyword("items") =>
            {
                match parse_schema(value, ctx, false, resolver, state)? {
                    Some(schema) => items = Some(schema),
                    None => return Ok(None),
                }
            }
            // The 2020-12 tuple: each element carries the schema at its index.
            ("prefixItems", Value::Array(schemas))
                if ctx.draft().is_known_keyword("prefixItems") =>
            {
                match parse_prefix(schemas, ctx, resolver, state)? {
                    Some(prefix) => item_prefix = Some(prefix),
                    None => return Ok(None),
                }
            }
            // Array-form `items` is the tuple in 2019-09 and earlier; 2020-12 spells it `prefixItems`.
            ("items", Value::Array(schemas))
                if matches!(
                    ctx.draft(),
                    Draft::Draft4 | Draft::Draft6 | Draft::Draft7 | Draft::Draft201909
                ) =>
            {
                match parse_prefix(schemas, ctx, resolver, state)? {
                    Some(prefix) => item_prefix = Some(prefix),
                    None => return Ok(None),
                }
            }
            // `additionalItems` constrains the elements beyond an array-form `items` tuple, and is
            // inert when `items` is a schema or absent. Its value is held raw and parsed only once
            // a tuple makes it live. 2020-12 spells the tuple `prefixItems`, which this keyword
            // never tails, and an array-form `items` there keeps the document raw.
            ("additionalItems", value @ (Value::Object(_) | Value::Bool(_))) => {
                additional_items = Some(value);
            }
            ("contains", value @ (Value::Object(_) | Value::Bool(_)))
                if ctx.draft().is_known_keyword("contains") =>
            {
                match parse_schema(value, ctx, false, resolver, state)? {
                    Some(schema) => contains_schema = Some(schema),
                    None => return Ok(None),
                }
            }
            ("minContains", Value::Number(number))
                if ctx.draft().is_known_keyword("minContains") =>
            {
                match BoundCardinality::from_number(number) {
                    Some(bound) => min_contains = Some(bound),
                    None => return Ok(None),
                }
            }
            ("maxContains", Value::Number(number))
                if ctx.draft().is_known_keyword("maxContains") =>
            {
                match BoundCardinality::from_number(number) {
                    Some(bound) => max_contains = Some(bound),
                    None => return Ok(None),
                }
            }
            ("required", Value::Array(names))
                if ctx.draft().is_known_keyword("required")
                    && names.iter().all(Value::is_string) =>
            {
                required.extend(names.iter().filter_map(Value::as_str).map(Arc::from));
            }
            ("properties", Value::Object(entries))
                if ctx.draft().is_known_keyword("properties") =>
            {
                for (key, value) in entries {
                    match parse_schema(value, ctx, false, resolver, state)? {
                        Some(schema) => {
                            properties.insert(Arc::from(key.as_str()), schema);
                        }
                        None => return Ok(None),
                    }
                }
            }
            ("patternProperties", Value::Object(entries))
                if ctx.draft().is_known_keyword("patternProperties") =>
            {
                for (pattern, value) in entries {
                    let pattern: Arc<str> = Arc::from(pattern.as_str());
                    if ctx.compile_regex(&pattern).is_none() {
                        return Err(CanonicalizationError::InvalidPattern {
                            pattern: pattern.to_string(),
                        });
                    }
                    match parse_schema(value, ctx, false, resolver, state)? {
                        Some(schema) => {
                            pattern_properties.insert(pattern, schema);
                        }
                        None => return Ok(None),
                    }
                }
            }
            ("propertyNames", value) if ctx.draft().is_known_keyword("propertyNames") => {
                match parse_schema(value, ctx, false, resolver, state)? {
                    Some(schema) => property_names = Some(schema),
                    None => return Ok(None),
                }
            }
            // A schema admitting everything says nothing about a key, so `true`/`{}` leaves no
            // trace; one admitting nothing forbids the unmatched keys, which the key constraint
            // carries; anything in between shields the named keys and constrains the rest.
            ("additionalProperties", value @ (Value::Object(_) | Value::Bool(_)))
                if ctx.draft().is_known_keyword("additionalProperties") =>
            {
                match parse_schema(value, ctx, false, resolver, state)? {
                    Some(schema) if matches!(schema.kind(), SchemaKind::True) => {}
                    Some(schema) if matches!(schema.kind(), SchemaKind::False) => {
                        forbid_unmatched_keys = true;
                    }
                    Some(schema) => additional_schema = Some(schema),
                    None => return Ok(None),
                }
            }
            ("minProperties", Value::Number(number))
                if ctx.draft().is_known_keyword("minProperties") =>
            {
                match BoundCardinality::from_number(number) {
                    Some(bound) => min_properties = Some(bound),
                    None => return Ok(None),
                }
            }
            ("maxProperties", Value::Number(number))
                if ctx.draft().is_known_keyword("maxProperties") =>
            {
                match BoundCardinality::from_number(number) {
                    Some(bound) => max_properties = Some(bound),
                    None => return Ok(None),
                }
            }
            ("pattern", Value::String(text)) if ctx.draft().is_known_keyword("pattern") => {
                let pattern: Arc<str> = Arc::from(text.as_str());
                if ctx.compile_regex(&pattern).is_none() {
                    return Err(CanonicalizationError::InvalidPattern {
                        pattern: pattern.to_string(),
                    });
                }
                // `pattern` matches anywhere in the string, so an empty one matches every string.
                if !pattern.is_empty() {
                    patterns.push(pattern);
                }
            }
            // An annotation-only `format` constrains nothing, so it leaves no trace in the IR.
            ("format", Value::String(name)) if ctx.draft().is_known_keyword("format") => {
                if ctx.validate_formats() {
                    formats.push(StringFormat::from_name(ctx.draft(), name));
                }
            }
            // `contentEncoding`/`contentMediaType`/`contentSchema` are annotations from 2019-09 on -
            // no draft asserts them there, so they leave no trace in the IR.
            ("contentEncoding" | "contentMediaType" | "contentSchema", _)
                if matches!(
                    ctx.draft(),
                    Draft::Draft201909 | Draft::Draft202012 | Draft::Unknown
                ) => {}
            // Together the two decode-then-check the encoded string, which the leaf's independent
            // facets cannot spell - each alone checks the string it sits beside directly, so the
            // guard only fires when both keywords share this schema object.
            ("contentMediaType", Value::String(name))
                if matches!(ctx.draft(), Draft::Draft6 | Draft::Draft7)
                    && !map.contains_key("contentEncoding") =>
            {
                content_media_types.push(Arc::from(name.as_str()));
            }
            ("contentEncoding", Value::String(name))
                if matches!(ctx.draft(), Draft::Draft6 | Draft::Draft7)
                    && !map.contains_key("contentMediaType") =>
            {
                content_encodings.push(Arc::from(name.as_str()));
            }
            // Only a positive divisor whose spelling denotes an exact rational is modeled; without
            // one the validator's own division is what decides membership.
            ("multipleOf", Value::Number(number)) if ctx.draft().is_known_keyword("multipleOf") => {
                match BoundRational::new(number) {
                    Some(step) => multiple_of = Divisors::one(step),
                    None => return Ok(None),
                }
            }
            ("minimum", Value::Number(number)) if ctx.draft().is_known_keyword("minimum") => {
                real_minimum = tighter_real(real_minimum, number, true, Side::Lower);
            }
            ("maximum", Value::Number(number)) if ctx.draft().is_known_keyword("maximum") => {
                real_maximum = tighter_real(real_maximum, number, true, Side::Upper);
            }
            // Draft 6+ spells an exclusive bound as its own numeric keyword.
            ("exclusiveMinimum", Value::Number(number))
                if !matches!(ctx.draft(), Draft::Draft4)
                    && ctx.draft().is_known_keyword("exclusiveMinimum") =>
            {
                real_minimum = tighter_real(real_minimum, number, false, Side::Lower);
            }
            ("exclusiveMaximum", Value::Number(number))
                if !matches!(ctx.draft(), Draft::Draft4)
                    && ctx.draft().is_known_keyword("exclusiveMaximum") =>
            {
                real_maximum = tighter_real(real_maximum, number, false, Side::Upper);
            }
            ("exclusiveMinimum", Value::Bool(flag)) if matches!(ctx.draft(), Draft::Draft4) => {
                draft4_exclusive_minimum = *flag;
            }
            ("exclusiveMaximum", Value::Bool(flag)) if matches!(ctx.draft(), Draft::Draft4) => {
                draft4_exclusive_maximum = *flag;
            }
            ("if", value) if ctx.draft().is_known_keyword("if") => {
                match parse_schema(value, ctx, false, resolver, state)? {
                    Some(schema) => if_schema = Some(schema),
                    None => return Ok(None),
                }
            }
            ("then", value) if ctx.draft().is_known_keyword("then") => {
                match parse_schema(value, ctx, false, resolver, state)? {
                    Some(schema) => then_schema = Some(schema),
                    None => return Ok(None),
                }
            }
            ("else", value) if ctx.draft().is_known_keyword("else") => {
                match parse_schema(value, ctx, false, resolver, state)? {
                    Some(schema) => else_schema = Some(schema),
                    None => return Ok(None),
                }
            }
            // Property dependencies: each key, when held by an object, demands its consequent -
            // more required keys in the array form, a whole-value schema in the schema form. Every
            // draft validates `dependencies`, 2019-09 onward also under its split spellings.
            ("dependencies", Value::Object(entries)) => {
                for (key, entry) in entries {
                    match entry {
                        Value::Array(names) if names.iter().all(Value::is_string) => {
                            conjuncts.push(required_dependency(key, names, ctx));
                        }
                        value @ (Value::Object(_) | Value::Bool(_)) => {
                            match parse_schema(value, ctx, false, resolver, state)? {
                                Some(schema) => {
                                    conjuncts.push(schema_dependency(key, schema, ctx));
                                }
                                None => return Ok(None),
                            }
                        }
                        Value::Null | Value::Number(_) | Value::String(_) | Value::Array(_) => {
                            return Ok(None)
                        }
                    }
                }
            }
            ("dependentRequired", Value::Object(entries))
                if ctx.draft().is_known_keyword("dependentRequired") =>
            {
                for (key, entry) in entries {
                    match entry {
                        Value::Array(names) if names.iter().all(Value::is_string) => {
                            conjuncts.push(required_dependency(key, names, ctx));
                        }
                        Value::Null
                        | Value::Bool(_)
                        | Value::Number(_)
                        | Value::String(_)
                        | Value::Array(_)
                        | Value::Object(_) => return Ok(None),
                    }
                }
            }
            ("dependentSchemas", Value::Object(entries))
                if ctx.draft().is_known_keyword("dependentSchemas") =>
            {
                for (key, entry) in entries {
                    match entry {
                        value @ (Value::Object(_) | Value::Bool(_)) => {
                            match parse_schema(value, ctx, false, resolver, state)? {
                                Some(schema) => {
                                    conjuncts.push(schema_dependency(key, schema, ctx));
                                }
                                None => return Ok(None),
                            }
                        }
                        Value::Null | Value::Number(_) | Value::String(_) | Value::Array(_) => {
                            return Ok(None)
                        }
                    }
                }
            }
            // Cancel a syntactic double complement before parsing its body, avoiding symbolic De Morgan expansion.
            ("not", Value::Object(inner))
                if ctx.draft().is_known_keyword("not")
                    && inner.len() == 1
                    && inner.contains_key("not") =>
            {
                let body = inner
                    .get("not")
                    .expect("the double-complement guard found its body");
                match parse_schema(body, ctx, false, resolver, state)? {
                    Some(schema) => conjuncts.push(schema),
                    None => return Ok(None),
                }
            }
            // The complement of the negated schema, when the IR can spell it; an unmodeled child or
            // an inexpressible complement keeps the whole document raw.
            ("not", value) if ctx.draft().is_known_keyword("not") => {
                if matches!(ctx.draft(), Draft::Draft4) && is_closed_pattern_map(value) {
                    return Ok(None);
                }
                match parse_schema(value, ctx, false, resolver, state)? {
                    Some(child) => match negate::negate_in_place(&child, &state.definitions, ctx) {
                        Some(complement) => conjuncts.push(complement),
                        None => return Ok(None),
                    },
                    None => return Ok(None),
                }
            }
            // TODO(canonical): not modeled yet - every other known keyword keeps the document raw.
            (other, _) if ctx.draft().is_known_keyword(other) => return Ok(None),
            _ => {}
        }
    }

    if draft4_exclusive_minimum {
        real_minimum = real_minimum.map(BoundNumber::excluded);
    }
    if draft4_exclusive_maximum {
        real_maximum = real_maximum.map(BoundNumber::excluded);
    }

    // `minLength: 0` is the type-default, so drop it: the leaf then compares equal to one without it.
    if min_length.as_ref().is_some_and(BoundCardinality::is_zero) {
        min_length = None;
    }
    if min_length.is_some()
        || max_length.is_some()
        || !patterns.is_empty()
        || !formats.is_empty()
        || !content_media_types.is_empty()
        || !content_encodings.is_empty()
    {
        patterns.sort();
        patterns.dedup();
        formats.sort();
        formats.dedup();
        content_media_types.sort();
        content_media_types.dedup();
        content_encodings.sort();
        content_encodings.dedup();
        let leaf = StringLeaf {
            lengths: LengthBounds {
                minimum: min_length,
                maximum: max_length,
            },
            patterns,
            excluded_patterns: Vec::new(),
            formats,
            excluded_formats: Vec::new(),
            content_media_types,
            content_encodings,
            excluded: Vec::new(),
        };
        conjuncts.push(string_facet_schema(leaf, ctx));
    }

    // `minItems: 0` is the type-default, so drop it: the window then compares equal to one without it.
    if min_items.as_ref().is_some_and(BoundCardinality::is_zero) {
        min_items = None;
    }
    // A tuple's tail is spelled `additionalItems` before 2020-12 and schema-form `items` in it. A
    // schema-form `items` with no tuple constrains every element, so it is the tail of an empty
    // prefix, and `additionalItems` is then inert.
    let (prefix, tail) = match item_prefix {
        Some(prefix)
            if matches!(
                ctx.draft(),
                Draft::Draft4 | Draft::Draft6 | Draft::Draft7 | Draft::Draft201909
            ) =>
        {
            let tail = match additional_items {
                Some(value) => match parse_schema(value, ctx, false, resolver, state)? {
                    Some(schema) => Some(schema),
                    None => return Ok(None),
                },
                None => None,
            };
            (prefix, tail)
        }
        Some(prefix) => {
            debug_assert!(
                !matches!(map.get("items"), Some(Value::Array(_))),
                "a prefix spelled `prefixItems` leaves no array-form `items` for `additionalItems` to tail"
            );
            (prefix, items)
        }
        None => {
            debug_assert!(
                !matches!(map.get("items"), Some(Value::Array(_))),
                "an array-form `items` either builds a prefix or keeps the document raw"
            );
            (Vec::new(), items)
        }
    };
    // `minContains`/`maxContains` constrain the `contains` count and say nothing without it.
    let contains: Vec<ContainsFacet> = contains_schema
        .map(|schema| ContainsFacet {
            schema,
            minimum: min_contains,
            maximum: max_contains,
        })
        .into_iter()
        .collect();
    if min_items.is_some()
        || max_items.is_some()
        || !matches!(distinctness, Distinctness::Unconstrained)
        || !prefix.is_empty()
        || tail.is_some()
        || !contains.is_empty()
    {
        conjuncts.push(array_facet_schema(
            ArrayLeaf {
                lengths: LengthBounds {
                    minimum: min_items,
                    maximum: max_items,
                },
                distinctness,
                prefix,
                items: tail,
                contains,
            },
            ctx,
        ));
    }

    // `minProperties: 0` is the type-default, so drop it: the window then compares equal to one without it.
    if min_properties
        .as_ref()
        .is_some_and(BoundCardinality::is_zero)
    {
        min_properties = None;
    }
    // A pattern matching finitely many keys names them outright, so its schema moves onto them and
    // the pattern goes. What is left decides whether this document meets the `additionalProperties`
    // pairing at all - `additionalProperties` already knows to skip a named key.
    fold_finite_key_patterns(&mut pattern_properties, &mut properties, ctx);
    // `additionalProperties: false` forbids every key the property map does not name and no
    // pattern matches, which a key constraint spells: the named keys and the patterns' keys,
    // met into any stored constraint.
    // e.g.  {"type": "object", "properties": {"a": {"type": "string"}}, "additionalProperties": false}
    //       =>  {"type": "object", "propertyNames": {"const": "a"}, "properties": {"a": {"type": "string"}}}
    if forbid_unmatched_keys {
        let mut allowed: Vec<Schema> =
            Vec::with_capacity(properties.len() + pattern_properties.len());
        for key in properties.keys() {
            allowed.push(Schema::new(SchemaKind::Const(CanonicalJson::from_value(
                &Value::String(key.to_string()),
            ))));
        }
        for pattern in pattern_properties.keys() {
            // An empty pattern matches every key, so it names them all rather than a subset.
            let patterns = if pattern.is_empty() {
                Vec::new()
            } else {
                vec![Arc::clone(pattern)]
            };
            allowed.push(algebra::string_leaf(
                StringLeaf {
                    lengths: LengthBounds::default(),
                    patterns,
                    excluded_patterns: Vec::new(),
                    formats: Vec::new(),
                    excluded_formats: Vec::new(),
                    content_media_types: Vec::new(),
                    content_encodings: Vec::new(),
                    excluded: Vec::new(),
                },
                ctx,
            ));
        }
        let allowed = algebra::union(allowed, ctx);
        property_names = Some(match property_names.take() {
            Some(names) => algebra::intersect(names, allowed, ctx),
            None => allowed,
        });
    }
    if min_properties.is_some()
        || max_properties.is_some()
        || !required.is_empty()
        || property_names.is_some()
        || !properties.is_empty()
        || !pattern_properties.is_empty()
        || additional_schema.is_some()
    {
        // Every draft marks `required` as unique, so the meta-validated list only needs ordering.
        required.sort();
        conjuncts.push(object_facet_schema(
            ObjectLeaf {
                sizes: LengthBounds {
                    minimum: min_properties,
                    maximum: max_properties,
                },
                required,
                property_names,
                properties,
                pattern_properties,
                additional: additional_schema,
                violations: Vec::new(),
            },
            ctx,
        ));
    }

    if real_minimum.is_some() || real_maximum.is_some() || !multiple_of.is_empty() {
        let leaf = NumberLeaf {
            minimum: real_minimum,
            maximum: real_maximum,
            multiple_of,
            not_multiple_of: ExcludedDivisors::default(),
            excludes_integers: false,
        };
        // The integers the interval admits must be representable: the interval may still meet
        // `integer` through an `allOf`, and there it is the only form left to express.
        let Some(bounds) = algebra::integer_bounds_within(&leaf) else {
            return Ok(None);
        };
        if type_set == Some(JsonTypeSet::from(JsonType::Integer)) {
            conjuncts.push(algebra::integer_leaf(
                IntegerLeaf {
                    bounds,
                    multiple_of: leaf.multiple_of,
                    not_multiple_of: ExcludedDivisors::default(),
                },
                ctx,
            ));
        } else {
            conjuncts.push(number_facet_schema(leaf, ctx));
        }
    }

    // `then`/`else` apply only beside a sibling `if`; either alone is an annotation with no effect.
    match (if_schema, then_schema, else_schema) {
        (None, _, _) | (Some(_), None, None) => {}
        // ¬if ∨ then: a value the condition rejects needs nothing further.
        (Some(condition), Some(then), None) => {
            match negate::negate_in_place(&condition, &state.definitions, ctx) {
                Some(complement) => conjuncts.push(algebra::union(vec![complement, then], ctx)),
                None => return Ok(None),
            }
        }
        // if ∨ else: a value the condition admits needs nothing further, so the complement is
        // never needed - unlike every other arm here, this one cannot force the document raw.
        (Some(condition), None, Some(else_branch)) => {
            conjuncts.push(algebra::union(vec![condition, else_branch], ctx));
        }
        // (if ∧ then) ∨ (¬if ∧ else)
        (Some(condition), Some(then), Some(else_branch)) => {
            match negate::negate_in_place(&condition, &state.definitions, ctx) {
                Some(complement) => {
                    let holds = algebra::intersect(condition, then, ctx);
                    let fails = algebra::intersect(complement, else_branch, ctx);
                    conjuncts.push(algebra::union(vec![holds, fails], ctx));
                }
                None => return Ok(None),
            }
        }
    }

    let base = match (type_set, admitted_values(enum_values, const_value)) {
        (None, None) => Schema::truthy(),
        (Some(set), None) => type_set_schema(set),
        (None, Some(values)) => canonicalize_value_set(values),
        (Some(set), Some(values)) => restrict_values_to_types(values, set, ctx),
    };
    // A schema object's keywords all apply to the same value at once, so combine them by intersection.
    Ok(Some(
        conjuncts.into_iter().fold(base, |result, conjunct| {
            algebra::intersect(result, conjunct, ctx)
        }),
    ))
}

/// Whether this schema has the Draft 4 closed-map spelling that negation must preserve directly.
fn is_closed_pattern_map(value: &Value) -> bool {
    let Some(map) = value.as_object() else {
        return false;
    };
    map.get("additionalProperties") == Some(&Value::Bool(false))
        && map
            .get("patternProperties")
            .and_then(Value::as_object)
            .is_some_and(|patterns| !patterns.is_empty())
}

/// Move every pattern matching finitely many keys onto those keys, met into whatever the property
/// map already demands of each, and drop the pattern.
fn fold_finite_key_patterns(
    pattern_properties: &mut PropertyMap,
    properties: &mut PropertyMap,
    ctx: &CanonicalizationContext,
) {
    pattern_properties.retain(|pattern, schema| {
        let Some(keys) = finite_pattern_keys(pattern) else {
            return true;
        };
        for key in keys {
            let merged = match properties.remove(&key) {
                Some(existing) => algebra::intersect(existing, schema.clone(), ctx),
                None => schema.clone(),
            };
            properties.insert(key, merged);
        }
        false
    });
}

/// The keys a pattern matches when it matches finitely many; `^` and `$` anchor the whole string,
/// so an exact or alternation spelling names its keys outright.
fn finite_pattern_keys(pattern: &str) -> Option<Vec<Arc<str>>> {
    match jsonschema_regex::analyze_pattern(pattern)? {
        jsonschema_regex::PatternAnalysis::Exact(key) => Some(vec![Arc::from(key.as_ref())]),
        jsonschema_regex::PatternAnalysis::Alternation(keys) => {
            Some(keys.iter().map(|key| Arc::from(key.as_str())).collect())
        }
        jsonschema_regex::PatternAnalysis::Prefix(_)
        | jsonschema_regex::PatternAnalysis::NoWhitespace => None,
    }
}

/// In-place applicators whose annotations depend on which branch the instance matched. `allOf` is
/// absent: every branch must pass, so [`property_cover`] can read its contribution off the document.
/// `not` is absent too: it succeeds only when its subschema fails, and a failure annotates nothing.
/// Whether an in-place applicator sits here that no cover reads. `anyOf`/`oneOf` are absent because
/// the cover handles them, taking their branches only when those agree.
fn has_unresolved_applicator(map: &serde_json::Map<String, Value>) -> bool {
    map.keys().any(|key| {
        matches!(
            key.as_str(),
            "$dynamicRef"
                | "$recursiveRef"
                | "dependencies"
                | "dependentSchemas"
                | "else"
                | "if"
                | "then"
        )
    })
}

fn has_instance_dependent_applicator(map: &serde_json::Map<String, Value>) -> bool {
    map.keys().any(|key| {
        matches!(
            key.as_str(),
            "$dynamicRef"
                | "$recursiveRef"
                | "anyOf"
                | "dependencies"
                | "dependentSchemas"
                | "else"
                | "if"
                | "oneOf"
                | "then"
        )
    })
}

/// The keys an in-place applicator evaluates beside an `unevaluatedProperties`.
#[derive(Default, PartialEq, Eq)]
struct PropertyCover {
    /// A branch reaches every key, leaving the `unevaluatedProperties` inert.
    everything: bool,
    keys: Vec<String>,
    patterns: Vec<String>,
}

impl PropertyCover {
    /// Spell one cover one way, so two of them compare by what they reach.
    fn normalize(&mut self) {
        for names in [&mut self.keys, &mut self.patterns] {
            names.sort();
            names.dedup();
        }
    }

    fn absorb(&mut self, other: Self) {
        self.everything |= other.everything;
        self.keys.extend(other.keys);
        self.patterns.extend(other.patterns);
    }
}

/// Name each covered key and pattern here as a vacuous entry, so the `additional*` twin skips it.
fn hoist_cover(degraded: &mut serde_json::Map<String, Value>, cover: &PropertyCover) {
    for (keyword, names) in [
        ("properties", &cover.keys),
        ("patternProperties", &cover.patterns),
    ] {
        if names.is_empty() {
            continue;
        }
        let Value::Object(entries) = degraded
            .entry(keyword)
            .or_insert_with(|| Value::Object(serde_json::Map::new()))
        else {
            continue;
        };
        for name in names {
            entries.entry(name.as_str()).or_insert(Value::Bool(true));
        }
    }
}

/// An on-path cycle guard (push/pop around a `$ref` fold) plus a total-fold budget - a diamond-shaped
/// `$ref` graph re-walks a shared subtree once per path reaching it, unbounded by the guard alone.
struct ReferenceWalk {
    visited: AHashSet<Arc<str>>,
    budget: u32,
}

/// Folds one `unevaluated*` computation may perform before giving up and leaving the document `Raw`.
const REFERENCE_FOLD_BUDGET: u32 = 10_000;

impl ReferenceWalk {
    fn new() -> Self {
        Self {
            visited: AHashSet::default(),
            budget: REFERENCE_FOLD_BUDGET,
        }
    }
}

/// Accumulate what `branch` evaluates; `None` when the instance decides it.
fn property_cover(
    branch: &Value,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'_>,
    walk: &mut ReferenceWalk,
    cover: &mut PropertyCover,
) -> Result<Option<()>, CanonicalizationError> {
    let map = match branch {
        // No keywords, so nothing is annotated.
        Value::Bool(_) => return Ok(Some(())),
        Value::Object(map) => map,
        Value::Null | Value::Number(_) | Value::String(_) | Value::Array(_) => return Ok(None),
    };
    // A branch may carry its own `$id`, shifting the base for a `$ref` inside it - same shift
    // `parse_schema` performs for every node it visits.
    let resolver = resolver.in_subresource(ctx.draft().create_resource_ref(branch))?;
    property_cover_in_scope(map, ctx, &resolver, walk, cover)
}

/// [`property_cover`]'s body, once scope is settled - also used for a resolved `$ref` target, whose
/// resolver `resolver.lookup` already scoped.
fn property_cover_in_scope(
    map: &serde_json::Map<String, Value>,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'_>,
    walk: &mut ReferenceWalk,
    cover: &mut PropertyCover,
) -> Result<Option<()>, CanonicalizationError> {
    if has_instance_dependent_applicator(map) {
        return Ok(None);
    }
    // `additionalProperties` is never rewritten, so its presence is a stable "reaches everything".
    if map.contains_key("additionalProperties") {
        cover.everything = true;
        return Ok(Some(()));
    }
    if map.contains_key("unevaluatedProperties") {
        // Not yet degraded, so crediting it outright would let a $ref cycle through this node credit
        // itself before `walk` catches the cycle. Recurse instead - a cycle fails via `walk`.
        let Some(_) = sibling_property_cover(map, ctx, resolver, walk)? else {
            return Ok(None);
        };
        cover.everything = true;
        return Ok(Some(()));
    }
    if let Some(Value::Object(properties)) = map.get("properties") {
        cover.keys.extend(properties.keys().cloned());
    }
    if let Some(Value::Object(patterns)) = map.get("patternProperties") {
        cover.patterns.extend(patterns.keys().cloned());
    }
    if let Some(Value::Array(nested)) = map.get("allOf") {
        for branch in nested {
            let Some(()) = property_cover(branch, ctx, resolver, walk, cover)? else {
                return Ok(None);
            };
        }
    }
    if let Some(Value::String(reference)) = map.get("$ref") {
        let Some(()) = fold_referenced_property_cover(reference, ctx, resolver, walk, cover)?
        else {
            return Ok(None);
        };
    }
    Ok(Some(()))
}

/// Fold `reference`'s raw target cover in, unconditionally - a `$ref` behaves as one more `allOf`
/// conjunct. Dispatches into [`property_cover_in_scope`] directly (not [`property_cover`], which
/// would shift scope a second time onto a base `resolver.lookup` already moved).
fn fold_referenced_property_cover(
    reference: &str,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'_>,
    walk: &mut ReferenceWalk,
    cover: &mut PropertyCover,
) -> Result<Option<()>, CanonicalizationError> {
    let Some(remaining) = walk.budget.checked_sub(1) else {
        return Ok(None);
    };
    walk.budget = remaining;
    let location = resolver.resolve_uri(&resolver.base_uri().borrow(), reference)?;
    let key: Arc<str> = Arc::from(location.as_str());
    if !walk.visited.insert(Arc::clone(&key)) {
        return Ok(None);
    }
    let result = (|| {
        let (target, target_resolver, target_draft) = resolver.lookup(reference)?.into_inner();
        if target_draft != ctx.draft() {
            return Ok(None);
        }
        match target {
            // No keywords, so nothing is annotated.
            Value::Bool(_) => Ok(Some(())),
            Value::Object(map) => property_cover_in_scope(map, ctx, &target_resolver, walk, cover),
            Value::Null | Value::Number(_) | Value::String(_) | Value::Array(_) => Ok(None),
        }
    })();
    walk.visited.remove(&key);
    result
}

/// The indexes an in-place applicator evaluates beside an `unevaluatedItems`.
#[derive(Default, PartialEq, Eq)]
struct ItemCover {
    /// A branch reaches every index, leaving the `unevaluatedItems` inert.
    everything: bool,
    /// The longest tuple prefix any branch evaluates.
    prefix: usize,
}

impl ItemCover {
    fn absorb(&mut self, other: &Self) {
        self.everything |= other.everything;
        self.prefix = self.prefix.max(other.prefix);
    }
}

/// Extend the local tuple so the tail keyword starts past every index the branches evaluate; the
/// branch that evaluated an index still carries its constraint.
fn pad_tuple(degraded: &mut serde_json::Map<String, Value>, prefix_items: bool, prefix: usize) {
    if prefix == 0 {
        return;
    }
    let keyword = if prefix_items { "prefixItems" } else { "items" };
    let Value::Array(tuple) = degraded
        .entry(keyword)
        .or_insert_with(|| Value::Array(Vec::new()))
    else {
        return;
    };
    tuple.resize(tuple.len().max(prefix), Value::Bool(true));
}

/// Accumulate the indexes `branch` evaluates; `None` when the instance decides them.
fn item_cover(
    branch: &Value,
    draft: Draft,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'_>,
    walk: &mut ReferenceWalk,
    cover: &mut ItemCover,
) -> Result<Option<()>, CanonicalizationError> {
    let map = match branch {
        // No keywords, so nothing is annotated.
        Value::Bool(_) => return Ok(Some(())),
        Value::Object(map) => map,
        Value::Null | Value::Number(_) | Value::String(_) | Value::Array(_) => return Ok(None),
    };
    let resolver = resolver.in_subresource(ctx.draft().create_resource_ref(branch))?;
    item_cover_in_scope(map, draft, ctx, &resolver, walk, cover)
}

/// [`item_cover`]'s body - same split, same reason, as [`property_cover_in_scope`].
fn item_cover_in_scope(
    map: &serde_json::Map<String, Value>,
    draft: Draft,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'_>,
    walk: &mut ReferenceWalk,
    cover: &mut ItemCover,
) -> Result<Option<()>, CanonicalizationError> {
    if has_instance_dependent_applicator(map) {
        return Ok(None);
    }
    // `contains` marks the indexes it matches, which no prefix length spells.
    if map.contains_key("contains") {
        return Ok(None);
    }
    // Same reasoning as the property-cover twin's `unevaluatedProperties` branch.
    if map.contains_key("unevaluatedItems") {
        let Some(_) = sibling_item_cover(map, draft, ctx, resolver, walk)? else {
            return Ok(None);
        };
        cover.everything = true;
        return Ok(Some(()));
    }
    let tuple_is_prefix_items = matches!(draft, Draft::Draft202012 | Draft::Unknown);
    match map.get("items") {
        // Schema-form `items` reaches every index past the tuple.
        Some(Value::Object(_) | Value::Bool(_)) => {
            cover.everything = true;
            return Ok(Some(()));
        }
        Some(Value::Array(items)) if !tuple_is_prefix_items => {
            cover.prefix = cover.prefix.max(items.len());
            if map.contains_key("additionalItems") {
                cover.everything = true;
                return Ok(Some(()));
            }
        }
        // An array `items` is not a tuple in 2020-12, where the parse loop keeps it raw.
        Some(Value::Array(_) | Value::Null | Value::Number(_) | Value::String(_)) | None => {}
    }
    if tuple_is_prefix_items {
        if let Some(Value::Array(prefix)) = map.get("prefixItems") {
            cover.prefix = cover.prefix.max(prefix.len());
        }
    }
    if let Some(Value::Array(nested)) = map.get("allOf") {
        for branch in nested {
            let Some(()) = item_cover(branch, draft, ctx, resolver, walk, cover)? else {
                return Ok(None);
            };
        }
    }
    if let Some(Value::String(reference)) = map.get("$ref") {
        let Some(()) = fold_referenced_item_cover(reference, draft, ctx, resolver, walk, cover)?
        else {
            return Ok(None);
        };
    }
    Ok(Some(()))
}

/// The item-cover twin of [`fold_referenced_property_cover`].
fn fold_referenced_item_cover(
    reference: &str,
    draft: Draft,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'_>,
    walk: &mut ReferenceWalk,
    cover: &mut ItemCover,
) -> Result<Option<()>, CanonicalizationError> {
    let Some(remaining) = walk.budget.checked_sub(1) else {
        return Ok(None);
    };
    walk.budget = remaining;
    let location = resolver.resolve_uri(&resolver.base_uri().borrow(), reference)?;
    let key: Arc<str> = Arc::from(location.as_str());
    if !walk.visited.insert(Arc::clone(&key)) {
        return Ok(None);
    }
    let result = (|| {
        let (target, target_resolver, target_draft) = resolver.lookup(reference)?.into_inner();
        if target_draft != ctx.draft() {
            return Ok(None);
        }
        match target {
            // No keywords, so nothing is annotated.
            Value::Bool(_) => Ok(Some(())),
            Value::Object(map) => {
                item_cover_in_scope(map, draft, ctx, &target_resolver, walk, cover)
            }
            Value::Null | Value::Number(_) | Value::String(_) | Value::Array(_) => Ok(None),
        }
    })();
    walk.visited.remove(&key);
    result
}

/// The keys the in-place applicators beside an `unevaluatedProperties` evaluate. Every `allOf`
/// branch succeeds, so their covers add up; a bare `$ref` sibling composes the same way. Alternatives
/// pin one only when each branch reaches the same keys, since otherwise which branch matched decides
/// what is left over.
fn sibling_property_cover(
    map: &serde_json::Map<String, Value>,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'_>,
    walk: &mut ReferenceWalk,
) -> Result<Option<PropertyCover>, CanonicalizationError> {
    let mut cover = PropertyCover::default();
    if let Some(Value::String(reference)) = map.get("$ref") {
        let Some(()) = fold_referenced_property_cover(reference, ctx, resolver, walk, &mut cover)?
        else {
            return Ok(None);
        };
    }
    if let Some(Value::Array(branches)) = map.get("allOf") {
        for branch in branches {
            let Some(()) = property_cover(branch, ctx, resolver, walk, &mut cover)? else {
                return Ok(None);
            };
        }
    }
    for keyword in ["anyOf", "oneOf"] {
        let Some(Value::Array(branches)) = map.get(keyword) else {
            continue;
        };
        let mut agreed: Option<PropertyCover> = None;
        for branch in branches {
            let mut reached = PropertyCover::default();
            let Some(()) = property_cover(branch, ctx, resolver, walk, &mut reached)? else {
                return Ok(None);
            };
            reached.normalize();
            match &agreed {
                Some(first) if *first != reached => return Ok(None),
                Some(_) => {}
                None => agreed = Some(reached),
            }
        }
        if let Some(agreed) = agreed {
            cover.absorb(agreed);
        }
    }
    Ok(Some(cover))
}

/// The indexes the in-place applicators beside an `unevaluatedItems` evaluate, on the same terms as
/// the key cover.
fn sibling_item_cover(
    map: &serde_json::Map<String, Value>,
    draft: Draft,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'_>,
    walk: &mut ReferenceWalk,
) -> Result<Option<ItemCover>, CanonicalizationError> {
    let mut cover = ItemCover::default();
    if let Some(Value::String(reference)) = map.get("$ref") {
        let Some(()) =
            fold_referenced_item_cover(reference, draft, ctx, resolver, walk, &mut cover)?
        else {
            return Ok(None);
        };
    }
    if let Some(Value::Array(branches)) = map.get("allOf") {
        for branch in branches {
            let Some(()) = item_cover(branch, draft, ctx, resolver, walk, &mut cover)? else {
                return Ok(None);
            };
        }
    }
    for keyword in ["anyOf", "oneOf"] {
        let Some(Value::Array(branches)) = map.get(keyword) else {
            continue;
        };
        let mut agreed: Option<ItemCover> = None;
        for branch in branches {
            let mut reached = ItemCover::default();
            let Some(()) = item_cover(branch, draft, ctx, resolver, walk, &mut reached)? else {
                return Ok(None);
            };
            match &agreed {
                Some(first) if *first != reached => return Ok(None),
                Some(_) => {}
                None => agreed = Some(reached),
            }
        }
        if let Some(agreed) = &agreed {
            cover.absorb(agreed);
        }
    }
    Ok(Some(cover))
}

/// Whether this object asserts an `unevaluated*` keyword. Both enter the vocabulary in the same
/// draft, so one `is_known_keyword` answer covers the pair.
fn has_unevaluated(map: &serde_json::Map<String, Value>, draft: Draft) -> bool {
    draft.is_known_keyword("unevaluatedProperties")
        && (map.contains_key("unevaluatedProperties") || map.contains_key("unevaluatedItems"))
}

/// Rewrite every asserted `unevaluated*` into its `additional*` twin. With no in-place applicator
/// beside it, `unevaluated*` sees exactly the keys or indices its twin sees; a live twin already
/// evaluates all of them, leaving it inert and dropped. `None` keeps the document raw.
fn degrade_unevaluated(
    map: &serde_json::Map<String, Value>,
    draft: Draft,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'_>,
) -> Result<Option<Value>, CanonicalizationError> {
    let mut no_op_stripped = map.clone();
    for key in ["unevaluatedProperties", "unevaluatedItems"] {
        if matches!(no_op_stripped.get(key), Some(Value::Bool(true))) {
            no_op_stripped.remove(key);
        }
    }
    if no_op_stripped.len() != map.len() {
        return Ok(Some(Value::Object(no_op_stripped)));
    }
    if has_unresolved_applicator(map) {
        return Ok(None);
    }
    let mut degraded = map.clone();
    // A local `additionalProperties` leaves nothing unevaluated, so the sibling is inert - and
    // hoisting would change which keys that `additionalProperties` reaches, so it must not run.
    if let Some(value) = degraded
        .remove("unevaluatedProperties")
        .filter(|_| !degraded.contains_key("additionalProperties"))
    {
        // Naming a covered key here is all `additionalProperties` needs to skip it; the branch
        // that named it still carries the constraint.
        let mut walk = ReferenceWalk::new();
        let Some(cover) = sibling_property_cover(map, ctx, resolver, &mut walk)? else {
            return Ok(None);
        };
        if !cover.everything {
            hoist_cover(&mut degraded, &cover);
            degraded.insert("additionalProperties".to_string(), value);
        }
    }
    if let Some(value) = degraded.remove("unevaluatedItems") {
        // An element `contains` matches is evaluated by it, so the tail takes either.
        // e.g.  {"contains": {"type": "integer"}, "unevaluatedItems": {"type": "string"}}
        //       =>  {"contains": {"type": "integer"},
        //            "items": {"anyOf": [{"type": "integer"}, {"type": "string"}]}}
        let value = match map.get("contains") {
            Some(contains @ (Value::Object(_) | Value::Bool(_))) => {
                let mut tail = serde_json::Map::new();
                tail.insert(
                    "anyOf".to_string(),
                    Value::Array(vec![contains.clone(), value]),
                );
                Value::Object(tail)
            }
            // A `contains` that is not a schema keeps the document raw anyway.
            Some(Value::Array(_) | Value::Null | Value::Number(_) | Value::String(_)) => {
                return Ok(None)
            }
            None => value,
        };
        let mut walk = ReferenceWalk::new();
        let Some(cover) = sibling_item_cover(map, draft, ctx, resolver, &mut walk)? else {
            return Ok(None);
        };
        if !cover.everything {
            // A tuple's tail is `additionalItems` before 2020-12 and schema-form `items` in it,
            // where the tuple itself is `prefixItems`. A schema-form `items` already reaches every
            // index past the tuple, leaving nothing for the twin.
            let tuple_is_prefix_items = matches!(draft, Draft::Draft202012 | Draft::Unknown);
            let tail = match (map.get("items"), tuple_is_prefix_items) {
                // A branch prefix needs a local tuple to sit behind, spelled `items` before 2020-12.
                (None, false) if cover.prefix > 0 => Some("additionalItems"),
                (None, _) => Some("items"),
                (Some(Value::Array(_)), false) => Some("additionalItems"),
                (
                    Some(
                        Value::Object(_)
                        | Value::Bool(_)
                        | Value::Array(_)
                        | Value::Null
                        | Value::Number(_)
                        | Value::String(_),
                    ),
                    _,
                ) => None,
            };
            if let Some(tail) = tail.filter(|tail| !degraded.contains_key(*tail)) {
                pad_tuple(&mut degraded, tuple_is_prefix_items, cover.prefix);
                degraded.insert(tail.to_string(), value);
            }
        }
    }
    // No asserted `unevaluated*` survives, so re-parsing this map terminates.
    Ok(Some(Value::Object(degraded)))
}

fn ref_has_assertion_siblings(map: &serde_json::Map<String, Value>, draft: Draft) -> bool {
    map.keys().any(|key| {
        !matches!(
            key.as_str(),
            "$ref"
                // Consumed beside `$ref`, so never left for the sibling parse.
                | "$dynamicRef"
                | "$recursiveRef"
                | "$schema"
                | "$id"
                | "id"
                | "$anchor"
                | "$dynamicAnchor"
                | "$recursiveAnchor"
                | "$defs"
                | "definitions"
                | "title"
                | "description"
                | "default"
                | "examples"
        ) && draft.is_known_keyword(key)
    })
}

/// The conjunction of the reference keywords an object spells, if any.
fn combine_references(references: Vec<Schema>, ctx: &CanonicalizationContext) -> Option<Schema> {
    let mut references = references.into_iter();
    let first = references.next()?;
    Some(references.fold(first, |left, right| algebra::intersect(left, right, ctx)))
}

/// A `$recursiveRef`, the 2019-09 spelling.
///
/// [`Resolver::lookup_recursive_ref`] follows the scope while each resource carries
/// `$recursiveAnchor: true`. Absent or `false`, it behaves as `$ref: "#"`.
fn resolve_recursive_reference<'a>(
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'a>,
    state: &mut ParseState<'a>,
) -> Result<Option<Schema>, CanonicalizationError> {
    let base_uri = resolver.base_uri();
    let location = resolver.resolve_uri(&base_uri.borrow(), "#")?;
    let resolved = resolver.lookup_recursive_ref()?;
    reference_to_definition("#", location.as_str(), resolved, ctx, state)
}

fn resolve_reference<'a>(
    reference: &str,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'a>,
    state: &mut ParseState<'a>,
) -> Result<Option<Schema>, CanonicalizationError> {
    let base_uri = resolver.base_uri();
    let location = resolver.resolve_uri(&base_uri.borrow(), reference)?;
    let resolved = resolver.lookup(reference)?;
    reference_to_definition(reference, location.as_str(), resolved, ctx, state)
}

/// Turn a resolved target into a symbolic reference plus the definition it names.
///
/// A dynamic reference's `location` is its lexical bookend, not where the scope walk landed, so
/// `reached_dynamically` is what separates the paths.
fn reference_to_definition<'a>(
    reference: &str,
    location: &str,
    resolved: referencing::Resolved<'a>,
    ctx: &CanonicalizationContext,
    state: &mut ParseState<'a>,
) -> Result<Option<Schema>, CanonicalizationError> {
    state.facts.has_references = true;
    let (target, target_resolver, target_draft) = resolved.into_inner();
    let env = if state.dynamic_scope.tracked() {
        dynamic_scope_digest(&target_resolver, ctx.draft())?
    } else {
        empty_environment()
    };
    // Sites inside the root parse see the root resource as their outermost scope entry, so a
    // back-reference whose digest binds everything to the root resource observes exactly what
    // those sites did; any other binding differs and mints its own definition below.
    if std::ptr::eq(target, state.root)
        && env
            .iter()
            .all(|(_, resource)| *resource == state.root_base_uri)
    {
        // The root is never keyed, so the fixpoint names it by the spelling emitted here.
        if state.assumes_empty(ROOT_DEFINITION_KEY) {
            return Ok(Some(Schema::falsy()));
        }
        if state.assumes_admits_all(ROOT_DEFINITION_KEY) {
            return Ok(Some(Schema::truthy()));
        }
        return Ok(Some(Schema::new(SchemaKind::Reference(Arc::from(
            ROOT_DEFINITION_KEY,
        )))));
    }
    if target_draft != ctx.draft() {
        return Ok(None);
    }
    let raw_key = canonical_reference_uri(reference, location, &state.root_base_uri);
    if !ensure_definition(&raw_key, target, ctx, &target_resolver, &env, state)? {
        return Ok(None);
    }
    // `ensure_definition` inserted under the specialized key; the reference must name that one.
    let key = specialized_key(&raw_key, &env);
    debug_assert!(
        state.definitions.contains_key(&key) || state.in_progress.contains(&key),
        "a resolved reference target is complete or actively being canonicalized"
    );
    // Unlike the fold below, canonical URIs are not exempt: a cycle closed through an `$id`-bearing
    // subresource is keyed by a minted URI, and exempting it would leave it live once proven empty.
    if state.assumes_empty(&key) {
        return Ok(Some(Schema::falsy()));
    }
    if state.assumes_admits_all(&key) {
        return Ok(Some(Schema::truthy()));
    }
    // Folding an empty target lets the surrounding leaf normalization see the contradiction:
    // `required: ["a"]` beside `properties: {"a": false}` collapses, a symbolic `Reference` does
    // not. A canonical URI is exempt because it must keep its local definition to stay idempotent.
    if !key.starts_with(CANONICAL_REFERENCE_PREFIX)
        && state
            .definitions
            .get(&key)
            .is_some_and(|body| matches!(body.kind(), SchemaKind::False))
    {
        return Ok(Some(Schema::falsy()));
    }
    Ok(Some(Schema::new(SchemaKind::Reference(key))))
}

fn canonical_reference_uri(reference: &str, location: &str, root_base_uri: &str) -> Arc<str> {
    for prefix in ["#/$defs/", "#/definitions/"] {
        let Some(encoded) = reference.strip_prefix(prefix) else {
            continue;
        };
        if encoded.starts_with(CANONICAL_REFERENCE_PREFIX) {
            if !encoded.contains('%') {
                let uri = referencing::unescape_segment(encoded);
                return Arc::from(uri.as_ref());
            }
        } else {
            match encoded.bytes().find(|byte| matches!(byte, b'%' | b'/')) {
                None if !encoded.is_empty()
                    && resource_uri(location) == resource_uri(root_base_uri) =>
                {
                    return Arc::from(reference);
                }
                Some(b'%') => {}
                None | Some(_) => break,
            }
        }
        if let Ok(decoded) = percent_encoding::percent_decode_str(encoded).decode_utf8() {
            if decoded.starts_with(CANONICAL_REFERENCE_PREFIX) {
                let uri = referencing::unescape_segment(&decoded);
                return Arc::from(uri.as_ref());
            }
            if !decoded.is_empty()
                && !decoded.contains('/')
                && resource_uri(location) == resource_uri(root_base_uri)
            {
                return Arc::from(reference);
            }
        }
        break;
    }
    if location.starts_with(CANONICAL_REFERENCE_PREFIX) {
        return Arc::from(location);
    }
    let location =
        percent_encoding::utf8_percent_encode(location, percent_encoding::NON_ALPHANUMERIC);
    let uri = format!("{CANONICAL_REFERENCE_PREFIX}{location}");
    let uri = referencing::uri::from_str(&uri).expect("a percent-encoded canonical URI is valid");
    Arc::from(uri.as_str())
}

fn resource_uri(uri: &str) -> &str {
    uri.split_once('#').map_or(uri, |(resource, _)| resource)
}

fn ensure_definition<'a>(
    key: &Arc<str>,
    target: &'a Value,
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'a>,
    env: &DynamicEnv,
    state: &mut ParseState<'a>,
) -> Result<bool, CanonicalizationError> {
    debug_assert!(
        state.dynamic_scope.tracked() || env.is_empty(),
        "an untracked parse keeps the digest empty"
    );
    // One target reached along two dynamic paths canonicalizes two ways, so a URI alone cannot key
    // the cache. The digest of the target's own scope is what its parse can observe.
    let key = specialized_key(key, env);
    // A `$defs` name spelling a canonical URI keys the same string that a reference to the resource
    // it encodes mints, so without this the early return below would alias the two targets.
    if let Some(existing) = state.sources.get(&key) {
        if !std::ptr::eq(*existing, target) {
            return Ok(false);
        }
    }
    if state.definitions.contains_key(&key) || state.in_progress.contains(&key) {
        return Ok(true);
    }
    state.sources.insert(Arc::clone(&key), target);
    state.in_progress.insert(Arc::clone(&key));
    let parsed = parse_schema_in_scope(target, ctx, false, resolver, state);
    // Removed before the `?`, keeping the restore-on-error contract.
    let was_in_progress = state.in_progress.remove(&key);
    debug_assert!(
        was_in_progress,
        "definition parsing balances its in-progress marker"
    );
    let Some(parsed) = parsed? else {
        return Ok(false);
    };
    let previous = state.definitions.insert(key, parsed);
    debug_assert!(
        previous.is_none(),
        "a canonical definition target is inserted once"
    );
    Ok(true)
}

/// Retain definitions referenced by the final IR. The registry resolves source references before algebra, but cannot know which
/// symbolic references survive canonical rewriting, so this is a linear liveness walk over already-resolved definition keys.
pub(crate) fn prune_unreachable_definitions(root: &Schema, definitions: &mut DefinitionMap) {
    let mut pending = Vec::new();
    collect_live_definition_references(root, &mut pending);
    let mut reachable = AHashSet::new();
    while let Some(uri) = pending.pop() {
        let Some((uri, schema)) = definitions.get_key_value(uri) else {
            continue;
        };
        if reachable.insert(Arc::clone(uri)) {
            collect_live_definition_references(schema, &mut pending);
        }
    }
    drop(pending);
    definitions.retain(|uri, _| reachable.contains(uri));
    #[cfg(debug_assertions)]
    {
        // Emit turns every surviving `Reference` into a `$ref` into the definition map, so a
        // dropped key leaves the emitted document pointing at nothing. The root is never keyed.
        // Walked through `collect_classified_references` rather than the collector the liveness
        // pass uses: a check sharing that collector cannot see it miss a field.
        let mut surviving = Vec::new();
        emptiness::collect_classified_references(
            root,
            emptiness::Position::InPlace,
            &mut surviving,
        );
        for schema in definitions.values() {
            emptiness::collect_classified_references(
                schema,
                emptiness::Position::InPlace,
                &mut surviving,
            );
        }
        for (uri, _) in surviving {
            // The root carries no definition entry of its own.
            if uri.as_ref() == ROOT_DEFINITION_KEY {
                continue;
            }
            debug_assert!(
                definitions.contains_key(uri),
                "a reference surviving the prune names a retained definition, got `{uri}`"
            );
        }
    }
}

fn collect_live_definition_references<'a>(schema: &'a Schema, references: &mut Vec<&'a str>) {
    // Derived from the emptiness walker rather than repeated: the two must agree on which fields
    // hold a schema, and a field missed here leaks a `$ref` to a pruned definition.
    let mut found = Vec::new();
    emptiness::collect_classified_references(schema, emptiness::Position::InPlace, &mut found);
    references.extend(found.into_iter().map(|(uri, _)| uri.as_ref()));
}

/// The array-form dependency on `key`: holding it demands the listed keys too.
fn required_dependency(key: &str, names: &[Value], ctx: &CanonicalizationContext) -> Schema {
    let mut required: Vec<Arc<str>> = names
        .iter()
        .filter_map(Value::as_str)
        .map(Arc::from)
        .collect();
    required.push(Arc::from(key));
    required.sort();
    required.dedup();
    dependency_conjunct(key, object_with_required(required, ctx), ctx)
}

/// The schema-form dependency on `key`: holding it demands the whole value meet `schema`.
fn schema_dependency(key: &str, schema: Schema, ctx: &CanonicalizationContext) -> Schema {
    dependency_conjunct(key, schema, ctx)
}

/// A dependency triggers only on objects holding `key`: non-objects and objects without the key
/// pass vacuously, everything else answers to `consequent`.
fn dependency_conjunct(key: &str, consequent: Schema, ctx: &CanonicalizationContext) -> Schema {
    let vacuous = type_set_schema(JsonTypeSet::all().remove(JsonType::Object));
    let absent = algebra::object_leaf(
        ObjectLeaf {
            sizes: LengthBounds::default(),
            required: Vec::new(),
            property_names: None,
            properties: PropertyMap::from_iter([(Arc::from(key), Schema::falsy())]),
            pattern_properties: PropertyMap::default(),
            additional: None,
            violations: Vec::new(),
        },
        ctx,
    );
    algebra::union(vec![vacuous, absent, consequent], ctx)
}

/// An object leaf demanding exactly the sorted `required` keys and nothing else.
fn object_with_required(required: Vec<Arc<str>>, ctx: &CanonicalizationContext) -> Schema {
    algebra::object_leaf(
        ObjectLeaf {
            sizes: LengthBounds::default(),
            required,
            property_names: None,
            properties: PropertyMap::default(),
            pattern_properties: PropertyMap::default(),
            additional: None,
            violations: Vec::new(),
        },
        ctx,
    )
}

/// The finite value set admitted by `const` and `enum` together: their conjunction.
fn admitted_values(
    enum_values: Option<&Vec<Value>>,
    const_value: Option<&Value>,
) -> Option<Vec<CanonicalJson>> {
    let mut values: Option<Vec<CanonicalJson>> =
        enum_values.map(|entries| entries.iter().map(CanonicalJson::from_value).collect());
    if let Some(constant) = const_value {
        let constant = CanonicalJson::from_value(constant);
        values = Some(match values {
            Some(members) => members
                .into_iter()
                .filter(|value| *value == constant)
                .collect(),
            None => vec![constant],
        });
    }
    values
}

/// Intersect admitted values with a `type` set: drop values outside it, then pack the rest.
pub(crate) fn restrict_values_to_types(
    values: Vec<CanonicalJson>,
    set: JsonTypeSet,
    ctx: &CanonicalizationContext,
) -> Schema {
    let cover = SchemaKind::semantic_cover(set);
    let filtered: Vec<CanonicalJson> = values
        .into_iter()
        .filter(|value| cover.contains(value.json_type()))
        .collect();
    if !keeps_draft4_integer_guard(set, ctx.draft()) {
        return canonicalize_value_set(filtered);
    }
    // Draft 4 cannot tell `1` from `1.0` by value equality, so integer members keep the integer type
    // guard; members of other types (which the set also admits) do not.
    let (integers, others): (Vec<_>, Vec<_>) = filtered
        .into_iter()
        .partition(|value| value.json_type() == JsonType::Integer);
    let mut branches = Vec::new();
    let integer_set = canonicalize_value_set(integers);
    if !matches!(integer_set.kind(), SchemaKind::False) {
        branches.push(typed_group(JsonType::Integer, integer_set));
    }
    let other_set = canonicalize_value_set(others);
    if !matches!(other_set.kind(), SchemaKind::False) {
        branches.push(other_set);
    }
    algebra::union(branches, ctx)
}

/// Whether every number nested in an instance-data value keeps a plain canonical spelling, which
/// holds until a magnitude needs more digits to write out than `MAX_EXPANDED_INTEGER_DIGITS`.
#[cfg(feature = "arbitrary-precision")]
fn finite_value_spelling_is_exact(value: &Value) -> bool {
    match value {
        Value::Number(number) => {
            let canonical = crate::canonical::json::canonical_number(number.as_str());
            let text = canonical.as_deref().unwrap_or(number.as_str());
            !text.bytes().any(|byte| matches!(byte, b'e' | b'E'))
        }
        Value::Array(items) => items.iter().all(finite_value_spelling_is_exact),
        Value::Object(map) => map.values().all(finite_value_spelling_is_exact),
        Value::Null | Value::Bool(_) | Value::String(_) => true,
    }
}

#[cfg(not(feature = "arbitrary-precision"))]
fn finite_value_spelling_is_exact(_value: &Value) -> bool {
    // Default-build numbers are `i64`/`u64`/`f64`; their canonical spellings never go scientific.
    true
}

/// Read a `type` keyword value - a single name or a list of names - into a [`JsonTypeSet`];
/// `None` when it is not a type declaration this build understands.
fn parse_type_set(value: &Value) -> Option<JsonTypeSet> {
    match value {
        Value::String(name) => Some(JsonTypeSet::from(name.parse::<JsonType>().ok()?)),
        Value::Array(names) => names.iter().try_fold(JsonTypeSet::empty(), |set, name| {
            Some(set.insert(name.as_str()?.parse::<JsonType>().ok()?))
        }),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => None,
    }
}

/// Keep whichever end admits fewer values.
fn tighter_real(
    current: Option<BoundNumber>,
    limit: &serde_json::Number,
    inclusive: bool,
    side: Side,
) -> Option<BoundNumber> {
    let bound = BoundNumber::new(limit, inclusive);
    match current {
        Some(current) if current.is_tighter_than(&bound, side) => Some(current),
        _ => Some(bound),
    }
}

/// A numeric facet constrains only numbers, so `{"minimum": 3}` becomes
/// `anyOf: [<non-number types>, {"type": "number", "minimum": 3}]`.
fn number_facet_schema(leaf: NumberLeaf, ctx: &CanonicalizationContext) -> Schema {
    let non_number = Schema::new(SchemaKind::MultiType(
        JsonTypeSet::all()
            .remove(JsonType::Number)
            .remove(JsonType::Integer),
    ));
    algebra::union(vec![non_number, algebra::number_leaf(leaf, ctx)], ctx)
}

/// A string facet constrains only strings, so `{"minLength": 3}` becomes
/// `anyOf: [<non-string types>, {"type": "string", "minLength": 3}]`.
fn string_facet_schema(leaf: StringLeaf, ctx: &CanonicalizationContext) -> Schema {
    let non_string = Schema::new(SchemaKind::MultiType(
        JsonTypeSet::all().remove(JsonType::String),
    ));
    algebra::union(vec![non_string, algebra::string_leaf(leaf, ctx)], ctx)
}

/// Parse a tuple's per-index schemas; `Ok(None)` when any element is unmodeled, keeping the document raw.
fn parse_prefix<'a>(
    schemas: &[Value],
    ctx: &CanonicalizationContext,
    resolver: &Resolver<'a>,
    state: &mut ParseState<'a>,
) -> Result<Option<Vec<Schema>>, CanonicalizationError> {
    let mut prefix = Vec::with_capacity(schemas.len());
    for schema in schemas {
        match parse_schema(schema, ctx, false, resolver, state)? {
            Some(schema) => prefix.push(schema),
            None => return Ok(None),
        }
    }
    Ok(Some(prefix))
}

/// An array facet constrains only arrays, so `{"minItems": 1}` becomes
/// `anyOf: [<non-array types>, {"type": "array", "minItems": 1}]`.
fn array_facet_schema(leaf: ArrayLeaf, ctx: &CanonicalizationContext) -> Schema {
    let non_array = Schema::new(SchemaKind::MultiType(
        JsonTypeSet::all().remove(JsonType::Array),
    ));
    algebra::union(vec![non_array, algebra::array_leaf(leaf, ctx)], ctx)
}

/// An object facet constrains only objects, so `{"minProperties": 1}` becomes
/// `anyOf: [<non-object types>, {"type": "object", "minProperties": 1}]`.
fn object_facet_schema(leaf: ObjectLeaf, ctx: &CanonicalizationContext) -> Schema {
    let non_object = Schema::new(SchemaKind::MultiType(
        JsonTypeSet::all().remove(JsonType::Object),
    ));
    algebra::union(vec![non_object, algebra::object_leaf(leaf, ctx)], ctx)
}

/// Draft 4 says `1.0` is not an integer, so its `integer` check cannot fold into value equality.
fn keeps_draft4_integer_guard(set: JsonTypeSet, draft: Draft) -> bool {
    matches!(draft, Draft::Draft4)
        && set.contains(JsonType::Integer)
        && !set.contains(JsonType::Number)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Anchor names from externally registered resources bypass `anchorString` validation, so a
    // name or resource URI may spell the join delimiters themselves.
    #[test]
    fn specialized_key_is_injective_for_delimiter_spelling_components() {
        let key: Arc<str> = Arc::from("https://example.com/target");
        let name_with_at: DynamicEnv =
            Arc::from(vec![(Arc::<str>::from("a@r"), Arc::<str>::from("x"))]);
        let resource_with_at: DynamicEnv =
            Arc::from(vec![(Arc::<str>::from("a"), Arc::<str>::from("r@x"))]);
        assert_ne!(
            specialized_key(&key, &name_with_at),
            specialized_key(&key, &resource_with_at)
        );

        let key_with_delimiter: Arc<str> = Arc::from("k|dyn=a@r");
        let one_binding: DynamicEnv =
            Arc::from(vec![(Arc::<str>::from("b"), Arc::<str>::from("s"))]);
        let plain_key: Arc<str> = Arc::from("k");
        let two_bindings: DynamicEnv = Arc::from(vec![
            (Arc::<str>::from("a"), Arc::<str>::from("r")),
            (Arc::<str>::from("b"), Arc::<str>::from("s")),
        ]);
        assert_ne!(
            specialized_key(&key_with_delimiter, &one_binding),
            specialized_key(&plain_key, &two_bindings)
        );
    }
}
