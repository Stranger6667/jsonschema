#[cfg(not(target_arch = "wasm32"))]
mod bench {
    use std::hint::black_box;

    use benchmark::Benchmark;
    use codspeed_criterion_compat::{criterion_group, BenchmarkId, Criterion};
    use serde_json::{json, Value};

    #[jsonschema::validator(path = "../benchmark/data/fast_schema.json")]
    struct FastSchemaValidator;
    #[jsonschema::validator(path = "../benchmark/data/openapi.json")]
    struct OpenAPIValidator;
    #[jsonschema::validator(path = "../benchmark/data/swagger.json")]
    struct SwaggerValidator;
    #[jsonschema::validator(path = "../benchmark/data/geojson.json")]
    struct GeoJSONValidator;
    #[jsonschema::validator(path = "../benchmark/data/citm_catalog_schema.json")]
    struct CITMValidator;
    #[jsonschema::validator(path = "../benchmark/data/fhir.schema.json")]
    struct FHIRValidator;
    #[jsonschema::validator(path = "../benchmark/data/recursive_schema.json")]
    struct RecursiveValidator;

    #[jsonschema::validator(schema = r#"{"items": {"type": "integer"}}"#)]
    struct ItemsValidator;
    #[jsonschema::validator(schema = r#"{"propertyNames": {"maxLength": 1}}"#)]
    struct PropertyNamesValidator;
    #[jsonschema::validator(
        schema = r##"{"definitions": {"node": {"oneOf": [{"required": ["a"], "properties": {"a": {"const": 1}, "c": {"$ref": "#/definitions/node"}}}, {"properties": {"b": {"const": 1}, "c": {"$ref": "#/definitions/node"}}}, {"type": "string"}]}}, "$ref": "#/definitions/node"}"##,
        draft = Draft7
    )]
    struct RefCycleInBranchesValidator;

    type IsValid = fn(&Value) -> bool;
    type Validate = for<'a> fn(&'a Value) -> Result<(), jsonschema::ValidationError<'a>>;
    type IterErrors = for<'a> fn(&'a Value) -> jsonschema::ErrorIterator<'a>;
    type Evaluate = fn(&Value) -> jsonschema::Evaluation;

    fn bench(c: &mut Criterion) {
        for benchmark in Benchmark::iter() {
            benchmark.run(&mut |name, _schema, instances| {
                let (is_valid, validate, iter_errors, evaluate): (
                    IsValid,
                    Validate,
                    IterErrors,
                    Evaluate,
                ) = match name {
                    "Fast" => (
                        FastSchemaValidator::is_valid,
                        FastSchemaValidator::validate,
                        FastSchemaValidator::iter_errors,
                        FastSchemaValidator::evaluate,
                    ),
                    "Open API" => (
                        OpenAPIValidator::is_valid,
                        OpenAPIValidator::validate,
                        OpenAPIValidator::iter_errors,
                        OpenAPIValidator::evaluate,
                    ),
                    "Swagger" => (
                        SwaggerValidator::is_valid,
                        SwaggerValidator::validate,
                        SwaggerValidator::iter_errors,
                        SwaggerValidator::evaluate,
                    ),
                    "GeoJSON" => (
                        GeoJSONValidator::is_valid,
                        GeoJSONValidator::validate,
                        GeoJSONValidator::iter_errors,
                        GeoJSONValidator::evaluate,
                    ),
                    "CITM" => (
                        CITMValidator::is_valid,
                        CITMValidator::validate,
                        CITMValidator::iter_errors,
                        CITMValidator::evaluate,
                    ),
                    "FHIR" => (
                        FHIRValidator::is_valid,
                        FHIRValidator::validate,
                        FHIRValidator::iter_errors,
                        FHIRValidator::evaluate,
                    ),
                    "Recursive" => (
                        RecursiveValidator::is_valid,
                        RecursiveValidator::validate,
                        RecursiveValidator::iter_errors,
                        RecursiveValidator::evaluate,
                    ),
                    _ => return,
                };
                for instance in instances {
                    c.bench_with_input(
                        BenchmarkId::new(format!("codegen/{name}/is_valid"), &instance.name),
                        &instance.data,
                        |b, data| b.iter(|| black_box(is_valid(data))),
                    );
                    c.bench_with_input(
                        BenchmarkId::new(format!("codegen/{name}/validate"), &instance.name),
                        &instance.data,
                        |b, data| b.iter(|| black_box(validate(data).is_ok())),
                    );
                    c.bench_with_input(
                        BenchmarkId::new(format!("codegen/{name}/iter_errors"), &instance.name),
                        &instance.data,
                        |b, data| b.iter(|| black_box(iter_errors(data).count())),
                    );
                    c.bench_with_input(
                        BenchmarkId::new(format!("codegen/{name}/evaluate"), &instance.name),
                        &instance.data,
                        |b, data| b.iter_with_large_drop(|| black_box(evaluate(data))),
                    );
                }
            });
        }
    }

    // An instance that sends every level below it through the first two `oneOf` branches.
    fn ref_cycle_in_branches(depth: usize) -> Value {
        (0..depth).fold(json!({}), |child, _| json!({"a": 1, "c": child}))
    }

    fn bench_evaluate(
        c: &mut Criterion,
        name: &str,
        instance_name: &str,
        runtime: &jsonschema::Validator,
        generated: fn(&Value) -> jsonschema::Evaluation,
        instance: &Value,
    ) {
        c.bench_with_input(
            BenchmarkId::new(format!("codegen/{name}/evaluate"), instance_name),
            instance,
            |b, instance| b.iter_with_large_drop(|| black_box(generated(instance))),
        );
        c.bench_with_input(
            BenchmarkId::new("evaluate", format!("{name}/{instance_name}")),
            instance,
            |b, instance| b.iter_with_large_drop(|| black_box(runtime.evaluate(instance))),
        );
    }

    // Instances failing one keyword many times, and `$ref` cycles reaching an instance twice.
    fn bench_synthetic(c: &mut Criterion) {
        let items = jsonschema::validator_for(&json!({"items": {"type": "integer"}}))
            .expect("Valid schema");
        let strings = Value::Array((0..1000).map(|i| json!(format!("s{i}"))).collect());
        bench_evaluate(
            c,
            "Items",
            "Strings",
            &items,
            ItemsValidator::evaluate,
            &strings,
        );
        c.bench_with_input(
            BenchmarkId::new("codegen/Items/iter_errors", "Strings"),
            &strings,
            |b, instance| b.iter(|| black_box(ItemsValidator::iter_errors(instance).count())),
        );
        c.bench_with_input(
            BenchmarkId::new("iter_errors", "Items/Strings"),
            &strings,
            |b, instance| b.iter(|| black_box(items.iter_errors(instance).count())),
        );

        let names = jsonschema::validator_for(&json!({"propertyNames": {"maxLength": 1}}))
            .expect("Valid schema");
        let long_names = Value::Object((0..1000).map(|i| (format!("k{i}"), json!(i))).collect());
        bench_evaluate(
            c,
            "Property names",
            "Long names",
            &names,
            PropertyNamesValidator::evaluate,
            &long_names,
        );

        let cycle = json!({"definitions": {"node": {"oneOf": [{"required": ["a"], "properties": {"a": {"const": 1}, "c": {"$ref": "#/definitions/node"}}}, {"properties": {"b": {"const": 1}, "c": {"$ref": "#/definitions/node"}}}, {"type": "string"}]}}, "$ref": "#/definitions/node"});
        let runtime = jsonschema::draft7::new(&cycle).expect("Valid schema");
        let deep = ref_cycle_in_branches(IS_VALID_DEPTH);
        let name = format!("Depth {IS_VALID_DEPTH}");
        c.bench_with_input(
            BenchmarkId::new("codegen/Ref cycle in branches/is_valid", &name),
            &deep,
            |b, instance| b.iter(|| black_box(RefCycleInBranchesValidator::is_valid(instance))),
        );
        c.bench_with_input(
            BenchmarkId::new("is_valid", format!("Ref cycle in branches/{name}")),
            &deep,
            |b, instance| b.iter(|| black_box(runtime.is_valid(instance))),
        );
        let shallow = ref_cycle_in_branches(EVALUATE_DEPTH);
        bench_evaluate(
            c,
            "Ref cycle in branches",
            &format!("Depth {EVALUATE_DEPTH}"),
            &runtime,
            RefCycleInBranchesValidator::evaluate,
            &shallow,
        );
    }

    const IS_VALID_DEPTH: usize = 14;
    const EVALUATE_DEPTH: usize = 6;

    criterion_group!(codegen, bench, bench_synthetic);
}

#[cfg(not(target_arch = "wasm32"))]
codspeed_criterion_compat::criterion_main!(bench::codegen);

#[cfg(target_arch = "wasm32")]
fn main() {}
