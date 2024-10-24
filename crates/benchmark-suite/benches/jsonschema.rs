use benchmark::Benchmark;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use serde_json::Value;

fn bench_build(c: &mut Criterion, name: &str, schema: &Value) {
    c.bench_function(&format!("jsonschema/{}/build", name), |b| {
        b.iter(|| jsonschema::validator_for(schema).expect("Valid schema"))
    });
}

fn bench_is_valid(c: &mut Criterion, name: &str, schema: &Value, instance: &Value) {
    let validator = jsonschema::validator_for(schema).expect("Valid schema");
    c.bench_with_input(
        BenchmarkId::new(name, "is_valid"),
        instance,
        |b, instance| {
            b.iter(|| {
                let _ = validator.is_valid(instance);
            })
        },
    );
}

fn bench_validate(c: &mut Criterion, name: &str, schema: &Value, instance: &Value) {
    let validator = jsonschema::validator_for(schema).expect("Valid schema");
    c.bench_with_input(
        BenchmarkId::new(name, "validate"),
        instance,
        |b, instance| {
            b.iter(|| {
                let _ = validator.iter_errors(instance);
            })
        },
    );
}

fn run_benchmarks(c: &mut Criterion) {
    for benchmark in Benchmark::iter() {
        benchmark.run(&mut |name, schema, instances| {
            bench_build(c, name, schema);
            for instance in instances {
                let name = format!("jsonschema/{}/{}", name, instance.name);
                bench_is_valid(c, &name, schema, &instance.data);
                bench_validate(c, &name, schema, &instance.data);
            }
        });
    }
}

criterion_group!(jsonschema, run_benchmarks);
criterion_main!(jsonschema);
