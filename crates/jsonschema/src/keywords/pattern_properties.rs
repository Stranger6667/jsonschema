use std::sync::Arc;

use crate::{
    compiler,
    error::{no_error, ErrorIterator, ValidationError},
    keywords::CompilationResult,
    node::SchemaNode,
    options::PatternEngineOptions,
    output::BasicOutput,
    paths::{LazyLocation, Location},
    regex::RegexEngine,
    types::JsonType,
    validator::{PartialApplication, Validate},
};
use serde_json::{Map, Value};

pub(crate) struct PatternPropertiesValidator<R> {
    patterns: Vec<(Arc<R>, SchemaNode)>,
}

impl<R: RegexEngine> Validate for PatternPropertiesValidator<R> {
    #[allow(clippy::needless_collect)]
    fn iter_errors<'i>(&self, instance: &'i Value, location: &LazyLocation) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let errors: Vec<_> = self
                .patterns
                .iter()
                .flat_map(move |(re, node)| {
                    item.iter()
                        .filter(move |(key, _)| re.is_match(key).unwrap_or(false))
                        .flat_map(move |(key, value)| {
                            let location = location.push(key.as_str());
                            node.iter_errors(value, &location)
                        })
                })
                .collect();
            Box::new(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Object(item) = instance {
            self.patterns.iter().all(move |(re, node)| {
                item.iter()
                    .filter(move |(key, _)| re.is_match(key).unwrap_or(false))
                    .all(move |(_key, value)| node.is_valid(value))
            })
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            for (key, value) in item {
                let key_location = location.push(key);
                for (re, node) in &self.patterns {
                    if re.is_match(key).unwrap_or(false) {
                        node.validate(value, &key_location)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn apply(&self, instance: &Value, location: &LazyLocation) -> PartialApplication {
        if let Value::Object(item) = instance {
            let mut matched_propnames = Vec::with_capacity(item.len());
            let mut sub_results = BasicOutput::default();
            for (pattern, node) in &self.patterns {
                for (key, value) in item {
                    if pattern.is_match(key).unwrap_or(false) {
                        let path = location.push(key.as_str());
                        matched_propnames.push(key.clone());
                        sub_results += node.apply_rooted(value, &path);
                    }
                }
            }
            let mut result: PartialApplication = sub_results.into();
            result.annotate(Value::from(matched_propnames).into());
            result
        } else {
            PartialApplication::valid_empty()
        }
    }
}

pub(crate) struct SingleValuePatternPropertiesValidator<R> {
    regex: Arc<R>,
    node: SchemaNode,
}

impl<R: RegexEngine> Validate for SingleValuePatternPropertiesValidator<R> {
    #[allow(clippy::needless_collect)]
    fn iter_errors<'i>(&self, instance: &'i Value, location: &LazyLocation) -> ErrorIterator<'i> {
        if let Value::Object(item) = instance {
            let errors: Vec<_> = item
                .iter()
                .filter(move |(key, _)| self.regex.is_match(key).unwrap_or(false))
                .flat_map(move |(key, value)| {
                    let instance_path = location.push(key.as_str());
                    self.node.iter_errors(value, &instance_path)
                })
                .collect();
            Box::new(errors.into_iter())
        } else {
            no_error()
        }
    }

    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::Object(item) = instance {
            item.iter()
                .filter(move |(key, _)| self.regex.is_match(key).unwrap_or(false))
                .all(move |(_key, value)| self.node.is_valid(value))
        } else {
            true
        }
    }

    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::Object(item) = instance {
            for (key, value) in item {
                if self.regex.is_match(key).unwrap_or(false) {
                    self.node.validate(value, &location.push(key))?;
                }
            }
        }
        Ok(())
    }

    fn apply(&self, instance: &Value, location: &LazyLocation) -> PartialApplication {
        if let Value::Object(item) = instance {
            let mut matched_propnames = Vec::with_capacity(item.len());
            let mut outputs = BasicOutput::default();
            for (key, value) in item {
                if self.regex.is_match(key).unwrap_or(false) {
                    let path = location.push(key.as_str());
                    matched_propnames.push(key.clone());
                    outputs += self.node.apply_rooted(value, &path);
                }
            }
            let mut result: PartialApplication = outputs.into();
            result.annotate(Value::from(matched_propnames).into());
            result
        } else {
            PartialApplication::valid_empty()
        }
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    parent: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    if matches!(
        parent.get("additionalProperties"),
        Some(Value::Bool(false) | Value::Object(_))
    ) {
        // This type of `additionalProperties` validator handles `patternProperties` logic
        return None;
    }

    let Value::Object(map) = schema else {
        return Some(Err(ValidationError::single_type_error(
            Location::new(),
            ctx.location().clone(),
            schema,
            JsonType::Object,
        )));
    };
    let ctx = ctx.new_at_location("patternProperties");
    let result = match ctx.config().pattern_options() {
        PatternEngineOptions::FancyRegex { .. } => {
            compile_pattern_entries(&ctx, map, |pctx, pattern, subschema| {
                pctx.get_or_compile_regex(pattern)
                    .map_err(|()| invalid_regex(pctx, subschema))
            })
            .map(|patterns| {
                build_validator_from_entries(patterns, |regex, node| {
                    Box::new(SingleValuePatternPropertiesValidator { regex, node })
                        as Box<dyn Validate>
                })
            })
        }
        PatternEngineOptions::Regex { .. } => {
            compile_pattern_entries(&ctx, map, |pctx, pattern, subschema| {
                pctx.get_or_compile_standard_regex(pattern)
                    .map_err(|()| invalid_regex(pctx, subschema))
            })
            .map(|patterns| {
                build_validator_from_entries(patterns, |regex, node| {
                    Box::new(SingleValuePatternPropertiesValidator { regex, node })
                        as Box<dyn Validate>
                })
            })
        }
    };
    Some(result)
}

fn invalid_regex<'a>(ctx: &compiler::Context, schema: &'a Value) -> ValidationError<'a> {
    ValidationError::format(Location::new(), ctx.location().clone(), schema, "regex")
}

/// Compile every `(pattern, subschema)` pair into `(regex, node)` tuples.
fn compile_pattern_entries<'a, R, F>(
    ctx: &compiler::Context,
    map: &'a Map<String, Value>,
    mut compile_regex: F,
) -> Result<Vec<(Arc<R>, SchemaNode)>, ValidationError<'a>>
where
    F: FnMut(&compiler::Context, &str, &'a Value) -> Result<Arc<R>, ValidationError<'a>>,
{
    let mut patterns = Vec::with_capacity(map.len());
    for (pattern, subschema) in map {
        let pctx = ctx.new_at_location(pattern.as_str());
        let regex = compile_regex(&pctx, pattern, subschema)?;
        let node = compiler::compile(&pctx, pctx.as_resource_ref(subschema))?;
        patterns.push((regex, node));
    }
    Ok(patterns)
}

/// Pick the optimal validator representation for the compiled pattern entries.
fn build_validator_from_entries<R>(
    mut entries: Vec<(Arc<R>, SchemaNode)>,
    single_factory: impl FnOnce(Arc<R>, SchemaNode) -> Box<dyn Validate>,
) -> Box<dyn Validate>
where
    R: RegexEngine + 'static,
{
    if entries.len() == 1 {
        let (regex, node) = entries.pop().expect("len checked");
        single_factory(regex, node)
    } else {
        Box::new(PatternPropertiesValidator { patterns: entries })
    }
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::{json, Value};
    use test_case::test_case;

    #[test_case(&json!({"patternProperties": {"^f": {"type": "string"}}}), &json!({"f": 42}), "/patternProperties/^f/type")]
    #[test_case(&json!({"patternProperties": {"^f": {"type": "string"}, "^x": {"type": "string"}}}), &json!({"f": 42}), "/patternProperties/^f/type")]
    fn location(schema: &Value, instance: &Value, expected: &str) {
        tests_util::assert_schema_location(schema, instance, expected);
    }
}
