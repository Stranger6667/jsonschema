use crate::LazyInstance;
use std::{borrow::Cow, sync::Arc};

use crate::{
    compiler,
    node::SchemaNode,
    paths::{LazyEvaluationPath, Location},
    regex::{analyze_pattern, is_ecma_whitespace, LiteralMatchError, PatternOptimization},
    validator::Validate as _,
    Json, Object, SerdeJson, ValidationContext,
};
use ahash::AHashMap;
use serde_json::{Map, Value};

use crate::ValidationError;

/// A compiled pattern that can be a literal optimized match or a full regex.
#[derive(Debug, Clone)]
pub(crate) enum CompiledPattern<R> {
    /// Simple prefix match using `starts_with()`.
    Prefix(Arc<str>),
    /// Exact match using `==` - for `^...$` patterns.
    Exact(Arc<str>),
    /// `^(a|b|c)$` — linear scan over a small sorted array of alternatives.
    Alternation(Arc<[String]>),
    /// `^\S*$` — no ECMA-262 whitespace characters.
    NoWhitespace,
    /// Full regex pattern.
    Regex(R),
}

impl<R: crate::regex::RegexEngine> crate::regex::RegexEngine for CompiledPattern<R> {
    type Error = LiteralMatchError;

    #[inline]
    fn is_match(&self, text: &str) -> Result<bool, Self::Error> {
        match self {
            CompiledPattern::Prefix(prefix) => Ok(text.starts_with(prefix.as_ref())),
            CompiledPattern::Exact(exact) => Ok(text == exact.as_ref()),
            CompiledPattern::Alternation(alts) => Ok(alts.iter().any(|a| a.as_str() == text)),
            CompiledPattern::NoWhitespace => Ok(!text.chars().any(is_ecma_whitespace)),
            // Treat regex errors as non-match for compatibility
            CompiledPattern::Regex(re) => Ok(re.is_match(text).unwrap_or(false)),
        }
    }
}

pub(crate) type FancyRegexValidators<F = SerdeJson> =
    Vec<(CompiledPattern<fancy_regex::Regex>, SchemaNode<F>)>;
pub(crate) type RegexValidators<F = SerdeJson> =
    Vec<(CompiledPattern<regex::Regex>, SchemaNode<F>)>;

/// A value that can look up property validators by name.
pub(crate) trait PropertiesValidatorsMap<F: Json = SerdeJson>: Send + Sync {
    fn get_validator(&self, property: &str) -> Option<&SchemaNode<F>>;
    fn get_key_validator(&self, property: &str) -> Option<(&str, &SchemaNode<F>)>;
}

/// Threshold for switching from linear scan to `HashMap`.
pub(crate) const HASHMAP_THRESHOLD: usize = 15;

/// A name's length with its first and last eight bytes, read as two overlapping words. Names of up
/// to 16 bytes are decided by the head alone; longer ones only reach `memcmp` when heads agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyHead {
    len: usize,
    first: u64,
    last: u64,
}

#[inline]
fn word(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().expect("eight bytes"))
}

#[inline]
fn half_word(bytes: &[u8]) -> u64 {
    u64::from(u32::from_le_bytes(bytes.try_into().expect("four bytes")))
}

impl KeyHead {
    const INLINE_BYTES: usize = 16;

    #[inline]
    pub(crate) fn of(name: &str) -> Self {
        let bytes = name.as_bytes();
        let len = bytes.len();
        let (first, last) = if len >= 8 {
            (word(&bytes[..8]), word(&bytes[len - 8..]))
        } else if len >= 4 {
            (half_word(&bytes[..4]), half_word(&bytes[len - 4..]))
        } else {
            let mut packed = 0_u64;
            for (index, byte) in bytes.iter().enumerate() {
                packed |= u64::from(*byte) << (index * 8);
            }
            (packed, 0)
        };
        KeyHead { len, first, last }
    }
}

/// A declared property name alongside its head, for scans that test it against many keys.
#[derive(Debug, Clone)]
pub(crate) struct PropertyName {
    head: KeyHead,
    text: String,
}

impl PropertyName {
    pub(crate) fn new(text: String) -> Self {
        PropertyName {
            head: KeyHead::of(&text),
            text,
        }
    }

    #[inline]
    pub(crate) fn as_str(&self) -> &str {
        &self.text
    }

    /// Whether `key`, whose head is `head`, is this name.
    #[inline]
    pub(crate) fn matches(&self, head: KeyHead, key: &str) -> bool {
        self.head == head
            && (head.len <= KeyHead::INLINE_BYTES
                || self.text.as_bytes()[8..head.len - 8] == key.as_bytes()[8..head.len - 8])
    }
}

pub(crate) type SmallValidatorsMap<F = SerdeJson> = Vec<(PropertyName, SchemaNode<F>)>;
pub(crate) type BigValidatorsMap<F = SerdeJson> = AHashMap<String, SchemaNode<F>>;

impl<F: Json> PropertiesValidatorsMap<F> for SmallValidatorsMap<F> {
    #[inline]
    fn get_validator(&self, property: &str) -> Option<&SchemaNode<F>> {
        let head = KeyHead::of(property);
        for (prop, node) in self {
            if prop.matches(head, property) {
                return Some(node);
            }
        }
        None
    }
    #[inline]
    fn get_key_validator(&self, property: &str) -> Option<(&str, &SchemaNode<F>)> {
        let head = KeyHead::of(property);
        for (prop, node) in self {
            if prop.matches(head, property) {
                return Some((prop.as_str(), node));
            }
        }
        None
    }
}

impl<F: Json> PropertiesValidatorsMap<F> for BigValidatorsMap<F> {
    #[inline]
    fn get_validator(&self, property: &str) -> Option<&SchemaNode<F>> {
        self.get(property)
    }

    #[inline]
    fn get_key_validator(&self, property: &str) -> Option<(&str, &SchemaNode<F>)> {
        self.get_key_value(property)
            .map(|(key, node)| (key.as_str(), node))
    }
}

pub(crate) fn compile_small_map<'a, F: Json>(
    ctx: &compiler::Context<F>,
    map: &'a Map<String, Value>,
) -> Result<SmallValidatorsMap<F>, ValidationError<'a>> {
    let mut properties = Vec::with_capacity(map.len());
    let kctx = ctx.new_at_location("properties");
    for (key, subschema) in map {
        let pctx = kctx.new_at_location(key.as_str());
        properties.push((
            PropertyName::new(key.clone()),
            compiler::compile(&pctx, pctx.as_resource_ref(subschema))?,
        ));
    }
    Ok(properties)
}

pub(crate) fn compile_big_map<'a, F: Json>(
    ctx: &compiler::Context<F>,
    map: &'a Map<String, Value>,
) -> Result<BigValidatorsMap<F>, ValidationError<'a>> {
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

pub(crate) fn are_properties_valid<'i, F, M, O, C>(
    prop_map: &M,
    object: &O,
    ctx: &mut ValidationContext,
    check: C,
) -> bool
where
    F: Json,
    M: PropertiesValidatorsMap<F>,
    O: Object<'i, F, Node = F::Node<'i>>,
    C: Fn(&F::Node<'i>, &mut ValidationContext) -> bool,
{
    for (property, instance) in object.members() {
        if let Some(validator) = prop_map.get_validator(property.as_ref()) {
            if !validator.is_valid(&instance, ctx) {
                return false;
            }
        } else if !check(&instance, ctx) {
            return false;
        }
    }
    true
}

/// Create a vector of pattern-validators pairs.
/// Uses prefix optimization when patterns are simple `^prefix` patterns.
#[inline]
pub(crate) fn compile_fancy_regex_patterns<'a, F: Json>(
    ctx: &compiler::Context<F>,
    obj: &'a Map<String, Value>,
) -> Result<FancyRegexValidators<F>, ValidationError<'a>> {
    let kctx = ctx.new_at_location("patternProperties");
    let mut compiled_patterns = Vec::with_capacity(obj.len());
    for (pattern, subschema) in obj {
        let pctx = kctx.new_at_location(pattern.as_str());
        let compiled_pattern = match analyze_pattern(pattern) {
            Some(PatternOptimization::Prefix(prefix)) => CompiledPattern::Prefix(Arc::from(prefix)),
            Some(PatternOptimization::Exact(exact)) => CompiledPattern::Exact(Arc::from(exact)),
            Some(PatternOptimization::Alternation(alts)) => {
                CompiledPattern::Alternation(Arc::from(alts.into_boxed_slice()))
            }
            Some(PatternOptimization::NoWhitespace) => CompiledPattern::NoWhitespace,
            None => {
                let regex = ctx.get_or_compile_regex(pattern).map_err(|()| {
                    ValidationError::format(
                        kctx.location().clone(),
                        LazyEvaluationPath::SameAsSchemaPath,
                        Location::new(),
                        LazyInstance::Ready(Cow::Borrowed(subschema)),
                        "regex",
                    )
                })?;
                CompiledPattern::Regex((*regex).clone())
            }
        };
        let node = compiler::compile(&pctx, pctx.as_resource_ref(subschema))?;
        compiled_patterns.push((compiled_pattern, node));
    }
    Ok(compiled_patterns)
}

/// Create a vector of pattern-validators pairs using standard regex.
/// Uses literal optimizations when patterns are simple prefix or exact-match patterns.
#[inline]
pub(crate) fn compile_regex_patterns<'a, F: Json>(
    ctx: &compiler::Context<F>,
    obj: &'a Map<String, Value>,
) -> Result<RegexValidators<F>, ValidationError<'a>> {
    let kctx = ctx.new_at_location("patternProperties");
    let mut compiled_patterns = Vec::with_capacity(obj.len());
    for (pattern, subschema) in obj {
        let pctx = kctx.new_at_location(pattern.as_str());
        let compiled_pattern = match analyze_pattern(pattern) {
            Some(PatternOptimization::Prefix(prefix)) => CompiledPattern::Prefix(Arc::from(prefix)),
            Some(PatternOptimization::Exact(exact)) => CompiledPattern::Exact(Arc::from(exact)),
            Some(PatternOptimization::Alternation(alts)) => {
                CompiledPattern::Alternation(Arc::from(alts.into_boxed_slice()))
            }
            Some(PatternOptimization::NoWhitespace) => CompiledPattern::NoWhitespace,
            None => {
                let regex = ctx.get_or_compile_standard_regex(pattern).map_err(|()| {
                    ValidationError::format(
                        kctx.location().clone(),
                        LazyEvaluationPath::SameAsSchemaPath,
                        Location::new(),
                        LazyInstance::Ready(Cow::Borrowed(subschema)),
                        "regex",
                    )
                })?;
                CompiledPattern::Regex((*regex).clone())
            }
        };
        let node = compiler::compile(&pctx, pctx.as_resource_ref(subschema))?;
        compiled_patterns.push((compiled_pattern, node));
    }
    Ok(compiled_patterns)
}

macro_rules! compile_dynamic_prop_map_validator {
    ($validator:tt, $properties:ident, $ctx:expr, $( $arg:expr ),* $(,)*) => {{
        if let Value::Object(map) = $properties {
            if map.len() < HASHMAP_THRESHOLD {
                Some($validator::<SmallValidatorsMap<F>>::compile(
                    map, $ctx, $($arg, )*
                ))
            } else {
                Some($validator::<BigValidatorsMap<F>>::compile(
                    map, $ctx, $($arg, )*
                ))
            }
        } else {
            let location = $ctx.location().clone();
            Some(Err(ValidationError::compile_error(
                location.clone(),
                location,
                Location::new(),
                LazyInstance::Ready(std::borrow::Cow::Borrowed($properties)),
                "Unexpected type",
            )))
        }
    }};
}

pub(crate) use compile_dynamic_prop_map_validator;
