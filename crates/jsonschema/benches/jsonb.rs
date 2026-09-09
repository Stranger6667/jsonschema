// `Jsonb` against `SerdeJson` on every validation path.
#[cfg(not(target_arch = "wasm32"))]
mod bench {
    use std::hint::black_box;

    pub(crate) use codspeed_criterion_compat::{criterion_group, Criterion};

    use benchmark::{read_json, FHIR_SCHEMA};
    use jsonschema::{
        json::{Json, Jsonb},
        Validator,
    };
    use serde_json::{json, Value};

    // `GEOJSON` stands in as the instance: `canada.json` exceeds one heap page under
    // `STORAGE PLAIN`. Pins its shallow shape only.
    fn geojson_instance_schema() -> Value {
        json!({
            "type": "object",
            "required": ["$schema", "$id", "title", "type", "required", "properties"],
            "additionalProperties": false,
            "properties": {
                "$schema": {"type": "string"},
                "$id": {"type": "string"},
                "title": {"type": "string"},
                "type": {"const": "object"},
                "required": {"type": "array", "items": {"type": "string"}},
                "properties": {"type": "object"}
            }
        })
    }

    fn bench_is_valid<'i, F: Json>(
        c: &mut Criterion,
        name: &str,
        validator: &Validator<F>,
        instance: F::Node<'i>,
    ) where
        F::Node<'i>: Copy,
    {
        c.bench_function(name, |b| {
            b.iter(|| black_box(validator.is_valid(instance)));
        });
    }

    fn bench_validate<'i, F: Json>(
        c: &mut Criterion,
        name: &str,
        validator: &'i Validator<F>,
        instance: F::Node<'i>,
    ) where
        F::Node<'i>: Copy,
    {
        c.bench_function(name, |b| {
            b.iter(|| black_box(validator.validate(instance)));
        });
    }

    fn bench_iter_errors<'i, F: Json>(
        c: &mut Criterion,
        name: &str,
        validator: &'i Validator<F>,
        instance: F::Node<'i>,
    ) where
        F::Node<'i>: Copy,
    {
        c.bench_function(name, |b| {
            b.iter_with_large_drop(|| black_box(validator.iter_errors(instance).count()));
        });
    }

    // One (workload, representation) pair, named `jsonb/<workload>/<repr>/<path>/<verdict>`.
    fn bench_paths<'i, F: Json>(
        c: &mut Criterion,
        workload: &str,
        repr: &str,
        validator: &'i Validator<F>,
        valid: F::Node<'i>,
        invalid: F::Node<'i>,
    ) where
        F::Node<'i>: Copy,
    {
        for (verdict, instance) in [("valid", valid), ("invalid", invalid)] {
            bench_is_valid(
                c,
                &format!("jsonb/{workload}/{repr}/is_valid/{verdict}"),
                validator,
                instance,
            );
            bench_validate(
                c,
                &format!("jsonb/{workload}/{repr}/validate/{verdict}"),
                validator,
                instance,
            );
            bench_iter_errors(
                c,
                &format!("jsonb/{workload}/{repr}/iter_errors/{verdict}"),
                validator,
                instance,
            );
        }
    }

    // Tiny instances, so this measures per-call overhead.
    fn bench_fast(c: &mut Criterion) {
        let schema = read_json(benchmark::FAST_SCHEMA);
        let valid = read_json(benchmark::FAST_VALID);
        let invalid = read_json(benchmark::FAST_INVALID);

        let serde_validator = jsonschema::validator_for(&schema).expect("schema builds");
        bench_paths(c, "fast", "serde_json", &serde_validator, &valid, &invalid);

        let jsonb_validator = jsonschema::options_for::<Jsonb>()
            .build(&schema)
            .expect("schema builds");
        let valid_root = Jsonb::root(benchmark::FAST_VALID_JSONB);
        let invalid_root = Jsonb::root(benchmark::FAST_INVALID_JSONB);
        bench_paths(
            c,
            "fast",
            "jsonb",
            &jsonb_validator,
            valid_root,
            invalid_root,
        );
    }

    // Deeply nested and array-heavy.
    fn bench_geojson(c: &mut Criterion) {
        let schema = geojson_instance_schema();
        let valid = read_json(benchmark::GEOJSON);
        let invalid = read_json(benchmark::GEOJSON_INSTANCE_INVALID);

        let serde_validator = jsonschema::validator_for(&schema).expect("schema builds");
        bench_paths(
            c,
            "geojson",
            "serde_json",
            &serde_validator,
            &valid,
            &invalid,
        );

        let jsonb_validator = jsonschema::options_for::<Jsonb>()
            .build(&schema)
            .expect("schema builds");
        let valid_root = Jsonb::root(benchmark::GEOJSON_INSTANCE_JSONB);
        let invalid_root = Jsonb::root(benchmark::GEOJSON_INSTANCE_INVALID_JSONB);
        bench_paths(
            c,
            "geojson",
            "jsonb",
            &jsonb_validator,
            valid_root,
            invalid_root,
        );
    }

    // 146-branch `oneOf`. Every branch fails on the invalid instance, so the error paths build
    // the full per-branch tree.
    fn bench_fhir(c: &mut Criterion) {
        let schema = read_json(FHIR_SCHEMA);
        let valid = read_json(benchmark::FHIR_PATIENT);
        let invalid = read_json(benchmark::FHIR_PATIENT_INVALID);

        let serde_validator = jsonschema::validator_for(&schema).expect("schema builds");
        bench_paths(c, "fhir", "serde_json", &serde_validator, &valid, &invalid);

        let jsonb_validator = jsonschema::options_for::<Jsonb>()
            .build(&schema)
            .expect("schema builds");
        let valid_root = Jsonb::root(benchmark::FHIR_PATIENT_JSONB);
        let invalid_root = Jsonb::root(benchmark::FHIR_PATIENT_INVALID_JSONB);
        bench_paths(
            c,
            "fhir",
            "jsonb",
            &jsonb_validator,
            valid_root,
            invalid_root,
        );
    }

    // No `citm_catalog` arm: Postgres rejects it under `STORAGE PLAIN` ("row is too big").

    pub(crate) fn run_benchmarks(c: &mut Criterion) {
        bench_fast(c);
        bench_geojson(c);
        bench_fhir(c);
    }

    criterion_group!(jsonb, run_benchmarks);
}

#[cfg(not(target_arch = "wasm32"))]
codspeed_criterion_compat::criterion_main!(bench::jsonb);

#[cfg(target_arch = "wasm32")]
fn main() {}
