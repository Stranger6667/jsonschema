# Benchmark Suite

A benchmarking suite for comparing different Rust JSON Schema implementations.

## Implementations

- `jsonschema` (latest version in this repo)
- [valico](https://crates.io/crates/valico) (v4.0.0)
- [jsonschema-valid](https://crates.io/crates/jsonschema-valid) (v0.5.2)
- [boon](https://crates.io/crates/boon) (v0.6.1)

## Usage

To run the benchmarks:

```console
$ cargo bench
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

### `jsonschema`: Dynamic `is_valid` vs Codegen `is_valid`

| Benchmark | Dynamic `is_valid` | Codegen `is_valid` | Speedup |
|-----------|---------------------|--------------------|---------|
| OpenAPI   | 965.22 µs           | 534.57 µs          | **1.81x** |
| Swagger   | 1.09 ms             | 528.77 µs          | **2.06x** |
| GeoJSON   | 135.99 µs           | 76.69 µs           | **1.77x** |
| CITM      | 216.35 µs           | 94.55 µs           | **2.29x** |
| Fast (Valid) | 51.04 ns            | 12.59 ns           | **4.05x** |
| Fast (Invalid) | 6.64 ns             | 1.54 ns            | **4.30x** |
| FHIR      | 357.43 ns           | 252.53 ns          | **1.42x** |
| Recursive | 2.55 µs             | 568.57 ns          | **4.49x** |

### Comparison with Other Libraries

| Benchmark     | jsonschema_valid | valico        | boon          | jsonschema (validate) |
|---------------|------------------|---------------|---------------|------------------------|
| OpenAPI       | 638.48 ms (**x655.71**) | - | 6.16 ms (**x6.32**) | 973.73 µs              |
| Swagger       | - | - | 9.10 ms (**x8.01**) | 1.14 ms                |
| GeoJSON       | 16.39 ms (**x118.15**) | 255.66 ms (**x1842.73**) | 15.87 ms (**x114.36**) | 138.74 µs              |
| CITM Catalog  | 2.52 ms (**x10.72**) | 26.41 ms (**x112.20**) | 1.01 ms (**x4.30**) | 235.40 µs              |
| Fast (Valid)  | 997.44 ns (**x16.68**) | 3.25 µs (**x54.41**) | 285.93 ns (**x4.78**) | 59.80 ns               |
| Fast (Invalid) | 219.39 ns (**x6.14**) | 3.35 µs (**x93.73**) | 385.81 ns (**x10.80**) | 35.72 ns               |
| FHIR          | 549.97 ms (**x1470114.94**) | 1.64 ms (**x4371.02**) | 170.19 µs (**x454.93**) | 374.10 ns              |
| Recursive     | - | - | 25.25 ms (**x9795.96**) | 2.58 µs                |

Notes:

1. `valico` does not support the Draft 4 schemas (OpenAPI, Swagger): it rejects a boolean `exclusiveMinimum` and accepts any value at a `$ref` to the Draft 4 meta-schema. It also ignores 2020-12 keywords (Recursive).

2. `jsonschema_valid` rejects valid Swagger instances under Draft 4 and panics while building the Recursive schema.

You can find benchmark code in [benches/](benches/) and in the main `jsonschema` crate. Rust version is `1.99.0`.

## Contributing

Contributions to improve, expand, or optimize the benchmark suite are welcome. This includes adding new benchmarks, ensuring fair representation of real-world use cases, and optimizing the configuration and usage of benchmarked libraries. Such efforts are highly appreciated as they ensure accurate and meaningful performance comparisons.
