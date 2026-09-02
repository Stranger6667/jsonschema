use crate::{
    compiler::Context,
    evaluation::{Annotations, EvaluationNode},
    keywords::{BoxedValidator, Keyword},
    paths::{LazyLocation, Location, RefTracker},
    validator::{EvaluationResult, Validate, ValidationContext},
    Json, Node, SerdeJson, ValidationError,
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

struct PendingTarget<F: Json> {
    inner: Weak<SchemaNodeInner<F>>,
    location: Location,
    absolute_path: Option<Arc<Uri<String>>>,
}

impl<F: Json> fmt::Debug for PendingTarget<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingTarget")
            .field("location", &self.location)
            .finish_non_exhaustive()
    }
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
}

struct KeywordValidatorEntry<F: Json> {
    validator: BoxedValidator<F>,
    location: Location,
    absolute_location: Option<Arc<Uri<String>>>,
    formatted_schema_location: OnceLock<Arc<str>>,
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

    pub(crate) fn initialize(&self, node: &SchemaNode<F>) {
        let target = PendingTarget {
            inner: Arc::downgrade(&node.inner),
            location: node.location.clone(),
            absolute_path: node.absolute_path.clone(),
        };
        self.cell
            .set(target)
            .expect("pending node initialized twice");
    }

    pub(crate) fn get(&self) -> Option<SchemaNode<F>> {
        self.cell.get().map(PendingTarget::materialize)
    }

    fn with_node<T, R>(&self, f: T) -> R
    where
        T: FnOnce(&SchemaNode<F>) -> R,
    {
        let target = self
            .cell
            .get()
            .expect("pending node accessed before initialization");
        let node = target.materialize();
        f(&node)
    }

    /// Get a unique identifier for this pending node.
    /// Uses the address of the inner cell as a stable identifier.
    #[inline]
    fn node_id(&self) -> usize {
        Arc::as_ptr(&self.cell) as usize
    }
}

impl<F: Json> PendingTarget<F> {
    fn materialize(&self) -> SchemaNode<F> {
        let inner = self.inner.upgrade().expect("pending schema target dropped");
        SchemaNode {
            inner,
            location: self.location.clone(),
            absolute_path: self.absolute_path.clone(),
        }
    }
}

impl<F: Json> Validate<F> for PendingSchemaNode<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        let node_id = self.node_id();
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
            let computed = self.with_node(|node| node.is_valid(instance, ctx));
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
        let identity = instance.identity();
        if ctx.enter(self.node_id(), identity) {
            return Ok(());
        }
        let result = self.with_node(|node| node.validate(instance, location, tracker, ctx));
        ctx.exit(self.node_id(), identity);
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
        let identity = instance.identity();
        if ctx.enter(self.node_id(), identity) {
            return;
        }
        self.with_node(|node| node.collect_errors(instance, location, tracker, ctx, errors));
        ctx.exit(self.node_id(), identity);
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        let identity = instance.identity();
        if ctx.enter(self.node_id(), identity) {
            return EvaluationResult::valid_empty();
        }
        let result = self.with_node(|node| node.evaluate(instance, location, tracker, ctx));
        ctx.exit(self.node_id(), identity);
        result
    }
}

impl<F: Json> SchemaNode<F> {
    pub(crate) fn from_boolean(
        ctx: &Context<'_, F>,
        validator: Option<BoxedValidator<F>>,
    ) -> SchemaNode<F> {
        let location = ctx.location().clone();
        let absolute_path = ctx.base_uri();
        SchemaNode {
            inner: Arc::new(SchemaNodeInner {
                validators: NodeValidators::Boolean { validator },
                formatted_schema_location: OnceLock::new(),
            }),
            location,
            absolute_path,
        }
    }

    pub(crate) fn from_keywords(
        ctx: &Context<'_, F>,
        mut validators: Vec<(Keyword, BoxedValidator<F>)>,
        unmatched_keywords: Option<Arc<Value>>,
    ) -> SchemaNode<F> {
        // Sort validators by priority (lower = execute first).
        // This enables "fail fast" by running cheap validators (type, const)
        // before expensive ones (allOf, $ref).
        validators.sort_by_key(|(keyword, _)| crate::keywords::keyword_priority(keyword));

        let location = ctx.location().clone();
        let absolute_path = ctx.base_uri();
        let validators = validators
            .into_iter()
            .map(|(keyword, validator)| {
                let location = ctx.location().join(&keyword);
                let absolute_location = ctx.absolute_location(&location);
                KeywordValidatorEntry {
                    validator,
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
        let absolute_path = ctx.base_uri();
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
            crate::evaluation::format_schema_location(&self.location, self.absolute_path.as_ref())
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
        I: Iterator<
                Item = (
                    &'a Location,
                    Option<&'a Arc<Uri<String>>>,
                    &'a OnceLock<Arc<str>>,
                    &'a BoxedValidator<F>,
                ),
            > + 'a,
    {
        let (lower_bound, _) = subschemas.size_hint();
        let mut children: Vec<EvaluationNode> = Vec::with_capacity(lower_bound);
        let mut invalid = false;

        for (child_location, absolute_location, cached_schema_location, validator) in subschemas {
            let child_result =
                validator.evaluate_with_location(instance, location, instance_loc, tracker, ctx);

            let absolute_location = absolute_location.cloned();

            let eval_path = crate::paths::evaluation_path(tracker, child_location, ctx);

            // schemaLocation: The canonical location WITHOUT $ref traversals.
            // Per JSON Schema spec: "MUST NOT include by-reference applicators such as $ref"
            // For by-reference validators like $ref, use the target's canonical location.
            // For regular validators, use the keyword's location.
            // schemaLocation is fixed per subschema, by-reference or not, so it is rendered once.
            let formatted_schema_location = Arc::clone(cached_schema_location.get_or_init(|| {
                crate::evaluation::format_schema_location(
                    validator.canonical_location().unwrap_or(child_location),
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
            children.push(child_node);
        }
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

impl<F: Json> Validate<F> for SchemaNode<F> {
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
                return entry
                    .validator
                    .validate(instance, location, tracker, ctx)
                    .map_err(|e| {
                        e.with_absolute_keyword_location(entry.absolute_location.clone())
                    });
            }
            NodeValidators::Keyword(kvs) => {
                for entry in &kvs.validators {
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
                return Err(ValidationError::false_schema(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance.lazy_value(),
                )
                .with_absolute_keyword_location(self.absolute_path.clone()));
            }
            NodeValidators::Boolean { validator: None } => return Ok(()),
        }
        Ok(())
    }

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
                        children: Vec::new(),
                    }
                }
            }
            NodeValidators::Keyword(ref kvals) => {
                let KeywordValidators {
                    ref unmatched_keywords,
                    ref validators,
                } = *kvals;
                let annotations: Option<Annotations> = unmatched_keywords
                    .as_ref()
                    .map(|v| Annotations::from_arc(Arc::clone(v)));
                Self::evaluate_subschemas(
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
                    annotations,
                    ctx,
                )
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
