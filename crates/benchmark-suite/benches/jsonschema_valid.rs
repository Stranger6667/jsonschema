use benchmark::{BenchInstance, Benchmark};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use jsonschema_valid::{schemas, Config};
use serde_json::Value;

fn draft(schema: &Value) -> schemas::Draft {
    match schema["$schema"].as_str() {
        Some("http://json-schema.org/draft-04/schema#") => schemas::Draft::Draft4,
        Some("http://json-schema.org/draft-06/schema#") => schemas::Draft::Draft6,
        _ => schemas::Draft::Draft7,
    }
}

fn bench_build(c: &mut Criterion, name: &str, schema: &Value) {
    c.bench_function(&format!("jsonschema_valid/{name}/build"), |b| {
        b.iter_with_large_drop(|| {
            Config::from_schema(schema, Some(draft(schema))).expect("Valid schema")
        });
    });
}

fn bench_validate(c: &mut Criterion, name: &str, schema: &Value, instance: &BenchInstance) {
    let cfg = Config::from_schema(schema, Some(draft(schema))).expect("Valid schema");
    assert_eq!(
        jsonschema_valid::validate(&cfg, &instance.data).is_ok(),
        instance.valid,
        "{name}"
    );
    c.bench_with_input(
        BenchmarkId::new(name, "validate"),
        &instance.data,
        |b, instance| b.iter(|| jsonschema_valid::validate(&cfg, instance).is_ok()),
    );
}

// Swagger is rejected incorrectly under draft 4, 2020-12 keywords are ignored, and Recursive panics
// at build.
static UNSUPPORTED_BENCHMARKS: &[&str] = &["Swagger", "Open API 3.1", "Recursive"];

fn run_benchmarks(c: &mut Criterion) {
    for benchmark in Benchmark::iter() {
        benchmark.run(&mut |name, schema, instances| {
            if !UNSUPPORTED_BENCHMARKS.contains(&name) {
                bench_build(c, name, schema);
                for instance in instances {
                    let name = format!("jsonschema_valid/{}/{}", name, instance.name);
                    bench_validate(c, &name, schema, instance);
                }
            }
        });
    }
}

criterion_group!(jsonschema_valid, run_benchmarks);
criterion_main!(jsonschema_valid);
