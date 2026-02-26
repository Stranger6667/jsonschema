# Benchmark Suite

A benchmarking suite for comparing different Python JSON Schema implementations.

## Implementations

- `jsonschema-rs` (latest version in this repo)
- [jsonschema](https://pypi.org/project/jsonschema/) (v4.26.0)
- [fastjsonschema](https://pypi.org/project/fastjsonschema/) (v2.21.2)

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
| Recursive| Nested data with `$dynamicRef`                 | 1.4 KB      | 449 B         |

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
| OpenAPI       | - (1)          | 536.05 ms (**x83.35**) | 6.43 ms |
| Swagger       | - (1)          | 950.95 ms (**x240.16**) | 3.96 ms |
| Canada (GeoJSON) | 9.05 ms (**x1.82**) | 732.78 ms (**x147.36**) | 4.97 ms |
| CITM Catalog  | 4.32 ms (**x2.51**) | 75.00 ms (**x43.58**) | 1.72 ms |
| Fast (Valid)  | 2.00 µs (**x5.83**) | 32.60 µs (**x94.87**) | 343.65 ns |
| Fast (Invalid)| 2.13 µs (**x3.83**) | 31.97 µs (**x57.44**) | 556.55 ns |
| FHIR          | 1.97 ms (**x440.25**) | 11.78 ms (**x2630**) | 4.48 µs |
| Recursive     | - (2)          | 1.16 s (**x116,115**) | 9.95 µs |

Notes:

1. `fastjsonschema` fails to compile the OpenAPI and Swagger specs due to the presence of the `uri-reference` format (not defined in Draft 4). However, unknown formats are explicitly supported by the spec.

2. `fastjsonschema` does not support `$dynamicRef`.

You can find benchmark code in [benches/](benches/), Python version `3.14.2`, Rust version `1.92`.

## Contributing

Contributions to improve, expand, or optimize the benchmark suite are welcome. This includes adding new benchmarks, ensuring fair representation of real-world use cases, and optimizing the configuration and usage of benchmarked libraries. Such efforts are highly appreciated as they ensure accurate and meaningful performance comparisons.
