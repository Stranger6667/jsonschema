use crate::paths::{LazyLocation, Location};

#[derive(Debug, Clone)]
pub struct TracingContext<'a, 'b, 'c> {
    pub instance_location: &'c LazyLocation<'a, 'b>,
    pub schema_location: &'c Location,
    pub result: NodeEvaluationResult,
}

impl<'a, 'b, 'c> TracingContext<'a, 'b, 'c> {
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
    pub fn call(self, callback: TracingCallback<'_>) {
        callback(self)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum NodeEvaluationResult {
    Valid,
    Invalid,
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

pub type TracingCallback<'a> = &'a mut dyn FnMut(TracingContext);
