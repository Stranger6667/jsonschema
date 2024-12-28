use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use jsonschema::paths::{Location, LocationSegment};

fn benchmark_into_iterator(c: &mut Criterion) {
    let empty = Location::new();
    let small = Location::from_iter(
        vec!["a", "b", "c"]
            .into_iter()
            .map(LocationSegment::from)
            .chain((0..3).map(LocationSegment::from)),
    );
    let large = Location::from_iter(
        (0..500)
            .map(|_| LocationSegment::from("abc"))
            .chain((0..500).map(LocationSegment::from)),
    );

    for (parameter, input) in [("empty", empty), ("small", small), ("large", large)] {
        c.bench_with_input(
            BenchmarkId::new("IntoIterator", parameter),
            &input,
            |b, i| b.iter(|| black_box(i).into_iter().collect::<Vec<_>>()),
        );
    }
}

fn benchmark_from_iterator(c: &mut Criterion) {
    let empty = vec![];
    let small = vec![
        LocationSegment::Property("a"),
        LocationSegment::Property("b"),
        LocationSegment::Property("c"),
    ];
    let large = (0..1000)
        .map(|_| LocationSegment::from("abc"))
        .collect::<Vec<_>>();

    for (parameter, input) in [("empty", empty), ("small", small), ("large", large)] {
        c.bench_with_input(
            BenchmarkId::new("FromIterator", parameter),
            &input,
            |b, i| b.iter(|| Location::from_iter(black_box(i.clone().into_iter()))),
        );
    }
}

criterion_group!(benches, benchmark_into_iterator, benchmark_from_iterator);
criterion_main!(benches);
