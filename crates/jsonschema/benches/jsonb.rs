// The shared suite read as Postgres `jsonb`; `benches/jsonschema.rs` holds the `serde_json` side.
#[cfg(not(target_arch = "wasm32"))]
mod bench {
    use std::hint::black_box;

    use benchmark::Benchmark;
    use codspeed_criterion_compat::{criterion_group, Criterion};
    use jsonschema::{json::Jsonb, Validator};
    use jsonschema_value::jsonb::encode::encode;

    fn bench_instance(c: &mut Criterion, name: &str, validator: &Validator<Jsonb>, bytes: &[u8]) {
        let instance = Jsonb::root(bytes);
        c.bench_function(&format!("jsonb/is_valid/{name}"), |b| {
            b.iter(|| black_box(validator.is_valid(instance)));
        });
        c.bench_function(&format!("jsonb/validate/{name}"), |b| {
            b.iter(|| black_box(validator.validate(instance)));
        });
        c.bench_function(&format!("jsonb/iter_errors/{name}"), |b| {
            b.iter_with_large_drop(|| black_box(validator.iter_errors(instance).count()));
        });
    }

    pub(crate) fn run_benchmarks(c: &mut Criterion) {
        for benchmark in Benchmark::iter() {
            benchmark.run(&mut |name, schema, instances| {
                let validator = jsonschema::options_for::<Jsonb>()
                    .build(schema)
                    .expect("Valid schema");
                for instance in instances {
                    let bytes = encode(&instance.data);
                    bench_instance(c, &format!("{name}/{}", instance.name), &validator, &bytes);
                }
            });
        }
    }

    criterion_group!(jsonb, run_benchmarks);
}

#[cfg(not(target_arch = "wasm32"))]
codspeed_criterion_compat::criterion_main!(bench::jsonb);

#[cfg(target_arch = "wasm32")]
fn main() {}
