#[cfg(not(target_arch = "wasm32"))]
mod bench {
    use std::hint::black_box;

    pub(crate) use benchmark::Benchmark;
    pub(crate) use codspeed_criterion_compat::{criterion_group, BenchmarkId, Criterion};
    pub(crate) use serde_json::Value;

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

    pub(crate) fn bench_build(c: &mut Criterion, name: &str, schema: &Value) {
        c.bench_with_input(BenchmarkId::new("build", name), schema, |b, schema| {
            b.iter_with_large_drop(|| jsonschema::validator_for(schema).expect("Valid schema"));
        });
    }

    pub(crate) fn bench_is_valid(c: &mut Criterion, name: &str, schema: &Value, instance: &Value) {
        let validator = jsonschema::validator_for(schema).expect("Valid schema");
        c.bench_with_input(
            BenchmarkId::new("is_valid", name),
            instance,
            |b, instance| {
                b.iter(|| black_box(validator.is_valid(instance)));
            },
        );
    }

    pub(crate) fn bench_validate(c: &mut Criterion, name: &str, schema: &Value, instance: &Value) {
        let validator = jsonschema::validator_for(schema).expect("Valid schema");
        c.bench_with_input(
            BenchmarkId::new("validate", name),
            instance,
            |b, instance| {
                b.iter(|| black_box(validator.validate(instance)));
            },
        );
    }

    pub(crate) fn bench_evaluate(c: &mut Criterion, name: &str, schema: &Value, instance: &Value) {
        let validator = jsonschema::validator_for(schema).expect("Valid schema");
        c.bench_with_input(
            BenchmarkId::new("evaluate", name),
            instance,
            |b, instance| {
                b.iter_with_large_drop(|| black_box(validator.evaluate(instance)));
            },
        );
    }

    pub(crate) fn bench_is_valid_with<F>(
        c: &mut Criterion,
        name: &str,
        instance: &Value,
        mut validate_fn: F,
    ) where
        F: FnMut(&Value) -> bool,
    {
        c.bench_with_input(
            BenchmarkId::new("is_valid", name),
            instance,
            |b, instance| {
                b.iter(|| black_box(validate_fn(instance)));
            },
        );
    }

    pub(crate) fn run_benchmarks(c: &mut Criterion) {
        for benchmark in Benchmark::iter() {
            benchmark.run(&mut |name, schema, instances| {
                bench_build(c, name, schema);
                for instance in instances {
                    let instance_name = format!("{}/{}", name, instance.name);
                    bench_is_valid(c, &instance_name, schema, &instance.data);
                    bench_validate(c, &instance_name, schema, &instance.data);
                    bench_evaluate(c, &instance_name, schema, &instance.data);

                    let is_valid = match name {
                        "Fast" => FastSchemaValidator::is_valid,
                        "Open API" => OpenAPIValidator::is_valid,
                        "Swagger" => SwaggerValidator::is_valid,
                        "GeoJSON" => GeoJSONValidator::is_valid,
                        "CITM" => CITMValidator::is_valid,
                        "FHIR" => FHIRValidator::is_valid,
                        "Recursive" => RecursiveValidator::is_valid,
                        _ => {
                            continue;
                        }
                    };

                    let name = format!("{name}/{}/codegen", instance.name);
                    bench_is_valid_with(c, &name, &instance.data, is_valid);
                }
            });
        }
    }

    criterion_group!(jsonschema, run_benchmarks);
}

#[cfg(not(target_arch = "wasm32"))]
codspeed_criterion_compat::criterion_main!(bench::jsonschema);

#[cfg(target_arch = "wasm32")]
fn main() {}
