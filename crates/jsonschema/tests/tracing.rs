//! Regression tests for the tracing implementation.
//!
//! Each test verifies a specific invariant of the tracing contract:
//!
//! 1. **Parity**: `trace()` return value == `is_valid()` for every schema/instance pair.
//! 2. **Accuracy**: the `result` field in each callback matches what that specific
//!    sub-validator would return.
//! 3. **No short-circuit**: all sub-schemas are visited even when one fails.
//! 4. **Correct instance paths**: `instance_location` in each callback points to
//!    the value being validated, not the parent.
//! 5. **Correct schema paths**: `schema_location` in each callback points to the
//!    right schema keyword.

use jsonschema::{validator_for, Draft, NodeEvaluationResult};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Event {
    instance_path: String,
    schema_path: String,
    result: NodeEvaluationResult,
}

/// Run `trace()`, collect all callback events, and assert the return value
/// matches `is_valid()`.  Returns `(events, overall_result)`.
#[track_caller]
fn collect(schema: &Value, instance: &Value) -> (Vec<Event>, bool) {
    let validator = validator_for(schema).expect("invalid schema");
    let mut events = Vec::new();
    let trace_result = validator.trace(instance, &mut |ctx| {
        events.push(Event {
            instance_path: jsonschema::paths::Location::from(ctx.instance_location)
                .as_str()
                .to_string(),
            schema_path: ctx.schema_location.as_str().to_string(),
            result: ctx.result,
        });
    });
    let is_valid_result = validator.is_valid(instance);
    assert_eq!(
        trace_result, is_valid_result,
        "trace() = {trace_result} but is_valid() = {is_valid_result}\n\
         schema  = {schema}\n\
         instance= {instance}"
    );
    (events, trace_result)
}

/// Like `collect` but compiles the schema under Draft 7 (for legacy `items` array syntax).
#[track_caller]
fn collect_draft7(schema: &Value, instance: &Value) -> (Vec<Event>, bool) {
    let validator = jsonschema::options()
        .with_draft(Draft::Draft7)
        .build(schema)
        .expect("invalid schema");
    let mut events = Vec::new();
    let trace_result = validator.trace(instance, &mut |ctx| {
        events.push(Event {
            instance_path: jsonschema::paths::Location::from(ctx.instance_location)
                .as_str()
                .to_string(),
            schema_path: ctx.schema_location.as_str().to_string(),
            result: ctx.result,
        });
    });
    let is_valid_result = validator.is_valid(instance);
    assert_eq!(
        trace_result, is_valid_result,
        "trace() = {trace_result} but is_valid() = {is_valid_result}\n\
         schema  = {schema}\n\
         instance= {instance}"
    );
    (events, trace_result)
}

/// Return every event whose `schema_path` equals `path`.
fn at_schema<'a>(events: &'a [Event], path: &str) -> Vec<&'a Event> {
    events.iter().filter(|e| e.schema_path == path).collect()
}

// ---------------------------------------------------------------------------
// Parity: trace() result must match is_valid() across many schema patterns
// ---------------------------------------------------------------------------

#[test]
fn parity_type() {
    let schema = json!({"type": "string"});
    collect(&schema, &json!("hello"));
    collect(&schema, &json!(42));
}

#[test]
fn parity_properties() {
    let schema = json!({"properties": {"x": {"type": "integer"}, "y": {"type": "string"}}});
    collect(&schema, &json!({"x": 1, "y": "a"})); // valid
    collect(&schema, &json!({"x": "bad", "y": 2})); // both fail
    collect(&schema, &json!({})); // no properties present
}

#[test]
fn parity_required() {
    let schema = json!({"required": ["a", "b", "c"]});
    collect(&schema, &json!({"a": 1, "b": 2, "c": 3}));
    collect(&schema, &json!({"a": 1}));
}

#[test]
fn parity_all_of() {
    let schema = json!({"allOf": [{"type": "integer"}, {"minimum": 5}]});
    collect(&schema, &json!(10));
    collect(&schema, &json!(3));
    collect(&schema, &json!("not-int"));
}

#[test]
fn parity_any_of() {
    let schema = json!({"anyOf": [{"type": "string"}, {"type": "integer"}]});
    collect(&schema, &json!("hello"));
    collect(&schema, &json!(1));
    collect(&schema, &json!(1.5));
}

#[test]
fn parity_one_of() {
    let schema = json!({"oneOf": [{"type": "string"}, {"minLength": 3}]});
    collect(&schema, &json!("hi")); // only first matches → valid
    collect(&schema, &json!("hello")); // both match → invalid
    collect(&schema, &json!(42)); // none match → invalid
}

#[test]
fn parity_not() {
    let schema = json!({"not": {"type": "string"}});
    collect(&schema, &json!(42)); // valid
    collect(&schema, &json!("hello")); // invalid
}

#[test]
fn parity_if_then_else() {
    let schema =
        json!({"if": {"type": "integer"}, "then": {"minimum": 0}, "else": {"minLength": 1}});
    collect(&schema, &json!(5)); // if passes, then passes
    collect(&schema, &json!(-1)); // if passes, then fails
    collect(&schema, &json!("hi")); // if fails, else passes
    collect(&schema, &json!("")); // if fails, else fails
}

#[test]
fn parity_contains() {
    let schema = json!({"contains": {"type": "integer"}});
    collect(&schema, &json!([1, "a"]));
    collect(&schema, &json!(["a", "b"]));
}

#[test]
fn parity_min_contains() {
    let schema = json!({"contains": {"type": "integer"}, "minContains": 2});
    collect(&schema, &json!([1, 2, "a"])); // valid: 2 ints
    collect(&schema, &json!([1, "a", "b"])); // invalid: only 1 int
    collect(&schema, &json!(["a", "b"])); // invalid: 0 ints
}

#[test]
fn parity_items_array() {
    let schema = json!({"items": [{"type": "integer"}, {"type": "string"}]});
    collect_draft7(&schema, &json!([1, "hello"]));
    collect_draft7(&schema, &json!(["bad", 42]));
}

#[test]
fn parity_additional_properties_false() {
    let schema = json!({"properties": {"x": {}}, "additionalProperties": false});
    collect(&schema, &json!({"x": 1}));
    collect(&schema, &json!({"x": 1, "extra": 2}));
}

#[test]
fn parity_pattern() {
    // Exercises PrefixPatternValidator, ExactPatternValidator, AlternationPatternValidator
    for pattern in ["^foo", r"^\$ref$", r"^(get|put|post)$"] {
        let schema = json!({"pattern": pattern});
        collect(&schema, &json!("foobar"));
        collect(&schema, &json!("nomatch"));
        collect(&schema, &json!(42)); // non-string: ignored
    }
}

#[test]
fn parity_enum() {
    let schema = json!({"enum": ["a", "b", "c"]});
    collect(&schema, &json!("a"));
    collect(&schema, &json!("z"));
    collect(&schema, &json!(1));
}

// ---------------------------------------------------------------------------
// Regression: not — exactly one /not callback, correct result
// ---------------------------------------------------------------------------

#[test]
fn regression_not_single_callback_valid_instance() {
    // 42 is not a string → not is satisfied
    let (events, result) = collect(&json!({"not": {"type": "string"}}), &json!(42));
    assert!(result);
    let not_events = at_schema(&events, "/not");
    assert_eq!(
        not_events.len(),
        1,
        "expected exactly one /not callback, got {}: {:?}",
        not_events.len(),
        not_events
    );
    assert_eq!(not_events[0].result, NodeEvaluationResult::Valid);
}

#[test]
fn regression_not_single_callback_invalid_instance() {
    // "hello" is a string → not is violated
    let (events, result) = collect(&json!({"not": {"type": "string"}}), &json!("hello"));
    assert!(!result);
    let not_events = at_schema(&events, "/not");
    assert_eq!(
        not_events.len(),
        1,
        "expected exactly one /not callback, got {}: {:?}",
        not_events.len(),
        not_events
    );
    assert_eq!(not_events[0].result, NodeEvaluationResult::Invalid);
}

#[test]
fn regression_not_inner_schema_result_is_negated() {
    // The /not callback result must be the negation of what the inner schema returned.
    // /not/type fires for the inner type validator; /not fires for the not keyword.
    let schema = json!({"not": {"type": "string"}});

    // Inner schema (type: string) passes on "hello" → not fails
    let (events, _) = collect(&schema, &json!("hello"));
    let inner = at_schema(&events, "/not/type");
    assert!(!inner.is_empty(), "expected /not/type callback");
    assert_eq!(
        inner[0].result,
        NodeEvaluationResult::Valid,
        "/not/type should be Valid for a string"
    );
    let outer = at_schema(&events, "/not");
    assert_eq!(
        outer[0].result,
        NodeEvaluationResult::Invalid,
        "/not should be Invalid"
    );

    // Inner schema (type: string) fails on 42 → not passes
    let (events, _) = collect(&schema, &json!(42));
    let inner = at_schema(&events, "/not/type");
    assert!(!inner.is_empty(), "expected /not/type callback");
    assert_eq!(
        inner[0].result,
        NodeEvaluationResult::Invalid,
        "/not/type should be Invalid for 42"
    );
    let outer = at_schema(&events, "/not");
    assert_eq!(
        outer[0].result,
        NodeEvaluationResult::Valid,
        "/not should be Valid"
    );
}

// ---------------------------------------------------------------------------
// Regression: properties — per-property summary uses property instance path
// ---------------------------------------------------------------------------

#[test]
fn regression_properties_instance_path_is_property_path() {
    let schema = json!({"properties": {"name": {"type": "string"}, "age": {"type": "integer"}}});
    let instance = json!({"name": "Alice", "age": 30});
    let (events, _) = collect(&schema, &instance);

    // The summary callback for /properties/name must have instance_path "/name"
    let name_events = at_schema(&events, "/properties/name");
    assert!(
        !name_events.is_empty(),
        "expected /properties/name callback"
    );
    for e in &name_events {
        assert_eq!(
            e.instance_path, "/name",
            "/properties/name callback must have instance_path '/name', got '{}'",
            e.instance_path
        );
    }

    // The summary callback for /properties/age must have instance_path "/age"
    let age_events = at_schema(&events, "/properties/age");
    assert!(!age_events.is_empty(), "expected /properties/age callback");
    for e in &age_events {
        assert_eq!(
            e.instance_path, "/age",
            "/properties/age callback must have instance_path '/age', got '{}'",
            e.instance_path
        );
    }
}

#[test]
fn regression_properties_absent_property_has_property_instance_path() {
    // When a property is in the schema but absent from the instance, the callback
    // should still use the property path (not the parent object path).
    let schema = json!({"properties": {"missing": {"type": "string"}}});
    let instance = json!({});
    let (events, _) = collect(&schema, &instance);

    let missing_events = at_schema(&events, "/properties/missing");
    assert!(
        !missing_events.is_empty(),
        "expected /properties/missing callback"
    );
    for e in &missing_events {
        assert_eq!(
            e.instance_path, "/missing",
            "absent property callback must have instance_path '/missing', got '{}'",
            e.instance_path
        );
        assert_eq!(e.result, NodeEvaluationResult::Ignored);
    }
}

#[test]
fn regression_properties_nested_instance_path() {
    // Validates that nested object properties carry the full path.
    let schema = json!({"properties": {"user": {"properties": {"name": {"type": "string"}}}}});
    let instance = json!({"user": {"name": 42}});
    let (events, _) = collect(&schema, &instance);

    // The /properties/user/properties/name/type callback should be at instance_path /user/name
    let type_events = at_schema(&events, "/properties/user/properties/name/type");
    assert!(!type_events.is_empty(), "expected nested type callback");
    assert_eq!(
        type_events[0].instance_path, "/user/name",
        "nested property type check must use full instance path"
    );
}

// ---------------------------------------------------------------------------
// Regression: contains + minContains — independent callbacks
// ---------------------------------------------------------------------------

#[test]
fn regression_min_contains_independent_callbacks_one_match() {
    // minContains: 2, but only 1 item matches the contains schema.
    // /contains should report Valid (≥1 match), /minContains should report Invalid (<2).
    let schema = json!({"contains": {"type": "integer"}, "minContains": 2});
    let instance = json!([1, "a", "b"]); // exactly 1 integer
    let (events, result) = collect(&schema, &instance);

    assert!(
        !result,
        "overall should be invalid (only 1 of 2 required matches)"
    );

    let contains_events = at_schema(&events, "/contains");
    assert!(!contains_events.is_empty(), "expected /contains callback");
    assert_eq!(
        contains_events[0].result,
        NodeEvaluationResult::Valid,
        "/contains should be Valid (at least one item matched the contains schema)"
    );

    let min_events = at_schema(&events, "/minContains");
    assert!(!min_events.is_empty(), "expected /minContains callback");
    assert_eq!(
        min_events[0].result,
        NodeEvaluationResult::Invalid,
        "/minContains should be Invalid (only 1 of 2 required)"
    );
}

#[test]
fn regression_min_contains_both_valid_when_enough_matches() {
    // 3 matches, minContains: 2 → both /contains and /minContains Valid.
    let schema = json!({"contains": {"type": "integer"}, "minContains": 2});
    let instance = json!([1, 2, 3, "a"]);
    let (events, result) = collect(&schema, &instance);

    assert!(result);

    let contains_events = at_schema(&events, "/contains");
    assert_eq!(contains_events[0].result, NodeEvaluationResult::Valid);

    let min_events = at_schema(&events, "/minContains");
    assert_eq!(min_events[0].result, NodeEvaluationResult::Valid);
}

#[test]
fn regression_min_contains_both_invalid_when_no_matches() {
    // 0 matches → /contains Invalid (0 < 1), /minContains Invalid (0 < 2).
    let schema = json!({"contains": {"type": "integer"}, "minContains": 2});
    let instance = json!(["a", "b", "c"]);
    let (events, result) = collect(&schema, &instance);

    assert!(!result);

    let contains_events = at_schema(&events, "/contains");
    assert_eq!(
        contains_events[0].result,
        NodeEvaluationResult::Invalid,
        "/contains should be Invalid (no matches at all)"
    );

    let min_events = at_schema(&events, "/minContains");
    assert_eq!(min_events[0].result, NodeEvaluationResult::Invalid);
}

// ---------------------------------------------------------------------------
// Regression: items (array form) — per-item callbacks use item instance path
// ---------------------------------------------------------------------------

#[test]
fn regression_items_array_instance_path_has_index() {
    let schema = json!({"items": [{"type": "integer"}, {"type": "string"}, {"type": "boolean"}]});
    let instance = json!([1, "hello", true]);
    let (events, _) = collect_draft7(&schema, &instance);

    // Per-schema-node summary callback for items[0] must point to instance[0]
    let items0 = at_schema(&events, "/items/0");
    assert!(!items0.is_empty(), "expected /items/0 callback");
    assert_eq!(
        items0[0].instance_path, "/0",
        "/items/0 callback must have instance_path '/0', got '{}'",
        items0[0].instance_path
    );

    let items1 = at_schema(&events, "/items/1");
    assert!(!items1.is_empty(), "expected /items/1 callback");
    assert_eq!(
        items1[0].instance_path, "/1",
        "/items/1 callback must have instance_path '/1', got '{}'",
        items1[0].instance_path
    );

    let items2 = at_schema(&events, "/items/2");
    assert!(!items2.is_empty(), "expected /items/2 callback");
    assert_eq!(
        items2[0].instance_path, "/2",
        "/items/2 callback must have instance_path '/2', got '{}'",
        items2[0].instance_path
    );
}

#[test]
fn regression_items_array_failing_item_has_correct_instance_path() {
    // Failing items must also carry the correct instance path.
    let schema = json!({"items": [{"type": "integer"}, {"type": "string"}]});
    let instance = json!(["bad", 42]); // both items fail their schema
    let (events, result) = collect_draft7(&schema, &instance);

    assert!(!result);

    let items0 = at_schema(&events, "/items/0");
    assert!(!items0.is_empty());
    assert_eq!(items0[0].instance_path, "/0");
    assert_eq!(items0[0].result, NodeEvaluationResult::Invalid);

    let items1 = at_schema(&events, "/items/1");
    assert!(!items1.is_empty());
    assert_eq!(items1[0].instance_path, "/1");
    assert_eq!(items1[0].result, NodeEvaluationResult::Invalid);
}

// ---------------------------------------------------------------------------
// No-short-circuit: all sub-schemas visited even when one fails
// ---------------------------------------------------------------------------

#[test]
fn no_short_circuit_properties_visits_all() {
    let schema = json!({
        "properties": {
            "a": {"type": "integer"},
            "b": {"type": "integer"},
            "c": {"type": "integer"}
        }
    });
    // All three properties fail their type check
    let instance = json!({"a": "x", "b": "y", "c": "z"});
    let (events, result) = collect(&schema, &instance);

    assert!(!result);
    // All three type callbacks must appear despite the first failing
    assert!(at_schema(&events, "/properties/a/type")
        .iter()
        .any(|e| e.result == NodeEvaluationResult::Invalid));
    assert!(at_schema(&events, "/properties/b/type")
        .iter()
        .any(|e| e.result == NodeEvaluationResult::Invalid));
    assert!(at_schema(&events, "/properties/c/type")
        .iter()
        .any(|e| e.result == NodeEvaluationResult::Invalid));
}

#[test]
fn no_short_circuit_all_of_visits_all_branches() {
    let schema = json!({"allOf": [{"minimum": 10}, {"maximum": 5}]});
    let instance = json!(7); // fails both: 7 < 10 and 7 > 5
    let (events, result) = collect(&schema, &instance);

    assert!(!result);
    assert!(
        !at_schema(&events, "/allOf/0/minimum").is_empty(),
        "allOf/0 must be visited"
    );
    assert!(
        !at_schema(&events, "/allOf/1/maximum").is_empty(),
        "allOf/1 must be visited"
    );
}

#[test]
fn no_short_circuit_one_of_visits_all_branches() {
    let schema = json!({"oneOf": [{"type": "string"}, {"type": "integer"}, {"type": "boolean"}]});
    let instance = json!(1.5); // fails all three
    let (events, _) = collect(&schema, &instance);

    assert!(
        !at_schema(&events, "/oneOf/0/type").is_empty(),
        "oneOf/0 must be visited"
    );
    assert!(
        !at_schema(&events, "/oneOf/1/type").is_empty(),
        "oneOf/1 must be visited"
    );
    assert!(
        !at_schema(&events, "/oneOf/2/type").is_empty(),
        "oneOf/2 must be visited"
    );
}

#[test]
fn no_short_circuit_any_of_visits_all_branches() {
    let schema = json!({"anyOf": [{"minimum": 100}, {"maximum": 0}]});
    let instance = json!(50); // fails both branches
    let (events, result) = collect(&schema, &instance);

    assert!(!result);
    assert!(
        !at_schema(&events, "/anyOf/0/minimum").is_empty(),
        "anyOf/0 must be visited"
    );
    assert!(
        !at_schema(&events, "/anyOf/1/maximum").is_empty(),
        "anyOf/1 must be visited"
    );
}

#[test]
fn no_short_circuit_contains_visits_all_items() {
    let schema = json!({"contains": {"type": "integer"}, "minContains": 3});
    let instance = json!([1, "a", 2, "b", 3]); // 3 integers — valid
    let (events, result) = collect(&schema, &instance);

    assert!(result);
    // All 5 items must have been visited by the contains sub-schema
    for idx in 0..5 {
        assert!(
            events.iter().any(|e| e.instance_path == format!("/{idx}")),
            "item {idx} must be visited by contains"
        );
    }
}

// ---------------------------------------------------------------------------
// Suite parity: trace() result must match is_valid() for every test suite case
// ---------------------------------------------------------------------------

mod suite_parity {
    use jsonschema::{Draft, NodeEvaluationResult};
    use serde_json::Value;
    use std::{fs, path::Path};

    struct SuiteCase<'a> {
        draft: &'a str,
        group: &'a str,
        test: &'a str,
        schema: &'a Value,
        instance: &'a Value,
    }

    /// Returns true if the schema (recursively) contains any `$ref`-family key.
    /// Such schemas have opaque tracing for the ref sub-tree, so we skip
    /// the `schema_path` assertion for them (tracked separately by tracecov).
    fn schema_uses_ref(v: &Value) -> bool {
        match v {
            Value::Object(map) => {
                map.contains_key("$ref")
                    || map.contains_key("$dynamicRef")
                    || map.contains_key("$recursiveRef")
                    || map.values().any(schema_uses_ref)
            }
            Value::Array(arr) => arr.iter().any(schema_uses_ref),
            _ => false,
        }
    }

    /// Check one suite case. Returns the count of empty-schema-path events.
    fn check_case(c: &SuiteCase<'_>) -> usize {
        let mut options = jsonschema::options();
        match c.draft {
            "draft4" => {
                options = options.with_draft(Draft::Draft4);
            }
            "draft6" => {
                options = options.with_draft(Draft::Draft6);
            }
            "draft7" => {
                options = options.with_draft(Draft::Draft7);
            }
            _ => {} // draft2019-09 / draft2020-12: auto-detect
        }

        let Ok(validator) = options.build(c.schema) else {
            return 0; // invalid meta-schema — skip
        };

        let is_valid_result = validator.is_valid(c.instance);

        // Count events with empty schema_path, split by whether instance_path is also empty.
        // Validator::trace() always emits exactly one root-summary event where both paths
        // are empty. Any additional both-empty events mean a root-level validator has a
        // broken schema_path. Events with empty schema_path but non-empty instance_path
        // are always validator bugs.
        let mut both_empty_count = 0usize;
        let mut only_schema_empty_count = 0usize;
        let trace_result = validator.trace(c.instance, &mut |ctx| {
            if ctx.result != NodeEvaluationResult::Ignored
                && ctx.schema_location.as_str().is_empty()
            {
                let ip = jsonschema::paths::Location::from(ctx.instance_location);
                if ip.as_str().is_empty() {
                    both_empty_count += 1;
                } else {
                    only_schema_empty_count += 1;
                }
            }
        });
        // Root-summary fires exactly once per Validator::trace() call (validator.rs:455).
        // More than 1 means a root-level validator is missing schema_path().
        let root_level_empty = both_empty_count.saturating_sub(1);
        let empty_path_count = only_schema_empty_count + root_level_empty;

        assert_eq!(
            trace_result, is_valid_result,
            "PARITY FAILURE\n  draft   = {}\n  group   = {}\n  test    = {}\n  schema  = {}\n  instance= {}\n  trace() = {} but is_valid() = {}",
            c.draft, c.group, c.test, c.schema, c.instance, trace_result, is_valid_result,
        );

        empty_path_count
    }

    fn run_draft(draft: &str, suite_dir: &Path) -> usize {
        let draft_path = suite_dir.join(draft);
        if !draft_path.exists() {
            return 0;
        }
        let mut empty_path_count = 0usize;

        let mut files: Vec<_> = fs::read_dir(&draft_path)
            .unwrap_or_else(|_| panic!("Cannot read {}", draft_path.display()))
            .filter_map(std::result::Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort_by_key(std::fs::DirEntry::file_name);

        for entry in files {
            let path = entry.path();
            let content = fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("Cannot read {}", path.display()));
            let groups: Value = serde_json::from_str(&content)
                .unwrap_or_else(|_| panic!("Cannot parse {}", path.display()));
            let groups = groups
                .as_array()
                .unwrap_or_else(|| panic!("suite file must be an array: {}", path.display()));

            for group in groups {
                let group_desc = group["description"].as_str().unwrap_or("?");
                let schema = &group["schema"];
                let tests = group["tests"]
                    .as_array()
                    .unwrap_or_else(|| panic!("tests must be array in {}", path.display()));

                // $ref tracing is opaque (out of scope); skip schema_path assertion
                // for any schema that transitively uses $ref or $dynamicRef.
                let has_ref = schema_uses_ref(schema);

                for test in tests {
                    let test_desc = test["description"].as_str().unwrap_or("?");
                    let instance = &test["data"];

                    let case = SuiteCase {
                        draft,
                        group: group_desc,
                        test: test_desc,
                        schema,
                        instance,
                    };

                    let empty_count = check_case(&case);
                    assert!(
                        has_ref || empty_count == 0,
                        "[schema_path] {} empty-path event(s) in non-ref schema\n  \
                         file  = {}\n  draft = {}\n  group = {}\n  test  = {}",
                        empty_count,
                        path.display(),
                        draft,
                        group_desc,
                        test_desc,
                    );
                    empty_path_count += empty_count;
                }
            }
        }

        empty_path_count
    }

    #[test]
    #[allow(clippy::print_stdout)]
    fn suite_parity_all_drafts() {
        let suite_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/suite/tests");

        let mut total_empty = 0usize;
        for draft in &["draft4", "draft6", "draft7", "draft2019-09", "draft2020-12"] {
            total_empty += run_draft(draft, &suite_dir);
        }

        if total_empty > 0 {
            println!(
                "\n[tracing suite] {total_empty} callback(s) had empty schema_path (result != Ignored).\n\
                 These are invisible in schema coverage reports.\n\
                 Most are from $ref validators that don't yet propagate trace().\n"
            );
        }
    }
}
