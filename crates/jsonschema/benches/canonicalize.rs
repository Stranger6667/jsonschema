#[cfg(not(target_arch = "wasm32"))]
mod bench {
    use benchmark::{
        read_json, FHIR_SCHEMA, GEOJSON, KUBERNETES, OPEN_API, RECURSIVE_SCHEMA, SWAGGER,
    };
    use codspeed_criterion_compat::{criterion_group, Criterion};
    use serde_json::{json, Map, Value};

    fn kestra_like(n: usize) -> Value {
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

    // The fixture is a Swagger 2.0 API document, not a schema: as a 2020-12 schema it has zero
    // constraint keywords and canonicalizes to `true`. Root it at real API objects so the
    // `definitions` closure is actually parsed and canonicalized.
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

        let kestra_80 = kestra_like(80);
        let kestra_160 = kestra_like(160);
        let kestra_320 = kestra_like(320);

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
            ("object_with_properties", &object_with_properties),
            ("kestra_like/80", &kestra_80),
            ("kestra_like/160", &kestra_160),
            ("kestra_like/320", &kestra_320),
            ("open_api", &open_api),
            ("swagger", &swagger),
            ("geojson", &geojson),
            ("recursive", &recursive),
            ("kubernetes", &kubernetes),
            ("fhir", &fhir),
        ];

        for (name, schema) in cases {
            c.bench_function(&format!("canonicalize/{name}"), |b| {
                b.iter(|| jsonschema::canonicalize(schema).expect("valid schema"));
            });
        }
    }

    // Symbolic mode (`inline_budget = 0`): refs stay symbolic and each definition body is
    // canonicalized individually - a different profile from full inlining.
    pub(crate) fn bench_canonicalize_symbolic(c: &mut Criterion) {
        let kestra_320 = kestra_like(320);
        let open_api = read_json(OPEN_API);
        let kubernetes = kubernetes_api_schema();
        for (name, schema) in [
            ("kestra_like/320", &kestra_320),
            ("open_api", &open_api),
            ("kubernetes", &kubernetes),
        ] {
            c.bench_function(&format!("canonicalize_symbolic/{name}"), |b| {
                b.iter(|| {
                    jsonschema::canonical::options()
                        .with_inline_budget(0)
                        .canonicalize(schema)
                        .expect("valid schema")
                });
            });
        }
    }

    fn canonical(schema: &Value) -> jsonschema::canonical::CanonicalSchema {
        jsonschema::canonicalize(schema).expect("valid schema")
    }

    fn object_with_properties_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "integer", "minimum": 0},
                "name": {"type": "string", "minLength": 1, "maxLength": 100},
                "tags": {"type": "array", "items": {"type": "string"}},
                "active": {"type": "boolean"},
            },
            "required": ["id", "name"],
            "additionalProperties": false,
        })
    }

    pub(crate) fn bench_negate(c: &mut Criterion) {
        let object_with_properties = canonical(&object_with_properties_schema());
        let wide_anyof = canonical(&json!({"anyOf": (0..40_usize)
            .map(|i| json!({"type": "integer", "minimum": i * 20, "maximum": i * 20 + 10}))
            .collect::<Vec<_>>()}));
        let geojson = canonical(&read_json(GEOJSON));
        let cases = [
            ("object_with_properties", &object_with_properties),
            ("wide_anyof", &wide_anyof),
            ("geojson", &geojson),
        ];
        for (name, schema) in cases {
            c.bench_function(&format!("negate/{name}"), |b| b.iter(|| schema.negate()));
        }
    }

    pub(crate) fn bench_intersect(c: &mut Criterion) {
        let object_with_properties = canonical(&object_with_properties_schema());
        let object_relaxed = canonical(&json!({
            "type": "object",
            "properties": {
                "id": {"type": "integer", "maximum": 1_000_000},
                "name": {"type": "string", "maxLength": 200},
            },
            "minProperties": 1,
        }));
        // Two separately canonicalized copies: no shared nodes, so the merge does full structural work.
        let geojson_left = canonical(&read_json(GEOJSON));
        let geojson_right = canonical(&read_json(GEOJSON));
        c.bench_function("intersect/objects", |b| {
            b.iter(|| object_with_properties.intersect(&object_relaxed));
        });
        c.bench_function("intersect/geojson_with_copy", |b| {
            b.iter(|| geojson_left.intersect(&geojson_right));
        });
    }

    // `X - X` over separately canonicalized copies: drives negate + intersect + the coverage prover
    // (memoized covers, partition fuel) to an empty residual.
    pub(crate) fn bench_subtract(c: &mut Criterion) {
        let object_left = canonical(&object_with_properties_schema());
        let object_right = canonical(&object_with_properties_schema());
        let partition_schema = json!({"not": {"allOf": [
            {"not": {"type": "integer", "minimum": 0, "maximum": 0}},
            {"anyOf": [
                {"type": "number", "multipleOf": 4},
                {"type": "integer", "minimum": 2, "maximum": 4}
            ]}
        ]}});
        let partition_left = canonical(&partition_schema);
        let partition_right = canonical(&partition_schema);
        c.bench_function("subtract/object_self", |b| {
            b.iter(|| object_left.subtract(&object_right));
        });
        c.bench_function("subtract/numeric_partition_self", |b| {
            b.iter(|| partition_left.subtract(&partition_right));
        });
    }

    pub(crate) fn bench_is_subschema_of(c: &mut Criterion) {
        let narrow = canonical(&object_with_properties_schema());
        let broad = canonical(&json!({
            "type": "object",
            "properties": {"id": {"type": "integer"}, "name": {"type": "string"}},
        }));
        let geojson_left = canonical(&read_json(GEOJSON));
        let geojson_right = canonical(&read_json(GEOJSON));
        c.bench_function("is_subschema_of/object_narrow_vs_broad", |b| {
            b.iter(|| narrow.is_subschema_of(&broad));
        });
        c.bench_function("is_subschema_of/geojson_vs_copy", |b| {
            b.iter(|| geojson_left.is_subschema_of(&geojson_right));
        });
    }

    pub(crate) fn bench_emit(c: &mut Criterion) {
        let open_api = canonical(&read_json(OPEN_API));
        let kestra_160 = canonical(&kestra_like(160));
        let kubernetes = canonical(&kubernetes_api_schema());
        let fhir = canonical(&read_json(FHIR_SCHEMA));
        let recursive = canonical(&read_json(RECURSIVE_SCHEMA));
        let cases = [
            ("open_api", &open_api),
            ("kestra_like/160", &kestra_160),
            ("kubernetes", &kubernetes),
            ("fhir", &fhir),
            ("recursive", &recursive),
        ];
        for (name, schema) in cases {
            c.bench_function(&format!("emit/{name}"), |b| {
                b.iter(|| schema.to_json_schema());
            });
        }
    }

    criterion_group!(
        benches,
        bench_canonicalize,
        bench_canonicalize_symbolic,
        bench_negate,
        bench_intersect,
        bench_subtract,
        bench_is_subschema_of,
        bench_emit
    );
}

#[cfg(not(target_arch = "wasm32"))]
codspeed_criterion_compat::criterion_main!(bench::benches);

#[cfg(target_arch = "wasm32")]
fn main() {}
