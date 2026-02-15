# Benchmark Suite

A benchmarking suite for comparing different Ruby JSON Schema implementations.

## Implementations

- `jsonschema_rs` (latest version in this repo)
- [json_schemer](https://rubygems.org/gems/json_schemer) (v2.5.0)
- [json-schema](https://rubygems.org/gems/json-schema) (v6.1.0)
- [rj_schema](https://rubygems.org/gems/rj_schema) (v1.0.5) - RapidJSON-based (C++)

## Usage

Install the dependencies:

```console
$ bundle install --with benchmark
```

Run the benchmarks:

```console
$ bundle exec ruby bench/benchmark.rb
```

## Overview

| Benchmark | Description                                    | Schema Size | Instance Size |
|-----------|------------------------------------------------|-------------|---------------|
| OpenAPI   | Zuora API validated against OpenAPI 3.0 schema | 18 KB       | 4.5 MB        |
| Swagger   | Kubernetes API (v1.10.0) with Swagger schema   | 25 KB       | 3.0 MB        |
| GeoJSON   | Canadian border in GeoJSON format              | 4.8 KB      | 2.1 MB        |
| CITM      | Concert data catalog with inferred schema      | 2.3 KB      | 501 KB        |
| Fast      | From fastjsonschema benchmarks (valid/invalid) | 595 B       | 55 B / 60 B   |
| FHIR      | Patient example validated against FHIR schema  | 3.3 MB      | 2.1 KB        |
| Recursive | Nested data with `$dynamicRef`                 | 1.4 KB      | 449 B         |

Sources:
- OpenAPI: [Zuora](https://github.com/APIs-guru/openapi-directory/blob/1afd351ddf50e050acdb52937a819ef1927f417a/APIs/zuora.com/2021-04-23/openapi.yaml), [Schema](https://spec.openapis.org/oas/3.0/schema/2021-09-28)
- Swagger: [Kubernetes](https://raw.githubusercontent.com/APIs-guru/openapi-directory/master/APIs/kubernetes.io/v1.10.0/swagger.yaml), [Schema](https://github.com/OAI/OpenAPI-Specification/blob/main/_archive_/schemas/v2.0/schema.json)
- GeoJSON: [Schema](https://geojson.org/schema/FeatureCollection.json)
- CITM: Schema inferred via [infers-jsonschema](https://github.com/Stranger6667/infers-jsonschema)
- Fast: [fastjsonschema benchmarks](https://github.com/horejsek/python-fastjsonschema/blob/master/performance.py#L15)
- FHIR: [Schema](http://hl7.org/fhir/R4/fhir.schema.json.zip) (R4 v4.0.1), [Example](http://hl7.org/fhir/R4/patient-example-d.json.html)

## Methodology

Not all libraries support the same compile-once, validate-many pattern, which affects what each iteration measures:

- **jsonschema_rs** and **json_schemer** both support pre-compiling a schema into a reusable validator object. The benchmark compiles the schema once and measures only validation time.
- **json-schema** only provides class methods (`JSON::Validator.validate`). There is no way to pre-compile a schema into a reusable validator object, so each iteration includes schema processing overhead.
- **rj_schema** accepts the schema as a string argument to `validate()` — the constructor only handles remote `$ref` mappings, not main schema compilation. Each iteration re-parses the schema. Additionally, `rj_schema` operates on JSON strings rather than parsed Ruby objects, so its timings include JSON parsing overhead.

## Results

### Comparison with Other Libraries

| Benchmark        | json-schema              | rj_schema                      | json_schemer                   | jsonschema_rs |
|------------------|--------------------------|--------------------------------|--------------------------------|-----------------------|
| OpenAPI          | 2.25 s (**x205.61**)     | 378.91 ms (**x34.68**)         | 398.68 ms (**x36.48**)         | 10.93 ms              |
| Swagger          | 3.32 s (**x473.37**)     | - (4)                          | - (2)                          | 7.02 ms               |
| Canada (GeoJSON) | - (1)                    | 75.41 ms (**x9.97**)           | 885.18 ms (**x117.01**)        | 7.56 ms               |
| CITM Catalog     | - (1)                    | 17.24 ms (**x6.96**)           | 74.15 ms (**x29.91**)          | 2.48 ms               |
| Fast (Valid)     | - (1)                    | 66.82 µs (**x118.46**)         | 31.64 µs (**x56.09**)          | 564.05 ns             |
| Fast (Invalid)   | - (1)                    | - (3)                          | 31.57 µs (**x70.08**)          | 450.55 ns             |
| FHIR             | 396.47 ms (**x71832.10**)| 2.15 s (**x388783.53**)        | 8.56 ms (**x1550.24**)         | 5.52 µs               |
| Recursive        | - (1)                    | 3.04 ms (**x214.51**)          | 20.73 s (**x1461544.33**)      | 14.19 µs              |

Notes:

1. `json-schema` does not support Draft 7 schemas.

2. `json_schemer` fails to resolve the Draft 4 meta-schema reference in the Swagger schema.

3. `rj_schema` uses Draft 4 semantics for `exclusiveMaximum` (boolean, not number), producing incorrect results for this Draft 7 schema.

4. `rj_schema` fails to resolve the Draft 4 meta-schema `$ref` in the Swagger schema.

You can find benchmark code in [bench/](bench/), Ruby version `4.0.1`, Rust version `1.92`.

## Contributing

Contributions to improve, expand, or optimize the benchmark suite are welcome. This includes adding new benchmarks, ensuring fair representation of real-world use cases, and optimizing the configuration and usage of benchmarked libraries. Such efforts are highly appreciated as they ensure accurate and meaningful performance comparisons.
