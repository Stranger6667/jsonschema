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
| OpenAPI       | 116.96 ms (**x65.05**) | 554.47 ms (**x308.39**) | 1.80 ms | 915.97 µs |
| Swagger       | 73.26 ms (**x32.49**) | 995.91 ms (**x441.72**) | 2.25 ms | 1.28 ms |
| Canada (GeoJSON) | 9.72 ms (**x15.78**) | 745.86 ms (**x1,210.63**) | 616.09 µs | 304.44 µs |
| CITM Catalog  | 4.54 ms (**x11.54**) | 77.97 ms (**x198.25**) | 393.28 µs | 201.68 µs |
| Fast (Valid)  | 2.02 µs (**x11.25**) | 33.16 µs (**x184.27**) | 179.96 ns | 130.00 ns |
| Fast (Invalid) | 901.00 ns (**x0.87**) | 5.05 µs (**x4.90**) | 1.03 µs | 500.00 ns |
| FHIR          | 2.02 ms (**x1,848.12**) | 11.85 ms (**x10,841.72**) | 1.09 µs | 601.00 ns |
| Recursive     | 1.03 ms (**x122.73**) | 1.20 s (**x142,801**) | 8.42 µs | 1.77 µs |

The codegen column is a validator compiled into an extension module at build time; see
[Compile-time Validators](#compile-time-validators).

### Compile-time Validators

`#[jsonschema::validator(path = ..., backend = Pyo3)]` compiles a schema into a validator when the
extension is built, so nothing is resolved or compiled at run time. The schema is fixed at build
time, which is the trade for the numbers below.

| Benchmark     | `is_valid` (runtime) | `is_valid` (codegen) | `validate` (runtime) | `validate` (codegen) |
|---------------|----------------------|----------------------|----------------------|----------------------|
| OpenAPI       | 1.77 ms | 918.77 µs (**x1.93**) | 1.80 ms | 915.97 µs (**x1.96**) |
| Swagger       | 2.20 ms | 1.32 ms (**x1.67**) | 2.25 ms | 1.28 ms (**x1.76**) |
| Canada (GeoJSON) | 608.92 µs | 306.84 µs (**x1.98**) | 616.09 µs | 304.44 µs (**x2.02**) |
| CITM Catalog  | 382.88 µs | 188.89 µs (**x2.03**) | 393.28 µs | 201.68 µs (**x1.95**) |
| Fast (Valid)  | 220.00 ns | 130.00 ns (**x1.69**) | 179.96 ns | 130.00 ns (**x1.38**) |
| Fast (Invalid) | 229.99 ns | 142.06 ns (**x1.62**) | 1.03 µs | 500.00 ns (**x2.06**) |
| FHIR          | 1.10 µs | 611.00 ns (**x1.79**) | 1.09 µs | 601.00 ns (**x1.82**) |
| Recursive     | 8.23 µs | 1.77 µs (**x4.64**) | 8.42 µs | 1.77 µs (**x4.75**) |

Compiled validators also skip schema preparation, where `validator_for` takes from 11.19 µs (Fast)
to 21.71 ms (FHIR).

You can find benchmark code in [benches/](benches/), Python version `3.14.7`, Rust version `1.98.0`.

## Contributing

Contributions to improve, expand, or optimize the benchmark suite are welcome. This includes adding new benchmarks, ensuring fair representation of real-world use cases, and optimizing the configuration and usage of benchmarked libraries. Such efforts are highly appreciated as they ensure accurate and meaningful performance comparisons.
