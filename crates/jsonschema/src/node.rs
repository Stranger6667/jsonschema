use crate::{
    compiler::Context,
    evaluation::{Annotations, ChildList, EvaluationNode},
    keywords::{items::AbsorbedKeywords, BoxedValidator, BuiltinKeyword, Keyword},
    paths::{LazyLocation, Location, RefTracker},
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
    validators: NodeValidators<F>,
    formatted_schema_location: OnceLock<Arc<str>>,
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
    absolute_path: Option<Arc<Uri<String>>>,
}

impl<F: Json> Clone for SchemaNode<F> {
    fn clone(&self) -> Self {
        SchemaNode {
            inner: Arc::clone(&self.inner),
            location: self.location.clone(),
            absolute_path: self.absolute_path.clone(),
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
    absolute_path: Option<Arc<Uri<String>>>,
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
    validators: Vec<KeywordValidatorEntry<F>>,
    absorbed: Option<AbsorbedUnits>,
}

/// Keywords a fused validator checks on its own. `evaluate` reports them as their own units, so
/// the output lists the same units as without the fusion.
struct AbsorbedUnits {
    keywords: Arc<AbsorbedKeywords>,
    /// How many of the node's validators run before `minItems` and `maxItems`; `type` runs first.
    counts_at: usize,
}

struct KeywordValidatorEntry<F: Json> {
    validator: BoxedValidator<F>,
    /// Asserts on the instance itself, so `is_valid` may run ahead of `validate`.
    is_leaf: bool,
    location: Location,
    absolute_location: Option<Arc<Uri<String>>>,
    formatted_schema_location: OnceLock<Arc<str>>,
}

/// A subschema's location, absolute location, cached `schemaLocation` and validator.
type EntryParts<'a, F> = (
    &'a Location,
    Option<&'a Arc<Uri<String>>>,
    &'a OnceLock<Arc<str>>,
    &'a BoxedValidator<F>,
);

impl<F: Json> KeywordValidatorEntry<F> {
    fn parts(&self) -> EntryParts<'_, F> {
        (
            &self.location,
            self.absolute_location.as_ref(),
            &self.formatted_schema_location,
            &self.validator,
        )
    }
}

struct ArrayValidatorEntry<F: Json> {
    validator: BoxedValidator<F>,
    location: Location,
    absolute_location: Option<Arc<Uri<String>>>,
    formatted_schema_location: OnceLock<Arc<str>>,
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
            absolute_path: node.absolute_path.clone(),
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
            absolute_path: self.absolute_path.clone(),
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
            let computed = target.materialize().is_valid(instance, ctx);
            // Cache result for recursive schemas
            ctx.cache_result(node_id, container_identity, computed);
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
        self.evaluate_with_location(instance, location, &location.into(), tracker, ctx)
    }

    fn evaluate_with_location(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: &Location,
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
        let location = ctx.location().clone();
        let absolute_path = ctx.absolute_location(&location);
        SchemaNode {
            inner: Arc::new(SchemaNodeInner {
                validators: NodeValidators::Boolean { validator },
                formatted_schema_location: OnceLock::new(),
            }),
            location,
            absolute_path,
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

        let location = ctx.location().clone();
        let absolute_path = ctx.absolute_location(&location);
        let validators = validators
            .into_iter()
            .map(|(keyword, validator)| {
                let location = ctx.location().join(&keyword);
                let absolute_location = ctx.absolute_location(&location);
                let is_leaf = crate::keywords::keyword_is_leaf(&keyword);
                KeywordValidatorEntry {
                    validator,
                    is_leaf,
                    location,
                    absolute_location,
                    formatted_schema_location: OnceLock::new(),
                }
            })
            .collect();
        SchemaNode {
            inner: Arc::new(SchemaNodeInner {
                validators: NodeValidators::Keyword(KeywordValidators {
                    unmatched_keywords,
                    validators,
                    absorbed,
                }),
                formatted_schema_location: OnceLock::new(),
            }),
            location,
            absolute_path,
        }
    }

    pub(crate) fn from_array(
        ctx: &Context<'_, F>,
        validators: Vec<BoxedValidator<F>>,
    ) -> SchemaNode<F> {
        let location = ctx.location().clone();
        let absolute_path = ctx.absolute_location(&location);
        let validators = validators
            .into_iter()
            .enumerate()
            .map(|(index, validator)| {
                let location = ctx.location().join(index);
                let absolute_location = ctx.absolute_location(&location);
                ArrayValidatorEntry {
                    validator,
                    location,
                    absolute_location,
                    formatted_schema_location: OnceLock::new(),
                }
            })
            .collect();
        SchemaNode {
            inner: Arc::new(SchemaNodeInner {
                validators: NodeValidators::Array { validators },
                formatted_schema_location: OnceLock::new(),
            }),
            location,
            absolute_path,
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
                NodeValidatorsIter::KeywordValidators(kvals.validators.iter())
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
        let instance_location: Location = location.into();
        self.evaluate_instance_at(instance, location, &instance_location, tracker, ctx)
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
        let instance_location = match ctx.instance_location() {
            Some(parent) => parent.join_pointer_segment(location.segment()),
            None => location.into(),
        };
        debug_assert_eq!(instance_location, Location::from(location));
        self.evaluate_instance_at(instance, location, &instance_location, tracker, ctx)
    }

    /// [`Self::evaluate_instance`] for a caller that already rendered this instance position.
    pub(crate) fn evaluate_instance_at(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: &Location,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationNode {
        let instance_location = instance_location.clone();

        let keyword_location = crate::paths::evaluation_path(tracker, &self.location, ctx);
        let schema_location = Arc::clone(self.inner.formatted_schema_location.get_or_init(|| {
            crate::evaluation::format_keyword_location(&self.location, self.absolute_path.as_ref())
        }));

        let previous = ctx.enter_instance_location(instance_location.clone());
        let result = self.evaluate_at(instance, location, &instance_location, tracker, ctx);
        ctx.leave_instance_location(previous);

        match result {
            EvaluationResult::Valid {
                annotations,
                children,
            } => EvaluationNode::valid(
                keyword_location,
                self.absolute_path.clone(),
                schema_location.clone(),
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
                self.absolute_path.clone(),
                schema_location,
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
        instance_loc: &Location,
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
        instance_loc: &Location,
        tracker: Option<&RefTracker>,
        validators: &[KeywordValidatorEntry<F>],
        absorbed: &AbsorbedUnits,
        annotations: Option<Annotations>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let count = instance.as_array().map(|array| array.len() as u64);
        let keywords = &*absorbed.keywords;
        let mut children = ChildList::default();
        let node =
            keywords.evaluate_type::<F>(instance, count.is_some(), instance_loc, tracker, ctx);
        children.push(&mut ctx.arena, node);
        let (before, after) = validators.split_at(absorbed.counts_at);
        Self::push_subschemas(
            instance,
            location,
            instance_loc,
            tracker,
            before.iter().map(KeywordValidatorEntry::parts),
            &mut children,
            ctx,
        );
        keywords.evaluate_counts::<F>(instance, count, instance_loc, tracker, ctx, &mut children);
        Self::push_subschemas(
            instance,
            location,
            instance_loc,
            tracker,
            after.iter().map(KeywordValidatorEntry::parts),
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
        instance_loc: &Location,
        tracker: Option<&RefTracker>,
        subschemas: I,
        children: &mut ChildList,
        ctx: &mut ValidationContext,
    ) -> bool
    where
        I: Iterator<Item = EntryParts<'a, F>> + 'a,
    {
        let mut invalid = false;

        for (child_location, absolute_location, cached_schema_location, validator) in subschemas {
            let child_result =
                validator.evaluate_with_location(instance, location, instance_loc, tracker, ctx);

            // schemaLocation: The canonical location WITHOUT $ref traversals.
            // Per JSON Schema spec: "MUST NOT include by-reference applicators such as $ref"
            // For by-reference validators like $ref, use the target's canonical location,
            // which lives in the target's resource.
            // For regular validators, use the keyword's location.
            let (schema_location, absolute_location) = validator
                .canonical_location()
                .unwrap_or((child_location, absolute_location));
            let absolute_location = absolute_location.cloned();

            let eval_path = crate::paths::evaluation_path(tracker, child_location, ctx);

            // schemaLocation is fixed per subschema, by-reference or not, so it is rendered once.
            let formatted_schema_location = Arc::clone(cached_schema_location.get_or_init(|| {
                crate::evaluation::format_keyword_location(
                    schema_location,
                    absolute_location.as_ref(),
                )
            }));

            let child_node = match child_result {
                EvaluationResult::Valid {
                    annotations,
                    children,
                } => EvaluationNode::valid(
                    eval_path,
                    absolute_location,
                    formatted_schema_location,
                    instance_loc.clone(),
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
                        instance_loc.clone(),
                        annotations,
                        errors,
                        children,
                    )
                }
            };
            children.push(&mut ctx.arena, child_node);
        }
        invalid
    }

    pub(crate) fn location(&self) -> &Location {
        &self.location
    }
}

fn stamp_absolute_location(errors: &mut [ValidationError<'_>], uri: Option<&Arc<Uri<String>>>) {
    let Some(uri) = uri else { return };
    for error in errors {
        error.set_absolute_keyword_location(uri);
    }
}

impl<F: Json> SchemaNode<F> {
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
        .with_absolute_keyword_location(self.absolute_path.clone())
    }
}

impl<F: Json> Validate<F> for SchemaNode<F> {
    #[inline]
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        match &self.inner.validators {
            // Single validator fast path
            NodeValidators::Keyword(kvs) if kvs.validators.len() == 1 => {
                kvs.validators[0].validator.is_valid(instance, ctx)
            }
            NodeValidators::Keyword(kvs) => {
                for entry in &kvs.validators {
                    if !entry.validator.is_valid(instance, ctx) {
                        return false;
                    }
                }
                true
            }
            NodeValidators::Array { validators } => validators
                .iter()
                .all(|entry| entry.validator.is_valid(instance, ctx)),
            NodeValidators::Boolean { validator: Some(_) } => false,
            NodeValidators::Boolean { validator: None } => true,
        }
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
            NodeValidators::Keyword(kvs) if kvs.validators.len() == 1 => {
                let entry = &kvs.validators[0];
                // A passing keyword costs its `is_valid`; only a failing one builds an error.
                if entry.is_leaf && entry.validator.is_valid(instance, ctx) {
                    return Ok(());
                }
                return entry
                    .validator
                    .validate(instance, location, tracker, ctx)
                    .map_err(|e| {
                        e.with_absolute_keyword_location(entry.absolute_location.clone())
                    });
            }
            NodeValidators::Keyword(kvs) => {
                for entry in &kvs.validators {
                    if entry.is_leaf && entry.validator.is_valid(instance, ctx) {
                        continue;
                    }
                    entry
                        .validator
                        .validate(instance, location, tracker, ctx)
                        .map_err(|e| {
                            e.with_absolute_keyword_location(entry.absolute_location.clone())
                        })?;
                }
            }
            NodeValidators::Array { validators } => {
                for entry in validators {
                    entry
                        .validator
                        .validate(instance, location, tracker, ctx)
                        .map_err(|e| {
                            e.with_absolute_keyword_location(entry.absolute_location.clone())
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
                for entry in &kvs.validators {
                    if entry.is_leaf && entry.validator.is_valid(instance, ctx) {
                        continue;
                    }
                    let start = errors.len();
                    entry
                        .validator
                        .collect_errors(instance, location, tracker, ctx, errors);
                    stamp_absolute_location(&mut errors[start..], entry.absolute_location.as_ref());
                }
            }
            NodeValidators::Boolean {
                validator: Some(v), ..
            } => {
                let start = errors.len();
                v.collect_errors(instance, location, tracker, ctx, errors);
                stamp_absolute_location(&mut errors[start..], self.absolute_path.as_ref());
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
                    stamp_absolute_location(&mut errors[start..], entry.absolute_location.as_ref());
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
        self.evaluate_at(instance, location, &location.into(), tracker, ctx)
    }

    fn evaluate_with_location(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        instance_location: &Location,
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
        instance_loc: &Location,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        match &self.inner.validators {
            NodeValidators::Array { ref validators } => Self::evaluate_subschemas(
                instance,
                location,
                instance_loc,
                tracker,
                validators.iter().map(|entry| {
                    (
                        &entry.location,
                        entry.absolute_location.as_ref(),
                        &entry.formatted_schema_location,
                        &entry.validator,
                    )
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
                    ref absorbed,
                } = *kvals;
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
                        validators.iter().map(KeywordValidatorEntry::parts),
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
    KeywordValidators(std::slice::Iter<'a, KeywordValidatorEntry<F>>),
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
