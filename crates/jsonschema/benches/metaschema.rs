#[cfg(not(target_arch = "wasm32"))]
mod bench {
    use std::hint::black_box;

    use benchmark::{read_json, FHIR_SCHEMA, RECURSIVE_SCHEMA, SWAGGER};
    use codspeed_criterion_compat::{criterion_group, Criterion};
    use serde_json::Value;

    type IsValidFn = fn(&Value) -> bool;

    #[jsonschema::validator(
        path = "../jsonschema-referencing/metaschemas/draft4.json",
        draft = referencing::Draft::Draft4
    )]
    struct Draft4Validator;

    #[jsonschema::validator(
        path = "../jsonschema-referencing/metaschemas/draft6.json",
        draft = referencing::Draft::Draft6
    )]
    struct Draft6Validator;

    #[jsonschema::validator(
        path = "../jsonschema-referencing/metaschemas/draft7.json",
        draft = referencing::Draft::Draft7
    )]
    struct Draft7Validator;

    fn run_benchmarks(c: &mut Criterion) {
        let cases: &[(&str, &[u8], IsValidFn, IsValidFn)] = &[
            (
                "Swagger",
                SWAGGER,
                jsonschema::draft4::meta::is_valid,
                Draft4Validator::is_valid,
            ),
            (
                "FHIR",
                FHIR_SCHEMA,
                jsonschema::draft6::meta::is_valid,
                Draft6Validator::is_valid,
            ),
            (
                "Recursive",
                RECURSIVE_SCHEMA,
                jsonschema::draft7::meta::is_valid,
                Draft7Validator::is_valid,
            ),
        ];
        for &(name, bytes, is_valid, codegen_is_valid) in cases {
            let schema = read_json(bytes);
            c.bench_function(&format!("metaschema/is_valid/{name}"), |b| {
                b.iter(|| black_box(is_valid(&schema)));
            });
            c.bench_function(&format!("metaschema/is_valid/{name}/codegen"), |b| {
                b.iter(|| black_box(codegen_is_valid(&schema)));
            });
        }
    }

    criterion_group!(metaschema, run_benchmarks);
}

#[cfg(not(target_arch = "wasm32"))]
codspeed_criterion_compat::criterion_main!(bench::metaschema);

#[cfg(target_arch = "wasm32")]
fn main() {}
