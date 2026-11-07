use crate::{
    compiler::{Context, DeferredAbsoluteLocation},
    evaluation::{Annotations, ChildList, EvaluationNode, InstancePosition},
    keywords::{items::AbsorbedKeywords, BoxedValidator, BuiltinKeyword, Keyword},
    paths::{LazyLocation, Location, RefTracker},
    types::{JsonType, JsonTypeSet},
    validator::{EvaluationResult, Validate, ValidationContext},
    Array, Json, Node, SerdeJson, ValidationError,
};
use referencing::Uri;
use serde_json::Value;
use std::{
    fmt,
    sync::{Arc, OnceLock, Weak},
};

struct SchemaNodeInner<F: Json> {
    /// The types this node accepts when it only checks `type`, itself or through `$ref`s to owned
    /// targets, so a parent can decide it without entering it.
    accepted_types: Option<JsonTypeSet>,
    validators: NodeValidators<F>,
    /// What this node's absolute locations resolve against; `None` without a base URI.
    absolute_base: Option<DeferredAbsoluteLocation>,
    schema_location: OnceLock<SchemaLocation>,
}

#[cfg(all(target_arch = "x86_64", target_pointer_width = "64"))]
const _: () = assert!(std::mem::size_of::<SchemaNodeInner<SerdeJson>>() == 128);

/// The `schemaLocation` an evaluation reports for a subschema, and its absolute location.
struct SchemaLocation {
    formatted: Arc<str>,
    absolute: Option<Arc<Uri<String>>>,
}

impl SchemaLocation {
    fn new(location: &Location, absolute: Option<&Arc<Uri<String>>>) -> Self {
        SchemaLocation {
            formatted: crate::evaluation::format_keyword_location(location, absolute),
            absolute: absolute.cloned(),
        }
    }
}

/// The absolute location of `location`, built on first use: most are never read.
///
/// Only errors reach it, so it stays out of the validation loops it is called from.
#[cold]
#[inline(never)]
fn absolute_location<'a>(
    base: Option<&DeferredAbsoluteLocation>,
    resolved: &'a OnceLock<Arc<Uri<String>>>,
    location: &Location,
) -> Option<&'a Arc<Uri<String>>> {
    let base = base?;
    Some(resolved.get_or_init(|| base.resolve(location)))
}

/// Numbers need their value checked: `integer` looks at the fraction, and a backend may report a
/// non-finite number as `number` while rejecting it. Sets with either never get a type shortcut.
fn type_shortcut(types: JsonTypeSet) -> Option<JsonTypeSet> {
    (!types.contains(JsonType::Number) && !types.contains(JsonType::Integer)).then_some(types)
}

impl<F: Json> fmt::Debug for SchemaNodeInner<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SchemaNodeInner")
            .field("validators", &self.validators)
            .finish_non_exhaustive()
    }
}

/// A node in the schema tree, returned by `compiler::compile`
pub(crate) struct SchemaNode<F: Json = SerdeJson> {
    inner: Arc<SchemaNodeInner<F>>,
    location: Location,
}

#[cfg(all(target_arch = "x86_64", target_pointer_width = "64"))]
const _: () = assert!(std::mem::size_of::<SchemaNode>() == 24);

impl<F: Json> Clone for SchemaNode<F> {
    fn clone(&self) -> Self {
        SchemaNode {
            inner: Arc::clone(&self.inner),
            location: self.location.clone(),
        }
    }
}

impl<F: Json> fmt::Debug for SchemaNode<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SchemaNode")
            .field("inner", &self.inner)
            .field("location", &self.location)
            .finish_non_exhaustive()
    }
}

// Separate type used only during compilation for handling recursive references
pub(crate) struct PendingSchemaNode<F: Json = SerdeJson> {
    cell: Arc<OnceLock<PendingTarget<F>>>,
}

impl<F: Json> Clone for PendingSchemaNode<F> {
    fn clone(&self) -> Self {
        PendingSchemaNode {
            cell: Arc::clone(&self.cell),
        }
    }
}

impl<F: Json> fmt::Debug for PendingSchemaNode<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingSchemaNode").finish_non_exhaustive()
    }
}

enum PendingTarget<F: Json> {
    /// Owns a target no `$ref` cycle passes through.
    Strong(SchemaNode<F>),
    /// A target a `$ref` cycle may pass through; owning it could close an `Arc` cycle.
    Weak(WeakTarget<F>),
}

struct WeakTarget<F: Json> {
    inner: Weak<SchemaNodeInner<F>>,
    location: Location,
}

enum NodeValidators<F: Json> {
    /// The result of compiling a boolean valued schema, e.g
    ///
    /// ```json
    /// {
    ///     "additionalProperties": false
    /// }
    /// ```
    ///
    /// Here the result of `compiler::compile` called with the `false` value will return a
    /// `SchemaNode` with a single `BooleanValidator` as it's `validators`.
    Boolean {
        validator: Option<BoxedValidator<F>>,
    },
    /// The result of compiling a schema which is composed of keywords (almost all schemas)
    Keyword(KeywordValidators<F>),
    /// The result of compiling a schema which is "array valued", e.g the "dependencies" keyword of
    /// draft 7 which can take values which are an array of other property names
    Array {
        validators: Vec<ArrayValidatorEntry<F>>,
    },
}

#[cfg(all(target_arch = "x86_64", target_pointer_width = "64"))]
const _: () = assert!(std::mem::size_of::<NodeValidators<SerdeJson>>() == 72);

impl<F: Json> NodeValidators<F> {
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        match self {
            NodeValidators::Keyword(kvs) => match &kvs.validators {
                KeywordList::One(entry) => entry.validator.is_valid(instance, ctx),
                KeywordList::Many(entries) => {
                    for entry in entries {
                        if !entry.validator.is_valid(instance, ctx) {
                            return false;
                        }
                    }
                    true
                }
            },
            NodeValidators::Array { validators } => validators
                .iter()
                .all(|entry| entry.validator.is_valid(instance, ctx)),
            NodeValidators::Boolean { validator: Some(_) } => false,
            NodeValidators::Boolean { validator: None } => true,
        }
    }
}

impl<F: Json> fmt::Debug for NodeValidators<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boolean { .. } => f.debug_struct("Boolean").finish(),
            Self::Keyword(_) => f.debug_tuple("Keyword").finish(),
            Self::Array { .. } => f.debug_struct("Array").finish(),
        }
    }
}

struct KeywordValidators<F: Json> {
    /// The keywords on this node which were not recognized by any vocabularies. These are
    /// stored so we can later produce them as annotations
    unmatched_keywords: Option<Arc<Value>>,
    // We should probably use AHashMap here but it breaks a bunch of tests which assume
    // validators are in a particular order
    validators: KeywordList<F>,
    /// Per-validator data only error and evaluation paths read, kept apart so `is_valid` walks
    /// densely packed validators.
    metadata: Vec<KeywordMetadata>,
    absorbed: Option<AbsorbedUnits>,
}

/// Keywords a fused validator checks on its own. `evaluate` reports them as their own units, so
/// the output lists the same units as without the fusion.
struct AbsorbedUnits {
    keywords: Arc<AbsorbedKeywords>,
    /// How many of the node's validators run before `minItems` and `maxItems`; `type` runs first.
    counts_at: usize,
}

/// Validators in priority order. A lone validator is stored inline, sparing `is_valid` a hop to the heap.
enum KeywordList<F: Json> {
    One(KeywordValidator<F>),
    Many(Box<[KeywordValidator<F>]>),
}

impl<F: Json> KeywordList<F> {
    fn as_slice(&self) -> &[KeywordValidator<F>] {
        match self {
            KeywordList::One(entry) => std::slice::from_ref(entry),
            KeywordList::Many(entries) => entries,
        }
    }
}

struct KeywordValidator<F: Json> {
    validator: BoxedValidator<F>,
    /// Asserts on the instance itself, so `is_valid` may run ahead of `validate`.
    is_leaf: bool,
}

#[cfg(all(target_arch = "x86_64", target_pointer_width = "64"))]
const _: () = assert!(std::mem::size_of::<KeywordValidator<SerdeJson>>() == 24);

struct KeywordMetadata {
    location: Location,
    /// Read by errors; evaluation reads `schema_location`.
    absolute_location: OnceLock<Arc<Uri<String>>>,
    schema_location: OnceLock<SchemaLocation>,
}

/// What evaluating one subschema of a node reads.
struct EntryParts<'a, F: Json> {
    location: &'a Location,
    absolute_base: Option<&'a DeferredAbsoluteLocation>,
    absolute_location: &'a OnceLock<Arc<Uri<String>>>,
    schema_location: &'a OnceLock<SchemaLocation>,
    validator: &'a BoxedValidator<F>,
}

impl<F: Json> EntryParts<'_, F> {
    /// Rendered once per subschema: a by-reference validator reports its target's location, which
    /// is fixed too.
    #[inline]
    fn schema_location(&self) -> &SchemaLocation {
        self.schema_location.get_or_init(|| {
            if let Some((location, absolute)) = self.validator.canonical_location() {
                SchemaLocation::new(location, absolute)
            } else {
                SchemaLocation::new(
                    self.location,
                    absolute_location(self.absolute_base, self.absolute_location, self.location),
                )
            }
        })
    }
}

impl KeywordMetadata {
    #[inline]
    fn absolute_location(
        &self,
        base: Option<&DeferredAbsoluteLocation>,
    ) -> Option<&Arc<Uri<String>>> {
        absolute_location(base, &self.absolute_location, &self.location)
    }

    fn parts<'a, F: Json>(
        &'a self,
        validator: &'a BoxedValidator<F>,
        absolute_base: Option<&'a DeferredAbsoluteLocation>,
    ) -> EntryParts<'a, F> {
        EntryParts {
            location: &self.location,
            absolute_base,
            absolute_location: &self.absolute_location,
            schema_location: &self.schema_location,
            validator,
        }
    }
}

struct ArrayValidatorEntry<F: Json> {
    validator: BoxedValidator<F>,
    location: Location,
    absolute_location: OnceLock<Arc<Uri<String>>>,
    schema_location: OnceLock<SchemaLocation>,
}

impl<F: Json> ArrayValidatorEntry<F> {
    #[inline]
    fn absolute_location(
        &self,
        base: Option<&DeferredAbsoluteLocation>,
    ) -> Option<&Arc<Uri<String>>> {
        absolute_location(base, &self.absolute_location, &self.location)
    }
}

impl<F: Json> PendingSchemaNode<F> {
    pub(crate) fn new() -> Self {
        PendingSchemaNode {
            cell: Arc::new(OnceLock::new()),
        }
    }

    /// A node pointing at `node`, which a `$ref` cycle may pass through.
    pub(crate) fn pointing_at(node: &SchemaNode<F>) -> Self {
        let pending = Self::new();
        pending.initialize(node);
        pending
    }

    /// Point at `node`, which a `$ref` cycle may pass through.
    pub(crate) fn initialize(&self, node: &SchemaNode<F>) {
        self.set(PendingTarget::Weak(WeakTarget {
            inner: Arc::downgrade(&node.inner),
            location: node.location.clone(),
        }));
    }

    /// Point at `node` and own it; no `$ref` cycle passes through it.
    pub(crate) fn initialize_owned(&self, node: SchemaNode<F>) {
        self.set(PendingTarget::Strong(node));
    }

    fn set(&self, target: PendingTarget<F>) {
        assert!(
            self.cell.set(target).is_ok(),
            "pending node initialized twice"
        );
    }

    fn target(&self) -> &PendingTarget<F> {
        self.cell
            .get()
            .expect("pending node accessed before initialization")
    }
}

impl<F: Json> WeakTarget<F> {
    /// The target's address: every guarded `$ref` to one node shares its cycle guard.
    #[inline]
    fn node_id(&self) -> usize {
        Weak::as_ptr(&self.inner) as usize
    }

    fn materialize(&self) -> SchemaNode<F> {
        let inner = self.inner.upgrade().expect("pending schema target dropped");
        SchemaNode {
            inner,
            location: self.location.clone(),
        }
    }
}

/// A `Strong` target validates as a direct child. Every `$ref` cycle passes a `Weak` target,
/// whose guard stops re-entry.
impl<F: Json> Validate<F> for PendingSchemaNode<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        let target = match self.target() {
            PendingTarget::Strong(node) => return node.is_valid(instance, ctx),
            PendingTarget::Weak(target) => target,
        };
        let node_id = target.node_id();
        let identity = instance.identity();
        // The cycle guard comes first: while this node sits on the stack the cycle answer is the
        // one the other modes reach, and a cached value would override it here alone.
        if ctx.enter(node_id, identity) {
            return true; // Cycle detected
        }
        // Check memoization cache (only for arrays/objects)
        let container_identity = instance.container_identity();
        let result = if let Some(cached) = ctx.get_cached_result(node_id, container_identity) {
            cached
        } else {
            let visit = ctx.start_ref_visit();
            // Only the validators run; the location and absolute path stay unshared.
            let inner = target
                .inner
                .upgrade()
                .expect("pending schema target dropped");
            let computed = inner.validators.is_valid(instance, ctx);
            ctx.cache_result(node_id, container_identity, computed, visit);
            computed
        };
        ctx.exit(node_id, identity);
        result
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        let target = match self.target() {
            PendingTarget::Strong(node) => return node.validate(instance, location, tracker, ctx),
            PendingTarget::Weak(target) => target,
        };
        let identity = instance.identity();
        if ctx.enter(target.node_id(), identity) {
            return Ok(());
        }
        let result = target
            .materialize()
            .validate(instance, location, tracker, ctx);
        ctx.exit(target.node_id(), identity);
        result
    }

    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        let target = match self.target() {
            PendingTarget::Strong(node) => {
                return node.collect_errors(instance, location, tracker, ctx, errors)
            }
            PendingTarget::Weak(target) => target,
        };
        let identity = instance.identity();
        if ctx.enter(target.node_id(), identity) {
            return;
        }
        target
            .materialize()
            .collect_errors(instance, location, tracker, ctx, errors);
        ctx.exit(target.node_id(), identity);
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        self.evaluate_with_location(
            instance,
            location,
            ctx.instance_position(location),
            tracker,
            ctx,
        )
    }

    fn evaluate_with_location(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: InstancePosition,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let target = match self.target() {
            PendingTarget::Strong(node) => {
                return node.evaluate_with_location(
                    instance,
                    location,
                    instance_location,
                    tracker,
                    ctx,
                )
            }
            PendingTarget::Weak(target) => target,
        };
        let identity = instance.identity();
        if ctx.enter(target.node_id(), identity) {
            return EvaluationResult::valid_empty();
        }
        let result = target.materialize().evaluate_with_location(
            instance,
            location,
            instance_location,
            tracker,
            ctx,
        );
        ctx.exit(target.node_id(), identity);
        result
    }
}

impl<F: Json> SchemaNode<F> {
    pub(crate) fn from_boolean(
        ctx: &Context<'_, F>,
        validator: Option<BoxedValidator<F>>,
    ) -> SchemaNode<F> {
        SchemaNode {
            inner: Arc::new(SchemaNodeInner {
                accepted_types: None,
                validators: NodeValidators::Boolean { validator },
                absolute_base: ctx.deferred_absolute_location(),
                schema_location: OnceLock::new(),
            }),
            location: ctx.location().clone(),
        }
    }

    /// `absorbed` holds the keywords a fused validator in `validators` checks on its own.
    pub(crate) fn from_keywords(
        ctx: &Context<'_, F>,
        mut validators: Vec<(Keyword, BoxedValidator<F>)>,
        absorbed: Option<Arc<AbsorbedKeywords>>,
        unmatched_keywords: Option<Arc<Value>>,
    ) -> SchemaNode<F> {
        // Sort validators by priority (lower = execute first).
        // This enables "fail fast" by running cheap validators (type, const)
        // before expensive ones (allOf, $ref).
        validators.sort_by_key(|(keyword, _)| crate::keywords::keyword_priority(keyword));
        let absorbed = absorbed.map(|keywords| {
            // Nothing sorts between `minItems` and `maxItems`, and nothing before `type`.
            let priority = crate::keywords::keyword_priority(&BuiltinKeyword::MinItems.into());
            AbsorbedUnits {
                keywords,
                counts_at: validators.partition_point(|(keyword, _)| {
                    crate::keywords::keyword_priority(keyword) < priority
                }),
            }
        });

        let (metadata, mut validators): (Vec<_>, Vec<_>) = validators
            .into_iter()
            .map(|(keyword, validator)| {
                let metadata = KeywordMetadata {
                    location: ctx.location().join(&keyword),
                    absolute_location: OnceLock::new(),
                    schema_location: OnceLock::new(),
                };
                // Only `required` may lean on a sibling keyword; asking every validator costs a call each.
                let is_leaf = crate::keywords::keyword_is_leaf(&keyword)
                    && (!matches!(
                        keyword,
                        crate::keywords::Keyword::Builtin(
                            crate::keywords::BuiltinKeyword::Required
                        )
                    ) || validator.is_valid_alone());
                let validator = KeywordValidator { validator, is_leaf };
                (metadata, validator)
            })
            .unzip();
        let accepted_types = match validators.as_slice() {
            [only] => match only.validator.owned_ref_target() {
                Some(target) => target.inner.accepted_types,
                None => only.validator.accepted_types().and_then(type_shortcut),
            },
            _ => None,
        };
        let validators = if validators.len() == 1 {
            KeywordList::One(validators.pop().expect("one validator"))
        } else {
            KeywordList::Many(validators.into_boxed_slice())
        };
        SchemaNode {
            inner: Arc::new(SchemaNodeInner {
                accepted_types,
                validators: NodeValidators::Keyword(KeywordValidators {
                    unmatched_keywords,
                    validators,
                    metadata,
                    absorbed,
                }),
                absolute_base: ctx.deferred_absolute_location(),
                schema_location: OnceLock::new(),
            }),
            location: ctx.location().clone(),
        }
    }

    pub(crate) fn from_array(
        ctx: &Context<'_, F>,
        validators: Vec<BoxedValidator<F>>,
    ) -> SchemaNode<F> {
        let validators = validators
            .into_iter()
            .enumerate()
            .map(|(index, validator)| ArrayValidatorEntry {
                validator,
                location: ctx.location().join(index),
                absolute_location: OnceLock::new(),
                schema_location: OnceLock::new(),
            })
            .collect();
        SchemaNode {
            inner: Arc::new(SchemaNodeInner {
                accepted_types: None,
                validators: NodeValidators::Array { validators },
                absolute_base: ctx.deferred_absolute_location(),
                schema_location: OnceLock::new(),
            }),
            location: ctx.location().clone(),
        }
    }

    pub(crate) fn validators(&self) -> impl ExactSizeIterator<Item = &BoxedValidator<F>> {
        match &self.inner.validators {
            NodeValidators::Boolean { validator } => {
                if let Some(v) = validator {
                    NodeValidatorsIter::BooleanValidators(std::iter::once(v))
                } else {
                    NodeValidatorsIter::NoValidator
                }
            }
            NodeValidators::Keyword(kvals) => {
                NodeValidatorsIter::KeywordValidators(kvals.validators.as_slice().iter())
            }
            NodeValidators::Array { validators } => {
                NodeValidatorsIter::ArrayValidators(validators.iter())
            }
        }
    }

    pub(crate) fn evaluate_instance(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationNode {
        let instance_location = ctx.instance_position(location);
        self.evaluate_instance_at(instance, location, instance_location, tracker, ctx)
    }

    /// [`Self::evaluate_instance`] for a position one segment below the one being evaluated.
    ///
    /// Renders from the location already on the context instead of walking the chain again.
    pub(crate) fn evaluate_instance_below(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationNode {
        let instance_location = ctx.instance_position_below(location);
        debug_assert_eq!(
            ctx.arena().render(instance_location),
            Location::from(location)
        );
        self.evaluate_instance_at(instance, location, instance_location, tracker, ctx)
    }

    /// [`Self::evaluate_instance`] for a caller that already rendered this instance position.
    pub(crate) fn evaluate_instance_at(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: InstancePosition,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationNode {
        let keyword_location = crate::paths::evaluation_path(tracker, &self.location, ctx);
        let SchemaLocation {
            formatted: schema_location,
            absolute,
        } = self.schema_location();

        let previous = ctx.enter_instance_location(instance_location);
        let result = self.evaluate_at(instance, location, instance_location, tracker, ctx);
        ctx.leave_instance_location(previous);

        match result {
            EvaluationResult::Valid {
                annotations,
                children,
            } => EvaluationNode::valid(
                keyword_location,
                absolute.clone(),
                Arc::clone(schema_location),
                instance_location,
                annotations,
                children,
            ),
            EvaluationResult::Invalid {
                errors,
                children,
                annotations,
            } => EvaluationNode::invalid(
                keyword_location,
                absolute.clone(),
                Arc::clone(schema_location),
                instance_location,
                annotations,
                errors,
                children,
            ),
        }
    }

    /// Helper function to evaluate subschemas which already know their locations.
    fn evaluate_subschemas<'a, 'i, I>(
        instance: &F::Node<'i>,
        location: &LazyLocation,
        instance_loc: InstancePosition,
        tracker: Option<&RefTracker>,
        subschemas: I,
        annotations: Option<Annotations>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult
    where
        I: Iterator<Item = EntryParts<'a, F>> + 'a,
    {
        let mut children = ChildList::default();
        let invalid = Self::push_subschemas(
            instance,
            location,
            instance_loc,
            tracker,
            subschemas,
            &mut children,
            ctx,
        );
        Self::keyword_result(children, invalid, annotations)
    }

    /// Evaluates the node's keywords, reporting `absorbed` beside them in priority order.
    ///
    /// Kept out of line so the common path in `evaluate_at` compiles as without it.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn evaluate_with_absorbed(
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_loc: InstancePosition,
        tracker: Option<&RefTracker>,
        validators: &[KeywordValidator<F>],
        metadata: &[KeywordMetadata],
        absolute_base: Option<&DeferredAbsoluteLocation>,
        absorbed: &AbsorbedUnits,
        annotations: Option<Annotations>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let count = instance.as_array().map(|array| array.len() as u64);
        let keywords = &*absorbed.keywords;
        let mut children = ChildList::default();
        let node =
            keywords.evaluate_type::<F>(instance, count.is_some(), instance_loc, tracker, ctx);
        children.push(ctx.arena(), node);
        let (before, after) = validators.split_at(absorbed.counts_at);
        let (metadata_before, metadata_after) = metadata.split_at(absorbed.counts_at);
        Self::push_subschemas(
            instance,
            location,
            instance_loc,
            tracker,
            metadata_before
                .iter()
                .zip(before)
                .map(|(m, v)| m.parts(&v.validator, absolute_base)),
            &mut children,
            ctx,
        );
        keywords.evaluate_counts::<F>(instance, count, instance_loc, tracker, ctx, &mut children);
        Self::push_subschemas(
            instance,
            location,
            instance_loc,
            tracker,
            metadata_after
                .iter()
                .zip(after)
                .map(|(m, v)| m.parts(&v.validator, absolute_base)),
            &mut children,
            ctx,
        );
        let invalid = !children.all_valid();
        Self::keyword_result(children, invalid, annotations)
    }

    fn keyword_result(
        children: ChildList,
        invalid: bool,
        annotations: Option<Annotations>,
    ) -> EvaluationResult {
        if invalid {
            EvaluationResult::Invalid {
                errors: Vec::new(),
                children,
                annotations,
            }
        } else {
            EvaluationResult::Valid {
                annotations,
                children,
            }
        }
    }

    /// Pushes the output unit of each subschema onto `children`; returns whether any failed.
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn push_subschemas<'a, 'i, I>(
        instance: &F::Node<'i>,
        location: &LazyLocation,
        instance_loc: InstancePosition,
        tracker: Option<&RefTracker>,
        subschemas: I,
        children: &mut ChildList,
        ctx: &mut ValidationContext,
    ) -> bool
    where
        I: Iterator<Item = EntryParts<'a, F>> + 'a,
    {
        let mut invalid = false;

        for parts in subschemas {
            let child_result = parts.validator.evaluate_with_location(
                instance,
                location,
                instance_loc,
                tracker,
                ctx,
            );

            // schemaLocation: The canonical location WITHOUT $ref traversals.
            // Per JSON Schema spec: "MUST NOT include by-reference applicators such as $ref"
            // For by-reference validators like $ref, use the target's canonical location,
            // which lives in the target's resource.
            // For regular validators, use the keyword's location.
            let schema_location = parts.schema_location();
            let absolute_location = schema_location.absolute.clone();
            let formatted_schema_location = Arc::clone(&schema_location.formatted);

            let eval_path = crate::paths::evaluation_path(tracker, parts.location, ctx);

            let child_node = match child_result {
                EvaluationResult::Valid {
                    annotations,
                    children,
                } => EvaluationNode::valid(
                    eval_path,
                    absolute_location,
                    formatted_schema_location,
                    instance_loc,
                    annotations,
                    children,
                ),
                EvaluationResult::Invalid {
                    errors,
                    children,
                    annotations,
                } => {
                    invalid = true;
                    EvaluationNode::invalid(
                        eval_path,
                        absolute_location,
                        formatted_schema_location,
                        instance_loc,
                        annotations,
                        errors,
                        children,
                    )
                }
            };
            children.push(ctx.arena(), child_node);
        }
        invalid
    }

    pub(crate) fn location(&self) -> &Location {
        &self.location
    }

    #[inline]
    fn schema_location(&self) -> &SchemaLocation {
        self.inner.schema_location.get_or_init(|| {
            let absolute = self
                .inner
                .absolute_base
                .as_ref()
                .map(|base| base.resolve(&self.location));
            SchemaLocation::new(&self.location, absolute.as_ref())
        })
    }

    #[inline]
    fn absolute_path(&self) -> Option<&Arc<Uri<String>>> {
        self.schema_location().absolute.as_ref()
    }
}

fn stamp_absolute_location(errors: &mut [ValidationError<'_>], uri: Option<&Arc<Uri<String>>>) {
    let Some(uri) = uri else { return };
    for error in errors {
        error.set_absolute_keyword_location(uri);
    }
}

impl<F: Json> SchemaNode<F> {
    /// The node's only validator, for loops that run the node on many values.
    #[inline]
    pub(crate) fn single_validator(&self) -> Option<&dyn Validate<F>> {
        match &self.inner.validators {
            NodeValidators::Keyword(KeywordValidators {
                validators: KeywordList::One(entry),
                ..
            }) => Some(&*entry.validator),
            _ => None,
        }
    }

    #[cold]
    #[inline(never)]
    fn false_schema_error<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
    ) -> ValidationError<'i> {
        ValidationError::false_schema(
            self.location.clone(),
            crate::paths::capture_evaluation_path(tracker, &self.location),
            location.into(),
            instance.lazy_value(),
        )
        .with_absolute_keyword_location(self.absolute_path().cloned())
    }
}

impl<F: Json> SchemaNode<F> {
    /// `is_valid` for a child a parent iterates over: shortcut nodes are decided without entering
    /// them. `is_valid` itself stays as is, since every call site inlines it.
    #[inline]
    pub(crate) fn is_valid_child(
        &self,
        instance: &F::Node<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        match self.inner.accepted_types {
            Some(types) => types.contains(instance.json_type()),
            None => self.inner.validators.is_valid(instance, ctx),
        }
    }
}

impl<F: Json> Validate<F> for SchemaNode<F> {
    fn as_owned_node(&self) -> Option<&SchemaNode<F>> {
        Some(self)
    }

    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        self.inner.validators.is_valid(instance, ctx)
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        match &self.inner.validators {
            NodeValidators::Keyword(
                kvs @ KeywordValidators {
                    validators: KeywordList::One(entry),
                    ..
                },
            ) => {
                // A passing keyword costs its `is_valid`; only a failing one builds an error.
                if entry.is_leaf && entry.validator.is_valid(instance, ctx) {
                    return Ok(());
                }
                return entry
                    .validator
                    .validate(instance, location, tracker, ctx)
                    .map_err(|e| {
                        e.with_absolute_keyword_location(
                            kvs.metadata[0]
                                .absolute_location(self.inner.absolute_base.as_ref())
                                .cloned(),
                        )
                    });
            }
            NodeValidators::Keyword(kvs) => {
                for (entry, metadata) in kvs.validators.as_slice().iter().zip(&kvs.metadata) {
                    if entry.is_leaf && entry.validator.is_valid(instance, ctx) {
                        continue;
                    }
                    entry
                        .validator
                        .validate(instance, location, tracker, ctx)
                        .map_err(|e| {
                            e.with_absolute_keyword_location(
                                metadata
                                    .absolute_location(self.inner.absolute_base.as_ref())
                                    .cloned(),
                            )
                        })?;
                }
            }
            NodeValidators::Array { validators } => {
                for entry in validators {
                    entry
                        .validator
                        .validate(instance, location, tracker, ctx)
                        .map_err(|e| {
                            e.with_absolute_keyword_location(
                                entry
                                    .absolute_location(self.inner.absolute_base.as_ref())
                                    .cloned(),
                            )
                        })?;
                }
            }
            NodeValidators::Boolean { validator: Some(_) } => {
                return Err(self.false_schema_error(instance, location, tracker));
            }
            NodeValidators::Boolean { validator: None } => return Ok(()),
        }
        Ok(())
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn collect_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
        errors: &mut Vec<ValidationError<'i>>,
    ) {
        match &self.inner.validators {
            NodeValidators::Keyword(kvs) => {
                for (entry, metadata) in kvs.validators.as_slice().iter().zip(&kvs.metadata) {
                    if entry.is_leaf && entry.validator.is_valid(instance, ctx) {
                        continue;
                    }
                    let start = errors.len();
                    entry
                        .validator
                        .collect_errors(instance, location, tracker, ctx, errors);
                    stamp_absolute_location(
                        &mut errors[start..],
                        metadata.absolute_location(self.inner.absolute_base.as_ref()),
                    );
                }
            }
            NodeValidators::Boolean {
                validator: Some(v), ..
            } => {
                let start = errors.len();
                v.collect_errors(instance, location, tracker, ctx, errors);
                stamp_absolute_location(&mut errors[start..], self.absolute_path());
            }
            NodeValidators::Boolean {
                validator: None, ..
            } => {}
            NodeValidators::Array { validators } => {
                for entry in validators {
                    let start = errors.len();
                    entry
                        .validator
                        .collect_errors(instance, location, tracker, ctx, errors);
                    stamp_absolute_location(
                        &mut errors[start..],
                        entry.absolute_location(self.inner.absolute_base.as_ref()),
                    );
                }
            }
        }
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        self.evaluate_at(
            instance,
            location,
            ctx.instance_position(location),
            tracker,
            ctx,
        )
    }

    fn evaluate_with_location(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: InstancePosition,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        self.evaluate_at(instance, location, instance_location, tracker, ctx)
    }
}

impl<F: Json> SchemaNode<F> {
    /// `evaluate` with the instance location already built.
    fn evaluate_at(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_loc: InstancePosition,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        match &self.inner.validators {
            NodeValidators::Array { ref validators } => Self::evaluate_subschemas(
                instance,
                location,
                instance_loc,
                tracker,
                validators.iter().map(|entry| EntryParts {
                    location: &entry.location,
                    absolute_base: self.inner.absolute_base.as_ref(),
                    absolute_location: &entry.absolute_location,
                    schema_location: &entry.schema_location,
                    validator: &entry.validator,
                }),
                None,
                ctx,
            ),
            NodeValidators::Boolean { ref validator } => {
                if let Some(validator) = validator {
                    validator.evaluate(instance, location, tracker, ctx)
                } else {
                    EvaluationResult::Valid {
                        annotations: None,
                        children: ChildList::default(),
                    }
                }
            }
            NodeValidators::Keyword(ref kvals) => {
                let KeywordValidators {
                    ref unmatched_keywords,
                    ref validators,
                    ref metadata,
                    ref absorbed,
                } = *kvals;
                let validators = validators.as_slice();
                let absolute_base = self.inner.absolute_base.as_ref();
                let annotations: Option<Annotations> = unmatched_keywords
                    .as_ref()
                    .map(|v| Annotations::from_arc(Arc::clone(v)));
                if let Some(absorbed) = absorbed {
                    Self::evaluate_with_absorbed(
                        instance,
                        location,
                        instance_loc,
                        tracker,
                        validators,
                        metadata,
                        absolute_base,
                        absorbed,
                        annotations,
                        ctx,
                    )
                } else {
                    Self::evaluate_subschemas(
                        instance,
                        location,
                        instance_loc,
                        tracker,
                        metadata
                            .iter()
                            .zip(validators)
                            .map(|(m, v)| m.parts(&v.validator, absolute_base)),
                        annotations,
                        ctx,
                    )
                }
            }
        }
    }
}

enum NodeValidatorsIter<'a, F: Json> {
    NoValidator,
    BooleanValidators(std::iter::Once<&'a BoxedValidator<F>>),
    KeywordValidators(std::slice::Iter<'a, KeywordValidator<F>>),
    ArrayValidators(std::slice::Iter<'a, ArrayValidatorEntry<F>>),
}

impl<'a, F: Json> Iterator for NodeValidatorsIter<'a, F> {
    type Item = &'a BoxedValidator<F>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::NoValidator => None,
            Self::BooleanValidators(i) => i.next(),
            Self::KeywordValidators(v) => v.next().map(|entry| &entry.validator),
            Self::ArrayValidators(v) => v.next().map(|entry| &entry.validator),
        }
    }

    fn all<T>(&mut self, mut f: T) -> bool
    where
        Self: Sized,
        T: FnMut(Self::Item) -> bool,
    {
        match self {
            Self::NoValidator => true,
            Self::BooleanValidators(i) => i.all(f),
            Self::KeywordValidators(v) => v.all(|entry| f(&entry.validator)),
            Self::ArrayValidators(v) => v.all(|entry| f(&entry.validator)),
        }
    }
}

impl<F: Json> ExactSizeIterator for NodeValidatorsIter<'_, F> {
    fn len(&self) -> usize {
        match self {
            Self::NoValidator => 0,
            Self::BooleanValidators(..) => 1,
            Self::KeywordValidators(v) => v.len(),
            Self::ArrayValidators(v) => v.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use test_case::test_case;

    // Applicators over subschemas that only check `type`
    #[test_case(&json!({"allOf": [{"type": "string"}, {"type": ["string", "null"]}]}), &json!("a"), true; "allOf both")]
    #[test_case(&json!({"allOf": [{"type": "string"}, {"type": ["string", "null"]}]}), &json!(null), false; "allOf one")]
    #[test_case(&json!({"anyOf": [{"type": "string"}, {"type": "array"}]}), &json!([]), true; "anyOf second")]
    #[test_case(&json!({"anyOf": [{"type": "string"}, {"type": "array"}]}), &json!(1), false; "anyOf none")]
    #[test_case(&json!({"oneOf": [{"type": "integer"}, {"type": "number"}]}), &json!(1), false; "oneOf both numeric")]
    #[test_case(&json!({"oneOf": [{"type": "integer"}, {"type": "number"}]}), &json!(1.5), true; "oneOf number only")]
    #[test_case(&json!({"oneOf": [{"type": "string"}, {"type": "null"}]}), &json!(true), false; "oneOf none")]
    #[test_case(&json!({"items": {"type": "string"}}), &json!(["a", "b"]), true; "items all")]
    #[test_case(&json!({"items": {"type": "string"}}), &json!(["a", 1]), false; "items one wrong")]
    #[test_case(&json!({"prefixItems": [{"type": "null"}, {"type": "boolean"}]}), &json!([null, true, 3]), true; "prefixItems")]
    #[test_case(&json!({"prefixItems": [{"type": "null"}, {"type": "boolean"}]}), &json!([null, 1]), false; "prefixItems wrong")]
    #[test_case(&json!({"$schema": "http://json-schema.org/draft-07/schema#", "items": [{"type": "null"}], "additionalItems": {"type": "string"}}), &json!([null, "a"]), true; "additionalItems")]
    #[test_case(&json!({"$schema": "http://json-schema.org/draft-07/schema#", "items": [{"type": "null"}], "additionalItems": {"type": "string"}}), &json!([null, 2]), false; "additionalItems wrong")]
    #[test_case(&json!({"contains": {"type": "object"}}), &json!([1, {}]), true; "contains")]
    #[test_case(&json!({"contains": {"type": "object"}}), &json!([1, 2]), false; "contains none")]
    #[test_case(&json!({"propertyNames": {"type": "string"}}), &json!({"a": 1}), true; "propertyNames")]
    #[test_case(&json!({"dependentSchemas": {"a": {"type": "object"}}}), &json!({"a": 1}), true; "dependentSchemas")]
    #[test_case(&json!({"not": {"type": "null"}}), &json!(null), false; "not")]
    #[test_case(&json!({"not": {"type": "null"}}), &json!(1), true; "not other")]
    #[test_case(&json!({"if": {"type": "string"}, "then": {"type": "string"}, "else": {"type": "integer"}}), &json!(1.0), true; "if else integer value")]
    #[test_case(&json!({"if": {"type": "string"}, "then": {"type": "string"}, "else": {"type": "integer"}}), &json!(1.5), false; "if else fraction")]
    #[test_case(&json!({"$defs": {"s": {"type": "string"}}, "$ref": "#/$defs/s"}), &json!("a"), true; "ref")]
    #[test_case(&json!({"$defs": {"s": {"type": "string"}}, "$ref": "#/$defs/s"}), &json!(1), false; "ref wrong")]
    fn child_loops_match_is_valid(schema: &Value, instance: &Value, expected: bool) {
        let validator = crate::validator_for(schema).expect("schema compiles");
        assert_eq!(validator.is_valid(instance), expected);
        assert_eq!(validator.validate(instance).is_ok(), expected);
        assert_eq!(validator.evaluate(instance).flag().valid, expected);
    }

    // Typed subschemas with more keywords, as `anyOf` branches beside `{"type": "boolean"}`
    #[test_case(&json!({"type": "string", "minLength": 2}), &json!(null), false; "string min length, null")]
    #[test_case(&json!({"type": "string", "minLength": 2}), &json!(true), true; "string min length, true")]
    #[test_case(&json!({"type": "string", "minLength": 2}), &json!(1), false; "string min length, 1")]
    #[test_case(&json!({"type": "string", "minLength": 2}), &json!(1.0), false; "string min length, 1.0")]
    #[test_case(&json!({"type": "string", "minLength": 2}), &json!(1.5), false; "string min length, 1.5")]
    #[test_case(&json!({"type": "string", "minLength": 2}), &json!(-1), false; "string min length, negative")]
    #[test_case(&json!({"type": "string", "minLength": 2}), &json!("a"), false; "string min length, short string")]
    #[test_case(&json!({"type": "string", "minLength": 2}), &json!("ab"), true; "string min length, string")]
    #[test_case(&json!({"type": "string", "minLength": 2}), &json!([]), false; "string min length, empty array")]
    #[test_case(&json!({"type": "string", "minLength": 2}), &json!([1]), false; "string min length, array")]
    #[test_case(&json!({"type": "string", "minLength": 2}), &json!({}), false; "string min length, empty object")]
    #[test_case(&json!({"type": "string", "minLength": 2}), &json!({"a": 1}), false; "string min length, object")]
    #[test_case(&json!({"type": "object", "required": ["a"]}), &json!(null), false; "object required, null")]
    #[test_case(&json!({"type": "object", "required": ["a"]}), &json!(true), true; "object required, true")]
    #[test_case(&json!({"type": "object", "required": ["a"]}), &json!(1), false; "object required, 1")]
    #[test_case(&json!({"type": "object", "required": ["a"]}), &json!(1.0), false; "object required, 1.0")]
    #[test_case(&json!({"type": "object", "required": ["a"]}), &json!(1.5), false; "object required, 1.5")]
    #[test_case(&json!({"type": "object", "required": ["a"]}), &json!(-1), false; "object required, negative")]
    #[test_case(&json!({"type": "object", "required": ["a"]}), &json!("a"), false; "object required, short string")]
    #[test_case(&json!({"type": "object", "required": ["a"]}), &json!("ab"), false; "object required, string")]
    #[test_case(&json!({"type": "object", "required": ["a"]}), &json!([]), false; "object required, empty array")]
    #[test_case(&json!({"type": "object", "required": ["a"]}), &json!([1]), false; "object required, array")]
    #[test_case(&json!({"type": "object", "required": ["a"]}), &json!({}), false; "object required, empty object")]
    #[test_case(&json!({"type": "object", "required": ["a"]}), &json!({"a": 1}), true; "object required, object")]
    #[test_case(&json!({"type": ["array", "null"], "minItems": 1}), &json!(null), true; "array or null min items, null")]
    #[test_case(&json!({"type": ["array", "null"], "minItems": 1}), &json!(true), true; "array or null min items, true")]
    #[test_case(&json!({"type": ["array", "null"], "minItems": 1}), &json!(1), false; "array or null min items, 1")]
    #[test_case(&json!({"type": ["array", "null"], "minItems": 1}), &json!(1.0), false; "array or null min items, 1.0")]
    #[test_case(&json!({"type": ["array", "null"], "minItems": 1}), &json!(1.5), false; "array or null min items, 1.5")]
    #[test_case(&json!({"type": ["array", "null"], "minItems": 1}), &json!(-1), false; "array or null min items, negative")]
    #[test_case(&json!({"type": ["array", "null"], "minItems": 1}), &json!("a"), false; "array or null min items, short string")]
    #[test_case(&json!({"type": ["array", "null"], "minItems": 1}), &json!("ab"), false; "array or null min items, string")]
    #[test_case(&json!({"type": ["array", "null"], "minItems": 1}), &json!([]), false; "array or null min items, empty array")]
    #[test_case(&json!({"type": ["array", "null"], "minItems": 1}), &json!([1]), true; "array or null min items, array")]
    #[test_case(&json!({"type": ["array", "null"], "minItems": 1}), &json!({}), false; "array or null min items, empty object")]
    #[test_case(&json!({"type": ["array", "null"], "minItems": 1}), &json!({"a": 1}), false; "array or null min items, object")]
    #[test_case(&json!({"type": "integer", "minimum": 0}), &json!(null), false; "integer minimum, null")]
    #[test_case(&json!({"type": "integer", "minimum": 0}), &json!(true), true; "integer minimum, true")]
    #[test_case(&json!({"type": "integer", "minimum": 0}), &json!(1), true; "integer minimum, 1")]
    #[test_case(&json!({"type": "integer", "minimum": 0}), &json!(1.0), true; "integer minimum, 1.0")]
    #[test_case(&json!({"type": "integer", "minimum": 0}), &json!(1.5), false; "integer minimum, 1.5")]
    #[test_case(&json!({"type": "integer", "minimum": 0}), &json!(-1), false; "integer minimum, negative")]
    #[test_case(&json!({"type": "integer", "minimum": 0}), &json!("a"), false; "integer minimum, short string")]
    #[test_case(&json!({"type": "integer", "minimum": 0}), &json!("ab"), false; "integer minimum, string")]
    #[test_case(&json!({"type": "integer", "minimum": 0}), &json!([]), false; "integer minimum, empty array")]
    #[test_case(&json!({"type": "integer", "minimum": 0}), &json!([1]), false; "integer minimum, array")]
    #[test_case(&json!({"type": "integer", "minimum": 0}), &json!({}), false; "integer minimum, empty object")]
    #[test_case(&json!({"type": "integer", "minimum": 0}), &json!({"a": 1}), false; "integer minimum, object")]
    fn typed_child_rejects_other_kinds(child: &Value, instance: &Value, expected: bool) {
        let schema = json!({"anyOf": [child, {"type": "boolean"}]});
        let validator = crate::validator_for(&schema).expect("schema compiles");
        assert_eq!(validator.is_valid(instance), expected);
        assert_eq!(validator.validate(instance).is_ok(), expected);
        assert_eq!(validator.evaluate(instance).flag().valid, expected);
    }

    fn chain(hops: usize, end: &Value, draft: &str) -> Value {
        let mut defs = serde_json::Map::new();
        for hop in 0..hops {
            defs.insert(
                format!("d{hop}"),
                json!({"$ref": format!("#/$defs/d{}", hop + 1)}),
            );
        }
        defs.insert(format!("d{hops}"), end.clone());
        json!({"$schema": draft, "$defs": defs, "properties": {"a": {"$ref": "#/$defs/d0"}}})
    }

    const DRAFT7: &str = "http://json-schema.org/draft-07/schema#";
    const DRAFT2020: &str = "https://json-schema.org/draft/2020-12/schema";

    // Members reached through chains of `$ref`-only subschemas
    #[test_case(1, &json!({"type": "string"}), &json!("x"), true; "one hop to a type")]
    #[test_case(1, &json!({"type": "string"}), &json!(1), false; "one hop to a type, wrong")]
    #[test_case(2, &json!({"type": "string"}), &json!("x"), true; "two hops to a type")]
    #[test_case(2, &json!({"type": "string"}), &json!(1), false; "two hops to a type, wrong")]
    #[test_case(3, &json!({"minLength": 2}), &json!("ab"), true; "three hops to a keyword")]
    #[test_case(3, &json!({"minLength": 2}), &json!("a"), false; "three hops to a keyword, wrong")]
    #[test_case(2, &json!(false), &json!(1), false; "two hops to false")]
    #[test_case(2, &json!(true), &json!(1), true; "two hops to true")]
    fn ref_chains_match_is_valid(hops: usize, end: &Value, value: &Value, expected: bool) {
        for draft in [DRAFT7, DRAFT2020] {
            let schema = chain(hops, end, draft);
            let instance = json!({"a": value});
            let validator = crate::validator_for(&schema).expect("schema compiles");
            assert_eq!(validator.is_valid(&instance), expected, "{draft}");
            assert_eq!(validator.validate(&instance).is_ok(), expected, "{draft}");
            assert_eq!(
                validator.evaluate(&instance).flag().valid,
                expected,
                "{draft}"
            );
        }
    }

    #[test_case(&json!({"n": {"n": {"n": {"v": 1}}}}), true; "deep valid")]
    #[test_case(&json!({"n": {"n": {"n": {"v": "x"}}}}), false; "deep invalid")]
    fn ref_cycle_matches_is_valid(instance: &Value, expected: bool) {
        let schema = json!({
            "$defs": {
                "node": {"properties": {"n": {"$ref": "#/$defs/next"}, "v": {"type": "integer"}}},
                "next": {"$ref": "#/$defs/node"}
            },
            "$ref": "#/$defs/node"
        });
        let validator = crate::validator_for(&schema).expect("schema compiles");
        assert_eq!(validator.is_valid(instance), expected);
        assert_eq!(validator.validate(instance).is_ok(), expected);
        assert_eq!(validator.evaluate(instance).flag().valid, expected);
    }
}
