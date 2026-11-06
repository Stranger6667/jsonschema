//! `oneOf` / `anyOf` branches that fix one property to their own strings or integers via `const`
//! or `enum`. Against an object holding that property only the branches listing its value, and
//! those that do not fix it, can pass.
use ahash::{AHashMap, AHashSet};
use referencing::{Draft, Resolver, Vocabulary};
use serde_json::{Map, Value};

use crate::{compiler, json::JsonNumber, Json, Node, Object};

// Deep enough for real `$ref` chains; a cycle stops here.
const MAX_DEPTH: usize = 16;
const MIN_BRANCHES: usize = 3;

pub(crate) struct Discriminator<F: Json> {
    key: F::PreparedKey,
    strings: AHashMap<String, Box<[usize]>>,
    integers: AHashMap<i64, Box<[usize]>>,
    // Branches accepting objects that do not fix the key.
    open: Box<[usize]>,
    // Branches accepting objects without the key.
    absent: Box<[usize]>,
    // Branches accepting objects.
    objects: Box<[usize]>,
}

/// The values one branch accepts for a property, and whether it requires the property.
struct Pin<'a> {
    values: Vec<&'a Value>,
    /// `values` lists the values the branch rejects rather than the ones it accepts.
    negated: bool,
    required: bool,
}

impl<F: Json> Discriminator<F> {
    pub(crate) fn compile(ctx: &compiler::Context<F>, items: &[Value]) -> Option<Self> {
        // With two branches a dispatch saves at most one branch, about what the analysis costs.
        if items.len() < MIN_BRANCHES
            || !ctx.has_vocabulary(&Vocabulary::Validation)
            || !ctx.has_vocabulary(&Vocabulary::Applicator)
            || [
                "required",
                "properties",
                "const",
                "enum",
                "allOf",
                "$ref",
                "type",
            ]
            .iter()
            .any(|keyword| ctx.is_keyword_overridden(keyword))
        {
            return None;
        }
        // `None` for a branch that rejects every object.
        let branches: Vec<Option<Pins>> = items
            .iter()
            .map(|item| {
                let mut pins = Pins::default();
                if let Some(resolver) = enter(ctx.resolver(), ctx.draft(), item) {
                    pins.collect(&resolver, ctx.draft(), item, 0);
                }
                (!pins.rejects_objects).then_some(pins)
            })
            .collect();
        let pins_of = |name: &str| {
            let pins: Vec<Option<Pin>> = branches
                .iter()
                .map(|pins| pins.as_ref().and_then(|pins| pins.pin(name)))
                .collect();
            // Fewer than two leaves nothing to skip.
            (pins.iter().flatten().filter(|pin| !pin.negated).count() >= 2).then_some(pins)
        };
        // Required names first: they also reject objects lacking them, and there are few of them.
        let required = ranked(
            branches
                .iter()
                .flatten()
                .flat_map(|pins| pins.required.iter().copied()),
        );
        let (key, pins) = required
            .into_iter()
            .find_map(|name| Some((name, pins_of(name)?)))
            .or_else(|| {
                ranked(branches.iter().flatten().flat_map(Pins::names))
                    .into_iter()
                    .find_map(|name| Some((name, pins_of(name)?)))
            })?;
        let mut objects = Vec::new();
        let mut open = Vec::new();
        let mut absent = Vec::new();
        let mut strings: AHashMap<String, Vec<usize>> = AHashMap::new();
        let mut integers: AHashMap<i64, Vec<usize>> = AHashMap::new();
        // Each negated branch with the values it rejects.
        let mut negated = Vec::new();
        for (idx, (branch, pin)) in branches.iter().zip(&pins).enumerate() {
            if branch.is_none() {
                continue;
            }
            objects.push(idx);
            let Some(pin) = pin else {
                open.push(idx);
                absent.push(idx);
                continue;
            };
            if !pin.required {
                absent.push(idx);
            }
            if pin.negated {
                open.push(idx);
                negated.push((idx, &pin.values));
            }
            for value in &pin.values {
                let list = match value {
                    Value::String(string) => strings.entry(string.clone()).or_default(),
                    _ => integers.entry(value.as_i64()?).or_default(),
                };
                if !pin.negated && list.last() != Some(&idx) {
                    list.push(idx);
                }
            }
        }
        // Branches that fix the value, then those open to it: every open one but the negated
        // branches rejecting it.
        let with_open = |value: &Value, mut list: Vec<usize>| {
            list.extend(open.iter().copied().filter(|idx| {
                !negated
                    .iter()
                    .any(|(negated, values)| negated == idx && values.contains(&value))
            }));
            list.sort_unstable();
            list.into_boxed_slice()
        };
        let strings = strings
            .into_iter()
            .map(|(value, list)| {
                let list = with_open(&Value::String(value.clone()), list);
                (value, list)
            })
            .collect();
        let integers = integers
            .into_iter()
            .map(|(value, list)| (value, with_open(&Value::from(value), list)))
            .collect();
        Some(Discriminator {
            key: F::prepare_key(key),
            strings,
            integers,
            open: open.into_boxed_slice(),
            absent: absent.into_boxed_slice(),
            objects: objects.into_boxed_slice(),
        })
    }

    #[inline]
    pub(crate) fn candidates(&self, instance: &F::Node<'_>) -> Candidates<'_> {
        // `properties` and `type` are what tell the branches apart, and only for objects.
        let Some(object) = instance.as_object() else {
            return Candidates::All;
        };
        let Some(value) = object.get(&self.key) else {
            return Candidates::Only(&self.absent);
        };
        let branches = if let Some(string) = value.as_string() {
            self.strings.get(string.as_ref())
        } else if let Some(number) = value.as_number() {
            // `3.0` equals the pinned `3`, and only `as_i64` maps both to one key.
            let Some(integer) = number.as_i64() else {
                return Candidates::Only(&self.objects);
            };
            self.integers.get(&integer)
        } else {
            None
        };
        Candidates::Only(branches.map_or(&self.open, |branches| branches))
    }
}

/// Narrows the branches of `oneOf` / `anyOf` that can accept an instance.
pub(crate) trait Dispatch<F: Json>: Send + Sync {
    fn candidates(&self, instance: &F::Node<'_>) -> Candidates<'_>;
}

/// Every branch stays a candidate; compiles down to the plain loop.
pub(crate) struct NoDispatch;

impl<F: Json> Dispatch<F> for NoDispatch {
    #[inline]
    fn candidates(&self, _: &F::Node<'_>) -> Candidates<'_> {
        Candidates::All
    }
}

impl<F: Json> Dispatch<F> for Discriminator<F> {
    #[inline]
    fn candidates(&self, instance: &F::Node<'_>) -> Candidates<'_> {
        Discriminator::candidates(self, instance)
    }
}

/// Which branches can accept an instance.
#[derive(Clone, Copy)]
pub(crate) enum Candidates<'a> {
    All,
    /// Ascending branch indices.
    Only(&'a [usize]),
}

/// What a branch provably requires: only keywords that certainly apply contribute, so anything
/// left unexamined can only constrain the branch further.
#[derive(Default)]
struct Pins<'a> {
    rejects_objects: bool,
    required: AHashSet<&'a str>,
    properties: Vec<(&'a Map<String, Value>, Resolver<'a>, Draft)>,
}

impl<'a> Pins<'a> {
    fn collect(&mut self, resolver: &Resolver<'a>, draft: Draft, schema: &'a Value, depth: usize) {
        let map = match schema {
            Value::Object(map) => map,
            Value::Bool(false) => {
                self.rejects_objects = true;
                return;
            }
            _ => return,
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
        self.rejects_objects |= match map.get("type") {
            Some(Value::String(name)) => name != "object",
            Some(Value::Array(names)) => !names.iter().any(|name| name == "object"),
            _ => false,
        };
        if let Some(Value::Array(required)) = map.get("required") {
            self.required
                .extend(required.iter().filter_map(Value::as_str));
        }
        if let Some(Value::Object(properties)) = map.get("properties") {
            self.properties.push((properties, resolver.clone(), draft));
        }
        if let Some(Value::Array(all_of)) = map.get("allOf") {
            for item in all_of {
                if let Some(resolver) = enter(resolver, draft, item) {
                    self.collect(&resolver, draft, item, depth + 1);
                }
            }
        }
    }

    fn names(&self) -> AHashSet<&'a str> {
        self.properties
            .iter()
            .flat_map(|(properties, _, _)| properties.keys().map(String::as_str))
            .collect()
    }

    /// The strings or integers `name` is fixed to, if any.
    fn pin(&self, name: &str) -> Option<Pin<'a>> {
        self.properties
            .iter()
            .find_map(|(properties, resolver, draft)| {
                let schema = Map::get(properties, name)?;
                let resolver = &enter(resolver, *draft, schema)?;
                let (values, negated) = match pinned_values(resolver, *draft, schema, 0) {
                    Some(values) => (values, false),
                    None => (negated_values(resolver, *draft, schema, 0)?, true),
                };
                Some(Pin {
                    values,
                    negated,
                    required: self.required.contains(name),
                })
            })
    }
}

/// Names seen in two or more branches, most frequent first.
fn ranked<'a>(names: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut counts: AHashMap<&str, usize> = AHashMap::new();
    for name in names {
        *counts.entry(name).or_default() += 1;
    }
    let mut names: Vec<(&str, usize)> = counts
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .collect();
    names.sort_unstable_by(|(left, left_count), (right, right_count)| {
        right_count.cmp(left_count).then_with(|| left.cmp(right))
    });
    names.into_iter().map(|(name, _)| name).collect()
}

// Strings and integers that fit `i64`; other values have no exact dispatch key.
fn is_pinnable(value: &Value) -> bool {
    match value {
        Value::String(_) => true,
        Value::Number(number) => number.as_i64().is_some(),
        _ => false,
    }
}

/// The values a subschema accepts, from `const` or `enum`.
fn pinned_values<'a>(
    resolver: &Resolver<'a>,
    draft: Draft,
    schema: &'a Value,
    depth: usize,
) -> Option<Vec<&'a Value>> {
    let Value::Object(map) = schema else {
        return None;
    };
    if depth > MAX_DEPTH {
        return None;
    }
    if let Some(Value::String(reference)) = map.get("$ref") {
        if let Some((contents, resolver)) = resolve(resolver, draft, reference) {
            if let Some(values) = pinned_values(&resolver, draft, contents, depth + 1) {
                return Some(values);
            }
        }
        if matches!(draft, Draft::Draft4 | Draft::Draft6 | Draft::Draft7) {
            return None;
        }
    }
    // Draft 4 has no `const`.
    let constant = map.get("const").filter(|_| draft != Draft::Draft4);
    match (constant, map.get("enum")) {
        (Some(value), _) if is_pinnable(value) => Some(vec![value]),
        (_, Some(Value::Array(values))) if !values.is_empty() && values.iter().all(is_pinnable) => {
            Some(values.iter().collect())
        }
        _ => None,
    }
}

/// The values a subschema rejects through `not` over exactly an `enum` or `const`.
fn negated_values<'a>(
    resolver: &Resolver<'a>,
    draft: Draft,
    schema: &'a Value,
    depth: usize,
) -> Option<Vec<&'a Value>> {
    let Value::Object(map) = schema else {
        return None;
    };
    if depth > MAX_DEPTH {
        return None;
    }
    if let Some(Value::String(reference)) = map.get("$ref") {
        if let Some((contents, resolver)) = resolve(resolver, draft, reference) {
            if let Some(values) = negated_values(&resolver, draft, contents, depth + 1) {
                return Some(values);
            }
        }
        if matches!(draft, Draft::Draft4 | Draft::Draft6 | Draft::Draft7) {
            return None;
        }
    }
    let negated = map.get("not")?;
    exact_values(&enter(resolver, draft, negated)?, draft, negated, depth + 1)
}

/// The values a subschema accepts when `const` or `enum` is all it checks.
fn exact_values<'a>(
    resolver: &Resolver<'a>,
    draft: Draft,
    schema: &'a Value,
    depth: usize,
) -> Option<Vec<&'a Value>> {
    let Value::Object(map) = schema else {
        return None;
    };
    if depth > MAX_DEPTH {
        return None;
    }
    let mut checks = map.keys().filter(|keyword| {
        !matches!(
            keyword.as_str(),
            "title" | "description" | "$comment" | "examples" | "default"
        )
    });
    match (checks.next().map(String::as_str), checks.next()) {
        (Some("$ref"), None) => {
            let Value::String(reference) = &map["$ref"] else {
                return None;
            };
            let (contents, resolver) = resolve(resolver, draft, reference)?;
            exact_values(&resolver, draft, contents, depth + 1)
        }
        (Some("enum" | "const"), None) => pinned_values(resolver, draft, schema, depth),
        _ => None,
    }
}

/// The resolver inside `schema`, whose own id, if any, rebases the references it holds. Targets
/// `resolve` returns are already entered.
fn enter<'a>(resolver: &Resolver<'a>, draft: Draft, schema: &'a Value) -> Option<Resolver<'a>> {
    resolver
        .in_subresource(draft.create_resource_ref(schema))
        .ok()
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

    // A subschema's `$id` changes what the `$ref`s inside it point to: `k.json` is `real` there.
    #[test_case(json!({"properties": {"kind": {"$id": "inner/", "$ref": "k.json"}}}), &json!({"kind": "real"}), true; "beside the reference")]
    #[test_case(json!({"properties": {"kind": {"$id": "inner/", "$ref": "k.json"}}}), &json!({"kind": "decoy"}), false; "beside the reference, other value")]
    #[test_case(json!({"$id": "inner/", "properties": {"kind": {"$ref": "k.json"}}}), &json!({"kind": "real"}), true; "on the branch")]
    #[test_case(json!({"allOf": [{"$id": "inner/", "properties": {"kind": {"$ref": "k.json"}}}]}), &json!({"kind": "real"}), true; "on an allOf item")]
    #[test_case(json!({"properties": {"kind": {"not": {"$id": "inner/", "$ref": "k.json"}}}}), &json!({"kind": "decoy"}), true; "under not")]
    #[test_case(json!({"properties": {"kind": {"not": {"$id": "inner/", "$ref": "k.json"}}}}), &json!({"kind": "real"}), false; "under not, excluded value")]
    #[allow(clippy::needless_pass_by_value)]
    fn id_rebases_references(branch: Value, instance: &Value, expected: bool) {
        let schema = json!({
            "$id": "http://example.com/root.json",
            "$defs": {
                "decoy": {"$id": "http://example.com/k.json", "const": "decoy"},
                "real": {"$id": "http://example.com/inner/k.json", "const": "real"}
            },
            "oneOf": [
                branch,
                {"properties": {"kind": {"const": "b"}}, "required": ["kind"]},
                {"properties": {"kind": {"const": "c"}}, "required": ["kind"]}
            ]
        });
        let validator = crate::validator_for(&schema).expect("schema builds");
        assert_eq!(validator.is_valid(instance), expected);
        assert_eq!(validator.validate(instance).is_ok(), expected);
        assert_eq!(validator.evaluate(instance).flag().valid, expected);
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

    // Branches that pin `kind` without requiring it.
    fn optional_union(keyword: &str, pins: &[Value]) -> Value {
        let branches: Vec<Value> = pins
            .iter()
            .map(|kind| {
                json!({
                    "properties": {"aaa": {"a-probe": true}, "kind": kind, "size": {"type": "integer"}}
                })
            })
            .collect();
        json!({ keyword: branches })
    }

    fn string_pins() -> Vec<Value> {
        ["a", "b", "c", "d", "e"]
            .iter()
            .map(|value| json!({"const": value}))
            .collect()
    }

    fn integer_pins() -> Vec<Value> {
        (1..=5).map(|value| json!({"const": value})).collect()
    }

    fn enum_pins() -> Vec<Value> {
        ["a", "b", "c", "d", "e"]
            .iter()
            .map(|value| json!({"enum": [format!("{value}1"), format!("{value}2")]}))
            .collect()
    }

    #[test_case("oneOf", string_pins(), &json!({"aaa": 0, "kind": "d"}); "oneOf optional string const")]
    #[test_case("anyOf", string_pins(), &json!({"aaa": 0, "kind": "d"}); "anyOf optional string const")]
    #[test_case("oneOf", integer_pins(), &json!({"aaa": 0, "kind": 4}); "oneOf integer const")]
    #[test_case("anyOf", integer_pins(), &json!({"aaa": 0, "kind": 4}); "anyOf integer const")]
    #[test_case("oneOf", enum_pins(), &json!({"aaa": 0, "kind": "d2"}); "oneOf enum of several values")]
    #[test_case("anyOf", enum_pins(), &json!({"aaa": 0, "kind": "d2"}); "anyOf enum of several values")]
    #[allow(clippy::needless_pass_by_value)]
    fn optional_key_evaluates_only_the_matching_branch(
        keyword: &str,
        pins: Vec<Value>,
        instance: &Value,
    ) {
        let (validator, calls) = build(&optional_union(keyword, &pins), Draft::Draft202012);
        assert!(validator.is_valid(instance));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        calls.store(0, Ordering::Relaxed);
        assert!(validator.validate(instance).is_ok());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    // Without `required`, every branch accepts an object lacking `kind`.
    #[test_case("oneOf", string_pins(), &json!({"kind": "c", "size": 1}), true; "oneOf string matching branch")]
    #[test_case("oneOf", string_pins(), &json!({"kind": "c", "size": "x"}), false; "oneOf string matching branch fails elsewhere")]
    #[test_case("oneOf", string_pins(), &json!({"kind": "z"}), false; "oneOf string unknown value")]
    #[test_case("oneOf", string_pins(), &json!({"kind": 1}), false; "oneOf string pins against a number")]
    #[test_case("oneOf", string_pins(), &json!({"size": 1}), false; "oneOf string missing key")]
    #[test_case("anyOf", string_pins(), &json!({"size": 1}), true; "anyOf string missing key")]
    #[test_case("anyOf", string_pins(), &json!({"kind": "z"}), false; "anyOf string unknown value")]
    #[test_case("anyOf", string_pins(), &json!({"kind": null}), false; "anyOf string pins against null")]
    #[test_case("oneOf", integer_pins(), &json!({"kind": 3, "size": 1}), true; "oneOf integer matching branch")]
    #[test_case("oneOf", integer_pins(), &json!({"kind": 3.0}), true; "oneOf integer written as a float")]
    #[test_case("anyOf", integer_pins(), &json!({"kind": 3.0}), true; "anyOf integer written as a float")]
    #[test_case("oneOf", integer_pins(), &json!({"kind": 3.5}), false; "oneOf integer pins against a fraction")]
    #[test_case("oneOf", integer_pins(), &json!({"kind": 9}), false; "oneOf integer unknown value")]
    #[test_case("oneOf", integer_pins(), &json!({"kind": "3"}), false; "oneOf integer pins against a string")]
    #[test_case("anyOf", integer_pins(), &json!({"kind": 18_446_744_073_709_551_615_u64}), false; "anyOf integer past i64")]
    #[test_case("oneOf", enum_pins(), &json!({"kind": "b1"}), true; "oneOf enum first value")]
    #[test_case("oneOf", enum_pins(), &json!({"kind": "b3"}), false; "oneOf enum unknown value")]
    #[test_case("oneOf", vec![json!({"enum": ["a", "z"]}), json!({"const": "b"}), json!({"const": "c"})], &json!({"kind": "z"}), true; "oneOf enum second value")]
    #[test_case("oneOf", vec![json!({"enum": ["a", 1]}), json!({"const": "b"}), json!({"const": 2})], &json!({"kind": 1}), true; "oneOf enum of mixed types")]
    #[test_case("oneOf", vec![json!({"enum": ["a", "b"]}), json!({"enum": ["b", "c"]}), json!({"const": "d"})], &json!({"kind": "b"}), false; "oneOf values shared by two branches")]
    #[test_case("anyOf", vec![json!({"enum": ["a", "b"]}), json!({"enum": ["b", "c"]}), json!({"const": "d"})], &json!({"kind": "b"}), true; "anyOf values shared by two branches")]
    #[test_case("oneOf", vec![json!({"const": 1}), json!({"const": 1}), json!({"const": 2})], &json!({"kind": 1}), false; "oneOf duplicate integers")]
    #[test_case("oneOf", vec![json!({"enum": [true]}), json!({"const": "b"}), json!({"const": "c"})], &json!({"kind": true}), true; "oneOf boolean pin")]
    #[test_case("oneOf", vec![json!({"const": 1.5}), json!({"const": 2}), json!({"const": 3})], &json!({"kind": 1.5}), true; "oneOf fractional pin")]
    #[allow(clippy::needless_pass_by_value)]
    fn optional_key_verdicts(keyword: &str, pins: Vec<Value>, instance: &Value, expected: bool) {
        let (validator, _) = build(&optional_union(keyword, &pins), Draft::Draft202012);
        assert_eq!(validator.is_valid(instance), expected);
        assert_eq!(validator.validate(instance).is_ok(), expected);
    }

    fn optional_branch(kind: &Value) -> Value {
        json!({"properties": {"aaa": {"a-probe": true}, "kind": kind}})
    }

    // A branch without a pin is tried next to the matching one; one that rejects objects is not.
    #[test_case(json!({"properties": {"aaa": {"a-probe": true}, "kind": {"type": "integer"}}}), 2; "branch without a pin")]
    #[test_case(json!({"properties": {"aaa": {"a-probe": true}, "kind": {"not": {"enum": ["a"]}, "type": "integer"}}}), 2; "branch bounding the key with negation")]
    #[test_case(json!({"type": "number"}), 1; "branch rejecting objects")]
    #[test_case(json!({"$ref": "#/$defs/number"}), 1; "branch rejecting objects through a reference")]
    #[test_case(json!(false), 1; "false branch")]
    #[allow(clippy::needless_pass_by_value)]
    fn open_branches_stay_candidates(extra: Value, expected_calls: usize) {
        let schema = json!({
            "$defs": {"number": {"type": ["number", "string"]}},
            "oneOf": [
                optional_branch(&json!({"const": "a"})),
                optional_branch(&json!({"const": "b"})),
                optional_branch(&json!({"const": "c"})),
                extra
            ]
        });
        let (validator, calls) = build(&schema, Draft::Draft202012);
        assert!(validator.is_valid(&json!({"aaa": 0, "kind": "b"})));
        assert_eq!(calls.load(Ordering::Relaxed), expected_calls);
    }

    // Verdicts with a branch that pins nothing, or one that rejects objects.
    #[test_case(json!({"properties": {"kind": {"type": "integer"}}}), &json!({"kind": 1}), true; "open branch alone")]
    #[test_case(json!({"properties": {"kind": {"type": "integer"}}}), &json!({"kind": "b"}), true; "pinned branch alone")]
    #[test_case(json!({"properties": {"kind": {"type": "string"}}}), &json!({"kind": "b"}), false; "pinned and open branches both")]
    #[test_case(json!({"properties": {"kind": {"type": "string"}}}), &json!({"kind": "z"}), true; "unknown value open branch")]
    #[test_case(json!({"properties": {"kind": {"type": "integer"}}}), &json!({"kind": "z"}), false; "unknown value no branch")]
    #[test_case(json!({"properties": {"kind": {"type": "integer"}}}), &json!({}), false; "missing key every branch")]
    #[test_case(json!({"type": "number"}), &json!(5), false; "non-object instance")]
    #[test_case(json!({"type": "number"}), &json!({"kind": "b"}), true; "object instance skips number branch")]
    #[test_case(json!({"type": "number"}), &json!({"kind": 1.5}), false; "fraction")]
    #[test_case(json!(true), &json!({"kind": "b"}), false; "true branch")]
    #[test_case(json!(false), &json!({"kind": "b"}), true; "false branch")]
    #[allow(clippy::needless_pass_by_value)]
    fn open_branch_verdicts(extra: Value, instance: &Value, expected: bool) {
        let schema = json!({
            "oneOf": [
                optional_branch(&json!({"const": "a"})),
                optional_branch(&json!({"const": "b"})),
                optional_branch(&json!({"const": "c"})),
                extra
            ]
        });
        let (validator, _) = build(&schema, Draft::Draft202012);
        assert_eq!(validator.is_valid(instance), expected);
        assert_eq!(validator.validate(instance).is_ok(), expected);
    }

    // A branch excluding values with `not` is skipped for them, and tried for any other value.
    #[test_case(&json!({"aaa": 0, "kind": "b"}), true, 1; "excluded value")]
    #[test_case(&json!({"aaa": 0, "kind": "z"}), true, 1; "value only the negated branch accepts")]
    #[test_case(&json!({"aaa": 0, "kind": "x"}), false, 0; "excluded value no branch fixes")]
    #[test_case(&json!({"aaa": 0, "kind": 1}), false, 1; "other type")]
    #[test_case(&json!({"aaa": 0}), false, 2; "missing key")]
    fn negated_values_exclude_a_branch(instance: &Value, expected: bool, expected_calls: usize) {
        for negated in [
            json!({"type": "string", "not": {"$ref": "#/$defs/known"}}),
            json!({"$ref": "#/$defs/unknown"}),
        ] {
            let schema = json!({
                "$defs": {
                    "known": {"enum": ["a", "b", "c", "x"]},
                    "unknown": {"type": "string", "not": {"$ref": "#/$defs/known"}}
                },
                "oneOf": [
                    optional_branch(&json!({"const": "a"})),
                    optional_branch(&json!({"const": "b"})),
                    optional_branch(&json!({"const": "c"})),
                    optional_branch(&negated)
                ]
            });
            let (validator, calls) = build(&schema, Draft::Draft202012);
            assert_eq!(validator.is_valid(instance), expected, "{negated}");
            assert_eq!(calls.load(Ordering::Relaxed), expected_calls, "{negated}");
            assert_eq!(validator.validate(instance).is_ok(), expected, "{negated}");
        }
    }

    // `not` over a reference cycle stops at the depth limit and excludes nothing.
    #[test_case(&json!({"aaa": 0, "kind": "b"}), true, 2; "fixed value")]
    #[test_case(&json!({"aaa": 0, "kind": "z"}), false, 1; "other value")]
    fn negated_reference_cycle_is_not_dispatched(
        instance: &Value,
        expected: bool,
        expected_calls: usize,
    ) {
        let schema = json!({
            "$defs": {"cycle": {"$ref": "#/$defs/cycle"}},
            "oneOf": [
                optional_branch(&json!({"const": "a"})),
                optional_branch(&json!({"const": "b"})),
                optional_branch(&json!({"const": "c"})),
                optional_branch(&json!({"not": {"$ref": "#/$defs/cycle"}}))
            ]
        });
        let (validator, calls) = build(&schema, Draft::Draft202012);
        assert_eq!(validator.is_valid(instance), expected);
        assert_eq!(calls.load(Ordering::Relaxed), expected_calls);
        assert_eq!(validator.validate(instance).is_ok(), expected);
    }

    // `not` over more than the values: `a` fails `minLength`, so the negation accepts it.
    #[test_case(json!({"not": false}); "boolean")]
    #[test_case(json!({"not": {"enum": ["a"], "minLength": 5}}); "extra keyword")]
    #[test_case(json!({"not": {"$ref": "#/$defs/short"}}); "extra keyword behind a reference")]
    #[test_case(json!({"not": {"$ref": "#/$defs/plain", "enum": ["a"]}}); "enum beside a reference")]
    #[allow(clippy::needless_pass_by_value)]
    fn partial_negation_does_not_exclude(kind: Value) {
        let schema = json!({
            "$defs": {"short": {"enum": ["a"], "minLength": 5}, "plain": {"minLength": 5}},
            "oneOf": [
                optional_branch(&json!({"const": "b"})),
                optional_branch(&json!({"const": "c"})),
                optional_branch(&json!({"const": "d"})),
                optional_branch(&kind)
            ]
        });
        let (validator, _) = build(&schema, Draft::Draft202012);
        assert!(validator.is_valid(&json!({"kind": "a"})));
        assert!(validator.validate(&json!({"kind": "a"})).is_ok());
    }

    // Branches sharing a value are all tried.
    #[test_case("oneOf", &json!({"aaa": 0, "kind": "b"}), false, 2; "oneOf shared value")]
    #[test_case("anyOf", &json!({"aaa": 0, "kind": "b"}), true, 1; "anyOf shared value")]
    #[test_case("oneOf", &json!({"aaa": 0, "kind": "c"}), true, 1; "oneOf value of one branch")]
    fn shared_values_dispatch(
        keyword: &str,
        instance: &Value,
        expected: bool,
        expected_calls: usize,
    ) {
        let pins = [
            json!({"enum": ["a", "b"]}),
            json!({"enum": ["b", "c"]}),
            json!({"const": "d"}),
            json!({"const": "e"}),
        ];
        let branches: Vec<Value> = pins.iter().map(optional_branch).collect();
        let (validator, calls) = build(&json!({ keyword: branches }), Draft::Draft202012);
        assert_eq!(validator.is_valid(instance), expected);
        assert_eq!(calls.load(Ordering::Relaxed), expected_calls);
    }

    // One branch requires `kind`, the others accept an object lacking it.
    #[test_case("oneOf", &json!({"size": 1}), false; "oneOf missing key")]
    #[test_case("anyOf", &json!({"size": 1}), true; "anyOf missing key")]
    #[test_case("oneOf", &json!({"kind": "a", "size": 1}), true; "oneOf required branch")]
    #[test_case("oneOf", &json!({"kind": "b", "size": 1}), true; "oneOf optional branch")]
    fn some_branches_require_the_key(keyword: &str, instance: &Value, expected: bool) {
        let optional = |value: &str| json!({"properties": {"kind": {"const": value}, "size": {"type": "integer"}}});
        let schema = json!({ keyword: [branch("a"), optional("b"), optional("c")] });
        let (validator, _) = build(&schema, Draft::Draft202012);
        assert_eq!(validator.is_valid(instance), expected);
        assert_eq!(validator.validate(instance).is_ok(), expected);
    }

    // The key is required next to the union, not inside its branches.
    #[test]
    fn key_required_beside_the_union() {
        let mut schema = optional_union("oneOf", &integer_pins());
        schema["required"] = json!(["kind"]);
        let (validator, calls) = build(&schema, Draft::Draft202012);
        assert!(validator.is_valid(&json!({"aaa": 0, "kind": 2})));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(!validator.is_valid(&json!({"aaa": 0})));
    }

    // A failing instance still reports every branch's errors.
    #[test]
    fn optional_key_failure_keeps_every_branch_error() {
        let (validator, _) = build(&optional_union("oneOf", &string_pins()), Draft::Draft202012);
        let instance = json!({"kind": "z"});
        let error = validator
            .validate(&instance)
            .expect_err("no branch accepts `z`");
        assert!(matches!(
            error.kind(),
            ValidationErrorKind::OneOfNotValid { context } if context.len() == 5
        ));
    }

    // Draft 4 has no `const`, so these branches pin nothing and each accepts any `kind`.
    #[test_case("oneOf", false; "oneOf")]
    #[test_case("anyOf", true; "anyOf")]
    fn draft4_const_does_not_pin(keyword: &str, expected: bool) {
        let (validator, _) = build(&union(keyword), Draft::Draft4);
        assert_eq!(validator.is_valid(&json!({"kind": "z"})), expected);
    }
}
