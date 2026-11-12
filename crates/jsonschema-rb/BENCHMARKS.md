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
| OpenAPI          | 2.68 s (**x1080.65**)          | 400.25 ms (**x161.39**)        | 404.86 ms (**x163.25**)        | 2.48 ms       | 2.33 ms                 |
| Swagger          | 3.86 s (**x987.21**)           | - (4)                          | - (2)                          | 3.91 ms       | 3.42 ms                 |
| Canada (GeoJSON) | - (1)                          | 76.57 ms (**x101.47**)         | 897.25 ms (**x1189.02**)       | 754.61 µs     | 538.75 µs               |
| CITM Catalog     | - (1)                          | 17.88 ms (**x25.01**)          | 68.74 ms (**x96.16**)          | 714.87 µs     | 468.06 µs               |
| Fast (Valid)     | - (1)                          | 68.73 µs (**x258.88**)         | 29.29 µs (**x110.32**)         | 265.49 ns     | 194.96 ns               |
| Fast (Invalid)   | - (1)                          | - (3)                          | 30.37 µs (**x525.98**)         | 57.74 ns      | 51.63 ns                |
| FHIR             | 444.81 ms (**x342161.54**)     | 2.13 s (**x1638461.54**)       | 8.40 ms (**x6461.54**)         | 1.30 µs       | 1.17 µs                 |
| Recursive        | - (1)                          | 3.18 ms (**x447.89**)          | 20.53 s (**x2891549.30**)      | 7.10 µs       | 3.83 µs                 |

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
| OpenAPI          | 2.48 ms | 2.33 ms (**x1.06**) | 2.50 ms | 2.36 ms (**x1.06**) |
| Swagger          | 3.91 ms | 3.42 ms (**x1.14**) | 4.00 ms | 3.50 ms (**x1.14**) |
| Canada (GeoJSON) | 754.61 µs | 538.75 µs (**x1.40**) | 687.44 µs | 577.83 µs (**x1.19**) |
| CITM Catalog     | 714.87 µs | 468.06 µs (**x1.53**) | 698.68 µs | 482.75 µs (**x1.45**) |
| Fast (Valid)     | 265.49 ns | 194.96 ns (**x1.36**) | 289.94 ns | 204.52 ns (**x1.42**) |
| Fast (Invalid)   | 57.74 ns | 51.63 ns (**x1.12**) | 1.84 µs | 708.61 ns (**x2.60**) |
| FHIR             | 1.30 µs | 1.17 µs (**x1.11**) | 1.36 µs | 1.21 µs (**x1.12**) |
| Recursive        | 7.10 µs | 3.83 µs (**x1.85**) | 7.14 µs | 3.87 µs (**x1.84**) |

You can find benchmark code in [bench/](bench/), Ruby version `4.0.1`, Rust version `1.99.0`.

## Contributing

Contributions to improve, expand, or optimize the benchmark suite are welcome. This includes adding new benchmarks, ensuring fair representation of real-world use cases, and optimizing the configuration and usage of benchmarked libraries. Such efforts are highly appreciated as they ensure accurate and meaningful performance comparisons.
