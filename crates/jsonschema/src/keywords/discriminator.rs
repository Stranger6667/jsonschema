//! `oneOf` / `anyOf` branches that each require one property and fix it to their own string
//! `const`. Against an object only the branch holding the instance's value can pass, so that
//! branch alone decides.
use ahash::{AHashMap, AHashSet};
use referencing::{Draft, Resolver, Vocabulary};
use serde_json::{Map, Value};

use crate::{compiler, Json, Node, Object};

// Deep enough for real `$ref` chains; a cycle stops here.
const MAX_DEPTH: usize = 16;
const MIN_BRANCHES: usize = 3;

pub(crate) struct Discriminator<F: Json> {
    key: F::PreparedKey,
    branches: AHashMap<String, usize>,
}

impl<F: Json> Discriminator<F> {
    pub(crate) fn compile(ctx: &compiler::Context<F>, items: &[Value]) -> Option<Self> {
        // With two branches a dispatch saves at most one branch, about what the analysis costs.
        if items.len() < MIN_BRANCHES
            || !ctx.has_vocabulary(&Vocabulary::Validation)
            || !ctx.has_vocabulary(&Vocabulary::Applicator)
            || ["required", "properties", "const", "enum", "allOf", "$ref"]
                .iter()
                .any(|keyword| ctx.is_keyword_overridden(keyword))
        {
            return None;
        }
        let mut branches: Vec<AHashMap<&str, &str>> = Vec::with_capacity(items.len());
        for item in items {
            let mut pins = Pins::default();
            pins.collect(ctx.resolver(), ctx.draft(), item, 0);
            // Only keys the first branch pins can pin every branch.
            let wanted = branches.first();
            let values = pins.required_values(wanted);
            if values.is_empty() {
                return None;
            }
            branches.push(values);
        }
        let mut keys: Vec<&str> = branches.first()?.keys().copied().collect();
        keys.sort_unstable();
        'keys: for key in keys {
            let mut map = AHashMap::with_capacity(items.len());
            for (idx, values) in branches.iter().enumerate() {
                let Some(value) = values.get(key) else {
                    continue 'keys;
                };
                if map.insert((*value).to_string(), idx).is_some() {
                    continue 'keys;
                }
            }
            return Some(Discriminator {
                key: F::prepare_key(key),
                branches: map,
            });
        }
        None
    }

    #[inline]
    pub(crate) fn candidates(&self, instance: &F::Node<'_>) -> Candidates {
        // `required` and `properties` do not constrain anything but an object.
        let Some(object) = instance.as_object() else {
            return Candidates::All;
        };
        object
            .get(&self.key)
            .and_then(|value| value.as_string())
            .and_then(|value| self.branches.get(value.as_ref()).copied())
            .map_or(Candidates::None, Candidates::One)
    }
}

/// Narrows the branches of `oneOf` / `anyOf` that can accept an instance.
pub(crate) trait Dispatch<F: Json>: Send + Sync {
    fn candidates(&self, instance: &F::Node<'_>) -> Candidates;
}

/// Every branch stays a candidate; compiles down to the plain loop.
pub(crate) struct NoDispatch;

impl<F: Json> Dispatch<F> for NoDispatch {
    #[inline]
    fn candidates(&self, _: &F::Node<'_>) -> Candidates {
        Candidates::All
    }
}

impl<F: Json> Dispatch<F> for Discriminator<F> {
    #[inline]
    fn candidates(&self, instance: &F::Node<'_>) -> Candidates {
        Discriminator::candidates(self, instance)
    }
}

/// Which branches can accept an instance.
pub(crate) enum Candidates {
    All,
    One(usize),
    None,
}

/// What a branch provably requires: only keywords that certainly apply contribute, so anything
/// left unexamined can only constrain the branch further.
#[derive(Default)]
struct Pins<'a> {
    required: AHashSet<&'a str>,
    properties: Vec<(&'a Map<String, Value>, Resolver<'a>, Draft)>,
}

impl<'a> Pins<'a> {
    fn collect(&mut self, resolver: &Resolver<'a>, draft: Draft, schema: &'a Value, depth: usize) {
        let Value::Object(map) = schema else {
            return;
        };
        if depth > MAX_DEPTH {
            return;
        }
        if let Some(Value::String(reference)) = map.get("$ref") {
            if let Some((contents, resolver)) = resolve(resolver, draft, reference) {
                self.collect(&resolver, draft, contents, depth + 1);
            }
            // Drafts 4 to 7 ignore everything next to `$ref`.
            if matches!(draft, Draft::Draft4 | Draft::Draft6 | Draft::Draft7) {
                return;
            }
        }
        if let Some(Value::Array(required)) = map.get("required") {
            self.required
                .extend(required.iter().filter_map(Value::as_str));
        }
        if let Some(Value::Object(properties)) = map.get("properties") {
            self.properties.push((properties, resolver.clone(), draft));
        }
        if let Some(Value::Array(all_of)) = map.get("allOf") {
            for item in all_of {
                self.collect(resolver, draft, item, depth + 1);
            }
        }
    }

    /// Required names fixed to one string, limited to the keys of `wanted` when given.
    fn required_values(self, wanted: Option<&AHashMap<&str, &str>>) -> AHashMap<&'a str, &'a str> {
        let mut values = AHashMap::new();
        for name in self.required {
            if wanted.is_some_and(|wanted| !wanted.contains_key(name)) {
                continue;
            }
            for (properties, resolver, draft) in &self.properties {
                if let Some(value) = Map::get(properties, name)
                    .and_then(|subschema| pinned_value(resolver, *draft, subschema, 0))
                {
                    values.insert(name, value);
                    break;
                }
            }
        }
        values
    }
}

/// The one string a subschema accepts, from `const` or a one-item `enum`.
fn pinned_value<'a>(
    resolver: &Resolver<'a>,
    draft: Draft,
    schema: &'a Value,
    depth: usize,
) -> Option<&'a str> {
    let Value::Object(map) = schema else {
        return None;
    };
    if depth > MAX_DEPTH {
        return None;
    }
    if let Some(Value::String(reference)) = map.get("$ref") {
        if let Some((contents, resolver)) = resolve(resolver, draft, reference) {
            if let Some(value) = pinned_value(&resolver, draft, contents, depth + 1) {
                return Some(value);
            }
        }
        if matches!(draft, Draft::Draft4 | Draft::Draft6 | Draft::Draft7) {
            return None;
        }
    }
    // Draft 4 has no `const`.
    let constant = map.get("const").filter(|_| draft != Draft::Draft4);
    match (constant, map.get("enum")) {
        (Some(Value::String(value)), _) => Some(value),
        (_, Some(Value::Array(values))) => match values.as_slice() {
            [Value::String(value)] => Some(value),
            _ => None,
        },
        _ => None,
    }
}

// A target under another draft reads keywords differently, so it contributes nothing.
fn resolve<'a>(
    resolver: &Resolver<'a>,
    draft: Draft,
    reference: &str,
) -> Option<(&'a Value, Resolver<'a>)> {
    let (contents, resolver, target_draft) = resolver.lookup(reference).ok()?.into_inner();
    (target_draft == draft).then_some((contents, resolver))
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use serde_json::{json, Map, Value};
    use test_case::test_case;

    use crate::{error::ValidationErrorKind, Draft, Keyword, ValidationError, Validator};

    // Counts the branches evaluated: members are visited in key order, so `aaa`, and the probe under
    // it, comes before the `kind` that rejects a branch.
    struct Probe(Arc<AtomicUsize>);

    impl<'i> Keyword<'i> for Probe {
        fn validate(&self, _: &'i Value) -> Result<(), ValidationError<'i>> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        fn is_valid(&self, _: &'i Value) -> bool {
            self.0.fetch_add(1, Ordering::Relaxed);
            true
        }
    }

    fn build(schema: &Value, draft: Draft) -> (Validator, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let validator = crate::options()
            .with_draft(draft)
            .with_keyword("a-probe", move |_: &Map<String, Value>, _, _| {
                Ok(Box::new(Probe(Arc::clone(&counter))) as Box<dyn for<'i> Keyword<'i>>)
            })
            .build(schema)
            .expect("schema builds");
        (validator, calls)
    }

    fn branch(value: &str) -> Value {
        json!({
            "required": ["kind"],
            "properties": {
                "aaa": {"a-probe": true},
                "kind": {"const": value},
                "size": {"type": "integer"}
            }
        })
    }

    fn union(keyword: &str) -> Value {
        let branches: Vec<Value> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|v| branch(v))
            .collect();
        json!({ keyword: branches })
    }

    #[test_case("oneOf")]
    #[test_case("anyOf")]
    fn is_valid_evaluates_only_the_matching_branch(keyword: &str) {
        let (validator, calls) = build(&union(keyword), Draft::Draft202012);
        assert!(validator.is_valid(&json!({"aaa": 0, "kind": "d", "size": 1})));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test_case("oneOf")]
    #[test_case("anyOf")]
    fn validate_evaluates_only_the_matching_branch(keyword: &str) {
        let (validator, calls) = build(&union(keyword), Draft::Draft202012);
        assert!(validator
            .validate(&json!({"aaa": 0, "kind": "d", "size": 1}))
            .is_ok());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test_case("oneOf", &json!({"kind": "c", "size": 1}), true; "oneOf matching branch")]
    #[test_case("oneOf", &json!({"kind": "c", "size": "x"}), false; "oneOf matching branch fails elsewhere")]
    #[test_case("oneOf", &json!({"kind": "z"}), false; "oneOf unknown value")]
    #[test_case("oneOf", &json!({"kind": 1}), false; "oneOf non-string value")]
    #[test_case("oneOf", &json!({"size": 1}), false; "oneOf missing discriminator")]
    #[test_case("oneOf", &json!(5), false; "oneOf non-object matches every branch")]
    #[test_case("anyOf", &json!({"kind": "c", "size": 1}), true; "anyOf matching branch")]
    #[test_case("anyOf", &json!({"kind": "c", "size": "x"}), false; "anyOf matching branch fails elsewhere")]
    #[test_case("anyOf", &json!({"kind": "z"}), false; "anyOf unknown value")]
    #[test_case("anyOf", &json!({"kind": 1}), false; "anyOf non-string value")]
    #[test_case("anyOf", &json!({"size": 1}), false; "anyOf missing discriminator")]
    #[test_case("anyOf", &json!(5), true; "anyOf non-object matches every branch")]
    fn verdicts(keyword: &str, instance: &Value, expected: bool) {
        let (validator, _) = build(&union(keyword), Draft::Draft202012);
        assert_eq!(validator.is_valid(instance), expected);
        assert_eq!(validator.validate(instance).is_ok(), expected);
    }

    // A failing instance still reports every branch's errors.
    #[test]
    fn one_of_failure_keeps_every_branch_error() {
        let (validator, calls) = build(&union("oneOf"), Draft::Draft202012);
        let instance = json!({"aaa": 0, "kind": "z"});
        let error = validator
            .validate(&instance)
            .expect_err("no branch accepts `z`");
        assert!(matches!(
            error.kind(),
            ValidationErrorKind::OneOfNotValid { context } if context.len() == 5
        ));
        // Collecting each branch's errors visits every branch's `aaa`.
        assert!(calls.load(Ordering::Relaxed) >= 5);
    }

    #[test_case(json!({"$defs": {"x": branch("a")}, "oneOf": [{"$ref": "#/$defs/x"}, branch("b"), branch("c")]}); "through a reference")]
    #[test_case(json!({"oneOf": [{"allOf": [branch("a")]}, branch("b"), branch("c")]}); "pinned inside allOf only")]
    #[allow(clippy::needless_pass_by_value)]
    fn is_valid_follows_references(schema: Value) {
        let (validator, _) = build(&schema, Draft::Draft202012);
        assert!(validator.is_valid(&json!({"kind": "a"})));
        assert!(!validator.is_valid(&json!({"kind": "z"})));
    }

    // Draft 7 ignores keywords next to `$ref`, so this branch does not pin `kind` and accepts any value.
    #[test]
    fn draft7_ignores_pins_beside_a_reference() {
        let schema = json!({
            "definitions": {"open": {}},
            "oneOf": [
                {"$ref": "#/definitions/open", "required": ["kind"], "properties": {"kind": {"const": "a"}}},
                branch("b"),
                branch("c"),
            ]
        });
        let (validator, _) = build(&schema, Draft::Draft7);
        assert!(validator.is_valid(&json!({"kind": "z"})));
    }

    // Two branches pinning the same value can both match.
    #[test]
    fn duplicate_values_are_not_dispatched() {
        let schema = json!({"oneOf": [branch("a"), branch("a"), branch("b")]});
        let (validator, _) = build(&schema, Draft::Draft202012);
        assert!(!validator.is_valid(&json!({"kind": "a"})));
    }

    // A branch that pins nothing can match any value.
    #[test]
    fn unpinned_branch_is_not_dispatched() {
        let schema = json!({"oneOf": [branch("a"), branch("b"), {"required": ["size"]}]});
        let (validator, _) = build(&schema, Draft::Draft202012);
        assert!(validator.is_valid(&json!({"kind": "z", "size": 1})));
    }

    fn pinned_by(kind: &Value) -> Value {
        json!({"required": ["kind"], "properties": {"aaa": {"a-probe": true}, "kind": kind}})
    }

    // Pins behind a `$ref`, or next to one where the draft applies both.
    #[test_case(json!({"$ref": "#/$defs/a"}); "through a reference")]
    #[test_case(json!({"$ref": "#/$defs/open", "const": "a"}); "beside an open reference")]
    #[allow(clippy::needless_pass_by_value)]
    fn value_behind_a_reference_pins(kind: Value) {
        let schema = json!({
            "$defs": {"a": {"const": "a"}, "open": {}},
            "oneOf": [pinned_by(&kind), branch("b"), branch("c")]
        });
        let (validator, calls) = build(&schema, Draft::Draft202012);
        assert!(validator.is_valid(&json!({"aaa": 0, "kind": "a"})));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    // `flavor` sorts first but only one branch pins it; `kind` is pinned by all of them.
    #[test]
    fn a_key_every_branch_pins_is_chosen() {
        let first = json!({
            "required": ["flavor", "kind"],
            "properties": {"aaa": {"a-probe": true}, "flavor": {"const": "x"}, "kind": {"const": "a"}}
        });
        let schema = json!({"oneOf": [first, branch("b"), branch("c")]});
        let (validator, calls) = build(&schema, Draft::Draft202012);
        assert!(validator.is_valid(&json!({"aaa": 0, "kind": "c"})));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    // `true` accepts everything, `z` included.
    #[test]
    fn boolean_branch_is_not_dispatched() {
        let schema = json!({"oneOf": [true, branch("b"), branch("c")]});
        let (validator, _) = build(&schema, Draft::Draft202012);
        assert!(validator.is_valid(&json!({"kind": "z"})));
    }

    // None of these branches pins `kind`, so each accepts `z`.
    #[test_case(json!(true), Draft::Draft202012; "boolean subschema")]
    #[test_case(json!({"enum": ["a", "z"]}), Draft::Draft202012; "enum of several values")]
    #[test_case(json!({"$ref": "#/definitions/open", "const": "a"}), Draft::Draft7; "const beside a reference in draft 7")]
    #[test_case(json!({"$ref": "#/definitions/cycle"}), Draft::Draft202012; "reference cycle")]
    #[allow(clippy::needless_pass_by_value)]
    fn unpinned_value_is_not_dispatched(kind: Value, draft: Draft) {
        let schema = json!({
            "definitions": {"open": {}, "cycle": {"$ref": "#/definitions/cycle"}},
            "oneOf": [pinned_by(&kind), branch("b"), branch("c")]
        });
        let (validator, _) = build(&schema, draft);
        assert!(validator.is_valid(&json!({"kind": "z"})));
    }

    // A branch that is a reference cycle pins nothing and accepts everything.
    #[test]
    fn reference_cycle_branch_is_not_dispatched() {
        let schema = json!({
            "definitions": {"cycle": {"$ref": "#/definitions/cycle"}},
            "oneOf": [{"$ref": "#/definitions/cycle"}, branch("b"), branch("c")]
        });
        let (validator, _) = build(&schema, Draft::Draft202012);
        assert!(!validator.is_valid(&json!({"kind": "b"})));
        assert!(validator.is_valid(&json!({"kind": "z"})));
    }

    // Draft 4 has no `const`; a one-item `enum` pins instead.
    #[test_case("oneOf")]
    #[test_case("anyOf")]
    fn draft4_enum_pins(keyword: &str) {
        let branches: Vec<Value> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|value| {
                json!({
                    "required": ["kind"],
                    "properties": {"aaa": {"a-probe": true}, "kind": {"enum": [value]}}
                })
            })
            .collect();
        let (validator, calls) = build(&json!({ keyword: branches }), Draft::Draft4);
        assert!(validator.is_valid(&json!({"aaa": 0, "kind": "d"})));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(!validator.is_valid(&json!({"aaa": 0, "kind": "z"})));
    }

    // Draft 4 has no `const`, so these branches pin nothing and each accepts any `kind`.
    #[test_case("oneOf", false; "oneOf")]
    #[test_case("anyOf", true; "anyOf")]
    fn draft4_const_does_not_pin(keyword: &str, expected: bool) {
        let (validator, _) = build(&union(keyword), Draft::Draft4);
        assert_eq!(validator.is_valid(&json!({"kind": "z"})), expected);
    }
}
