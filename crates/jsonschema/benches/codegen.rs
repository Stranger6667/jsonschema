#[cfg(not(target_arch = "wasm32"))]
mod bench {
    use std::hint::black_box;

    use benchmark::Benchmark;
    use codspeed_criterion_compat::{criterion_group, BenchmarkId, Criterion};
    use serde_json::Value;

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

    criterion_group!(codegen, bench);
}

#[cfg(not(target_arch = "wasm32"))]
codspeed_criterion_compat::criterion_main!(bench::codegen);

#[cfg(target_arch = "wasm32")]
fn main() {}
