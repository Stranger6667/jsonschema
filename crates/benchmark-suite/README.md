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
| OpenAPI   | 976.26 µs           | 536.43 µs          | **1.82x** |
| Swagger   | 1.08 ms             | 525.78 µs          | **2.06x** |
| GeoJSON   | 147.78 µs           | 82.37 µs           | **1.79x** |
| CITM      | 217.41 µs           | 96.04 µs           | **2.26x** |
| Fast (Valid) | 52.18 ns            | 12.44 ns           | **4.19x** |
| Fast (Invalid) | 6.85 ns             | 1.27 ns            | **5.40x** |
| FHIR      | 340.88 ns           | 251.13 ns          | **1.36x** |
| Recursive | 2.68 µs             | 574.24 ns          | **4.66x** |

### Comparison with Other Libraries

| Benchmark     | jsonschema_valid | valico        | boon          | jsonschema (validate) |
|---------------|------------------|---------------|---------------|------------------------|
| OpenAPI       | 646.67 ms (**x653.59**) | - | 6.44 ms (**x6.51**) | 989.41 µs              |
| Swagger       | - | - | 9.34 ms (**x8.14**) | 1.15 ms                |
| GeoJSON       | 16.24 ms (**x112.68**) | 260.16 ms (**x1805.04**) | 16.03 ms (**x111.24**) | 144.13 µs              |
| CITM Catalog  | 2.57 ms (**x10.69**) | 25.34 ms (**x105.39**) | 1.02 ms (**x4.25**) | 240.44 µs              |
| Fast (Valid)  | 988.65 ns (**x16.40**) | 3.22 µs (**x53.47**) | 293.44 ns (**x4.87**) | 60.28 ns               |
| Fast (Invalid) | 222.48 ns (**x4.93**) | 3.34 µs (**x74.01**) | 379.49 ns (**x8.41**) | 45.13 ns               |
| FHIR          | 551.44 ms (**x1538444.37**) | 1.63 ms (**x4549.16**) | 171.63 µs (**x478.82**) | 358.44 ns              |
| Recursive     | - | - | 25.91 ms (**x9656.69**) | 2.68 µs                |

Notes:

1. `valico` does not support the Draft 4 schemas (OpenAPI, Swagger): it rejects a boolean `exclusiveMinimum` and accepts any value at a `$ref` to the Draft 4 meta-schema. It also ignores 2020-12 keywords (Recursive).

2. `jsonschema_valid` rejects valid Swagger instances under Draft 4 and panics while building the Recursive schema.

You can find benchmark code in [benches/](benches/) and in the main `jsonschema` crate. Rust version is `1.99.0`.

## Contributing

Contributions to improve, expand, or optimize the benchmark suite are welcome. This includes adding new benchmarks, ensuring fair representation of real-world use cases, and optimizing the configuration and usage of benchmarked libraries. Such efforts are highly appreciated as they ensure accurate and meaningful performance comparisons.
