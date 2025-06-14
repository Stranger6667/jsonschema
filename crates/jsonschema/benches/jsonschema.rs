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

fn bench_is_valid2(c: &mut Criterion) {
    // let schema = serde_json::json!({"properties": {"name": {"maxLength": 5}}});

    let schema = serde_json::json!({
        "properties": {
            "name": {
                "$ref": "#/$defs/Name"
            }
        },
        "$defs": {
            "Name": {
                "maxLength": 5
            }
        }
    });
    let schema = serde_json::json!({
        "properties": {
            "name": {"maxLength": 3},
            "child": {"$ref": "#"}
        }
    });
    let config = jsonschema::options();
    let validator2 = jsonschema::compiler_v2::build(config, &schema);
    let validator = jsonschema::validator_for(&schema).expect("Valid schema");
    c.bench_function("new_is_valid", |b| {
        // let instance = serde_json::json!({"name": "abc"});
        let instance = serde_json::json!({
            "name": "Bob",
            "child": {
                "name": "Ann",
                "child": {
                    "name": "Joe",
                    "child": {
                        "name": "Sam",
                        "child": {
                            "name": "Max",
                            "child": {
                                "name": "Eve",
                                "child": {
                                    "name": "Roy",
                                    "child": {
                                        "name": "Zoe",
                                        "child": {
                                            "name": "Leo",
                                            "child": {
                                                "name": "Amy"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
        b.iter(|| {
            let _ = validator2.is_valid(&instance);
        })
    });
}
criterion_group!(jsonschema, run_benchmarks, bench_is_valid2);
criterion_main!(jsonschema);
