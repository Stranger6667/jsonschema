use std::sync::Arc;

use crate::{
    compiler,
    keywords::pattern_properties::invalid_regex,
    node::SchemaNode,
    regex::{
        analyze_pattern, contains_ecma_whitespace, Compiled, LazyRegex, PatternOptimization,
        RegexBackendPanic, SequenceMatcher,
    },
    Json, Object, SerdeJson, ValidationContext,
};
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
    /// An anchored class/literal sequence, with a lazily built regex for what it cannot decide.
    Sequence(Arc<SequenceMatcher<LazyRegex>>),
    /// Full regex pattern.
    Regex(R),
}

impl<R: crate::regex::RegexEngine> crate::regex::RegexEngine for CompiledPattern<R>
where
    R::Error: From<RegexBackendPanic>,
{
    type Error = R::Error;

    #[inline]
    fn is_match(&self, text: &str) -> Result<bool, Self::Error> {
        match self {
            CompiledPattern::Prefix(prefix) => Ok(text.starts_with(prefix.as_ref())),
            CompiledPattern::Exact(exact) => Ok(text == exact.as_ref()),
            CompiledPattern::Alternation(alts) => Ok(alts.iter().any(|a| a.as_str() == text)),
            CompiledPattern::NoWhitespace => Ok(!contains_ecma_whitespace(text)),
            CompiledPattern::Sequence(sequence) => sequence.is_match(text).map_err(Into::into),
            CompiledPattern::Regex(re) => re.is_match(text),
        }
    }
}

/// A `patternProperties` pattern matched against property names during validation.
pub(crate) trait PropertyPattern: crate::regex::RegexEngine {
    /// Whether `text` matches; `ctx` resolves a match the engine cannot finish, reported at `site`.
    fn is_match_in(
        &self,
        text: &str,
        ctx: &mut ValidationContext,
        site: impl FnOnce() -> crate::unfinished_matches::MatchSite,
    ) -> bool;
}

impl<R: crate::regex::RegexEngine> PropertyPattern for CompiledPattern<R>
where
    R::Error: From<RegexBackendPanic>,
{
    #[inline]
    fn is_match_in(
        &self,
        text: &str,
        ctx: &mut ValidationContext,
        site: impl FnOnce() -> crate::unfinished_matches::MatchSite,
    ) -> bool {
        match self {
            CompiledPattern::Prefix(prefix) => text.starts_with(prefix.as_ref()),
            CompiledPattern::Exact(exact) => text == exact.as_ref(),
            CompiledPattern::Alternation(alts) => alts.iter().any(|a| a.as_str() == text),
            CompiledPattern::NoWhitespace => !contains_ecma_whitespace(text),
            CompiledPattern::Sequence(sequence) => ctx.is_match(&**sequence, text, site),
            CompiledPattern::Regex(re) => ctx.is_match(re, text, site),
        }
    }
}

pub(crate) type FancyRegexValidators<F = SerdeJson> =
    Vec<(CompiledPattern<Compiled<fancy_regex::Regex>>, SchemaNode<F>)>;
pub(crate) type RegexValidators<F = SerdeJson> =
    Vec<(CompiledPattern<Compiled<regex::Regex>>, SchemaNode<F>)>;

/// A value that can look up property validators by name.
pub(crate) trait PropertiesValidatorsMap<F: Json = SerdeJson>: Send + Sync {
    fn get_validator(&self, property: &str) -> Option<&SchemaNode<F>>;
    fn get_key_validator(&self, property: &str) -> Option<(&str, &SchemaNode<F>)>;
}

/// Threshold for switching from linear scan to `HashMap`.
pub(crate) const HASHMAP_THRESHOLD: usize = 15;

/// Whether one pass over an object's members beats a lookup per declared property.
///
/// A members pass costs about one cheap `KeyHead` compare per pair; a lookup is a tree descent or a
/// hash of the key. Past four members per declared property the lookups win under both map orders.
#[inline]
pub(crate) fn scan_members(members: usize, properties: usize) -> bool {
    members <= properties.saturating_mul(4)
}

/// Lengths of a set of names, one bit per length modulo 64; a key whose length has no bit cannot be
/// among them.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct NameLengths(u64);

impl NameLengths {
    pub(crate) fn of<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        NameLengths(
            names
                .into_iter()
                .fold(0, |bits, name| bits | Self::bit(name.len())),
        )
    }

    #[inline]
    fn bit(len: usize) -> u64 {
        1 << (len & 63)
    }

    #[inline]
    pub(crate) fn may_contain(self, len: usize) -> bool {
        self.0 & Self::bit(len) != 0
    }
}

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

    #[inline]
    fn fold(self) -> u64 {
        self.first ^ self.last.rotate_left(32) ^ self.len as u64
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

/// Declared names bucketed by a multiply-shift of their `KeyHead`.
pub(crate) struct BigValidatorsMap<F: Json = SerdeJson> {
    entries: Box<[(PropertyName, SchemaNode<F>)]>,
    /// `entries[starts[slot]..starts[slot + 1]]` is the bucket of `slot`.
    starts: Box<[u32]>,
    seed: u64,
    shift: u32,
}

const SEEDS: [u64; 4] = [
    0x9e37_79b9_7f4a_7c15,
    0xc2b2_ae3d_27d4_eb4f,
    0x1656_67b1_9e37_79f9,
    0xd6e8_feb8_6659_fd93,
];

// The shift keeps only slot-index bits.
#[allow(clippy::cast_possible_truncation)]
#[inline]
fn slot(fold: u64, seed: u64, shift: u32) -> usize {
    (fold.wrapping_mul(seed) >> shift) as usize
}

impl<F: Json> BigValidatorsMap<F> {
    pub(crate) fn new(names: Vec<(PropertyName, SchemaNode<F>)>) -> Self {
        let bits = names.len().next_power_of_two().trailing_zeros() + 1;
        let shift = 64 - bits;
        let slots = 1_usize << bits;
        let folds: Vec<u64> = names.iter().map(|(name, _)| name.head.fold()).collect();
        let mut sizes = vec![0_u32; slots];
        let (mut seed, mut fewest) = (SEEDS[0], usize::MAX);
        for candidate in SEEDS {
            sizes.fill(0);
            // Sum of squared bucket sizes.
            let mut compares = 0;
            for &fold in &folds {
                let size = &mut sizes[slot(fold, candidate, shift)];
                compares += 2 * *size as usize + 1;
                *size += 1;
            }
            if compares < fewest {
                (seed, fewest) = (candidate, compares);
                if compares == folds.len() {
                    break;
                }
            }
        }
        let mut entries = names;
        entries.sort_by_cached_key(|(name, _)| slot(name.head.fold(), seed, shift));
        let mut starts = vec![0_u32; slots + 1];
        for &fold in &folds {
            starts[slot(fold, seed, shift) + 1] += 1;
        }
        for index in 1..=slots {
            starts[index] += starts[index - 1];
        }
        BigValidatorsMap {
            entries: entries.into_boxed_slice(),
            starts: starts.into_boxed_slice(),
            seed,
            shift,
        }
    }

    #[inline]
    fn find(&self, key: &str) -> Option<&(PropertyName, SchemaNode<F>)> {
        let head = KeyHead::of(key);
        let slot = slot(head.fold(), self.seed, self.shift);
        let bucket = self.starts[slot] as usize..self.starts[slot + 1] as usize;
        self.entries[bucket]
            .iter()
            .find(|(name, _)| name.matches(head, key))
    }
}

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
        self.find(property).map(|(_, node)| node)
    }

    #[inline]
    fn get_key_validator(&self, property: &str) -> Option<(&str, &SchemaNode<F>)> {
        self.find(property)
            .map(|(name, node)| (name.as_str(), node))
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
    Ok(BigValidatorsMap::new(compile_small_map(ctx, map)?))
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
            if !validator.is_valid_child(&instance, ctx) {
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
                if let Some(sequence) = ctx.sequence_without_limits(pattern) {
                    CompiledPattern::Sequence(sequence)
                } else {
                    let regex = ctx
                        .get_or_compile_regex(pattern)
                        .map_err(|()| invalid_regex(&pctx, pattern))?;
                    CompiledPattern::Regex((*regex).clone())
                }
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
                if let Some(sequence) = ctx.sequence_without_limits(pattern) {
                    CompiledPattern::Sequence(sequence)
                } else {
                    let regex = ctx
                        .get_or_compile_standard_regex(pattern)
                        .map_err(|()| invalid_regex(&pctx, pattern))?;
                    CompiledPattern::Regex((*regex).clone())
                }
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
