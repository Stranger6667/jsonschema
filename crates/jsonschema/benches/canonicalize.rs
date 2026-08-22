#[cfg(not(target_arch = "wasm32"))]
mod bench {
    use benchmark::{
        read_json, FHIR_SCHEMA, GEOJSON, KUBERNETES, OPEN_API, RECURSIVE_SCHEMA, SWAGGER,
    };
    use codspeed_criterion_compat::{criterion_group, Criterion};
    use serde_json::{json, Map, Value};

    // Each `$def` references the two before it, so canonicalizing the tail resolves a dense chain.
    fn chained_ref_defs(n: usize) -> Value {
        let mut defs = Map::new();
        defs.insert(
            "base".into(),
            json!({"type": "object", "properties": {"id": {"type": "string"}}}),
        );
        for i in 0..n {
            let a = if i == 0 {
                "base".to_string()
            } else {
                format!("d{}", i - 1)
            };
            let b = if i < 2 {
                "base".to_string()
            } else {
                format!("d{}", i - 2)
            };
            defs.insert(
                format!("d{i}"),
                json!({"allOf": [
                    {"$ref": format!("#/$defs/{a}")},
                    {"$ref": format!("#/$defs/{b}")},
                    {"type": "object", "properties": {
                        format!("p{i}"): {"type": "integer", "minimum": i},
                        format!("q{i}"): {"type": "string"},
                    }},
                ]}),
            );
        }
        json!({"$ref": format!("#/$defs/d{}", n - 1), "$defs": defs})
    }

    // The fixture is a Swagger document, not a schema; root it at real API objects so its
    // `definitions` closure is actually canonicalized instead of collapsing to `true`.
    fn kubernetes_api_schema() -> Value {
        let mut document = read_json(KUBERNETES);
        json!({
            "anyOf": [
                {"$ref": "#/definitions/io.k8s.api.core.v1.Pod"},
                {"$ref": "#/definitions/io.k8s.api.apps.v1.Deployment"},
                {"$ref": "#/definitions/io.k8s.api.core.v1.Service"},
            ],
            "definitions": document["definitions"].take(),
        })
    }

    pub(crate) fn bench_canonicalize(c: &mut Criterion) {
        let wide_anyof_in_allof = json!({"allOf": [
            {"anyOf": (0..40_usize)
                .map(|i| json!({"type": "integer", "minimum": i, "maximum": i + 10}))
                .collect::<Vec<_>>()},
            {"type": "integer", "minimum": 5},
        ]});

        let deep_allof_chain = {
            let mut s = json!({"type": "integer", "minimum": 0});
            for _ in 0..30 {
                s = json!({"allOf": [s, {"type": "integer", "maximum": 1000}]});
            }
            s
        };

        let many_small_allofs_inside_object = {
            let mut props = Map::with_capacity(50);
            for i in 0..50_usize {
                props.insert(
                    format!("p{i}"),
                    json!({"allOf": [
                        {"type": "integer", "minimum": 0},
                        {"type": "integer", "maximum": 100},
                    ]}),
                );
            }
            json!({"type": "object", "properties": props})
        };

        // Bounds far from zero on a fractional grid: each end is snapped onto a multiple, which is
        // where the exact rational behind a spelling earns its keep.
        let wide_numeric_grid = json!({
            "type": "number",
            "multipleOf": 0.5,
            "minimum": -1e308,
            "maximum": 1e308,
        });

        let object_with_properties = json!({
            "type": "object",
            "properties": {
                "id": {"type": "integer", "minimum": 0},
                "name": {"type": "string", "minLength": 1, "maxLength": 100},
                "tags": {"type": "array", "items": {"type": "string"}},
                "active": {"type": "boolean"},
            },
            "required": ["id", "name"],
            "additionalProperties": false,
        });

        let chained_refs = chained_ref_defs(160);

        // Each `if` complements into a union, and the conjunction takes their product, so the same
        // pair of branches is intersected over and over.
        let negated_branches_in_allof = json!({"allOf": (0..3_usize)
            .map(|i| json!({
                "if": {"type": "object", "properties": {"kind": {
                    "type": "array",
                    "items": {"type": "string"},
                    "anyOf": [
                        {"contains": {"const": format!("c{i}")}},
                        {"contains": {"const": format!("prefix:c{i}")}},
                    ],
                }}},
                "then": {"type": "object", "properties": {
                    "payload": {"type": "string", "minLength": i + 1},
                }},
            }))
            .collect::<Vec<_>>()});

        // `not` over an object turns each property into its own branch, and the union then reads
        // every branch against every other.
        let negated_wide_object = {
            let mut props = Map::with_capacity(128);
            for i in 0..128_usize {
                props.insert(
                    format!("p{i}"),
                    json!({"type": "string", "minLength": i % 3}),
                );
            }
            json!({"not": {"type": "object", "properties": props}})
        };

        // A closed map under `not`: every branch it turns into is read against every other.
        let negated_closed_object = {
            let mut props = Map::with_capacity(64);
            for i in 0..64_usize {
                props.insert(format!("p{i}"), json!({"type": "string"}));
            }
            json!({"not": {
                "type": "object",
                "additionalProperties": false,
                "properties": props,
            }})
        };

        // A closed map whose entries are unions, the shape a linter configuration takes: `not`
        // over it crosses two wide unions before the pool is minimized.
        let negated_closed_object_of_unions = {
            let mut rules = Map::with_capacity(50);
            for i in 0..50_usize {
                rules.insert(
                    format!("R{i}"),
                    json!({"anyOf": [
                        {"enum": ["off", "warning"]},
                        {"type": "object", "additionalProperties": false, "properties": {
                            "severity": {"type": "string"},
                        }},
                    ]}),
                );
            }
            json!({"not": {
                "type": "object",
                "additionalProperties": false,
                "properties": {"rules": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": rules,
                }},
            }})
        };

        let open_api = read_json(OPEN_API);
        let swagger = read_json(SWAGGER);
        let geojson = read_json(GEOJSON);
        let recursive = read_json(RECURSIVE_SCHEMA);
        let kubernetes = kubernetes_api_schema();
        let fhir = read_json(FHIR_SCHEMA);

        let cases: &[(&str, &Value)] = &[
            ("wide_anyof_in_allof", &wide_anyof_in_allof),
            ("deep_allof_chain", &deep_allof_chain),
            (
                "many_small_allofs_inside_object",
                &many_small_allofs_inside_object,
            ),
            ("negated_branches_in_allof", &negated_branches_in_allof),
            ("negated_wide_object", &negated_wide_object),
            ("negated_closed_object", &negated_closed_object),
            (
                "negated_closed_object_of_unions",
                &negated_closed_object_of_unions,
            ),
            ("wide_numeric_grid", &wide_numeric_grid),
            ("object_with_properties", &object_with_properties),
            ("chained_refs", &chained_refs),
            ("open_api", &open_api),
            ("swagger", &swagger),
            ("geojson", &geojson),
            ("recursive", &recursive),
            ("kubernetes", &kubernetes),
            ("fhir", &fhir),
        ];

        for (name, schema) in cases {
            c.bench_function(&format!("canonicalize/{name}"), |b| {
                b.iter_with_large_drop(|| jsonschema::canonicalize(schema).expect("valid schema"));
            });
        }
    }

    fn canonical(schema: &Value) -> jsonschema::canonical::CanonicalSchema {
        jsonschema::canonicalize(schema).expect("valid schema")
    }

    pub(crate) fn bench_emit(c: &mut Criterion) {
        let open_api = canonical(&read_json(OPEN_API));
        let chained_refs = canonical(&chained_ref_defs(160));
        let kubernetes = canonical(&kubernetes_api_schema());
        let fhir = canonical(&read_json(FHIR_SCHEMA));
        let recursive = canonical(&read_json(RECURSIVE_SCHEMA));
        let cases = [
            ("open_api", &open_api),
            ("chained_refs", &chained_refs),
            ("kubernetes", &kubernetes),
            ("fhir", &fhir),
            ("recursive", &recursive),
        ];
        for (name, schema) in cases {
            c.bench_function(&format!("emit/{name}"), |b| {
                b.iter_with_large_drop(|| schema.to_json_schema());
            });
        }
    }

    criterion_group!(benches, bench_canonicalize, bench_emit);
}

#[cfg(not(target_arch = "wasm32"))]
codspeed_criterion_compat::criterion_main!(bench::benches);

#[cfg(target_arch = "wasm32")]
fn main() {}
