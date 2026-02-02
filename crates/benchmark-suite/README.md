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

### Comparison with Other Libraries

| Benchmark     | jsonschema_valid | valico        | boon          | jsonschema (validate) |
|---------------|------------------|---------------|---------------|------------------------|
| OpenAPI       | -                | -             | 7.05 ms (**x4.90**) | 1.44 ms            |
| Swagger       | -                | 110.63 ms (**x78.46**)   | 10.27 ms (**x7.28**)     | 1.41 ms            |
| GeoJSON       | 16.44 ms (**x31.02**)      | 323.62 ms (**x610.54**)   | 19.08 ms (**x36.00**)  | 530.05 µs            |
| CITM Catalog  | 2.45 ms (**x5.70**)        | 28.33 ms (**x65.88**)    | 1.06 ms (**x2.47**)     | 430.00 µs            |
| Fast (Valid)  | 928.88 ns (**x11.66**)       | 3.34 µs (**x41.92**)     | 327.17 ns (**x4.11**)   | 79.69 ns            |
| Fast (Invalid)| 209.16 ns (**x6.20**)      | 3.42 µs (**x101.45**)     | 394.97 ns (**x11.72**)   | 33.71 ns            |
| FHIR          | 590.04 ms (**x151682.26**)        | 1.68 ms (**x431.88**)    | 179.24 µs (**x46.08**)     | 3.89 µs            |
| Recursive     | -        | -    | 28.48 ms (**x4601.94**)     | 6.19 µs            |

Notes:

1. `jsonschema_valid` and `valico` do not handle valid path instances matching the `^\\/` regex.

2. `jsonschema_valid` fails to resolve local references (e.g. `#/definitions/definitions`) in OpenAPI/Swagger schemas.

3. `jsonschema_valid` and `valico` fail to resolve local references in the Recursive schema.

You can find benchmark code in [benches/](benches/) and in the main `jsonschema` crate. Rust version is `1.92`.

## Contributing

Contributions to improve, expand, or optimize the benchmark suite are welcome. This includes adding new benchmarks, ensuring fair representation of real-world use cases, and optimizing the configuration and usage of benchmarked libraries. Such efforts are highly appreciated as they ensure accurate and meaningful performance comparisons.

