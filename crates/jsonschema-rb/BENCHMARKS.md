# Benchmark Suite

A benchmarking suite for comparing different Ruby JSON Schema implementations.

## Implementations

- `jsonschema_rs` (latest version in this repo)
- [json_schemer](https://rubygems.org/gems/json_schemer) (v2.5.0)
- [json-schema](https://rubygems.org/gems/json-schema) (v6.2.0)
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

| Benchmark        | json-schema                    | rj_schema                      | json_schemer                   | jsonschema_rs | jsonschema_rs (codegen) |
|------------------|--------------------------------|--------------------------------|--------------------------------|---------------|-------------------------|
| OpenAPI          | 2.62 s (**x1039.68**)          | 402.91 ms (**x159.88**)        | 432.74 ms (**x171.72**)        | 2.52 ms       | 2.39 ms                 |
| Swagger          | 3.88 s (**x1021.05**)          | - (4)                          | - (2)                          | 3.80 ms       | 3.44 ms                 |
| Canada (GeoJSON) | - (1)                          | 76.26 ms (**x114.38**)         | 1.04 s (**x1559.88**)          | 666.72 µs     | 552.84 µs               |
| CITM Catalog     | - (1)                          | 17.95 ms (**x24.78**)          | 74.80 ms (**x103.25**)         | 724.49 µs     | 485.09 µs               |
| Fast (Valid)     | - (1)                          | 69.45 µs (**x263.21**)         | 29.05 µs (**x110.10**)         | 263.86 ns     | 180.29 ns               |
| Fast (Invalid)   | - (1)                          | - (3)                          | 30.57 µs (**x506.80**)         | 60.32 ns      | 52.16 ns                |
| FHIR             | 432.18 ms (**x315459.85**)     | 2.14 s (**x1562043.80**)       | 8.32 ms (**x6072.99**)         | 1.37 µs       | 1.16 µs                 |
| Recursive        | - (1)                          | 3.16 ms (**x469.54**)          | 20.72 s (**x3078751.86**)      | 6.73 µs       | 3.88 µs                 |

Notes:

1. `json-schema` does not support Draft 7 schemas.

2. `json_schemer` fails to resolve the Draft 4 meta-schema reference in the Swagger schema.

3. `rj_schema` uses Draft 4 semantics for `exclusiveMaximum` (boolean, not number), producing incorrect results for this Draft 7 schema.

4. `rj_schema` fails to resolve the Draft 4 meta-schema `$ref` in the Swagger schema.

The codegen column is a validator compiled into an extension at build time; see
[Compile-time Validators](#compile-time-validators).

### Compile-time Validators

`#[jsonschema::validator(path = ..., backend = Magnus)]` compiles a schema into a validator when the
extension is built, so nothing is resolved or compiled at run time. The schema is fixed at build
time, which is the trade for the numbers below.

| Benchmark        | `valid?` (runtime) | `valid?` (codegen) | `validate!` (runtime) | `validate!` (codegen) |
|------------------|--------------------|--------------------|-----------------------|-----------------------|
| OpenAPI          | 2.52 ms | 2.39 ms (**x1.05**) | 2.54 ms | 2.42 ms (**x1.05**) |
| Swagger          | 3.80 ms | 3.44 ms (**x1.10**) | 3.94 ms | 3.51 ms (**x1.12**) |
| Canada (GeoJSON) | 666.72 µs | 552.84 µs (**x1.21**) | 730.54 µs | 566.04 µs (**x1.29**) |
| CITM Catalog     | 724.49 µs | 485.09 µs (**x1.49**) | 721.86 µs | 513.03 µs (**x1.41**) |
| Fast (Valid)     | 263.86 ns | 180.29 ns (**x1.46**) | 302.05 ns | 203.82 ns (**x1.48**) |
| Fast (Invalid)   | 60.32 ns | 52.16 ns (**x1.16**) | 1.87 µs | 715.54 ns (**x2.61**) |
| FHIR             | 1.37 µs | 1.16 µs (**x1.18**) | 1.38 µs | 1.19 µs (**x1.16**) |
| Recursive        | 6.73 µs | 3.88 µs (**x1.73**) | 6.87 µs | 3.96 µs (**x1.73**) |

You can find benchmark code in [bench/](bench/), Ruby version `4.0.1`, Rust version `1.99.0`.

## Contributing

Contributions to improve, expand, or optimize the benchmark suite are welcome. This includes adding new benchmarks, ensuring fair representation of real-world use cases, and optimizing the configuration and usage of benchmarked libraries. Such efforts are highly appreciated as they ensure accurate and meaningful performance comparisons.
