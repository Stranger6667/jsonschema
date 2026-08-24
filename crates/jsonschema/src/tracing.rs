use crate::paths::{LazyLocation, Location};

/// Context information passed to tracing callbacks during schema validation.
///
/// This struct provides information about the validation state at a specific point
/// in the validation tree, including the instance location, schema location, and
/// the evaluation result.
#[derive(Debug, Clone)]
pub struct TracingContext<'a, 'b, 'c> {
    /// The location in the instance being validated
    pub instance_location: &'c LazyLocation<'a, 'b>,
    /// The location in the schema performing the validation
    pub schema_location: &'c Location,
    /// The result of evaluating this node
    pub result: NodeEvaluationResult,
}

impl<'a, 'b, 'c> TracingContext<'a, 'b, 'c> {
    /// Create a new tracing context
    pub fn new(
        instance_location: &'c LazyLocation<'a, 'b>,
        schema_location: &'c Location,
        result: impl Into<NodeEvaluationResult>,
    ) -> Self {
        Self {
            instance_location,
            schema_location,
            result: result.into(),
        }
    }

    /// Call the tracing callback with this context
    pub fn call(self, callback: TracingCallback<'_>) {
        callback(self);
    }
}

/// Result of evaluating a schema node against an instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeEvaluationResult {
    /// The validation passed
    Valid,
    /// The validation failed
    Invalid,
    /// The validation was not applicable (e.g., type mismatch)
    Ignored,
}

impl From<bool> for NodeEvaluationResult {
    fn from(value: bool) -> Self {
        if value {
            Self::Valid
        } else {
            Self::Invalid
        }
    }
}

impl From<Option<bool>> for NodeEvaluationResult {
    fn from(value: Option<bool>) -> Self {
        match value {
            Some(true) => Self::Valid,
            Some(false) => Self::Invalid,
            None => Self::Ignored,
        }
    }
}

/// Type alias for tracing callbacks.
///
/// A tracing callback is called for each node in the validation tree,
/// providing visibility into the validation process.
pub type TracingCallback<'a> = &'a mut dyn FnMut(TracingContext);
