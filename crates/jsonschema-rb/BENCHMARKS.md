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
| OpenAPI          | 2.56 s (**x1033.08**)          | 446.82 ms (**x179.96**)        | 445.83 ms (**x179.57**)        | 2.48 ms       | 2.32 ms                 |
| Swagger          | 3.73 s (**x960.34**)           | - (4)                          | - (2)                          | 3.88 ms       | 3.42 ms                 |
| Canada (GeoJSON) | - (1)                          | 75.55 ms (**x106.83**)         | 877.97 ms (**x1241.45**)       | 707.21 µs     | 503.89 µs               |
| CITM Catalog     | - (1)                          | 17.38 ms (**x24.07**)          | 68.83 ms (**x95.35**)          | 721.86 µs     | 492.45 µs               |
| Fast (Valid)     | - (1)                          | 67.05 µs (**x260.45**)         | 30.43 µs (**x118.21**)         | 257.45 ns     | 170.65 ns               |
| Fast (Invalid)   | - (1)                          | - (3)                          | 29.92 µs (**x492.43**)         | 60.75 ns      | 53.74 ns                |
| FHIR             | 418.06 ms (**x293701.06**)     | 1.96 s (**x1379601.17**)       | 8.31 ms (**x5836.63**)         | 1.42 µs       | 1.18 µs                 |
| Recursive        | - (1)                          | 3.04 ms (**x461.18**)          | 20.38 s (**x3092897.27**)      | 6.59 µs       | 3.66 µs                 |

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
| OpenAPI          | 2.48 ms | 2.32 ms (**x1.07**) | 2.51 ms | 2.33 ms (**x1.08**) |
| Swagger          | 3.88 ms | 3.42 ms (**x1.13**) | 4.06 ms | 3.43 ms (**x1.18**) |
| Canada (GeoJSON) | 707.21 µs | 503.89 µs (**x1.40**) | 702.19 µs | 522.04 µs (**x1.35**) |
| CITM Catalog     | 721.86 µs | 492.45 µs (**x1.47**) | 712.33 µs | 481.88 µs (**x1.48**) |
| Fast (Valid)     | 257.45 ns | 170.65 ns (**x1.51**) | 306.12 ns | 191.42 ns (**x1.60**) |
| Fast (Invalid)   | 60.75 ns | 53.74 ns (**x1.13**) | 1.84 µs | 714.34 ns (**x2.58**) |
| FHIR             | 1.42 µs | 1.18 µs (**x1.20**) | 1.46 µs | 1.19 µs (**x1.23**) |
| Recursive        | 6.59 µs | 3.66 µs (**x1.80**) | 6.69 µs | 3.75 µs (**x1.78**) |

You can find benchmark code in [bench/](bench/), Ruby version `4.0.1`, Rust version `1.99.0`.

## Contributing

Contributions to improve, expand, or optimize the benchmark suite are welcome. This includes adding new benchmarks, ensuring fair representation of real-world use cases, and optimizing the configuration and usage of benchmarked libraries. Such efforts are highly appreciated as they ensure accurate and meaningful performance comparisons.
