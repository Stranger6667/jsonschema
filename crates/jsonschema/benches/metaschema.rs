#[cfg(not(target_arch = "wasm32"))]
mod bench {
    use std::hint::black_box;

    use benchmark::{read_json, FHIR_SCHEMA, RECURSIVE_SCHEMA, SWAGGER};
    use codspeed_criterion_compat::{criterion_group, Criterion};
    use serde_json::Value;

    type IsValidFn = fn(&Value) -> bool;

    fn run_benchmarks(c: &mut Criterion) {
        // Under `macros`, `draftN::meta::is_valid` is the compile-time validator; the `/runtime`
        // arm builds a tree-walking validator from the same meta-schema for comparison.
        let cases: &[(&str, &[u8], IsValidFn, &Value)] = &[
            (
                "Swagger",
                SWAGGER,
                jsonschema::draft4::meta::is_valid,
                &referencing::meta::DRAFT4,
            ),
            (
                "FHIR",
                FHIR_SCHEMA,
                jsonschema::draft6::meta::is_valid,
                &referencing::meta::DRAFT6,
            ),
            (
                "Recursive",
                RECURSIVE_SCHEMA,
                jsonschema::draft7::meta::is_valid,
                &referencing::meta::DRAFT7,
            ),
        ];
        for &(name, bytes, meta_is_valid, metaschema) in cases {
            let schema = read_json(bytes);
            let runtime = jsonschema::options()
                .build(metaschema)
                .expect("meta-schema builds");
            c.bench_function(&format!("metaschema/is_valid/{name}"), |b| {
                b.iter(|| black_box(meta_is_valid(&schema)));
            });
            c.bench_function(&format!("metaschema/is_valid/{name}/runtime"), |b| {
                b.iter(|| black_box(runtime.is_valid(&schema)));
            });
        }

        // Meta-validating a meta-schema exercises `$vocabulary` and the dynamic-scope machinery.
        let meta: &Value = &referencing::meta::DRAFT202012;
        let runtime = jsonschema::options()
            .build(meta)
            .expect("meta-schema builds");
        c.bench_function("metaschema/is_valid/Draft2020-12", |b| {
            b.iter(|| black_box(jsonschema::draft202012::meta::is_valid(meta)));
        });
        c.bench_function("metaschema/is_valid/Draft2020-12/runtime", |b| {
            b.iter(|| black_box(runtime.is_valid(meta)));
        });
    }

    criterion_group!(metaschema, run_benchmarks);
}

#[cfg(not(target_arch = "wasm32"))]
codspeed_criterion_compat::criterion_main!(bench::metaschema);

#[cfg(target_arch = "wasm32")]
fn main() {}
