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

| Benchmark     | fastjsonschema | jsonschema    | jsonschema-rs (validate) | jsonschema-rs codegen (validate) |
|---------------|----------------|---------------|--------------------------|----------------------------------|
| OpenAPI       | 117.50 ms (**x62.96**) | 562.77 ms (**x301.55**) | 1.87 ms | 938.89 µs |
| Swagger       | 73.74 ms (**x31.79**) | 991.43 ms (**x427.43**) | 2.32 ms | 1.29 ms |
| Canada (GeoJSON) | 9.81 ms (**x15.62**) | 755.65 ms (**x1,203.08**) | 628.09 µs | 457.91 µs |
| CITM Catalog  | 4.43 ms (**x11.30**) | 77.74 ms (**x198.28**) | 392.07 µs | 373.87 µs |
| Fast (Valid)  | 2.00 µs (**x11.34**) | 32.39 µs (**x183.38**) | 176.63 ns | 170.00 ns |
| Fast (Invalid) | 871.00 ns (**x0.87**) | 4.98 µs (**x4.97**) | 1.00 µs | 501.00 ns |
| FHIR          | 2.02 ms (**x510.04**) | 12.09 ms (**x3,047.50**) | 3.97 µs | 577.05 ns |
| Recursive     | 1.01 ms (**x123.50**) | 1.19 s (**x145,948**) | 8.18 µs | 1.87 µs |

The codegen column is a validator compiled into an extension module at build time; see
[Compile-time Validators](#compile-time-validators).

### Compile-time Validators

`#[jsonschema::validator(path = ..., backend = Pyo3)]` compiles a schema into a validator when the
extension is built, so nothing is resolved or compiled at run time. The schema is fixed at build
time, which is the trade for the numbers below.

| Benchmark     | `is_valid` (runtime) | `is_valid` (codegen) | `validate` (runtime) | `validate` (codegen) |
|---------------|----------------------|----------------------|----------------------|----------------------|
| OpenAPI       | 1.84 ms | 934.32 µs (**x1.97**) | 1.87 ms | 938.89 µs (**x1.99**) |
| Swagger       | 2.29 ms | 1.28 ms (**x1.79**) | 2.32 ms | 1.29 ms (**x1.80**) |
| Canada (GeoJSON) | 650.88 µs | 430.19 µs (**x1.51**) | 628.09 µs | 457.91 µs (**x1.37**) |
| CITM Catalog  | 386.04 µs | 191.38 µs (**x2.02**) | 392.07 µs | 373.87 µs (**x1.05**) |
| Fast (Valid)  | 220.00 ns | 149.99 ns (**x1.47**) | 176.63 ns | 170.00 ns (**x1.04**) |
| Fast (Invalid) | 231.00 ns | 147.13 ns (**x1.57**) | 1.00 µs | 501.00 ns (**x2.00**) |
| FHIR          | 3.88 µs | 611.00 ns (**x6.35**) | 3.97 µs | 577.05 ns (**x6.87**) |
| Recursive     | 8.11 µs | 1.85 µs (**x4.38**) | 8.18 µs | 1.87 µs (**x4.36**) |

Compiled validators also skip schema preparation, where `validator_for` takes from 11.38 µs (Fast)
to 23.23 ms (FHIR).

You can find benchmark code in [benches/](benches/), Python version `3.14.7`, Rust version `1.98.0`.

## Contributing

Contributions to improve, expand, or optimize the benchmark suite are welcome. This includes adding new benchmarks, ensuring fair representation of real-world use cases, and optimizing the configuration and usage of benchmarked libraries. Such efforts are highly appreciated as they ensure accurate and meaningful performance comparisons.
