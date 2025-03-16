use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use jsonschema::{validator_for, ErrorIteratorV2, SchemaEvaluationVM, ValidatorV2};
use serde_json::{json, Value};

fn get_test_cases() -> Vec<(
    &'static str,
    Value,
    Vec<(&'static str, Value)>,
    Vec<(&'static str, Value)>,
)> {
    vec![
        (
            "integer_range",
            json!({
                "type": "integer",
                "minimum": 10,
                "maximum": 100
            }),
            vec![("default", json!(50))],
            vec![("too_low", json!(5))],
        ),
        (
            "minimum",
            json!({"minimum": 10}),
            vec![("default", json!(50))],
            vec![("too_low", json!(5))],
        ),
    ]
}

#[inline(never)]
fn is_valid_integer_range(value: &serde_json::Value) -> bool {
    if let serde_json::Value::Number(number) = value {
        if let Some(v) = number.as_u64() {
            v >= 10 && v <= 100
        } else if let Some(v) = value.as_i64() {
            v >= 10 && v <= 100
        } else {
            false
        }
    } else {
        false
    }
}

enum ValidationMethod {
    IsValid,
    Validate,
    IterErrors,
}

impl ValidationMethod {
    const ALL: &'static [ValidationMethod] = &[
        ValidationMethod::IsValid,
        ValidationMethod::Validate,
        ValidationMethod::IterErrors,
    ];

    fn name(&self) -> &'static str {
        match self {
            ValidationMethod::IsValid => "is_valid",
            ValidationMethod::Validate => "validate",
            ValidationMethod::IterErrors => "iter_errors",
        }
    }
}

enum ValidatorVersion {
    V1,
    V2,
}

impl ValidatorVersion {
    const ALL: &'static [ValidatorVersion] = &[ValidatorVersion::V1, ValidatorVersion::V2];

    fn name(&self) -> &'static str {
        match self {
            ValidatorVersion::V1 => "V1",
            ValidatorVersion::V2 => "V2",
        }
    }
}

fn bench_validators(c: &mut Criterion) {
    for (case_name, schema, valid, invalid) in get_test_cases() {
        let mut group = c.benchmark_group(case_name);
        let v1 = validator_for(&schema).unwrap();
        let v2 = ValidatorV2::new(&schema);
        let mut vm = SchemaEvaluationVM::new();

        for method in ValidationMethod::ALL {
            for version in ValidatorVersion::ALL {
                for (name, instance) in &valid {
                    group.bench_with_input(
                        BenchmarkId::new(
                            format!("{}_{}", version.name(), method.name()),
                            format!("valid_{name}"),
                        ),
                        &instance,
                        |b, instance| match (version, method) {
                            (ValidatorVersion::V1, ValidationMethod::IsValid) => b.iter(|| {
                                let _ = v1.is_valid(black_box(instance));
                            }),
                            (ValidatorVersion::V2, ValidationMethod::IsValid) => b.iter(|| {
                                let _ = vm.is_valid(&v2.program, black_box(instance));
                            }),
                            (ValidatorVersion::V1, ValidationMethod::Validate) => b.iter(|| {
                                let _ = v1.validate(black_box(instance));
                            }),
                            (ValidatorVersion::V2, ValidationMethod::Validate) => b.iter(|| {
                                let _ = vm.validate(&v2.program, black_box(instance));
                            }),
                            (ValidatorVersion::V1, ValidationMethod::IterErrors) => b.iter(|| {
                                let _: Vec<_> = v1.iter_errors(black_box(instance)).collect();
                            }),
                            (ValidatorVersion::V2, ValidationMethod::IterErrors) => b.iter(|| {
                                let _: Vec<_> =
                                    ErrorIteratorV2::new(black_box(instance), &v2.program)
                                        .collect();
                            }),
                        },
                    );
                }

                for (name, instance) in &invalid {
                    group.bench_with_input(
                        BenchmarkId::new(
                            format!("{}_{}", version.name(), method.name()),
                            format!("invalid_{}", name),
                        ),
                        &instance,
                        |b, instance| match (version, method) {
                            (ValidatorVersion::V1, ValidationMethod::IsValid) => b.iter(|| {
                                let _ = v1.is_valid(black_box(instance));
                            }),
                            (ValidatorVersion::V2, ValidationMethod::IsValid) => b.iter(|| {
                                let _ = vm.is_valid(&v2.program, black_box(instance));
                            }),
                            (ValidatorVersion::V1, ValidationMethod::Validate) => b.iter(|| {
                                let _ = v1.validate(black_box(instance));
                            }),
                            (ValidatorVersion::V2, ValidationMethod::Validate) => b.iter(|| {
                                let _ = vm.validate(&v2.program, black_box(instance));
                            }),
                            (ValidatorVersion::V1, ValidationMethod::IterErrors) => b.iter(|| {
                                let _: Vec<_> = v1.iter_errors(black_box(instance)).collect();
                            }),
                            (ValidatorVersion::V2, ValidationMethod::IterErrors) => b.iter(|| {
                                let _: Vec<_> =
                                    ErrorIteratorV2::new(black_box(instance), &v2.program)
                                        .collect();
                            }),
                        },
                    );
                }
            }
        }
    }
}

criterion_group!(basic_validators, bench_validators);
criterion_main!(basic_validators);
