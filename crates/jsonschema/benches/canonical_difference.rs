#[cfg(not(target_arch = "wasm32"))]
mod bench {
    use codspeed_criterion_compat::{criterion_group, Criterion};
    use jsonschema::{
        canonical::{self, CanonicalSchema, Satisfiability},
        Draft,
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

    /// The difference both ways, as a schema comparison reads it.
    fn differences(old: &CanonicalSchema, new: &CanonicalSchema) -> [Satisfiability; 2] {
        [
            old.subtract(new).expect("subtracts").satisfiability(),
            new.subtract(old).expect("subtracts").satisfiability(),
        ]
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

    pub(crate) fn bench_difference(c: &mut Criterion) {
        for group in GROUPS {
            let cases: Vec<Value> = serde_json::from_str(group.file).expect("valid suite file");
            for (slug, description) in group.rows {
                let case = cases
                    .iter()
                    .find(|case| case["description"] == *description)
                    .unwrap_or_else(|| panic!("no row `{description}`"));
                assert!(case.get("draft").is_none(), "`{description}` sets a draft");
                let old = canonical(&case["old"]);
                let new = canonical(&case["new"]);
                c.bench_function(&format!("difference/{}/{slug}", group.name), |b| {
                    b.iter(|| differences(&old, &new));
                });
            }
        }
        let wide = canonical(&api_document(200, false));
        let twin = canonical(&api_document(200, false));
        let narrow = canonical(&api_document(200, true));
        for (name, new) in [("identical", &twin), ("narrowed", &narrow)] {
            c.bench_function(&format!("difference/api/{name}"), |b| {
                b.iter(|| differences(&wide, new));
            });
        }
    }

    criterion_group!(benches, bench_difference);
}

#[cfg(not(target_arch = "wasm32"))]
codspeed_criterion_compat::criterion_main!(bench::benches);

#[cfg(target_arch = "wasm32")]
fn main() {}
