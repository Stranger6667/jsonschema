use std::sync::Arc;

use crate::{
    compiler,
    node::SchemaNode,
    paths::{LazyEvaluationPath, Location},
    regex::pattern_as_prefix,
    validator::Validate as _,
    ValidationContext,
};
use ahash::AHashMap;
use serde_json::{Map, Value};

use crate::ValidationError;

/// A compiled pattern that can be either a simple prefix (optimized) or a full regex.
#[derive(Debug, Clone)]
pub(crate) enum CompiledPattern<R> {
    /// Simple prefix match using `starts_with()` - much faster than regex.
    Prefix(Arc<str>),
    /// Full regex pattern.
    Regex(R),
}

/// A dummy error type for prefix matching (which never fails).
#[derive(Debug)]
pub(crate) struct PrefixMatchError;

impl crate::regex::RegexError for PrefixMatchError {
    fn into_backtrack_error(self) -> Option<fancy_regex::Error> {
        None
    }
}

impl<R: crate::regex::RegexEngine> crate::regex::RegexEngine for CompiledPattern<R> {
    type Error = PrefixMatchError;

    #[inline]
    fn is_match(&self, text: &str) -> Result<bool, Self::Error> {
        match self {
            CompiledPattern::Prefix(prefix) => Ok(text.starts_with(prefix.as_ref())),
            // Treat regex errors as non-match for compatibility
            CompiledPattern::Regex(re) => Ok(re.is_match(text).unwrap_or(false)),
        }
    }

    fn pattern(&self) -> &str {
        match self {
            CompiledPattern::Prefix(prefix) => prefix.as_ref(),
            CompiledPattern::Regex(re) => re.pattern(),
        }
    }
}

pub(crate) type FancyRegexValidators = Vec<(CompiledPattern<fancy_regex::Regex>, SchemaNode)>;
pub(crate) type RegexValidators = Vec<(CompiledPattern<regex::Regex>, SchemaNode)>;

/// A value that can look up property validators by name.
pub(crate) trait PropertiesValidatorsMap: Send + Sync {
    fn get_validator(&self, property: &str) -> Option<&SchemaNode>;
    fn get_key_validator(&self, property: &str) -> Option<(&String, &SchemaNode)>;
}

// We're defining two different property validator map implementations, one for small map sizes and
// one for large map sizes, to optimize the performance depending on the number of properties
// present.
//
// Implementors should use `compile_dynamic_prop_map_validator!` for building their validator maps
// at runtime, as it wraps up all of the logic to choose the right map size and then build and
// compile the validator.
pub(crate) type SmallValidatorsMap = Vec<(String, SchemaNode)>;
pub(crate) type BigValidatorsMap = AHashMap<String, SchemaNode>;

impl PropertiesValidatorsMap for SmallValidatorsMap {
    #[inline]
    fn get_validator(&self, property: &str) -> Option<&SchemaNode> {
        for (prop, node) in self {
            if prop == property {
                return Some(node);
            }
        }
        None
    }
    #[inline]
    fn get_key_validator(&self, property: &str) -> Option<(&String, &SchemaNode)> {
        for (prop, node) in self {
            if prop == property {
                return Some((prop, node));
            }
        }
        None
    }
}

impl PropertiesValidatorsMap for BigValidatorsMap {
    #[inline]
    fn get_validator(&self, property: &str) -> Option<&SchemaNode> {
        self.get(property)
    }

    #[inline]
    fn get_key_validator(&self, property: &str) -> Option<(&String, &SchemaNode)> {
        self.get_key_value(property)
    }
}

/// Fused property validator map that stores both the validator and pre-computed pattern indices.
/// This eliminates one `HashMap` lookup per property during validation.
pub(crate) trait FusedPropertiesMap: Send + Sync {
    /// Get both the validator and pattern indices in a single lookup.
    fn get_validator_and_pattern_indices(&self, property: &str) -> Option<(&SchemaNode, &[usize])>;
    /// Get the key, validator, and pattern indices in a single lookup.
    fn get_key_validator_and_pattern_indices(
        &self,
        property: &str,
    ) -> Option<(&String, &SchemaNode, &[usize])>;
}

/// Small fused map using `Vec` for < 40 properties.
pub(crate) type SmallFusedMap = Vec<(String, SchemaNode, Box<[usize]>)>;
/// Big fused map using `AHashMap` for >= 40 properties.
pub(crate) type BigFusedMap = AHashMap<String, (SchemaNode, Box<[usize]>)>;

impl FusedPropertiesMap for SmallFusedMap {
    #[inline]
    fn get_validator_and_pattern_indices(&self, property: &str) -> Option<(&SchemaNode, &[usize])> {
        for (prop, node, indices) in self {
            if prop == property {
                return Some((node, indices));
            }
        }
        None
    }

    #[inline]
    fn get_key_validator_and_pattern_indices(
        &self,
        property: &str,
    ) -> Option<(&String, &SchemaNode, &[usize])> {
        for (prop, node, indices) in self {
            if prop == property {
                return Some((prop, node, indices));
            }
        }
        None
    }
}

impl FusedPropertiesMap for BigFusedMap {
    #[inline]
    fn get_validator_and_pattern_indices(&self, property: &str) -> Option<(&SchemaNode, &[usize])> {
        self.get(property).map(|(node, indices)| (node, &**indices))
    }

    #[inline]
    fn get_key_validator_and_pattern_indices(
        &self,
        property: &str,
    ) -> Option<(&String, &SchemaNode, &[usize])> {
        self.get_key_value(property)
            .map(|(key, (node, indices))| (key, node, &**indices))
    }
}

pub(crate) fn compile_small_map<'a>(
    ctx: &compiler::Context,
    map: &'a Map<String, Value>,
) -> Result<SmallValidatorsMap, ValidationError<'a>> {
    let mut properties = Vec::with_capacity(map.len());
    let kctx = ctx.new_at_location("properties");
    for (key, subschema) in map {
        let pctx = kctx.new_at_location(key.as_str());
        properties.push((
            key.clone(),
            compiler::compile(&pctx, pctx.as_resource_ref(subschema))?,
        ));
    }
    Ok(properties)
}

pub(crate) fn compile_big_map<'a>(
    ctx: &compiler::Context,
    map: &'a Map<String, Value>,
) -> Result<BigValidatorsMap, ValidationError<'a>> {
    let mut properties = AHashMap::with_capacity(map.len());
    let kctx = ctx.new_at_location("properties");
    for (key, subschema) in map {
        let pctx = kctx.new_at_location(key.as_str());
        properties.insert(
            key.clone(),
            compiler::compile(&pctx, pctx.as_resource_ref(subschema))?,
        );
    }
    Ok(properties)
}

/// Compile a small fused map with pre-computed pattern indices.
pub(crate) fn compile_small_fused_map<'a, R: crate::regex::RegexEngine>(
    ctx: &compiler::Context,
    map: &'a Map<String, Value>,
    patterns: &[(R, SchemaNode)],
) -> Result<SmallFusedMap, ValidationError<'a>> {
    let mut properties = Vec::with_capacity(map.len());
    let kctx = ctx.new_at_location("properties");
    for (key, subschema) in map {
        let pctx = kctx.new_at_location(key.as_str());
        let node = compiler::compile(&pctx, pctx.as_resource_ref(subschema))?;
        let pattern_indices = compute_pattern_indices(key, patterns);
        properties.push((key.clone(), node, pattern_indices));
    }
    Ok(properties)
}

/// Compile a big fused map with pre-computed pattern indices.
pub(crate) fn compile_big_fused_map<'a, R: crate::regex::RegexEngine>(
    ctx: &compiler::Context,
    map: &'a Map<String, Value>,
    patterns: &[(R, SchemaNode)],
) -> Result<BigFusedMap, ValidationError<'a>> {
    let mut properties = AHashMap::with_capacity(map.len());
    let kctx = ctx.new_at_location("properties");
    for (key, subschema) in map {
        let pctx = kctx.new_at_location(key.as_str());
        let node = compiler::compile(&pctx, pctx.as_resource_ref(subschema))?;
        let pattern_indices = compute_pattern_indices(key, patterns);
        properties.insert(key.clone(), (node, pattern_indices));
    }
    Ok(properties)
}

/// Compute which pattern indices match a given property name.
#[inline]
fn compute_pattern_indices<R: crate::regex::RegexEngine>(
    property: &str,
    patterns: &[(R, SchemaNode)],
) -> Box<[usize]> {
    patterns
        .iter()
        .enumerate()
        .filter(|(_, (re, _))| re.is_match(property).unwrap_or(false))
        .map(|(i, _)| i)
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

pub(crate) fn are_properties_valid<M, F>(
    prop_map: &M,
    props: &Map<String, Value>,
    ctx: &mut ValidationContext,
    check: F,
) -> bool
where
    M: PropertiesValidatorsMap,
    F: Fn(&Value, &mut ValidationContext) -> bool,
{
    for (property, instance) in props {
        if let Some(validator) = prop_map.get_validator(property) {
            if !validator.is_valid(instance, ctx) {
                return false;
            }
        } else if !check(instance, ctx) {
            return false;
        }
    }
    true
}

/// Create a vector of pattern-validators pairs.
/// Uses prefix optimization when patterns are simple `^prefix` patterns.
#[inline]
pub(crate) fn compile_fancy_regex_patterns<'a>(
    ctx: &compiler::Context,
    obj: &'a Map<String, Value>,
) -> Result<FancyRegexValidators, ValidationError<'a>> {
    let kctx = ctx.new_at_location("patternProperties");
    let mut compiled_patterns = Vec::with_capacity(obj.len());
    for (pattern, subschema) in obj {
        let pctx = kctx.new_at_location(pattern.as_str());
        let compiled_pattern = if let Some(prefix) = pattern_as_prefix(pattern) {
            CompiledPattern::Prefix(Arc::from(prefix))
        } else {
            let regex = ctx.get_or_compile_regex(pattern).map_err(|()| {
                ValidationError::format(
                    kctx.location().clone(),
                    LazyEvaluationPath::SameAsSchemaPath,
                    Location::new(),
                    subschema,
                    "regex",
                )
            })?;
            CompiledPattern::Regex((*regex).clone())
        };
        let node = compiler::compile(&pctx, pctx.as_resource_ref(subschema))?;
        compiled_patterns.push((compiled_pattern, node));
    }
    Ok(compiled_patterns)
}

/// Create a vector of pattern-validators pairs using standard regex.
/// Uses prefix optimization when patterns are simple `^prefix` patterns.
#[inline]
pub(crate) fn compile_regex_patterns<'a>(
    ctx: &compiler::Context,
    obj: &'a Map<String, Value>,
) -> Result<RegexValidators, ValidationError<'a>> {
    let kctx = ctx.new_at_location("patternProperties");
    let mut compiled_patterns = Vec::with_capacity(obj.len());
    for (pattern, subschema) in obj {
        let pctx = kctx.new_at_location(pattern.as_str());
        let compiled_pattern = if let Some(prefix) = pattern_as_prefix(pattern) {
            CompiledPattern::Prefix(Arc::from(prefix))
        } else {
            let regex = ctx.get_or_compile_standard_regex(pattern).map_err(|()| {
                ValidationError::format(
                    kctx.location().clone(),
                    LazyEvaluationPath::SameAsSchemaPath,
                    Location::new(),
                    subschema,
                    "regex",
                )
            })?;
            CompiledPattern::Regex((*regex).clone())
        };
        let node = compiler::compile(&pctx, pctx.as_resource_ref(subschema))?;
        compiled_patterns.push((compiled_pattern, node));
    }
    Ok(compiled_patterns)
}

macro_rules! compile_dynamic_prop_map_validator {
    ($validator:tt, $properties:ident, $ctx:expr, $( $arg:expr ),* $(,)*) => {{
        if let Value::Object(map) = $properties {
            if map.len() < 40 {
                Some($validator::<SmallValidatorsMap>::compile(
                    map, $ctx, $($arg, )*
                ))
            } else {
                Some($validator::<BigValidatorsMap>::compile(
                    map, $ctx, $($arg, )*
                ))
            }
        } else {
            let location = $ctx.location().clone();
            Some(Err(ValidationError::compile_error(
                location.clone(),
                location,
                Location::new(),
                $properties,
                "Unexpected type",
            )))
        }
    }};
}

pub(crate) use compile_dynamic_prop_map_validator;
