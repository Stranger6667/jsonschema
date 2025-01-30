use benchmark::Benchmark;
use codspeed_criterion_compat::{criterion_group, criterion_main, BenchmarkId, Criterion};
use serde_json::Value;

fn bench_build(c: &mut Criterion, name: &str, schema: &Value) {
    c.bench_with_input(BenchmarkId::new("build", name), schema, |b, schema| {
        b.iter(|| jsonschema::validator_for(schema).expect("Valid schema"))
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

fn bench_compare(c: &mut Criterion) {
    for (parameter, schema, instance, instructions) in [
        (
            "minmax",
            serde_json::json!({"minimum": 5, "maximum": 10}),
            serde_json::json!(7),
            vec![
                jsonschema::Instruction::MaximumU64 { limit: 10 },
                jsonschema::Instruction::MinimumU64 { limit: 5 },
            ],
        ),
        (
            "properties",
            serde_json::json!({"properties": {"foo": {"maximum": 1}, "bar": {"minimum": 3}}}),
            serde_json::json!({"foo": 0, "bar": 4}),
            vec![
                jsonschema::Instruction::Properties { start: 0, end: 2 }, // For "foo" and "bar"
                jsonschema::Instruction::MinimumU64 { limit: 3 },         // Validate "bar"
                jsonschema::Instruction::Pop, // Return to parent context
                jsonschema::Instruction::MaximumU64 { limit: 1 }, // Validate "foo"
                jsonschema::Instruction::Pop, // Return to parent context
            ],
        ),
        (
            "properties - 4",
            serde_json::json!({"properties": {"foo": {"maximum": 1}, "bar": {"minimum": 3}, "spam": {"maximum": 4}, "baz": {"minimum": 2}}}),
            serde_json::json!({"foo": 0, "bar": 4, "spam": 3, "baz": 2}),
            vec![
                jsonschema::Instruction::Properties { start: 0, end: 4 }, // For "foo" and "bar"
                jsonschema::Instruction::MinimumU64 { limit: 2 },         // Validate "baz"
                jsonschema::Instruction::Pop, // Return to parent context
                jsonschema::Instruction::MaximumU64 { limit: 4 }, // Validate "spam"
                jsonschema::Instruction::Pop, // Return to parent context
                jsonschema::Instruction::MinimumU64 { limit: 3 }, // Validate "bar"
                jsonschema::Instruction::Pop, // Return to parent context
                jsonschema::Instruction::MaximumU64 { limit: 1 }, // Validate "foo"
                jsonschema::Instruction::Pop, // Return to parent context
            ],
        ),
    ] {
        let validator = jsonschema::validator_for(&schema).expect("Valid schema");
        c.bench_with_input(
            BenchmarkId::new("compare", &format!("current/{parameter}")),
            &instance,
            |b, instance| {
                b.iter(|| {
                    let _ = validator.is_valid(instance);
                })
            },
        );
        c.bench_with_input(
            BenchmarkId::new("compare", &format!("vm/{parameter}")),
            &(instance, instructions),
            |b, (instance, instructions)| {
                let mut vm = jsonschema::VirtualMachine::new();
                b.iter(|| {
                    let _ = vm.execute(instructions, instance);
                })
            },
        );
    }
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
        b.iter(|| {
            let _ = validator.apply(instance).basic();
        })
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

criterion_group!(jsonschema, run_benchmarks, bench_compare);
criterion_main!(jsonschema);
