use crate::{
    compiler::Context,
    error::ErrorIterator,
    evaluation::{Annotations, EvaluationNode},
    keywords::{BoxedValidator, Keyword},
    paths::{LazyLocation, Location, RefTracker},
    validator::{EvaluationResult, Validate, ValidationContext},
    ValidationError,
};
use referencing::Uri;
use serde_json::Value;
use std::{
    fmt,
    sync::{Arc, OnceLock, Weak},
};

/// A node in the schema tree, returned by `compiler::compile`
#[derive(Clone, Debug)]
pub(crate) struct SchemaNode {
    validators: Arc<NodeValidators>,
    location: Location,
    absolute_path: Option<Arc<Uri<String>>>,
}

// Separate type used only during compilation for handling recursive references
#[derive(Clone, Debug)]
pub(crate) struct PendingSchemaNode {
    cell: Arc<OnceLock<PendingTarget>>,
}

#[derive(Debug)]
struct PendingTarget {
    validators: Weak<NodeValidators>,
    location: Location,
    absolute_path: Option<Arc<Uri<String>>>,
}

enum NodeValidators {
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
    Boolean { validator: Option<BoxedValidator> },
    /// The result of compiling a schema which is composed of keywords (almost all schemas)
    Keyword(KeywordValidators),
    /// The result of compiling a schema which is "array valued", e.g the "dependencies" keyword of
    /// draft 7 which can take values which are an array of other property names
    Array {
        validators: Vec<ArrayValidatorEntry>,
    },
}

impl fmt::Debug for NodeValidators {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boolean { .. } => f.debug_struct("Boolean").finish(),
            Self::Keyword(_) => f.debug_tuple("Keyword").finish(),
            Self::Array { .. } => f.debug_struct("Array").finish(),
        }
    }
}

struct KeywordValidators {
    /// The keywords on this node which were not recognized by any vocabularies. These are
    /// stored so we can later produce them as annotations
    unmatched_keywords: Option<Arc<Value>>,
    // We should probably use AHashMap here but it breaks a bunch of tests which assume
    // validators are in a particular order
    validators: Vec<KeywordValidatorEntry>,
}

struct KeywordValidatorEntry {
    validator: BoxedValidator,
    location: Location,
    absolute_location: Option<Arc<Uri<String>>>,
}

struct ArrayValidatorEntry {
    validator: BoxedValidator,
    location: Location,
    absolute_location: Option<Arc<Uri<String>>>,
}

impl PendingSchemaNode {
    pub(crate) fn new() -> Self {
        PendingSchemaNode {
            cell: Arc::new(OnceLock::new()),
        }
    }

    pub(crate) fn initialize(&self, node: &SchemaNode) {
        let target = PendingTarget {
            validators: Arc::downgrade(&node.validators),
            location: node.location.clone(),
            absolute_path: node.absolute_path.clone(),
        };
        self.cell
            .set(target)
            .expect("pending node initialized twice");
    }

    pub(crate) fn get(&self) -> Option<SchemaNode> {
        self.cell.get().map(PendingTarget::materialize)
    }

    fn with_node<F, R>(&self, f: F) -> R
    where
        F: FnOnce(SchemaNode) -> R,
    {
        let node = self
            .cell
            .get()
            .expect("pending node accessed before initialization")
            .materialize();
        f(node)
    }

    /// Get a unique identifier for this pending node.
    /// Uses the address of the inner cell as a stable identifier.
    #[inline]
    fn node_id(&self) -> usize {
        Arc::as_ptr(&self.cell) as usize
    }
}

impl PendingTarget {
    fn materialize(&self) -> SchemaNode {
        let validators = self
            .validators
            .upgrade()
            .expect("pending schema target dropped");
        SchemaNode {
            validators,
            location: self.location.clone(),
            absolute_path: self.absolute_path.clone(),
        }
    }
}

impl Validate for PendingSchemaNode {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        let node_id = self.node_id();
        // Check memoization cache first (only for arrays/objects)
        if let Some(cached) = ctx.get_cached_result(node_id, instance) {
            return cached;
        }
        if ctx.enter(node_id, instance) {
            return true; // Cycle detected
        }
        let result = self.with_node(|node| node.is_valid(instance, ctx));
        ctx.exit(node_id, instance);
        // Cache result for recursive schemas
        ctx.cache_result(node_id, instance, result);
        result
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        if ctx.enter(self.node_id(), instance) {
            return Ok(());
        }
        let result = self.with_node(|node| node.validate(instance, location, tracker, ctx));
        ctx.exit(self.node_id(), instance);
        result
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        if ctx.enter(self.node_id(), instance) {
            return crate::error::no_error();
        }
        let result = self.with_node(|node| node.iter_errors(instance, location, tracker, ctx));
        ctx.exit(self.node_id(), instance);
        result
    }

    fn evaluate(
        &self,
        instance: &Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        if ctx.enter(self.node_id(), instance) {
            return EvaluationResult::valid_empty();
        }
        let result = self.with_node(|node| node.evaluate(instance, location, tracker, ctx));
        ctx.exit(self.node_id(), instance);
        result
    }
}

impl SchemaNode {
    pub(crate) fn from_boolean(ctx: &Context<'_>, validator: Option<BoxedValidator>) -> SchemaNode {
        SchemaNode {
            location: ctx.location().clone(),
            absolute_path: ctx.base_uri(),
            validators: Arc::new(NodeValidators::Boolean { validator }),
        }
    }

    pub(crate) fn from_keywords(
        ctx: &Context<'_>,
        mut validators: Vec<(Keyword, BoxedValidator)>,
        unmatched_keywords: Option<Arc<Value>>,
    ) -> SchemaNode {
        // Sort validators by priority (lower = execute first).
        // This enables "fail fast" by running cheap validators (type, const)
        // before expensive ones (allOf, $ref).
        validators.sort_by_key(|(keyword, _)| crate::keywords::keyword_priority(keyword));

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
                }
            })
            .collect();
        SchemaNode {
            location: ctx.location().clone(),
            absolute_path,
            validators: Arc::new(NodeValidators::Keyword(KeywordValidators {
                unmatched_keywords,
                validators,
            })),
        }
    }

    pub(crate) fn from_array(ctx: &Context<'_>, validators: Vec<BoxedValidator>) -> SchemaNode {
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
                }
            })
            .collect();
        SchemaNode {
            location: ctx.location().clone(),
            absolute_path,
            validators: Arc::new(NodeValidators::Array { validators }),
        }
    }

    pub(crate) fn validators(&self) -> impl ExactSizeIterator<Item = &BoxedValidator> {
        match self.validators.as_ref() {
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
        instance: &Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationNode {
        let instance_location: Location = location.into();

        let keyword_location = crate::paths::evaluation_path(tracker, &self.location);

        let schema_location =
            crate::evaluation::format_schema_location(&self.location, self.absolute_path.as_ref());

        match self.evaluate(instance, location, tracker, ctx) {
            EvaluationResult::Valid {
                annotations,
                children,
            } => EvaluationNode::valid(
                keyword_location,
                self.absolute_path.clone(),
                schema_location,
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
    fn evaluate_subschemas<'a, I>(
        instance: &Value,
        location: &LazyLocation,
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
                    &'a BoxedValidator,
                ),
            > + 'a,
    {
        let (lower_bound, _) = subschemas.size_hint();
        let mut children: Vec<EvaluationNode> = Vec::with_capacity(lower_bound);
        let mut invalid = false;

        let instance_loc: Location = location.into();

        for (child_location, absolute_location, validator) in subschemas {
            let child_result = validator.evaluate(instance, location, tracker, ctx);

            let absolute_location = absolute_location.cloned();

            let eval_path = crate::paths::evaluation_path(tracker, child_location);

            // schemaLocation: The canonical location WITHOUT $ref traversals.
            // Per JSON Schema spec: "MUST NOT include by-reference applicators such as $ref"
            // For by-reference validators like $ref, use the target's canonical location.
            // For regular validators, use the keyword's location.
            let schema_location = validator.canonical_location().unwrap_or(child_location);
            let formatted_schema_location = crate::evaluation::format_schema_location(
                schema_location,
                absolute_location.as_ref(),
            );

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

impl Validate for SchemaNode {
    fn is_valid(&self, instance: &Value, ctx: &mut ValidationContext) -> bool {
        match self.validators.as_ref() {
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
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        match self.validators.as_ref() {
            NodeValidators::Keyword(kvs) if kvs.validators.len() == 1 => {
                return kvs.validators[0]
                    .validator
                    .validate(instance, location, tracker, ctx);
            }
            NodeValidators::Keyword(kvs) => {
                for entry in &kvs.validators {
                    entry.validator.validate(instance, location, tracker, ctx)?;
                }
            }
            NodeValidators::Array { validators } => {
                for entry in validators {
                    entry.validator.validate(instance, location, tracker, ctx)?;
                }
            }
            NodeValidators::Boolean { validator: Some(_) } => {
                return Err(ValidationError::false_schema(
                    self.location.clone(),
                    crate::paths::capture_evaluation_path(tracker, &self.location),
                    location.into(),
                    instance,
                ));
            }
            NodeValidators::Boolean { validator: None } => return Ok(()),
        }
        Ok(())
    }

    fn iter_errors<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        match self.validators.as_ref() {
            NodeValidators::Keyword(kvs) if kvs.validators.len() == 1 => kvs.validators[0]
                .validator
                .iter_errors(instance, location, tracker, ctx),
            NodeValidators::Keyword(kvs) => ErrorIterator::from_iterator(
                kvs.validators
                    .iter()
                    .flat_map(|entry| {
                        entry
                            .validator
                            .iter_errors(instance, location, tracker, ctx)
                    })
                    .collect::<Vec<_>>()
                    .into_iter(),
            ),
            NodeValidators::Boolean {
                validator: Some(v), ..
            } => v.iter_errors(instance, location, tracker, ctx),
            NodeValidators::Boolean {
                validator: None, ..
            } => ErrorIterator::from_iterator(std::iter::empty()),
            NodeValidators::Array { validators } => ErrorIterator::from_iterator(
                validators
                    .iter()
                    .flat_map(move |entry| {
                        entry
                            .validator
                            .iter_errors(instance, location, tracker, ctx)
                    })
                    .collect::<Vec<_>>()
                    .into_iter(),
            ),
        }
    }

    fn evaluate(
        &self,
        instance: &Value,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        match self.validators.as_ref() {
            NodeValidators::Array { ref validators } => Self::evaluate_subschemas(
                instance,
                location,
                tracker,
                validators.iter().map(|entry| {
                    (
                        &entry.location,
                        entry.absolute_location.as_ref(),
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
                    tracker,
                    validators.iter().map(|entry| {
                        (
                            &entry.location,
                            entry.absolute_location.as_ref(),
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

enum NodeValidatorsIter<'a> {
    NoValidator,
    BooleanValidators(std::iter::Once<&'a BoxedValidator>),
    KeywordValidators(std::slice::Iter<'a, KeywordValidatorEntry>),
    ArrayValidators(std::slice::Iter<'a, ArrayValidatorEntry>),
}

impl<'a> Iterator for NodeValidatorsIter<'a> {
    type Item = &'a BoxedValidator;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::NoValidator => None,
            Self::BooleanValidators(i) => i.next(),
            Self::KeywordValidators(v) => v.next().map(|entry| &entry.validator),
            Self::ArrayValidators(v) => v.next().map(|entry| &entry.validator),
        }
    }

    fn all<F>(&mut self, mut f: F) -> bool
    where
        Self: Sized,
        F: FnMut(Self::Item) -> bool,
    {
        match self {
            Self::NoValidator => true,
            Self::BooleanValidators(i) => i.all(f),
            Self::KeywordValidators(v) => v.all(|entry| f(&entry.validator)),
            Self::ArrayValidators(v) => v.all(|entry| f(&entry.validator)),
        }
    }
}

impl ExactSizeIterator for NodeValidatorsIter<'_> {
    fn len(&self) -> usize {
        match self {
            Self::NoValidator => 0,
            Self::BooleanValidators(..) => 1,
            Self::KeywordValidators(v) => v.len(),
            Self::ArrayValidators(v) => v.len(),
        }
    }
}
