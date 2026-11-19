#[cfg(not(target_arch = "wasm32"))]
mod bench {
    use std::hint::black_box;

    pub(crate) use benchmark::Benchmark;
    pub(crate) use codspeed_criterion_compat::{criterion_group, BenchmarkId, Criterion};
    pub(crate) use serde_json::{json, Value};

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

    pub(crate) fn bench_iter_errors(
        c: &mut Criterion,
        name: &str,
        schema: &Value,
        instance: &Value,
    ) {
        let validator = jsonschema::validator_for(schema).expect("Valid schema");
        c.bench_with_input(
            BenchmarkId::new("iter_errors", name),
            instance,
            |b, instance| {
                b.iter_with_large_drop(|| black_box(validator.iter_errors(instance).count()));
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
                bench_build(c, name, schema);
                for instance in instances {
                    let instance_name = format!("{}/{}", name, instance.name);
                    bench_is_valid(c, &instance_name, schema, &instance.data);
                    bench_validate(c, &instance_name, schema, &instance.data);
                    bench_iter_errors(c, &instance_name, schema, &instance.data);
                    bench_evaluate(c, &instance_name, schema, &instance.data);
                }
            });
        }
    }

    // Every nested `c` member is reached through both `properties` and `patternProperties`, and
    // `allOf` re-enters the root at each level.
    pub(crate) fn run_synthetic_benchmarks(c: &mut Criterion) {
        let schema = json!({
            "allOf": [{"$ref": "#"}],
            "properties": {"c": {"$ref": "#"}},
            "patternProperties": {"^c": {"$ref": "#"}}
        });
        let instance = (0..REF_CYCLE_DEPTH).fold(json!({}), |child, _| json!({"c": child}));
        let name = format!("Ref cycle through several keywords/Depth {REF_CYCLE_DEPTH}");
        bench_is_valid(c, &name, &schema, &instance);
        bench_validate(c, &name, &schema, &instance);
        bench_iter_errors(c, &name, &schema, &instance);
    }

    const REF_CYCLE_DEPTH: usize = 16;

    criterion_group!(jsonschema, run_benchmarks, run_synthetic_benchmarks);
}

#[cfg(not(target_arch = "wasm32"))]
codspeed_criterion_compat::criterion_main!(bench::jsonschema);

#[cfg(target_arch = "wasm32")]
fn main() {}
