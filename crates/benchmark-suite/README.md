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
| OpenAPI   | 969.45 µs           | 517.78 µs          | **1.87x** |
| Swagger   | 1.08 ms             | 529.94 µs          | **2.04x** |
| GeoJSON   | 139.46 µs           | 69.01 µs           | **2.02x** |
| CITM      | 210.40 µs           | 88.71 µs           | **2.37x** |
| Fast (Valid) | 50.97 ns            | 11.85 ns           | **4.30x** |
| Fast (Invalid) | 8.09 ns             | 1.25 ns            | **6.48x** |
| FHIR      | 463.56 ns           | 269.19 ns          | **1.72x** |
| Recursive | 2.65 µs             | 547.69 ns          | **4.83x** |

### Comparison with Other Libraries

| Benchmark     | jsonschema_valid | valico        | boon          | jsonschema (validate) |
|---------------|------------------|---------------|---------------|------------------------|
| OpenAPI       | -                | -             | 6.15 ms (**x6.35**) | 969.28 µs              |
| Swagger       | -                | 103.66 ms (**x88.08**) | 8.98 ms (**x7.63**) | 1.18 ms                |
| GeoJSON       | 16.09 ms (**x116.82**) | 254.95 ms (**x1850.82**) | 15.63 ms (**x113.47**) | 137.75 µs              |
| CITM Catalog  | 2.63 ms (**x11.43**) | 24.95 ms (**x108.44**) | 1.01 ms (**x4.37**) | 230.08 µs              |
| Fast (Valid)  | 1.00 µs (**x16.77**) | 3.21 µs (**x53.80**) | 284.76 ns (**x4.77**) | 59.67 ns               |
| Fast (Invalid) | 224.23 ns (**x5.64**) | 3.31 µs (**x83.13**) | 365.34 ns (**x9.19**) | 39.77 ns               |
| FHIR          | 553.94 ms (**x1123633.34**) | 1.59 ms (**x3235.16**) | 168.29 µs (**x341.37**) | 492.99 ns              |
| Recursive     | -                | -             | 25.03 ms (**x9331.20**) | 2.68 µs                |

Notes:

1. `jsonschema_valid` and `valico` do not handle valid path instances matching the `^\\/` regex.

2. `jsonschema_valid` fails to resolve local references (e.g. `#/definitions/definitions`) in OpenAPI/Swagger schemas.

3. `jsonschema_valid` and `valico` fail to resolve local references in the Recursive schema.

You can find benchmark code in [benches/](benches/) and in the main `jsonschema` crate. Rust version is `1.99.0`.

## Contributing

Contributions to improve, expand, or optimize the benchmark suite are welcome. This includes adding new benchmarks, ensuring fair representation of real-world use cases, and optimizing the configuration and usage of benchmarked libraries. Such efforts are highly appreciated as they ensure accurate and meaningful performance comparisons.
