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

    fn bench(c: &mut Criterion) {
        for benchmark in Benchmark::iter() {
            benchmark.run(&mut |name, _schema, instances| {
                let (is_valid, validate): (IsValid, Validate) = match name {
                    "Fast" => (FastSchemaValidator::is_valid, FastSchemaValidator::validate),
                    "Open API" => (OpenAPIValidator::is_valid, OpenAPIValidator::validate),
                    "Swagger" => (SwaggerValidator::is_valid, SwaggerValidator::validate),
                    "GeoJSON" => (GeoJSONValidator::is_valid, GeoJSONValidator::validate),
                    "CITM" => (CITMValidator::is_valid, CITMValidator::validate),
                    "FHIR" => (FHIRValidator::is_valid, FHIRValidator::validate),
                    "Recursive" => (RecursiveValidator::is_valid, RecursiveValidator::validate),
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
