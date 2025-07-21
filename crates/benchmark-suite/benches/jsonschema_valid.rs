use benchmark::Benchmark;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use jsonschema_valid::{schemas, Config};
use serde_json::Value;

fn bench_build(c: &mut Criterion, name: &str, schema: &Value) {
    c.bench_function(&format!("jsonschema_valid/{name}/build"), |b| {
        b.iter_with_large_drop(|| {
            Config::from_schema(schema, Some(schemas::Draft::Draft7)).expect("Valid schema")
        })
    });
}

fn bench_validate(c: &mut Criterion, name: &str, schema: &Value, instance: &Value) {
    let cfg = Config::from_schema(schema, Some(schemas::Draft::Draft7)).expect("Valid schema");
    c.bench_with_input(
        BenchmarkId::new(name, "validate"),
        instance,
        |b, instance| {
            b.iter(|| {
                let _ = jsonschema_valid::validate(&cfg, instance);
            })
        },
    );
}

static UNSUPPORTED_BENCHMARKS: &[&str] = &["Open API", "Swagger"];

fn run_benchmarks(c: &mut Criterion) {
    for benchmark in Benchmark::iter() {
        benchmark.run(&mut |name, schema, instances| {
            if !UNSUPPORTED_BENCHMARKS.contains(&name) {
                bench_build(c, name, schema);
                for instance in instances {
                    let name = format!("jsonschema_valid/{}/{}", name, instance.name);
                    bench_validate(c, &name, schema, &instance.data);
                }
            }
        });
    }
}

criterion_group!(jsonschema_valid, run_benchmarks);
criterion_main!(jsonschema_valid);
