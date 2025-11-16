use crate::{
    compiler::Context,
    error::ErrorIterator,
    keywords::{BoxedValidator, Keyword},
    output::{Annotations, BasicOutput, ErrorDescription, OutputUnit},
    paths::{LazyLocation, Location},
    thread::{Shared, SharedWeak},
    validator::{PartialApplication, Validate},
    ValidationError,
};
use referencing::Uri;
use serde_json::Value;
use std::{
    cell::OnceCell,
    fmt,
    sync::{Arc, OnceLock},
};

/// A node in the schema tree, returned by [`compiler::compile`]
#[derive(Clone, Debug)]
pub(crate) struct SchemaNode {
    validators: Shared<NodeValidators>,
    location: Location,
    absolute_path: Option<Arc<Uri<String>>>,
}

// Separate type used only during compilation for handling recursive references
#[derive(Clone, Debug)]
pub(crate) struct PendingSchemaNode {
    cell: Shared<OnceLock<PendingTarget>>,
}

#[derive(Debug)]
struct PendingTarget {
    validators: SharedWeak<NodeValidators>,
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
            cell: Shared::new(OnceLock::new()),
        }
    }

    pub(crate) fn initialize(&self, node: &SchemaNode) {
        let target = PendingTarget {
            validators: Shared::downgrade(&node.validators),
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
    fn is_valid(&self, instance: &Value) -> bool {
        self.with_node(|node| node.is_valid(instance))
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        self.with_node(|node| node.validate(instance, location))
    }

    fn iter_errors<'i>(&self, instance: &'i Value, location: &LazyLocation) -> ErrorIterator<'i> {
        self.with_node(|node| node.iter_errors(instance, location))
    }

    fn apply(&self, instance: &Value, location: &LazyLocation) -> PartialApplication {
        self.with_node(|node| node.apply(instance, location))
    }
}

impl SchemaNode {
    pub(crate) fn from_boolean(ctx: &Context<'_>, validator: Option<BoxedValidator>) -> SchemaNode {
        SchemaNode {
            location: ctx.location().clone(),
            absolute_path: ctx.base_uri(),
            validators: Shared::new(NodeValidators::Boolean { validator }),
        }
    }

    pub(crate) fn from_keywords(
        ctx: &Context<'_>,
        validators: Vec<(Keyword, BoxedValidator)>,
        unmatched_keywords: Option<Arc<Value>>,
    ) -> SchemaNode {
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
            validators: Shared::new(NodeValidators::Keyword(KeywordValidators {
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
            validators: Shared::new(NodeValidators::Array { validators }),
        }
    }

    pub(crate) fn clone_with_location(
        &self,
        location: Location,
        absolute_path: Option<Arc<Uri<String>>>,
    ) -> SchemaNode {
        SchemaNode {
            validators: self.validators.clone(),
            location,
            absolute_path,
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

    /// This is similar to `Validate::apply` except that `SchemaNode` knows where it is in the
    /// validator tree and so rather than returning a `PartialApplication` it is able to return a
    /// complete `BasicOutput`. This is the mechanism which compositional validators use to combine
    /// results from sub-schemas
    pub(crate) fn apply_rooted(&self, instance: &Value, location: &LazyLocation) -> BasicOutput {
        match self.apply(instance, location) {
            PartialApplication::Valid {
                annotations,
                child_results,
            } => {
                if let Some(annotations) = annotations {
                    let mut outputs = Vec::with_capacity(child_results.len() + 1);
                    outputs.push(self.annotation_at(location, annotations));
                    outputs.extend(child_results);
                    BasicOutput::Valid(outputs)
                } else {
                    BasicOutput::Valid(child_results)
                }
            }
            PartialApplication::Invalid {
                errors,
                child_results,
            } => {
                if errors.is_empty() {
                    BasicOutput::Invalid(child_results)
                } else {
                    let mut outputs = Vec::with_capacity(child_results.len() + errors.len());
                    for error in errors {
                        outputs.push(self.error_at(location, error));
                    }
                    outputs.extend(child_results);
                    BasicOutput::Invalid(outputs)
                }
            }
        }
    }

    /// Create an error output which is marked as occurring at this schema node
    pub(crate) fn error_at(
        &self,
        location: &LazyLocation,
        error: ErrorDescription,
    ) -> OutputUnit<ErrorDescription> {
        OutputUnit::<ErrorDescription>::error(
            self.location.clone(),
            location.into(),
            self.absolute_path.clone(),
            error,
        )
    }

    /// Create an annotation output which is marked as occurring at this schema node
    pub(crate) fn annotation_at(
        &self,
        location: &LazyLocation,
        annotations: Annotations,
    ) -> OutputUnit<Annotations> {
        OutputUnit::<Annotations>::annotations(
            self.location.clone(),
            location.into(),
            self.absolute_path.clone(),
            annotations,
        )
    }

    /// Helper function to apply subschemas which already know their locations.
    fn apply_subschemas<'a, I>(
        instance: &Value,
        location: &LazyLocation,
        subschemas: I,
        annotations: Option<Annotations>,
    ) -> PartialApplication
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
        let mut success_annotations: Vec<OutputUnit<Annotations>> = Vec::with_capacity(lower_bound);
        let mut success_children: Vec<OutputUnit<Annotations>> = Vec::with_capacity(lower_bound);
        let mut error_results: Vec<OutputUnit<ErrorDescription>> = Vec::with_capacity(lower_bound);
        let instance_location: OnceCell<Location> = OnceCell::new();

        macro_rules! instance_location {
            () => {
                instance_location.get_or_init(|| location.into()).clone()
            };
        }

        for (child_location, absolute_location, validator) in subschemas {
            let schema_location = child_location.clone();
            let absolute_location = absolute_location.cloned();
            match validator.apply(instance, location) {
                PartialApplication::Valid {
                    annotations,
                    mut child_results,
                } => {
                    if let Some(annotations) = annotations {
                        success_annotations.push(OutputUnit::<Annotations>::annotations(
                            schema_location.clone(),
                            instance_location!(),
                            absolute_location.clone(),
                            annotations,
                        ));
                    }
                    success_children.append(&mut child_results);
                }
                PartialApplication::Invalid {
                    errors: these_errors,
                    mut child_results,
                } => {
                    let child_len = child_results.len();
                    error_results.reserve(child_len + these_errors.len());
                    error_results.append(&mut child_results);
                    error_results.extend(these_errors.into_iter().map(|error| {
                        OutputUnit::<ErrorDescription>::error(
                            schema_location.clone(),
                            instance_location!(),
                            absolute_location.clone(),
                            error,
                        )
                    }));
                }
            }
        }
        if error_results.is_empty() {
            let mut child_results = success_children;
            if success_annotations.is_empty() {
                PartialApplication::Valid {
                    annotations,
                    child_results,
                }
            } else {
                let mut annotations_front = success_annotations;
                annotations_front.reverse();
                annotations_front.append(&mut child_results);
                PartialApplication::Valid {
                    annotations,
                    child_results: annotations_front,
                }
            }
        } else {
            PartialApplication::Invalid {
                errors: Vec::new(),
                child_results: error_results,
            }
        }
    }

    pub(crate) fn location(&self) -> &Location {
        &self.location
    }
}

impl Validate for SchemaNode {
    fn iter_errors<'i>(&self, instance: &'i Value, location: &LazyLocation) -> ErrorIterator<'i> {
        match self.validators.as_ref() {
            NodeValidators::Keyword(kvs) if kvs.validators.len() == 1 => {
                kvs.validators[0].validator.iter_errors(instance, location)
            }
            NodeValidators::Keyword(kvs) => Box::new(
                kvs.validators
                    .iter()
                    .flat_map(|entry| entry.validator.iter_errors(instance, location))
                    .collect::<Vec<_>>()
                    .into_iter(),
            ),
            NodeValidators::Boolean {
                validator: Some(v), ..
            } => v.iter_errors(instance, location),
            NodeValidators::Boolean {
                validator: None, ..
            } => Box::new(std::iter::empty()),
            NodeValidators::Array { validators } => Box::new(
                validators
                    .iter()
                    .flat_map(move |entry| entry.validator.iter_errors(instance, location))
                    .collect::<Vec<_>>()
                    .into_iter(),
            ),
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        match self.validators.as_ref() {
            NodeValidators::Keyword(kvs) => {
                for entry in &kvs.validators {
                    entry.validator.validate(instance, location)?;
                }
            }
            NodeValidators::Array { validators } => {
                for entry in validators {
                    entry.validator.validate(instance, location)?;
                }
            }
            NodeValidators::Boolean { validator: Some(_) } => {
                return Err(ValidationError::false_schema(
                    self.location.clone(),
                    location.into(),
                    instance,
                ));
            }
            NodeValidators::Boolean { validator: None } => return Ok(()),
        }
        Ok(())
    }

    fn is_valid(&self, instance: &Value) -> bool {
        match self.validators.as_ref() {
            // If we only have one validator then calling it's `is_valid` directly does
            // actually save the 20 or so instructions required to call the `slice::Iter::all`
            // implementation. Validators at the leaf of a tree are all single node validators so
            // this optimization can have significant cumulative benefits
            NodeValidators::Keyword(kvs) if kvs.validators.len() == 1 => {
                kvs.validators[0].validator.is_valid(instance)
            }
            NodeValidators::Keyword(kvs) => {
                for entry in &kvs.validators {
                    if !entry.validator.is_valid(instance) {
                        return false;
                    }
                }
                true
            }
            NodeValidators::Array { validators } => validators
                .iter()
                .all(|entry| entry.validator.is_valid(instance)),
            NodeValidators::Boolean { validator: Some(_) } => false,
            NodeValidators::Boolean { validator: None } => true,
        }
    }

    fn apply(&self, instance: &Value, location: &LazyLocation) -> PartialApplication {
        match self.validators.as_ref() {
            NodeValidators::Array { ref validators } => SchemaNode::apply_subschemas(
                instance,
                location,
                validators.iter().map(|entry| {
                    (
                        &entry.location,
                        entry.absolute_location.as_ref(),
                        &entry.validator,
                    )
                }),
                None,
            ),
            NodeValidators::Boolean { ref validator } => {
                if let Some(validator) = validator {
                    validator.apply(instance, location)
                } else {
                    PartialApplication::Valid {
                        annotations: None,
                        child_results: Vec::new(),
                    }
                }
            }
            NodeValidators::Keyword(ref kvals) => {
                let KeywordValidators {
                    ref unmatched_keywords,
                    ref validators,
                } = *kvals;
                let annotations: Option<Annotations> =
                    unmatched_keywords.as_ref().map(Annotations::from);
                SchemaNode::apply_subschemas(
                    instance,
                    location,
                    validators.iter().map(|entry| {
                        (
                            &entry.location,
                            entry.absolute_location.as_ref(),
                            &entry.validator,
                        )
                    }),
                    annotations,
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
