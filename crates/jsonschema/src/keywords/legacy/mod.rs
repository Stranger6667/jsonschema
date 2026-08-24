use crate::{
    error::ErrorIterator,
    keywords::BoxedValidator,
    paths::{LazyLocation, Location, RefTracker},
    tracing::{TracingCallback, TracingContext},
    validator::{EvaluationResult, Validate, ValidationContext},
    Json, ValidationError,
};

pub(crate) mod maximum_draft_4;
pub(crate) mod minimum_draft_4;
pub(crate) mod type_draft_4;

/// Draft 4 writes an exclusive bound as two keywords: `exclusiveMaximum: true` next to
/// `maximum` (likewise for the minimum). The inner validator does the comparison; this
/// reports the paired keyword as evaluated during tracing.
pub(crate) struct Draft4ExclusiveValidator<F: Json> {
    inner: BoxedValidator<F>,
    paired_location: Location,
}

impl<F: Json> Draft4ExclusiveValidator<F> {
    pub(crate) fn new(inner: BoxedValidator<F>, paired_location: Location) -> Self {
        Self {
            inner,
            paired_location,
        }
    }
}

impl<F: Json> Validate<F> for Draft4ExclusiveValidator<F> {
    fn is_valid(&self, instance: &F::Node<'_>, ctx: &mut ValidationContext) -> bool {
        self.inner.is_valid(instance, ctx)
    }

    fn validate<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> Result<(), ValidationError<'i>> {
        self.inner.validate(instance, location, tracker, ctx)
    }

    fn iter_errors<'i>(
        &self,
        instance: &F::Node<'i>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> ErrorIterator<'i> {
        self.inner.iter_errors(instance, location, tracker, ctx)
    }

    fn evaluate(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        tracker: Option<&RefTracker>,
        ctx: &mut ValidationContext,
    ) -> EvaluationResult {
        self.inner.evaluate(instance, location, tracker, ctx)
    }

    fn canonical_location(&self) -> Option<&Location> {
        self.inner.canonical_location()
    }

    fn schema_path(&self) -> &Location {
        self.inner.schema_path()
    }

    fn matches_type(&self, instance: &F::Node<'_>) -> bool {
        self.inner.matches_type(instance)
    }

    fn trace(
        &self,
        instance: &F::Node<'_>,
        location: &LazyLocation,
        callback: TracingCallback<'_>,
        ctx: &mut ValidationContext,
    ) -> bool {
        let result = self.inner.is_valid(instance, ctx);
        let evaluation_result = if self.matches_type(instance) {
            Some(result)
        } else {
            None
        };
        TracingContext::new(location, self.schema_path(), evaluation_result).call(callback);
        TracingContext::new(location, &self.paired_location, evaluation_result).call(callback);
        result
    }
}
