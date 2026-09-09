#[cfg(not(target_arch = "wasm32"))]
mod bench {
    use benchmark::{read_json, SMALL_SCHEMAS};
    use codspeed_criterion_compat::{criterion_group, Criterion};
    use jsonschema::{
        canonical::{self, CanonicalSchema, CanonicalizeOptions},
        PatternOptions,
    };
    use serde_json::{json, Value};

    /// What a generator canonicalizing an `OpenAPI` document uses: formats assert, and the pattern
    /// engine is the backtracking one with the limits raised.
    fn options() -> CanonicalizeOptions<'static> {
        canonical::options()
            .should_validate_formats(true)
            .with_pattern_options(
                PatternOptions::fancy_regex()
                    .backtrack_limit(usize::MAX)
                    .size_limit(128 * 1024 * 1024)
                    .dfa_size_limit(128 * 1024 * 1024),
            )
    }

    fn bucket(fixture: &Value, name: &str) -> Vec<Value> {
        fixture[name]
            .as_array()
            .expect("the fixture holds this bucket")
            .clone()
    }

    /// One document: prepare it, then canonicalize it. The pair a generator runs per schema.
    fn prepare_and_canonicalize(schema: &Value) -> Option<CanonicalSchema> {
        options().prepare(schema).ok()?.canonicalize().ok()
    }

    pub(crate) fn bench_canonicalize_many(c: &mut Criterion) {
        let fixture = read_json(SMALL_SCHEMAS);
        for name in ["parameters", "properties"] {
            let schemas = bucket(&fixture, name);
            c.bench_function(&format!("canonicalize_many/{name}"), |b| {
                b.iter_with_large_drop(|| {
                    schemas.iter().filter_map(prepare_and_canonicalize).count()
                });
            });
        }
    }

    /// A negative target reads the whole document and then each root branch on its own, so the
    /// branch reads share one prepared document with the whole-document read.
    /// The pointers a document's branches sit at, which the fixture fixes.
    fn branch_pointers(schema: &Value) -> Vec<String> {
        let mut pointers = Vec::new();
        for key in ["oneOf", "anyOf"] {
            let Some(branches) = schema.get(key).and_then(Value::as_array) else {
                continue;
            };
            for index in 0..branches.len() {
                pointers.push(format!("/{key}/{index}"));
            }
        }
        pointers
    }

    pub(crate) fn bench_canonicalize_branches(c: &mut Criterion) {
        let schemas = bucket(&read_json(SMALL_SCHEMAS), "branches");
        // Built once: writing the pointers is the fixture's shape, not work a caller repeats.
        let cases: Vec<(Value, Vec<String>)> = schemas
            .into_iter()
            .map(|schema| {
                let pointers = branch_pointers(&schema);
                (schema, pointers)
            })
            .collect();
        c.bench_function("canonicalize_many/branches", |b| {
            b.iter_with_large_drop(|| {
                let mut reads = 0_usize;
                for (schema, pointers) in &cases {
                    let Ok(prepared) = options().prepare(schema) else {
                        continue;
                    };
                    if prepared.canonicalize().is_ok() {
                        reads += 1;
                    }
                    for pointer in pointers {
                        if prepared.canonicalize_at(pointer).is_ok() {
                            reads += 1;
                        }
                    }
                }
                reads
            });
        });
    }

    /// Every positive target intersects the parameter's form with a small constraint restating one
    /// keyword, so the operands are one large-ish form and one tiny one.
    pub(crate) fn bench_constraint_intersect(c: &mut Criterion) {
        let schemas = bucket(&read_json(SMALL_SCHEMAS), "properties");
        let canonical: Vec<CanonicalSchema> = schemas
            .iter()
            .filter_map(prepare_and_canonicalize)
            .take(200)
            .collect();
        let constraint = options()
            .canonicalize(&json!({"type": "object", "required": ["a"]}))
            .expect("a valid constraint");
        c.bench_function("canonicalize_many/constraint_intersect", |b| {
            b.iter_with_large_drop(|| {
                canonical
                    .iter()
                    .filter(|schema| schema.intersect(&constraint).is_ok())
                    .count()
            });
        });
    }

    criterion_group!(
        benches,
        bench_canonicalize_many,
        bench_canonicalize_branches,
        bench_constraint_intersect
    );
}

#[cfg(not(target_arch = "wasm32"))]
codspeed_criterion_compat::criterion_main!(bench::benches);

#[cfg(target_arch = "wasm32")]
fn main() {}
