use benchmark::Benchmark;
use codspeed_criterion_compat::{criterion_group, criterion_main, BenchmarkId, Criterion};
use jsonschema_macros_core::bench::{generate, prepare};

fn bench(c: &mut Criterion) {
    for benchmark in Benchmark::iter() {
        benchmark.run(&mut |name, schema, _instances| {
            let input = prepare(schema.clone());
            c.bench_with_input(BenchmarkId::new("generate", name), &input, |b, input| {
                b.iter_with_large_drop(|| generate(input));
            });
        });
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
