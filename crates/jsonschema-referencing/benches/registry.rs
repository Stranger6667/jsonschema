use codspeed_criterion_compat::{
    criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion,
};
use referencing::{Draft, Registry, SPECIFICATIONS};

static DRAFT4: &[u8] = include_bytes!("../../benchmark/data/subresources/draft4.json");
static DRAFT6: &[u8] = include_bytes!("../../benchmark/data/subresources/draft6.json");
static DRAFT7: &[u8] = include_bytes!("../../benchmark/data/subresources/draft7.json");
static DRAFT201909: &[u8] = include_bytes!("../../benchmark/data/subresources/draft201909.json");
static DRAFT202012: &[u8] = include_bytes!("../../benchmark/data/subresources/draft202012.json");

fn bench_subresources(c: &mut Criterion) {
    let drafts = [
        (Draft::Draft4, DRAFT4, "draft 4"),
        (Draft::Draft6, DRAFT6, "draft 6"),
        (Draft::Draft7, DRAFT7, "draft 7"),
        (Draft::Draft201909, DRAFT201909, "draft 2019-09"),
        (Draft::Draft202012, DRAFT202012, "draft 2020-12"),
    ];

    let mut group = c.benchmark_group("registry");

    for (draft, data, name) in &drafts {
        let schema = benchmark::read_json(data);

        let schema_ref = draft.create_resource_ref(&schema);
        group.bench_with_input(
            BenchmarkId::new("try_new", name),
            &schema_ref,
            |b, schema_ref| {
                b.iter(|| {
                    Registry::try_new(
                        "http://example.com/schema.json",
                        (schema_ref.contents(), schema_ref.draft()),
                    )
                    .expect("Invalid registry input")
                });
            },
        );
    }
    let drafts = [
        (Draft::Draft4, benchmark::GEOJSON, "GeoJSON"),
        (Draft::Draft4, benchmark::SWAGGER, "Swagger"),
        (Draft::Draft4, benchmark::OPEN_API, "Open API"),
        (Draft::Draft4, benchmark::CITM_SCHEMA, "CITM"),
        (Draft::Draft7, benchmark::FAST_SCHEMA, "Fast"),
    ];

    for (draft, data, name) in &drafts {
        let schema = benchmark::read_json(data);

        group.bench_with_input(
            BenchmarkId::new("try_with_resource", name),
            &schema,
            |b, schema| {
                b.iter_batched(
                    || SPECIFICATIONS.clone(),
                    |registry| {
                        registry
                            .try_with_resource("http://example.com/schema.json", (schema, *draft))
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_subresources);
criterion_main!(benches);
