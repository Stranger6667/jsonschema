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
| OpenAPI       | 116.40 ms (**x84.16**) | 564.67 ms (**x408.30**) | 1.38 ms | 923.15 µs |
| Swagger       | 74.00 ms (**x39.30**) | 1.00 s (**x533.64**) | 1.88 ms | 1.26 ms |
| Canada (GeoJSON) | 9.86 ms (**x18.57**) | 771.32 ms (**x1,453.04**) | 530.83 µs | 338.40 µs |
| CITM Catalog  | 4.52 ms (**x12.36**) | 80.06 ms (**x218.74**) | 366.01 µs | 204.06 µs |
| Fast (Valid)  | 1.98 µs (**x12.17**) | 33.38 µs (**x204.85**) | 162.97 ns | 149.99 ns |
| Fast (Invalid) | 862.00 ns (**x0.89**) | 4.97 µs (**x5.11**) | 972.00 ns | 490.01 ns |
| FHIR          | 2.02 ms (**x2,618.84**) | 12.29 ms (**x15,939.98**) | 770.99 ns | 610.99 ns |
| Recursive     | 1.02 ms (**x227.05**) | 1.21 s (**x269,141.80**) | 4.50 µs | 1.63 µs |

The codegen column is a validator compiled into an extension module at build time; see
[Compile-time Validators](#compile-time-validators).

### Compile-time Validators

`#[jsonschema::validator(path = ..., backend = Pyo3)]` compiles a schema into a validator when the
extension is built, so nothing is resolved or compiled at run time. The schema is fixed at build
time, which is the trade for the numbers below.

| Benchmark     | `is_valid` (runtime) | `is_valid` (codegen) | `validate` (runtime) | `validate` (codegen) |
|---------------|----------------------|----------------------|----------------------|----------------------|
| OpenAPI       | 1.36 ms | 917.26 µs (**x1.49**) | 1.38 ms | 923.15 µs (**x1.50**) |
| Swagger       | 1.81 ms | 1.23 ms (**x1.48**) | 1.88 ms | 1.26 ms (**x1.49**) |
| Canada (GeoJSON) | 530.71 µs | 345.53 µs (**x1.54**) | 530.83 µs | 338.40 µs (**x1.57**) |
| CITM Catalog  | 379.75 µs | 193.51 µs (**x1.96**) | 366.01 µs | 204.06 µs (**x1.79**) |
| Fast (Valid)  | 199.99 ns | 139.99 ns (**x1.43**) | 162.97 ns | 149.99 ns (**x1.09**) |
| Fast (Invalid) | 240.00 ns | 149.66 ns (**x1.60**) | 972.00 ns | 490.01 ns (**x1.98**) |
| FHIR          | 771.00 ns | 621.00 ns (**x1.24**) | 770.99 ns | 610.99 ns (**x1.26**) |
| Recursive     | 4.41 µs | 1.62 µs (**x2.72**) | 4.50 µs | 1.63 µs (**x2.75**) |

Compiled validators also skip schema preparation, where `validator_for` takes from 10.51 µs (Fast)
to 17.23 ms (FHIR).

You can find benchmark code in [benches/](benches/), Python version `3.14.7`, Rust version `1.99.0`.

## Contributing

Contributions to improve, expand, or optimize the benchmark suite are welcome. This includes adding new benchmarks, ensuring fair representation of real-world use cases, and optimizing the configuration and usage of benchmarked libraries. Such efforts are highly appreciated as they ensure accurate and meaningful performance comparisons.
