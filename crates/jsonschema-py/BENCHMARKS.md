# Benchmark Suite

A benchmarking suite for comparing different Python JSON Schema implementations.

## Implementations

- `jsonschema-rs` (latest version in this repo)
- [jsonschema](https://pypi.org/project/jsonschema/) (v4.26.0)
- [fastjsonschema](https://pypi.org/project/fastjsonschema/) (v2.22.2)

## Usage

Install the dependencies:

```console
$ pip install -e ".[bench]"
```

Run the benchmarks:

```console
$ pytest benches/bench.py
```

`jsonschema-rs` must be built in release mode:

```console
$ maturin develop --release
```

The compile-time validators live in a separate extension. Cargo names the artifact `lib*.so`, so it
has to be copied under the module name before Python can import it; without it the
`jsonschema-rs-codegen-*` variants are skipped:

```console
$ cargo build --release -p jsonschema-bench-pyo3
$ mkdir -p .codegen
$ cp ../../target/release/libjsonschema_bench_pyo3.so .codegen/jsonschema_bench_pyo3.so
$ PYTHONPATH=.codegen pytest benches/bench.py
```

On macOS the artifact is `libjsonschema_bench_pyo3.dylib`, and it still has to land as
`jsonschema_bench_pyo3.so`.

## Overview

| Benchmark     | Description                                    | Schema Size | Instance Size |
|----------|------------------------------------------------|-------------|---------------|
| OpenAPI  | Zuora API validated against OpenAPI 3.0 schema | 18 KB       | 4.5 MB        |
| Swagger  | Kubernetes API (v1.10.0) with Swagger schema   | 25 KB       | 3.0 MB        |
| GeoJSON  | Canadian border in GeoJSON format              | 4.8 KB      | 2.1 MB        |
| CITM     | Concert data catalog with inferred schema      | 2.3 KB      | 501 KB        |
| Fast     | From fastjsonschema benchmarks (valid/invalid) | 595 B       | 55 B / 60 B   |
| FHIR     | Patient example validated against FHIR schema  | 3.3 MB      | 2.1 KB        |
| Recursive| Nested data with a self-recursive `$ref`       | 1.4 KB      | 449 B         |

Sources:
- OpenAPI: [Zuora](https://github.com/APIs-guru/openapi-directory/blob/1afd351ddf50e050acdb52937a819ef1927f417a/APIs/zuora.com/2021-04-23/openapi.yaml), [Schema](https://spec.openapis.org/oas/3.0/schema/2021-09-28)
- Swagger: [Kubernetes](https://raw.githubusercontent.com/APIs-guru/openapi-directory/master/APIs/kubernetes.io/v1.10.0/swagger.yaml), [Schema](https://github.com/OAI/OpenAPI-Specification/blob/main/_archive_/schemas/v2.0/schema.json)
- GeoJSON: [Schema](https://geojson.org/schema/FeatureCollection.json)
- CITM: Schema inferred via [infers-jsonschema](https://github.com/Stranger6667/infers-jsonschema)
- Fast: [fastjsonschema benchmarks](https://github.com/horejsek/python-fastjsonschema/blob/master/performance.py#L15)
- FHIR: [Schema](http://hl7.org/fhir/R4/fhir.schema.json.zip) (R4 v4.0.1), [Example](http://hl7.org/fhir/R4/patient-example-d.json.html)

## Results

### Comparison with Other Libraries

| Benchmark     | fastjsonschema | jsonschema    | jsonschema-rs (validate) |
|---------------|----------------|---------------|--------------------------|
| OpenAPI       | 118.01 ms (**x58.36**) | 570.63 ms (**x282.20**) | 2.02 ms |
| Swagger       | 73.67 ms (**x29.18**) | 985.98 ms (**x390.58**) | 2.52 ms |
| Canada (GeoJSON) | 10.01 ms (**x15.28**) | 687.54 ms (**x1,049.85**) | 0.65 ms |
| CITM Catalog  | 4.62 ms (**x8.06**) | 78.94 ms (**x137.60**) | 0.57 ms |
| Fast (Valid)  | 2.09 µs (**x6.89**) | 33.78 µs (**x111.45**) | 303.10 ns |
| Fast (Invalid)| 977.92 ns (**x0.91**) | 5.23 µs (**x4.86**) | 1.08 µs |
| FHIR          | 2.04 ms (**x470.16**) | 12.34 ms (**x2,840.53**) | 4.34 µs |
| Recursive     | 1.03 ms (**x114.37**) | 1.20 s (**x133,784**) | 9.00 µs |

### Compile-time Validators

`#[jsonschema::validator(path = ..., backend = Pyo3)]` compiles a schema into a validator when the
extension is built, so nothing is resolved or compiled at run time. The schema is fixed at build
time, which is the trade for the numbers below.

| Benchmark     | `is_valid` (runtime) | `is_valid` (codegen) | `validate` (runtime) | `validate` (codegen) |
|---------------|----------------------|----------------------|----------------------|----------------------|
| OpenAPI       | 1.82 ms              | 855.71 µs (**x2.13**) | 1.83 ms             | 865.40 µs (**x2.11**) |
| Swagger       | 2.16 ms              | 1.16 ms (**x1.86**)   | 2.25 ms             | 1.20 ms (**x1.87**)   |
| Canada (GeoJSON) | 636.73 µs         | 417.41 µs (**x1.53**) | 668.50 µs           | 423.11 µs (**x1.58**) |
| CITM Catalog  | 391.49 µs            | 189.70 µs (**x2.06**) | 547.40 µs           | 414.17 µs (**x1.32**) |
| Fast (Valid)  | 230.00 ns            | 170.00 ns (**x1.35**) | 280.00 ns           | 180.00 ns (**x1.56**) |
| Fast (Invalid)| 280.00 ns            | 177.74 ns (**x1.58**) | 1.11 µs             | 581.00 ns (**x1.91**) |
| FHIR          | 3.95 µs              | 601.00 ns (**x6.57**) | 3.98 µs             | 611.00 ns (**x6.51**) |
| Recursive     | 8.01 µs              | 1.86 µs (**x4.29**)   | 8.17 µs             | 1.87 µs (**x4.36**)   |

Schema preparation disappears altogether, since the validator is built into the extension:

| Benchmark     | `validator_for` (runtime) | codegen |
|---------------|---------------------------|---------|
| OpenAPI       | 578.74 µs                 | none    |
| Swagger       | 656.67 µs                 | none    |
| Canada (GeoJSON) | 98.81 µs               | none    |
| CITM Catalog  | 36.55 µs                  | none    |
| Fast          | 10.94 µs                  | none    |
| FHIR          | 22.24 ms                  | none    |
| Recursive     | 193.85 µs                 | none    |

Building the seven validators takes about 2 minutes and 3 GB of peak memory, most of it the 3.3 MB
FHIR schema.

You can find benchmark code in [benches/](benches/), Python version `3.14.7`, Rust version `1.98.0`.

## Contributing

Contributions to improve, expand, or optimize the benchmark suite are welcome. This includes adding new benchmarks, ensuring fair representation of real-world use cases, and optimizing the configuration and usage of benchmarked libraries. Such efforts are highly appreciated as they ensure accurate and meaningful performance comparisons.
