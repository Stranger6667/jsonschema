use benchmark::Benchmark;
use boon::{Compiler, Schemas};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use serde_json::Value;

fn bench_build(c: &mut Criterion, name: &str, schema: &Value) {
    let mut compiler = Compiler::new();
    compiler
        .add_resource("schema.json", schema.clone())
        .expect("Failed to add resource");
    c.bench_function(&format!("boon/{name}/build"), |b| {
        b.iter(|| {
            compiler
                .compile("schema.json", &mut Schemas::new())
                .expect("Failed to build");
        })
    });
}

fn bench_validate(c: &mut Criterion, name: &str, schema: &Value, instance: &Value) {
    let mut schemas = Schemas::new();
    let mut compiler = Compiler::new();
    compiler
        .add_resource("schema.json", schema.clone())
        .expect("Failed to add resource");
    let id = compiler
        .compile("schema.json", &mut schemas)
        .expect("Failed to build");

    c.bench_with_input(
        BenchmarkId::new(name, "validate"),
        instance,
        |b, instance| b.iter(|| schemas.validate(instance, id).is_ok()),
    );
}

fn run_benchmarks(c: &mut Criterion) {
    for benchmark in Benchmark::iter() {
        benchmark.run(&mut |name, schema, instances| {
            bench_build(c, name, schema);
            for instance in instances {
                let name = format!("boon/{}/{}", name, instance.name);
                bench_validate(c, &name, schema, &instance.data);
            }
        });
    }
}

criterion_group!(boon, run_benchmarks);
criterion_main!(boon);
