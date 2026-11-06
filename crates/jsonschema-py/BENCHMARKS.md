# Benchmark Suite

A benchmarking suite for comparing different Python JSON Schema implementations.

## Implementations

- `jsonschema-rs` (latest version in this repo)
- [jsonschema](https://pypi.org/project/jsonschema/) (v4.26.0)
- [fastjsonschema](https://pypi.org/project/fastjsonschema/) (v2.22.3)

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
| OpenAPI       | 115.89 ms (**x75.93**) | 562.58 ms (**x368.57**) | 1.53 ms | 985.04 µs |
| Swagger       | 73.20 ms (**x33.64**) | 994.87 ms (**x457.18**) | 2.18 ms | 1.34 ms |
| Canada (GeoJSON) | 9.82 ms (**x14.71**) | 776.35 ms (**x1,162.93**) | 667.58 µs | 339.17 µs |
| CITM Catalog  | 4.48 ms (**x10.89**) | 77.58 ms (**x188.49**) | 411.55 µs | 220.03 µs |
| Fast (Valid)  | 2.06 µs (**x10.57**) | 33.66 µs (**x172.45**) | 195.19 ns | 160.00 ns |
| Fast (Invalid) | 902.00 ns (**x0.87**) | 5.21 µs (**x5.00**) | 1.04 µs | 521.00 ns |
| FHIR          | 2.02 ms (**x1,924.30**) | 12.16 ms (**x11,557.69**) | 1.05 µs | 711.00 ns |
| Recursive     | 1.03 ms (**x188.20**) | 1.22 s (**x224,068.26**) | 5.46 µs | 1.97 µs |

The codegen column is a validator compiled into an extension module at build time; see
[Compile-time Validators](#compile-time-validators).

### Compile-time Validators

`#[jsonschema::validator(path = ..., backend = Pyo3)]` compiles a schema into a validator when the
extension is built, so nothing is resolved or compiled at run time. The schema is fixed at build
time, which is the trade for the numbers below.

| Benchmark     | `is_valid` (runtime) | `is_valid` (codegen) | `validate` (runtime) | `validate` (codegen) |
|---------------|----------------------|----------------------|----------------------|----------------------|
| OpenAPI       | 1.52 ms | 993.41 µs (**x1.53**) | 1.53 ms | 985.04 µs (**x1.55**) |
| Swagger       | 2.06 ms | 1.33 ms (**x1.55**) | 2.18 ms | 1.34 ms (**x1.62**) |
| Canada (GeoJSON) | 622.62 µs | 339.49 µs (**x1.83**) | 667.58 µs | 339.17 µs (**x1.97**) |
| CITM Catalog  | 422.17 µs | 213.93 µs (**x1.97**) | 411.55 µs | 220.03 µs (**x1.87**) |
| Fast (Valid)  | 231.00 ns | 150.00 ns (**x1.54**) | 195.19 ns | 160.00 ns (**x1.22**) |
| Fast (Invalid) | 280.00 ns | 152.09 ns (**x1.84**) | 1.04 µs | 521.00 ns (**x2.00**) |
| FHIR          | 1.05 µs | 702.00 ns (**x1.50**) | 1.05 µs | 711.00 ns (**x1.48**) |
| Recursive     | 5.34 µs | 1.96 µs (**x2.72**) | 5.46 µs | 1.97 µs (**x2.77**) |

Compiled validators also skip schema preparation, where `validator_for` takes from 11.15 µs (Fast)
to 19.13 ms (FHIR).

You can find benchmark code in [benches/](benches/), Python version `3.14.7`, Rust version `1.99.0`.

## Contributing

Contributions to improve, expand, or optimize the benchmark suite are welcome. This includes adding new benchmarks, ensuring fair representation of real-world use cases, and optimizing the configuration and usage of benchmarked libraries. Such efforts are highly appreciated as they ensure accurate and meaningful performance comparisons.
