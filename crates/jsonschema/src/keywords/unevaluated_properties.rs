use std::sync::{Arc, OnceLock};

use ahash::AHashSet;
use fancy_regex::Regex;
use referencing::{Draft, List, Registry, Resource, Uri, VocabularySet};
use serde_json::{Map, Value};

use crate::{
    compiler, ecma,
    node::SchemaNode,
    paths::{LazyLocation, Location},
    thread::ThreadBound,
    validator::Validate,
    ValidationError, ValidationOptions,
};

use super::CompilationResult;

pub(crate) trait PropertiesFilter: ThreadBound + Sized + 'static {
    fn new<'a>(
        ctx: &'a compiler::Context<'_>,
        parent: &'a Map<String, Value>,
    ) -> Result<Self, ValidationError<'a>>;
    fn unevaluated(&self) -> Option<&SchemaNode>;

    fn is_valid(&self, instance: &Value) -> bool {
        self.unevaluated()
            .as_ref()
            .is_some_and(|u| u.is_valid(instance))
    }

    fn mark_evaluated_properties<'i>(
        &self,
        instance: &'i Value,
        properties: &mut AHashSet<&'i String>,
    );
}

pub(crate) struct UnevaluatedPropertiesValidator<F: PropertiesFilter> {
    location: Location,
    filter: F,
}

impl<F: PropertiesFilter> UnevaluatedPropertiesValidator<F> {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &'a compiler::Context,
        parent: &'a Map<String, Value>,
    ) -> CompilationResult<'a> {
        Ok(Box::new(UnevaluatedPropertiesValidator {
            location: ctx.location().join("unevaluatedProperties"),
            filter: F::new(ctx, parent)?,
        }))
    }
}

impl<F: PropertiesFilter> Validate for UnevaluatedPropertiesValidator<F> {
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(properties) = instance {
            let mut evaluated = AHashSet::new();
            self.filter
                .mark_evaluated_properties(instance, &mut evaluated);

            let mut unevaluated = vec![];
            for (property, value) in properties {
                if !evaluated.contains(property) && !self.filter.is_valid(value) {
                    unevaluated.push(property.clone());
                }
            }
            if !unevaluated.is_empty() {
                return Err(ValidationError::unevaluated_properties(
                    self.location.clone(),
                    location.into(),
                    instance,
                    unevaluated,
                ));
            }
        }
        Ok(())
    }

    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Object(properties) = instance {
            let mut evaluated = AHashSet::new();
            self.filter
                .mark_evaluated_properties(instance, &mut evaluated);

            for (property, value) in properties {
                if !evaluated.contains(property) && !self.filter.is_valid(value) {
                    return false;
                }
            }
        }
        true
    }
}

struct Draft2019PropertiesFilter {
    unevaluated: Option<SchemaNode>,
    additional: Option<SchemaNode>,
    properties: Vec<(String, SchemaNode)>,
    dependent: Vec<(String, Self)>,
    pattern_properties: Vec<(fancy_regex::Regex, SchemaNode)>,
    ref_: Option<Box<Self>>,
    recursive_ref: Option<LazyReference<Self>>,
    conditional: Option<Box<ConditionalFilter<Self>>>,
    all_of: Option<CombinatorFilter<Self>>,
    any_of: Option<CombinatorFilter<Self>>,
    one_of: Option<CombinatorFilter<Self>>,
}

enum ReferenceFilter<T> {
    Recursive(Box<LazyReference<T>>),
    Default(Box<T>),
}

impl<F: PropertiesFilter> ReferenceFilter<F> {
    fn mark_evaluated_properties<'i>(
        &self,
        instance: &'i Value,
        properties: &mut AHashSet<&'i String>,
    ) {
        match self {
            ReferenceFilter::Recursive(filter) => filter
                .get_or_init()
                .mark_evaluated_properties(instance, properties),
            ReferenceFilter::Default(filter) => {
                filter.mark_evaluated_properties(instance, properties);
            }
        }
    }
}

struct LazyReference<T> {
    resource: Resource,
    config: ValidationOptions,
    registry: Registry,
    scopes: List<Uri<String>>,
    base_uri: Arc<Uri<String>>,
    vocabularies: VocabularySet,
    draft: Draft,
    inner: OnceLock<Box<T>>,
}

impl<T: PropertiesFilter> LazyReference<T> {
    fn new<'a>(ctx: &compiler::Context) -> Result<Self, ValidationError<'a>> {
        let scopes = ctx.scopes();
        let resolved = ctx.lookup_recursive_reference()?;
        let resource = ctx.draft().create_resource(resolved.contents().clone());
        let resolver = resolved.resolver();
        let mut base_uri = resolver.base_uri();
        if let Some(id) = resource.id() {
            base_uri = resolver.resolve_against(&base_uri.borrow(), id)?;
        }

        Ok(LazyReference {
            resource,
            config: ctx.config().clone(),
            registry: ctx.registry.clone(),
            base_uri,
            scopes,
            vocabularies: ctx.vocabularies().clone(),
            draft: ctx.draft(),
            inner: OnceLock::default(),
        })
    }

    fn get_or_init(&self) -> &T {
        self.inner.get_or_init(|| {
            let resolver = self
                .registry
                .resolver_from_raw_parts(self.base_uri.clone(), self.scopes.clone());

            let ctx = compiler::Context::new(
                &self.config,
                &self.registry,
                resolver,
                self.vocabularies.clone(),
                self.draft,
                Location::new(),
            );

            Box::new(
                T::new(
                    &ctx,
                    self.resource
                        .contents()
                        .as_object()
                        .expect("Invalid schema"),
                )
                .expect("Invalid schema during lazy compilation"),
            )
        })
    }
}

impl PropertiesFilter for Draft2019PropertiesFilter {
    fn new<'a>(
        ctx: &'a compiler::Context<'_>,
        parent: &'a Map<String, Value>,
    ) -> Result<Self, ValidationError<'a>> {
        let mut ref_ = None;

        if let Some(Value::String(reference)) = parent.get("$ref") {
            let resolved = ctx.lookup(reference)?;
            if let Value::Object(subschema) = resolved.contents() {
                ref_ = Some(Box::new(
                    Self::new(ctx, subschema).map_err(ValidationError::to_owned)?,
                ));
            }
        }

        let mut recursive_ref = None;
        if parent.contains_key("$recursiveRef") {
            recursive_ref = Some(LazyReference::new(ctx).map_err(ValidationError::to_owned)?);
        }

        let mut conditional = None;

        if let Some(subschema) = parent.get("if") {
            if let Value::Object(if_parent) = subschema {
                let if_ctx = ctx.new_at_location("if");
                let mut then_ = None;
                if let Some(Value::Object(subschema)) = parent.get("then") {
                    let then_ctx = ctx.new_at_location("then");
                    then_ =
                        Some(Self::new(&then_ctx, subschema).map_err(ValidationError::to_owned)?);
                }
                let mut else_ = None;
                if let Some(Value::Object(subschema)) = parent.get("else") {
                    let else_ctx = ctx.new_at_location("else");
                    else_ =
                        Some(Self::new(&else_ctx, subschema).map_err(ValidationError::to_owned)?);
                }
                conditional = Some(Box::new(ConditionalFilter {
                    condition: compiler::compile(&if_ctx, if_ctx.as_resource_ref(subschema))
                        .map_err(ValidationError::to_owned)?,
                    if_: Self::new(&if_ctx, if_parent).map_err(ValidationError::to_owned)?,
                    then_,
                    else_,
                }));
            }
        }

        let mut properties = Vec::new();
        if let Some(Value::Object(map)) = parent.get("properties") {
            let properties_ctx = ctx.new_at_location("properties");
            for (property, subschema) in map {
                let prop_ctx = properties_ctx.new_at_location(property.as_str());
                properties.push((
                    property.clone(),
                    compiler::compile(&prop_ctx, prop_ctx.as_resource_ref(subschema))
                        .map_err(ValidationError::to_owned)?,
                ));
            }
        }

        let mut dependent = Vec::new();
        if let Some(Value::Object(map)) = parent.get("dependentSchemas") {
            let dependent_ctx = ctx.new_at_location("dependentSchemas");
            for (property, subschema) in map {
                if let Value::Object(subschema) = subschema {
                    let property_ctx = dependent_ctx.new_at_location(property.as_str());
                    dependent.push((
                        property.clone(),
                        Self::new(&property_ctx, subschema).map_err(ValidationError::to_owned)?,
                    ));
                }
            }
        }

        let mut additional = None;
        if let Some(subschema) = parent.get("additionalProperties") {
            let additional_ctx = ctx.new_at_location("additionalProperties");
            additional = Some(
                compiler::compile(&additional_ctx, additional_ctx.as_resource_ref(subschema))
                    .map_err(ValidationError::to_owned)?,
            );
        }

        let mut pattern_properties = Vec::new();
        if let Some(Value::Object(patterns)) = parent.get("patternProperties") {
            let pat_ctx = ctx.new_at_location("patternProperties");
            for (pattern, schema) in patterns {
                let schema_ctx = pat_ctx.new_at_location(pattern.as_str());
                pattern_properties.push((
                    match ecma::to_rust_regex(pattern).map(|pattern| Regex::new(&pattern)) {
                        Ok(Ok(r)) => r,
                        _ => {
                            return Err(ValidationError::format(
                                Location::new(),
                                ctx.location().clone(),
                                schema,
                                "regex",
                            ))
                        }
                    },
                    compiler::compile(&schema_ctx, schema_ctx.as_resource_ref(schema))
                        .map_err(ValidationError::to_owned)?,
                ));
            }
        }

        let mut unevaluated = None;
        if let Some(subschema) = parent.get("unevaluatedProperties") {
            let unevaluated_ctx = ctx.new_at_location("unevaluatedProperties");
            unevaluated = Some(
                compiler::compile(&unevaluated_ctx, unevaluated_ctx.as_resource_ref(subschema))
                    .map_err(ValidationError::to_owned)?,
            );
        }

        let mut all_of = None;
        if let Some(Some(subschemas)) = parent.get("allOf").map(Value::as_array) {
            let all_of_ctx = ctx.new_at_location("allOf");
            all_of = Some(
                CombinatorFilter::new(&all_of_ctx, subschemas)
                    .map_err(ValidationError::to_owned)?,
            );
        }
        let mut any_of = None;
        if let Some(Some(subschemas)) = parent.get("anyOf").map(Value::as_array) {
            let any_of_ctx = ctx.new_at_location("anyOf");
            any_of = Some(
                CombinatorFilter::new(&any_of_ctx, subschemas)
                    .map_err(ValidationError::to_owned)?,
            );
        }

        let mut one_of = None;
        if let Some(Some(subschemas)) = parent.get("oneOf").map(Value::as_array) {
            let one_of_ctx = ctx.new_at_location("oneOf");
            one_of = Some(
                CombinatorFilter::new(&one_of_ctx, subschemas)
                    .map_err(ValidationError::to_owned)?,
            );
        }

        Ok(Draft2019PropertiesFilter {
            unevaluated,
            additional,
            properties,
            dependent,
            pattern_properties,
            ref_,
            recursive_ref,
            conditional,
            all_of,
            any_of,
            one_of,
        })
    }

    fn mark_evaluated_properties<'i>(
        &self,
        instance: &'i Value,
        properties: &mut AHashSet<&'i String>,
    ) {
        if let Some(ref_) = &self.ref_ {
            ref_.mark_evaluated_properties(instance, properties);
        }

        if let Some(recursive_ref) = &self.recursive_ref {
            recursive_ref
                .get_or_init()
                .mark_evaluated_properties(instance, properties);
        }

        if let Value::Object(obj) = instance {
            for (property, value) in obj {
                for (p, node) in &self.properties {
                    if property == p && node.is_valid(value) {
                        properties.insert(property);
                    }
                }
                if let Some(additional) = self.additional.as_ref() {
                    if additional.is_valid(value) {
                        properties.insert(property);
                        continue;
                    }
                }
                if let Some(unevaluated) = self.unevaluated.as_ref() {
                    if unevaluated.is_valid(value) {
                        properties.insert(property);
                        continue;
                    }
                }
                for (pattern, _) in &self.pattern_properties {
                    if pattern.is_match(property).unwrap() {
                        properties.insert(property);
                    }
                }
            }
            for (property, subschema) in &self.dependent {
                if !obj.contains_key(property) {
                    continue;
                }
                subschema.mark_evaluated_properties(instance, properties);
            }
        }

        if let Some(conditional) = &self.conditional {
            conditional.mark_evaluated_properties(instance, properties);
        }

        if let Some(combinator) = &self.all_of {
            if combinator
                .subschemas
                .iter()
                .all(|(v, _)| v.is_valid(instance))
            {
                combinator.mark_evaluated_properties(instance, properties);
            }
        }

        if let Some(combinator) = &self.any_of {
            if combinator
                .subschemas
                .iter()
                .any(|(v, _)| v.is_valid(instance))
            {
                combinator.mark_evaluated_properties(instance, properties);
            }
        }

        if let Some(combinator) = &self.one_of {
            let result = combinator
                .subschemas
                .iter()
                .map(|(v, _)| v.is_valid(instance))
                .collect::<Vec<_>>();
            if result.iter().filter(|v| **v).count() == 1 {
                for ((_, subschema), is_valid) in combinator.subschemas.iter().zip(result) {
                    if is_valid {
                        subschema.mark_evaluated_properties(instance, properties);
                        break;
                    }
                }
            }
        }
    }

    fn unevaluated(&self) -> Option<&SchemaNode> {
        self.unevaluated.as_ref()
    }
}

struct DefaultPropertiesFilter {
    unevaluated: Option<SchemaNode>,
    additional: Option<SchemaNode>,
    properties: Vec<(String, SchemaNode)>,
    dependent: Vec<(String, Self)>,
    pattern_properties: Vec<(fancy_regex::Regex, SchemaNode)>,
    ref_: Option<ReferenceFilter<Self>>,
    dynamic_ref: Option<Box<Self>>,
    conditional: Option<Box<ConditionalFilter<Self>>>,
    all_of: Option<CombinatorFilter<Self>>,
    any_of: Option<CombinatorFilter<Self>>,
    one_of: Option<CombinatorFilter<Self>>,
}

impl PropertiesFilter for DefaultPropertiesFilter {
    fn new<'a>(
        ctx: &'a compiler::Context<'_>,
        parent: &'a Map<String, Value>,
    ) -> Result<Self, ValidationError<'a>> {
        let mut ref_ = None;

        if let Some(Value::String(reference)) = parent.get("$ref") {
            if ctx.is_circular_reference(reference)? {
                let scopes = ctx.scopes();
                let resolved = ctx.lookup(reference)?;
                let resource = ctx.draft().create_resource(resolved.contents().clone());
                let resolver = resolved.resolver();
                let mut base_uri = resolver.base_uri();
                if let Some(id) = resource.id() {
                    base_uri = resolver.resolve_against(&base_uri.borrow(), id)?;
                }

                ref_ = Some(ReferenceFilter::Recursive(Box::new(LazyReference {
                    resource,
                    config: ctx.config().clone(),
                    registry: ctx.registry.clone(),
                    base_uri,
                    scopes,
                    vocabularies: ctx.vocabularies().clone(),
                    draft: ctx.draft(),
                    inner: OnceLock::default(),
                })));
            } else {
                ctx.mark_seen(reference)?;
                let resolved = ctx.lookup(reference)?;
                if let Value::Object(subschema) = resolved.contents() {
                    ref_ = Some(ReferenceFilter::Default(Box::new(
                        Self::new(ctx, subschema).map_err(ValidationError::to_owned)?,
                    )));
                }
            }
        }

        let mut dynamic_ref = None;

        if let Some(Value::String(reference)) = parent.get("$dynamicRef") {
            let resolved = ctx.lookup(reference)?;
            if let Value::Object(subschema) = resolved.contents() {
                dynamic_ref = Some(Box::new(
                    Self::new(ctx, subschema).map_err(ValidationError::to_owned)?,
                ));
            }
        }

        let mut conditional = None;

        if let Some(subschema) = parent.get("if") {
            if let Value::Object(if_parent) = subschema {
                let if_ctx = ctx.new_at_location("if");
                let mut then_ = None;
                if let Some(Value::Object(subschema)) = parent.get("then") {
                    let then_ctx = ctx.new_at_location("then");
                    then_ =
                        Some(Self::new(&then_ctx, subschema).map_err(ValidationError::to_owned)?);
                }
                let mut else_ = None;
                if let Some(Value::Object(subschema)) = parent.get("else") {
                    let else_ctx = ctx.new_at_location("else");
                    else_ =
                        Some(Self::new(&else_ctx, subschema).map_err(ValidationError::to_owned)?);
                }
                conditional = Some(Box::new(ConditionalFilter {
                    condition: compiler::compile(&if_ctx, if_ctx.as_resource_ref(subschema))
                        .map_err(ValidationError::to_owned)?,
                    if_: Self::new(&if_ctx, if_parent).map_err(ValidationError::to_owned)?,
                    then_,
                    else_,
                }));
            }
        }

        let mut properties = Vec::new();
        if let Some(Value::Object(map)) = parent.get("properties") {
            let properties_ctx = ctx.new_at_location("properties");
            for (property, subschema) in map {
                let prop_ctx = properties_ctx.new_at_location(property.as_str());
                properties.push((
                    property.clone(),
                    compiler::compile(&prop_ctx, prop_ctx.as_resource_ref(subschema))
                        .map_err(ValidationError::to_owned)?,
                ));
            }
        }

        let mut dependent = Vec::new();
        if let Some(Value::Object(map)) = parent.get("dependentSchemas") {
            let dependent_ctx = ctx.new_at_location("dependentSchemas");
            for (property, subschema) in map {
                if let Value::Object(subschema) = subschema {
                    let property_ctx = dependent_ctx.new_at_location(property.as_str());
                    dependent.push((
                        property.clone(),
                        Self::new(&property_ctx, subschema).map_err(ValidationError::to_owned)?,
                    ));
                }
            }
        }

        let mut additional = None;
        if let Some(subschema) = parent.get("additionalProperties") {
            let additional_ctx = ctx.new_at_location("additionalProperties");
            additional = Some(
                compiler::compile(&additional_ctx, additional_ctx.as_resource_ref(subschema))
                    .map_err(ValidationError::to_owned)?,
            );
        }

        let mut pattern_properties = Vec::new();
        if let Some(Value::Object(patterns)) = parent.get("patternProperties") {
            let pat_ctx = ctx.new_at_location("patternProperties");
            for (pattern, schema) in patterns {
                let schema_ctx = pat_ctx.new_at_location(pattern.as_str());
                pattern_properties.push((
                    match ecma::to_rust_regex(pattern).map(|pattern| Regex::new(&pattern)) {
                        Ok(Ok(r)) => r,
                        _ => {
                            return Err(ValidationError::format(
                                Location::new(),
                                ctx.location().clone(),
                                schema,
                                "regex",
                            ))
                        }
                    },
                    compiler::compile(&schema_ctx, schema_ctx.as_resource_ref(schema))
                        .map_err(ValidationError::to_owned)?,
                ));
            }
        }

        let mut unevaluated = None;
        if let Some(subschema) = parent.get("unevaluatedProperties") {
            let unevaluated_ctx = ctx.new_at_location("unevaluatedProperties");
            unevaluated = Some(
                compiler::compile(&unevaluated_ctx, unevaluated_ctx.as_resource_ref(subschema))
                    .map_err(ValidationError::to_owned)?,
            );
        }

        let mut all_of = None;
        if let Some(Some(subschemas)) = parent.get("allOf").map(Value::as_array) {
            let all_of_ctx = ctx.new_at_location("allOf");
            all_of = Some(
                CombinatorFilter::new(&all_of_ctx, subschemas)
                    .map_err(ValidationError::to_owned)?,
            );
        }
        let mut any_of = None;
        if let Some(Some(subschemas)) = parent.get("anyOf").map(Value::as_array) {
            let any_of_ctx = ctx.new_at_location("anyOf");
            any_of = Some(
                CombinatorFilter::new(&any_of_ctx, subschemas)
                    .map_err(ValidationError::to_owned)?,
            );
        }

        let mut one_of = None;
        if let Some(Some(subschemas)) = parent.get("oneOf").map(Value::as_array) {
            let one_of_ctx = ctx.new_at_location("oneOf");
            one_of = Some(
                CombinatorFilter::new(&one_of_ctx, subschemas)
                    .map_err(ValidationError::to_owned)?,
            );
        }

        Ok(DefaultPropertiesFilter {
            unevaluated,
            additional,
            properties,
            dependent,
            pattern_properties,
            ref_,
            dynamic_ref,
            conditional,
            all_of,
            any_of,
            one_of,
        })
    }

    fn mark_evaluated_properties<'i>(
        &self,
        instance: &'i Value,
        properties: &mut AHashSet<&'i String>,
    ) {
        if let Some(ref_) = &self.ref_ {
            ref_.mark_evaluated_properties(instance, properties);
        }

        if let Some(recursive_ref) = &self.dynamic_ref {
            recursive_ref.mark_evaluated_properties(instance, properties);
        }

        if let Value::Object(obj) = instance {
            for (property, value) in obj {
                for (p, node) in &self.properties {
                    if property == p && node.is_valid(value) {
                        properties.insert(property);
                    }
                }
                if let Some(additional) = self.additional.as_ref() {
                    if additional.is_valid(value) {
                        properties.insert(property);
                        continue;
                    }
                }
                if let Some(unevaluated) = self.unevaluated.as_ref() {
                    if unevaluated.is_valid(value) {
                        properties.insert(property);
                        continue;
                    }
                }
                for (pattern, _) in &self.pattern_properties {
                    if pattern.is_match(property).unwrap() {
                        properties.insert(property);
                    }
                }
            }
            for (property, subschema) in &self.dependent {
                if !obj.contains_key(property) {
                    continue;
                }
                subschema.mark_evaluated_properties(instance, properties);
            }
        }

        if let Some(conditional) = &self.conditional {
            conditional.mark_evaluated_properties(instance, properties);
        }

        if let Some(combinator) = &self.all_of {
            if combinator
                .subschemas
                .iter()
                .all(|(v, _)| v.is_valid(instance))
            {
                combinator.mark_evaluated_properties(instance, properties);
            }
        }

        if let Some(combinator) = &self.any_of {
            if combinator
                .subschemas
                .iter()
                .any(|(v, _)| v.is_valid(instance))
            {
                combinator.mark_evaluated_properties(instance, properties);
            }
        }

        if let Some(combinator) = &self.one_of {
            let result = combinator
                .subschemas
                .iter()
                .map(|(v, _)| v.is_valid(instance))
                .collect::<Vec<_>>();
            if result.iter().filter(|v| **v).count() == 1 {
                for ((_, subschema), is_valid) in combinator.subschemas.iter().zip(result) {
                    if is_valid {
                        subschema.mark_evaluated_properties(instance, properties);
                        break;
                    }
                }
            }
        }
    }

    fn unevaluated(&self) -> Option<&SchemaNode> {
        self.unevaluated.as_ref()
    }
}

struct CombinatorFilter<F> {
    subschemas: Vec<(SchemaNode, F)>,
}

impl<F: PropertiesFilter> CombinatorFilter<F> {
    fn mark_evaluated_properties<'i>(
        &self,
        instance: &'i Value,
        properties: &mut AHashSet<&'i String>,
    ) {
        for (_, subschema) in &self.subschemas {
            subschema.mark_evaluated_properties(instance, properties);
        }
    }
}

impl<F: PropertiesFilter> CombinatorFilter<F> {
    fn new<'a>(
        ctx: &compiler::Context<'a>,
        subschemas: &'a [Value],
    ) -> Result<CombinatorFilter<F>, ValidationError<'a>> {
        let mut buffer = Vec::with_capacity(subschemas.len());
        for (idx, subschema) in subschemas.iter().enumerate() {
            if let Value::Object(parent) = subschema {
                let subschema_ctx = ctx.new_at_location(idx);
                buffer.push((
                    compiler::compile(&subschema_ctx, subschema_ctx.as_resource_ref(subschema))
                        .map_err(ValidationError::to_owned)?,
                    F::new(&subschema_ctx, parent).map_err(ValidationError::to_owned)?,
                ));
            }
        }
        Ok(CombinatorFilter { subschemas: buffer })
    }
}

struct ConditionalFilter<F> {
    condition: SchemaNode,
    if_: F,
    then_: Option<F>,
    else_: Option<F>,
}

impl<F: PropertiesFilter> ConditionalFilter<F> {
    fn mark_evaluated_properties<'i>(
        &self,
        instance: &'i Value,
        properties: &mut AHashSet<&'i String>,
    ) {
        if self.condition.is_valid(instance) {
            self.if_.mark_evaluated_properties(instance, properties);
            if let Some(then_) = &self.then_ {
                then_.mark_evaluated_properties(instance, properties);
            }
        } else if let Some(else_) = &self.else_ {
            else_.mark_evaluated_properties(instance, properties);
        }
    }
}

pub(crate) fn compile<'a>(
    ctx: &'a compiler::Context,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    match schema.as_bool() {
        Some(true) => None,
        _ => {
            if ctx.draft() == Draft::Draft201909 {
                Some(
                    UnevaluatedPropertiesValidator::<Draft2019PropertiesFilter>::compile(
                        ctx, parent,
                    ),
                )
            } else {
                Some(
                    UnevaluatedPropertiesValidator::<DefaultPropertiesFilter>::compile(ctx, parent),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{tests_util, Draft};
    use serde_json::json;

    #[test]
    fn one_of() {
        tests_util::is_valid_with_draft(
            Draft::Draft202012,
            &json!({
                "oneOf": [
                    { "properties": { "foo": { "const": "bar" } } },
                    { "properties": { "foo": { "const": "quux" } } }
                ],
                "unevaluatedProperties": false
            }),
            &json!({ "foo": "quux" }),
        );
    }

    #[test]
    fn any_of() {
        tests_util::is_valid_with_draft(
            Draft::Draft202012,
            &json!({
                "anyOf": [
                    { "properties": { "foo": { "minLength": 10 } } },
                    { "properties": { "foo": { "type": "string" } } }
                ],
                "unevaluatedProperties": false
            }),
            &json!({ "foo": "rut roh" }),
        );
    }

    #[test]
    fn all_of() {
        tests_util::is_not_valid_with_draft(
            Draft::Draft202012,
            &json!({
                "allOf": [
                    { "properties": { "foo": { "type": "string" } } },
                    { "properties": { "foo": { "minLength": 10 } } }
                ],
                "unevaluatedProperties": false
            }),
            &json!({ "foo": "rut roh" }),
        );
    }

    #[test]
    fn all_of_with_additional_props_subschema() {
        let schema = json!({
            "allOf": [
                {
                    "type": "object",
                    "required": [
                        "foo"
                    ],
                    "properties": {
                        "foo": { "type": "string" }
                    }
                },
                {
                    "type": "object",
                    "additionalProperties": { "type": "string" }
                }
            ],
            "unevaluatedProperties": false
        });

        tests_util::is_valid_with_draft(
            Draft::Draft202012,
            &schema,
            &json!({ "foo": "wee", "another": "thing" }),
        );

        tests_util::is_not_valid_with_draft(
            Draft::Draft202012,
            &schema,
            &json!({ "foo": "wee", "another": false }),
        );
    }

    #[test]
    fn test_unevaluated_properties_with_allof_oneof() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "allOf": [{}],
            "oneOf": [
                {
                    "properties": {
                        "blah": true
                    }
                }
            ],
            "unevaluatedProperties": false
        });

        let valid = json!({
            "blah": 1
        });

        let validator = crate::validator_for(&schema).expect("Schema should compile");

        assert!(validator.validate(&valid).is_ok(), "Validation should pass");
        assert!(validator.is_valid(&valid), "Instance should be valid");

        let invalid = json!({
            "blah": 1,
            "extra": "property"
        });

        assert!(
            !validator.is_valid(&invalid),
            "Instance with extra property should be invalid"
        );
        assert!(
            validator.validate(&invalid).is_err(),
            "Validation should fail for instance with extra property"
        );
    }

    #[test]
    fn test_unevaluated_properties_with_recursion() {
        // See GH-420
        let schema = json!({
          "$schema": "https://json-schema.org/draft/2020-12/schema",
          "allOf": [
            {
              "$ref": "#/$defs/1_1"
            }
          ],
          "unevaluatedProperties": false,
          "$defs": {
            "1_1": {
              "type": "object",
              "properties": {
                "b": {
                  "allOf": [
                    {
                      "$ref": "#/$defs/1_2"
                    }
                  ],
                  "unevaluatedProperties": false
                }
              },
              "required": [
                "b"
              ]
            },
            "1_2": {
              "type": "object",
              "properties": {
                "f": {
                  "allOf": [
                    {
                      "$ref": "#/$defs/1_1"
                    }
                  ],
                  "unevaluatedProperties": false
                }
              },
              "required": [
                "f"
              ]
            }
          }
        });

        let validator = crate::validator_for(&schema).expect("Schema should compile");

        let instance = json!({"b": {"f": null}});
        assert!(!validator.is_valid(&instance));
        assert!(validator.validate(&instance).is_err());
    }
}
