#[cfg(not(target_arch = "wasm32"))]
mod bench {
    use codspeed_criterion_compat::{criterion_group, Criterion};
    use jsonschema::{
        canonical::{self, CanonicalSchema, Satisfiability},
        Draft, Registry,
    };
    use serde_json::{json, Map, Value};

    /// Rows of one `tests/canonical-difference` file, as bench slug and row description.
    struct Group {
        name: &'static str,
        file: &'static str,
        rows: &'static [(&'static str, &'static str)],
    }

    const GROUPS: &[Group] = &[
        Group {
            name: "decided",
            file: include_str!("../tests/canonical-difference/baseline.json"),
            rows: &[
                ("string_max_length", "string maxLength added"),
                ("required_added", "object required added"),
                ("enum_widened", "enum value added"),
                ("object_prop_type", "object prop type changed"),
            ],
        },
        Group {
            name: "number",
            file: include_str!("../tests/canonical-difference/numbers.json"),
            rows: &[
                ("integer_number", "integer -> number"),
                ("multipleof_2_4", "multipleOf 2 -> 4"),
                (
                    "number_multipleof_0_1_0_01",
                    "number multipleOf 0.1 -> 0.01",
                ),
                ("anyof_branch_widened", "anyOf branch widened"),
            ],
        },
        Group {
            name: "array",
            file: include_str!("../tests/canonical-difference/arrays.json"),
            rows: &[
                ("array_items_narrowed_inline", "array items narrowed inline"),
                (
                    "array_items_narrowed_via_ref",
                    "array items narrowed via ref",
                ),
                (
                    "array_of_objects_field_became_required",
                    "array of objects: field became required",
                ),
                ("array_items_widened", "array items widened"),
                ("array_items_type_changed", "array items type changed"),
                (
                    "array_items_narrowed_maxitems",
                    "array items narrowed + maxItems",
                ),
                ("nested_array_items_narrowed", "nested array items narrowed"),
                (
                    "object_prop_array_of_ref_narrowed",
                    "object prop array of ref narrowed",
                ),
            ],
        },
        Group {
            name: "object",
            file: include_str!("../tests/canonical-difference/objects.json"),
            rows: &[
                (
                    "object_optional_prop_added_closed",
                    "object optional prop added (closed)",
                ),
                ("object_prop_removed_closed", "object prop removed (closed)"),
                (
                    "additionalproperties_true_false",
                    "additionalProperties true -> false",
                ),
                ("dict_values_narrowed", "dict values narrowed"),
                ("dict_of_arrays_narrowed", "dict of arrays narrowed"),
                ("patternproperties_narrowed", "patternProperties narrowed"),
                ("propertynames_added", "propertyNames added"),
                (
                    "unevaluatedproperties_prop_added",
                    "unevaluatedProperties: prop added",
                ),
            ],
        },
        Group {
            name: "recursive",
            file: include_str!("../tests/canonical-difference/recursive.json"),
            rows: &[
                (
                    "recursive_optional_prop_added_open",
                    "recursive: optional prop added (open)",
                ),
                ("recursive_required_added", "recursive: required added"),
                (
                    "recursive_via_property_leaf_narrowed",
                    "recursive via property: leaf narrowed",
                ),
                (
                    "recursive_nullable_link_leaf_narrowed",
                    "recursive nullable link: leaf narrowed",
                ),
                (
                    "mutual_recursion_leaf_narrowed",
                    "mutual recursion: leaf narrowed",
                ),
            ],
        },
    ];

    fn canonical(schema: &Value) -> CanonicalSchema {
        canonical::options()
            .with_draft(Draft::Draft202012)
            .should_validate_formats(true)
            .canonicalize(schema)
            .expect("valid schema")
    }

    /// Satisfiability of `old` minus `new` and of `new` minus `old`.
    fn differences(old: &CanonicalSchema, new: &CanonicalSchema) -> [Satisfiability; 2] {
        [
            old.subtract(new).expect("subtracts").satisfiability(),
            new.subtract(old).expect("subtracts").satisfiability(),
        ]
    }

    /// Whether each `new` response accepts a value its `old` one rejects.
    #[inline(never)]
    fn response_differences(
        new: &[CanonicalSchema],
        old: &[CanonicalSchema],
    ) -> Vec<Satisfiability> {
        new.iter()
            .zip(old)
            .map(|(new, old)| new.subtract(old).expect("subtracts").satisfiability())
            .collect()
    }

    /// `models` object schemas, each holding a string, an integer, a list of the model before it and
    /// a map of the one before that, rooted at the last, which reaches them all.
    fn api_document(models: usize, narrow: bool) -> Value {
        let mut defs = Map::with_capacity(models);
        for i in 0..models {
            let mut name = json!({"type": "string"});
            if narrow {
                name["maxLength"] = json!(3);
            }
            let mut properties = Map::new();
            properties.insert("name".into(), name);
            properties.insert("count".into(), json!({"type": "integer"}));
            if i >= 1 {
                properties.insert(
                    "previous".into(),
                    json!({"type": "array", "items": {"$ref": format!("#/$defs/M{}", i - 1)}}),
                );
            }
            if i >= 2 {
                properties.insert(
                    "older".into(),
                    json!({
                        "type": "object",
                        "additionalProperties": {"$ref": format!("#/$defs/M{}", i - 2)},
                    }),
                );
            }
            defs.insert(
                format!("M{i}"),
                json!({"type": "object", "properties": properties}),
            );
        }
        json!({"$ref": format!("#/$defs/M{}", models - 1), "$defs": defs})
    }

    /// `targets` objects, each holding an integer and a property that may be any one of them, nested
    /// two properties deep. `narrow` bounds the last one's integer.
    fn union_document(targets: usize, narrow: bool) -> Value {
        let any: Vec<Value> = (0..targets)
            .map(|i| json!({"$ref": format!("#/$defs/A{i}")}))
            .collect();
        let mut defs = Map::with_capacity(targets);
        for i in 0..targets {
            let leaf = recursive_leaf(narrow && i == targets - 1);
            let mut body =
                json!({"type": "object", "properties": {"p": {"anyOf": any}, "v": leaf}});
            for _ in 0..2 {
                body = json!({"type": "object", "properties": {"q": body}});
            }
            defs.insert(format!("A{i}"), body);
        }
        json!({"$ref": "#/$defs/A0", "$defs": defs})
    }

    /// `targets` objects, each holding an integer and one property per target naming it. `narrow`
    /// bounds the last one's integer.
    fn dense_document(targets: usize, narrow: bool) -> Value {
        let mut defs = Map::with_capacity(targets);
        for i in 0..targets {
            let mut properties = Map::new();
            for j in 0..targets {
                properties.insert(format!("p{j}"), json!({"$ref": format!("#/$defs/A{j}")}));
            }
            properties.insert("v".into(), recursive_leaf(narrow && i == targets - 1));
            defs.insert(
                format!("A{i}"),
                json!({"type": "object", "properties": properties}),
            );
        }
        json!({"$ref": "#/$defs/A0", "$defs": defs})
    }

    /// An API description of `operations` operations, each answering a user and two errors whose
    /// details are an array of objects. `narrow` bounds every string.
    fn service_document(operations: usize, narrow: bool) -> Value {
        let string = if narrow {
            json!({"type": "string", "maxLength": 3})
        } else {
            json!({"type": "string"})
        };
        let schemas = json!({
            "User": {
                "type": "object",
                "properties": {
                    "name": string,
                    "email": string,
                    "tags": {"type": "array", "items": string}
                },
                "required": ["name"]
            },
            "ErrorDetail": {
                "type": "object",
                "properties": {
                    "msg": string,
                    "type": string,
                    "loc": {"type": "array", "items": {"anyOf": [{"type": "integer"}, string]}}
                },
                "required": ["msg"]
            },
            "ErrorModel": {
                "type": "object",
                "properties": {
                    "detail": {
                        "type": "array",
                        "items": {"$ref": "#/components/schemas/ErrorDetail"}
                    }
                },
                "required": ["detail"]
            }
        });
        let answer =
            |model: &str| json!({"schema": {"$ref": format!("#/components/schemas/{model}")}});
        let mut paths = Map::with_capacity(operations);
        for i in 0..operations {
            paths.insert(
                format!("/op{i}"),
                json!({"get": {"responses": {
                    "200": answer("User"),
                    "400": answer("ErrorModel"),
                    "422": answer("ErrorModel")
                }}}),
            );
        }
        json!({"openapi": "3.1.0", "paths": paths, "components": {"schemas": schemas}})
    }

    /// Every response schema of `service_document(operations, ..)`, canonicalized from the
    /// document registered as `uri`.
    fn responses(registry: &Registry, uri: &str, operations: usize) -> Vec<CanonicalSchema> {
        (0..operations)
            .flat_map(|i| {
                ["200", "400", "422"].map(|status| {
                    let pointer = format!("{uri}#/paths/~1op{i}/get/responses/{status}/schema");
                    canonical::options()
                        .with_draft(Draft::Draft202012)
                        .should_validate_formats(true)
                        .with_registry(registry)
                        .canonicalize(&json!({"$ref": pointer}))
                        .expect("valid schema")
                })
            })
            .collect()
    }

    fn recursive_leaf(narrow: bool) -> Value {
        if narrow {
            json!({"type": "integer", "minimum": 0})
        } else {
            json!({"type": "integer"})
        }
    }

    /// Each row of `group` as its bench slug and the suite case of its description.
    fn cases(group: &Group) -> Vec<(&'static str, Value)> {
        let cases: Vec<Value> = serde_json::from_str(group.file).expect("valid suite file");
        group
            .rows
            .iter()
            .map(|(slug, description)| {
                let case = cases
                    .iter()
                    .find(|case| case["description"] == *description)
                    .unwrap_or_else(|| panic!("{}: no row `{description}`", group.name));
                assert!(case.get("draft").is_none(), "`{description}` sets a draft");
                (*slug, case.clone())
            })
            .collect()
    }

    // Built into the `canonical_difference` test, so a renamed suite row fails `cargo test`
    // rather than only the bench run.
    #[test]
    fn every_row_is_a_suite_case() {
        for group in GROUPS {
            cases(group);
        }
    }

    pub(crate) fn bench_difference(c: &mut Criterion) {
        for group in GROUPS {
            for (slug, case) in cases(group) {
                let old = canonical(&case["old"]);
                let new = canonical(&case["new"]);
                c.bench_function(&format!("difference/{}/{slug}", group.name), |b| {
                    b.iter(|| differences(&old, &new));
                });
            }
        }
        for (slug, old, new) in [
            (
                "union_k8",
                union_document(8, false),
                union_document(8, true),
            ),
            (
                "dense_k8",
                dense_document(8, false),
                dense_document(8, true),
            ),
        ] {
            let old = canonical(&old);
            let new = canonical(&new);
            c.bench_function(&format!("difference/recursive/{slug}"), |b| {
                b.iter(|| differences(&old, &new));
            });
        }
        let wide = canonical(&api_document(200, false));
        let twin = canonical(&api_document(200, false));
        let narrow = canonical(&api_document(200, true));
        for (name, new) in [("identical", &twin), ("narrowed", &narrow)] {
            c.bench_function(&format!("difference/api/{name}"), |b| {
                b.iter(|| differences(&wide, new));
            });
        }
        // Both versions of one API in a registry, every response of the new one widened.
        let operations = 16;
        let registry = Registry::new()
            .draft(Draft::Draft202012)
            .add("urn:old", service_document(operations, true))
            .expect("valid document")
            .add("urn:new", service_document(operations, false))
            .expect("valid document")
            .prepare()
            .expect("valid registry");
        let old = responses(&registry, "urn:old", operations);
        let new = responses(&registry, "urn:new", operations);
        c.bench_function("difference/api/responses_widened", |b| {
            b.iter(|| response_differences(&new, &old));
        });
    }

    criterion_group!(benches, bench_difference);
}

#[cfg(not(target_arch = "wasm32"))]
codspeed_criterion_compat::criterion_main!(bench::benches);

#[cfg(target_arch = "wasm32")]
fn main() {}
