use criterion::{black_box, criterion_group, criterion_main, Criterion};
use jsonschema::{validator_for, ErrorIteratorV2, SchemaEvaluationVM, ValidatorV2};
use serde_json::json;

fn bench_validators(c: &mut Criterion) {
    let schema = json!({
        "type": "integer",
        "minimum": 10,
        "maximum": 100
    });
    let valid_instance = json!(50);
    let invalid_instance_low = json!(5);
    let invalid_instance_high = json!(150);

    let v1 = validator_for(&schema).unwrap();
    let v2 = ValidatorV2::new(&schema);
    let mut vm = SchemaEvaluationVM::new();

    c.bench_function("V1 is_valid valid", |b| {
        b.iter(|| {
            let _ = v1.is_valid(black_box(&valid_instance));
        })
    });
    c.bench_function("V2 is_valid valid", |b| {
        b.iter(|| {
            vm.is_valid(&v2.program, black_box(&valid_instance));
        })
    });
    c.bench_function("V1 is_valid invalid", |b| {
        b.iter(|| {
            let _ = v1.is_valid(black_box(&invalid_instance_low));
        })
    });
    c.bench_function("V2 is_valid invalid", |b| {
        b.iter(|| {
            vm.is_valid(&v2.program, black_box(&invalid_instance_low));
        })
    });

    c.bench_function("V1 validate valid", |b| {
        b.iter(|| {
            let _ = v1.validate(black_box(&valid_instance));
        })
    });
    c.bench_function("V2 validate valid", |b| {
        b.iter(|| {
            let _ = vm.validate(&v2.program, black_box(&valid_instance));
        })
    });
    c.bench_function("V1 validate invalid", |b| {
        b.iter(|| {
            let _ = v1.validate(black_box(&invalid_instance_high));
        })
    });
    c.bench_function("V2 validate invalid", |b| {
        b.iter(|| {
            let _ = vm.validate(&v2.program, black_box(&invalid_instance_high));
        })
    });

    c.bench_function("V1 iter_errors valid", |b| {
        b.iter(|| {
            let _: Vec<_> = v1.iter_errors(black_box(&valid_instance)).collect();
        })
    });
    c.bench_function("V2 iter_errors valid", |b| {
        b.iter(|| {
            let _: Vec<_> = ErrorIteratorV2::new(black_box(&valid_instance), &v2.program).collect();
        })
    });
    c.bench_function("V1 iter_errors invalid", |b| {
        b.iter(|| {
            let _: Vec<_> = v1.iter_errors(black_box(&invalid_instance_low)).collect();
        })
    });
    c.bench_function("V2 iter_errors invalid", |b| {
        b.iter(|| {
            let _: Vec<_> =
                ErrorIteratorV2::new(black_box(&invalid_instance_low), &v2.program).collect();
        })
    });
}

criterion_group!(benches, bench_validators);
criterion_main!(benches);
