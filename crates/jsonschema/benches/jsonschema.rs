use benchmark::Benchmark;
use codspeed_criterion_compat::{criterion_group, criterion_main, BenchmarkId, Criterion};
use serde_json::Value;

fn bench_build(c: &mut Criterion, name: &str, schema: &Value) {
    c.bench_with_input(BenchmarkId::new("build", name), schema, |b, schema| {
        b.iter_with_large_drop(|| jsonschema::validator_for(schema).expect("Valid schema"))
    });
}

fn bench_is_valid(c: &mut Criterion, name: &str, schema: &Value, instance: &Value) {
    let validator = jsonschema::validator_for(schema).expect("Valid schema");
    c.bench_with_input(
        BenchmarkId::new("is_valid", name),
        instance,
        |b, instance| {
            b.iter(|| {
                let _ = validator.is_valid(instance);
            })
        },
    );
}

fn bench_is_valid2(c: &mut Criterion) {
    let schema = serde_json::json!({
        "properties": {
            "age": {
                "properties": {
                    "inner": {
                        "minimum": 18,
                        "maximum": 65
                    }
                }
            },
            "score": {
                "properties": {
                    "inner": {
                        "minimum": 0,
                        "maximum": 100
                    }
                }
            },
            "name": {
                "properties": {
                    "inner": {
                        "minLength": 3,
                        "maxLength": 100
                    }
                }
            }
        }
    });
    //let schema = serde_json::json!({
    //    "minimum": 18,
    //    "maximum": 65
    //});
    //let schema = serde_json::json!({
    //    "minLength": 3,
    //    "maxLength": 65
    //});
    let validatorv1 = jsonschema::validator_for(&schema).unwrap();
    let validatorv2 = jsonschema::ValidatorV2::new(&schema);
    let instance = serde_json::json!({
        "age": {"inner": 30},
        "score": {"inner": 85},
        "name": {"inner": "John"},
    });
    //let instance = serde_json::json!(30);
    //let instance = serde_json::json!("abcdef");

    let mut g = c.benchmark_group("stringLength");
    g.bench_with_input("v1", &instance, |b, instance| {
        b.iter(|| {
            let _ = validatorv1.is_valid(instance);
        })
    });
    g.bench_with_input("v2", &instance, |b, instance| {
        b.iter(|| {
            let _ = validatorv2.is_valid(instance);
        })
    });

    g.bench_with_input("v2-with-vm", &instance, |b, instance| {
        let mut vm = jsonschema::SchemaEvaluationVM::new();
        b.iter(|| {
            let _ = validatorv2.is_valid_with(instance, &mut vm);
        })
    });
    g.finish();
}

fn bench_logical_operations(c: &mut Criterion) {
    // Create a schema with many anyOf branches
    let mut any_of_schemas = Vec::new();

    // Create 100 number validation schemas with different minimum values
    for i in 1..101 {
        any_of_schemas.push(serde_json::json!({ "minimum": i }));
    }

    let schema = serde_json::json!({
        "anyOf": any_of_schemas
    });

    // Test instances - one that matches early, one that matches late, one that doesn't match
    let early_match = serde_json::json!(5); // Matches the first few branches
    let late_match = serde_json::json!(95); // Matches only later branches
    let no_match = serde_json::json!(0); // Matches no branches

    let validatorv1 = jsonschema::validator_for(&schema).unwrap();
    let validatorv2 = jsonschema::ValidatorV2::new(&schema);

    let mut g = c.benchmark_group("anyOf_early_match");

    // Benchmark early match case
    g.bench_with_input("v1", &early_match, |b, instance| {
        b.iter(|| {
            let _ = validatorv1.is_valid(instance);
        })
    });

    g.bench_with_input("v2", &early_match, |b, instance| {
        b.iter(|| {
            let _ = validatorv2.is_valid(instance);
        })
    });

    g.bench_with_input("v2-with-vm", &early_match, |b, instance| {
        let mut vm = jsonschema::SchemaEvaluationVM::new();
        b.iter(|| {
            let _ = validatorv2.is_valid_with(instance, &mut vm);
        })
    });
    g.finish();

    let mut g = c.benchmark_group("anyOf_late_match");
    // Benchmark late match case
    g.bench_with_input("v1", &late_match, |b, instance| {
        b.iter(|| {
            let _ = validatorv1.is_valid(instance);
        })
    });

    g.bench_with_input("v2", &late_match, |b, instance| {
        b.iter(|| {
            let _ = validatorv2.is_valid(instance);
        })
    });

    g.bench_with_input("v2-with-vm", &late_match, |b, instance| {
        let mut vm = jsonschema::SchemaEvaluationVM::new();
        b.iter(|| {
            let _ = validatorv2.is_valid_with(instance, &mut vm);
        })
    });

    g.finish();
    let mut g = c.benchmark_group("anyOf_no_match");
    g.bench_with_input("v1", &no_match, |b, instance| {
        b.iter(|| {
            let _ = validatorv1.is_valid(instance);
        })
    });

    g.bench_with_input("v2", &no_match, |b, instance| {
        b.iter(|| {
            let _ = validatorv2.is_valid(instance);
        })
    });

    g.bench_with_input("v2-with-vm", &no_match, |b, instance| {
        let mut vm = jsonschema::SchemaEvaluationVM::new();
        b.iter(|| {
            let _ = validatorv2.is_valid_with(instance, &mut vm);
        })
    });
    g.finish();
}

fn bench_nested_logical_operations(c: &mut Criterion) {
    let mut level3_schemas = Vec::new();
    for i in 1..6 {
        level3_schemas.push(serde_json::json!({ "minimum": i * 20 }));
    }

    let mut level2_schemas = Vec::new();
    for i in 1..6 {
        level2_schemas.push(serde_json::json!({
            "anyOf": level3_schemas.clone()
        }));
    }

    let schema = serde_json::json!({
        "anyOf": level2_schemas
    });

    let instance = serde_json::json!(95);

    let validatorv1 = jsonschema::validator_for(&schema).unwrap();
    let validatorv2 = jsonschema::ValidatorV2::new(&schema);

    let mut g = c.benchmark_group("nested_anyOf");
    g.bench_with_input("v1", &instance, |b, instance| {
        b.iter(|| {
            let _ = validatorv1.is_valid(instance);
        })
    });

    g.bench_with_input("v2", &instance, |b, instance| {
        b.iter(|| {
            let _ = validatorv2.is_valid(instance);
        })
    });

    g.bench_with_input("v2-with-vm", &instance, |b, instance| {
        let mut vm = jsonschema::SchemaEvaluationVM::new();
        b.iter(|| {
            let _ = validatorv2.is_valid_with(instance, &mut vm);
        })
    });
    g.finish();
}

fn bench_validate(c: &mut Criterion, name: &str, schema: &Value, instance: &Value) {
    let validator = jsonschema::validator_for(schema).expect("Valid schema");
    c.bench_with_input(
        BenchmarkId::new("validate", name),
        instance,
        |b, instance| {
            b.iter(|| {
                let _ = validator.validate(instance);
            })
        },
    );
}

fn bench_apply(c: &mut Criterion, name: &str, schema: &Value, instance: &Value) {
    let validator = jsonschema::validator_for(schema).expect("Valid schema");
    c.bench_with_input(BenchmarkId::new("apply", name), instance, |b, instance| {
        b.iter_with_large_drop(|| validator.apply(instance).basic())
    });
}

fn run_benchmarks(c: &mut Criterion) {
    for benchmark in Benchmark::iter() {
        benchmark.run(&mut |name, schema, instances| {
            bench_build(c, name, schema);
            for instance in instances {
                let name = format!("{}/{}", name, instance.name);
                bench_is_valid(c, &name, schema, &instance.data);
                bench_validate(c, &name, schema, &instance.data);
                bench_apply(c, &name, schema, &instance.data);
            }
        });
    }
}

criterion_group!(
    jsonschema,
    run_benchmarks,
    bench_is_valid2,
    bench_logical_operations,
    bench_nested_logical_operations
);
criterion_main!(jsonschema);
