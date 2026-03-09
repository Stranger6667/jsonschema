#[cfg(not(target_arch = "wasm32"))]
mod bench {
    use std::hint::black_box;

    pub(crate) use benchmark::Benchmark;
    pub(crate) use codspeed_criterion_compat::{criterion_group, BenchmarkId, Criterion};
    use referencing::{DefaultRetriever, Draft, Registry, ResourceRef};
    pub(crate) use serde_json::Value;

    const BASE_URI: &str = "https://bench.example/root.json";

    /// Benchmark the full preparation pipeline: registry construction + index building.
    /// This is where the skeleton optimisation applies.
    pub(crate) fn bench_prepare(c: &mut Criterion, name: &str, schema: &Value) {
        c.bench_with_input(BenchmarkId::new("prepare", name), schema, |b, schema| {
            b.iter_with_large_drop(|| {
                let draft = Draft::default().detect(schema);
                let registry = Registry::try_from_resources_and_retriever(
                    [(BASE_URI, ResourceRef::new(schema, draft))],
                    &DefaultRetriever,
                    draft,
                )
                .expect("Valid schema");
                // `let _ =` drops the index immediately so `registry` can be returned
                // for deferred drop outside the timing window.
                let _ = registry.build_index().expect("Valid index");
                registry
            });
        });
    }

    pub(crate) fn bench_build(c: &mut Criterion, name: &str, schema: &Value) {
        let draft = Draft::default().detect(schema);
        let registry = Registry::try_from_resources_and_retriever(
            [(BASE_URI, ResourceRef::new(schema, draft))],
            &DefaultRetriever,
            draft,
        )
        .expect("Valid schema");
        let index = registry.build_index().expect("Valid index");
        c.bench_with_input(BenchmarkId::new("build", name), schema, |b, schema| {
            b.iter_with_large_drop(|| {
                jsonschema::options()
                    .with_index(&index)
                    .with_base_uri(BASE_URI)
                    .build(schema)
                    .expect("Valid schema")
            });
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

    pub(crate) fn run_benchmarks(c: &mut Criterion) {
        for benchmark in Benchmark::iter() {
            benchmark.run(&mut |name, schema, instances| {
                bench_prepare(c, name, schema);
                bench_build(c, name, schema);
                for instance in instances {
                    let name = format!("{}/{}", name, instance.name);
                    bench_is_valid(c, &name, schema, &instance.data);
                    bench_validate(c, &name, schema, &instance.data);
                    bench_evaluate(c, &name, schema, &instance.data);
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
