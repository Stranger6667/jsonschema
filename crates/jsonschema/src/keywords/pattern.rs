use crate::{
    compiler, ecma,
    error::ValidationError,
    keywords::CompilationResult,
    paths::{LazyLocation, Location},
    primitive_type::PrimitiveType,
    validator::Validate,
};
use ahash::AHashMap;
use once_cell::sync::Lazy;
use serde_json::{Map, Value};

use std::{collections::VecDeque, sync::Mutex};

static REGEX_CACHE: Lazy<Mutex<LruCache>> = Lazy::new(|| Mutex::new(LruCache::new(10)));

struct LruCache {
    map: AHashMap<String, fancy_regex::Regex>,
    queue: VecDeque<String>,
    capacity: usize,
}

impl LruCache {
    fn new(capacity: usize) -> Self {
        LruCache {
            map: AHashMap::new(),
            queue: VecDeque::new(),
            capacity,
        }
    }

    fn get(&mut self, key: &str) -> Option<&fancy_regex::Regex> {
        if let Some(value) = self.map.get(key) {
            let index = self.queue.iter().position(|x| x == key).unwrap();
            let k = self.queue.remove(index).unwrap();
            self.queue.push_back(k);
            Some(value)
        } else {
            None
        }
    }

    fn insert(&mut self, key: String, value: fancy_regex::Regex) -> Option<fancy_regex::Regex> {
        if self.map.len() >= self.capacity && !self.map.contains_key(&key) {
            if let Some(lru_key) = self.queue.pop_front() {
                self.map.remove(&lru_key);
            }
        }

        let old_value = self.map.insert(key.clone(), value);
        if old_value.is_some() {
            let index = self.queue.iter().position(|x| x == &key).unwrap();
            self.queue.remove(index);
        }
        self.queue.push_back(key);
        old_value
    }
}

pub(crate) struct PatternValidator {
    original: String,
    pattern: fancy_regex::Regex,
    location: Location,
}

impl PatternValidator {
    #[inline]
    pub(crate) fn compile<'a>(
        ctx: &compiler::Context,
        pattern: &'a Value,
    ) -> CompilationResult<'a> {
        match pattern {
            Value::String(item) => {
                let mut cache = REGEX_CACHE.lock().expect("Lock is poisoned");
                let pattern = if let Some(regex) = cache.get(item) {
                    regex.clone()
                } else {
                    let regex = match ecma::to_rust_regex(item)
                        .map(|pattern| fancy_regex::Regex::new(&pattern))
                    {
                        Ok(Ok(r)) => r,
                        _ => {
                            return Err(ValidationError::format(
                                Location::new(),
                                ctx.location().clone(),
                                pattern,
                                "regex",
                            ))
                        }
                    };
                    cache.insert(item.clone(), regex.clone());
                    regex
                };
                Ok(Box::new(PatternValidator {
                    original: item.clone(),
                    pattern,
                    location: ctx.location().join("pattern"),
                }))
            }
            _ => Err(ValidationError::single_type_error(
                Location::new(),
                ctx.location().clone(),
                pattern,
                PrimitiveType::String,
            )),
        }
    }
}

impl Validate for PatternValidator {
    fn validate<'i>(
        &self,
        instance: &'i Value,
        location: &LazyLocation,
    ) -> Result<(), ValidationError<'i>> {
        if let Value::String(item) = instance {
            match self.pattern.is_match(item) {
                Ok(is_match) => {
                    if !is_match {
                        return Err(ValidationError::pattern(
                            self.location.clone(),
                            location.into(),
                            instance,
                            self.original.clone(),
                        ));
                    }
                }
                Err(e) => {
                    return Err(ValidationError::backtrack_limit(
                        self.location.clone(),
                        location.into(),
                        instance,
                        e,
                    ));
                }
            }
        }
        Ok(())
    }

    fn is_valid(&self, instance: &Value) -> bool {
        if let Value::String(item) = instance {
            return self.pattern.is_match(item).unwrap_or(false);
        }
        true
    }
}

#[inline]
pub(crate) fn compile<'a>(
    ctx: &compiler::Context,
    _: &'a Map<String, Value>,
    schema: &'a Value,
) -> Option<CompilationResult<'a>> {
    Some(PatternValidator::compile(ctx, schema))
}

#[cfg(test)]
mod tests {
    use crate::tests_util;
    use serde_json::json;
    use test_case::test_case;

    #[test_case("^(?!eo:)", "eo:bands", false)]
    #[test_case("^(?!eo:)", "proj:epsg", true)]
    fn negative_lookbehind_match(pattern: &str, text: &str, is_matching: bool) {
        let text = json!(text);
        let schema = json!({"pattern": pattern});
        let validator = crate::validator_for(&schema).unwrap();
        assert_eq!(validator.is_valid(&text), is_matching)
    }

    #[test]
    fn location() {
        tests_util::assert_schema_location(&json!({"pattern": "^f"}), &json!("b"), "/pattern")
    }
}
