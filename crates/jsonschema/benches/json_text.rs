// The shared suite read as JSON text; `benches/jsonschema.rs` holds the `serde_json` side.
// `parse` times the text check, and the validation benches start from a checked document.
#[cfg(not(target_arch = "wasm32"))]
mod bench {
    use std::hint::black_box;

    use benchmark::Benchmark;
    use codspeed_criterion_compat::{criterion_group, Criterion};
    use jsonschema::{json::JsonText, Validator};

    fn bench_instance(c: &mut Criterion, name: &str, validator: &Validator<JsonText>, text: &[u8]) {
        c.bench_function(&format!("json_text/parse/{name}"), |b| {
            b.iter(|| black_box(JsonText::parse(black_box(text)).expect("Valid JSON")));
        });
        let document = JsonText::parse(text).expect("Valid JSON");
        let instance = document.root();
        c.bench_function(&format!("json_text/is_valid/{name}"), |b| {
            b.iter(|| black_box(validator.is_valid(black_box(instance))));
        });
        c.bench_function(&format!("json_text/validate/{name}"), |b| {
            b.iter(|| black_box(validator.validate(black_box(instance)).is_ok()));
        });
    }

    pub(crate) fn run_benchmarks(c: &mut Criterion) {
        for benchmark in Benchmark::iter() {
            benchmark.run(&mut |name, schema, instances| {
                let validator = jsonschema::options_for::<JsonText>()
                    .build(schema)
                    .expect("Valid schema");
                for instance in instances {
                    let text = serde_json::to_vec(&instance.data).expect("Serializable");
                    bench_instance(c, &format!("{name}/{}", instance.name), &validator, &text);
                }
            });
        }
    }

    criterion_group!(json_text, run_benchmarks);
}

#[cfg(not(target_arch = "wasm32"))]
codspeed_criterion_compat::criterion_main!(bench::json_text);

#[cfg(target_arch = "wasm32")]
fn main() {}
