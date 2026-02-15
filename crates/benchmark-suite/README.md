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
| OpenAPI   | 1.16 ms             | 502.94 µs          | **2.30x** |
| Swagger   | 1.38 ms             | 508.83 µs          | **2.71x** |
| GeoJSON   | 370.51 µs           | 59.959 µs          | **6.18x** |
| CITM      | 346.39 µs           | 182.85 µs          | **1.89x** |
| Fast (Valid) | 64.854 ns        | 19.237 ns          | **3.37x** |
| Fast (Invalid) | 6.0212 ns      | 1.4149 ns          | **4.26x** |
| FHIR      | 3.82 µs             | 2.3284 µs          | **1.64x** |
| Recursive | 6.47 µs             | 702.35 ns          | **9.21x** |

### Comparison with Other Libraries

| Benchmark     | jsonschema_valid | valico        | boon          | jsonschema (validate) |
|---------------|------------------|---------------|---------------|------------------------|
| OpenAPI       | -                | -             | 7.50 ms (**x6.45**) | 1.16 ms            |
| Swagger       | -                | 120.84 ms (**x85.41**)   | 10.46 ms (**x7.39**)     | 1.41 ms            |
| GeoJSON       | 29.15 ms (**x78.65**)      | 256.55 ms (**x692.24**)   | 14.99 ms (**x40.43**)  | 370.61 µs            |
| CITM Catalog  | 4.00 ms (**x10.77**)        | 28.05 ms (**x75.62**)    | 1.05 ms (**x2.84**)     | 371.00 µs            |
| Fast (Valid)  | 1.68 µs (**x19.87**)       | 4.21 µs (**x49.70**)     | 350.39 ns (**x4.14**)   | 84.74 ns            |
| Fast (Invalid)| 269.97 ns (**x9.54**)      | 4.26 µs (**x150.65**)     | 480.63 ns (**x16.99**)   | 28.29 ns            |
| FHIR          | 572.59 ms (**x160559.00**)        | 1.82 ms (**x509.13**)    | 163.69 µs (**x45.90**)     | 3.57 µs            |
| Recursive     | -        | -    | 32.68 ms (**x5058.90**)     | 6.46 µs            |

Notes:

1. `jsonschema_valid` and `valico` do not handle valid path instances matching the `^\\/` regex.

2. `jsonschema_valid` fails to resolve local references (e.g. `#/definitions/definitions`) in OpenAPI/Swagger schemas.

3. `jsonschema_valid` and `valico` fail to resolve local references in the Recursive schema.

You can find benchmark code in [benches/](benches/) and in the main `jsonschema` crate. Rust version is `1.96.1`.

## Contributing

Contributions to improve, expand, or optimize the benchmark suite are welcome. This includes adding new benchmarks, ensuring fair representation of real-world use cases, and optimizing the configuration and usage of benchmarked libraries. Such efforts are highly appreciated as they ensure accurate and meaningful performance comparisons.
