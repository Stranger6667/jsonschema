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
| OpenAPI          | 2.70 s (**x958.98**)           | 398.46 ms (**x141.71**)        | 460.61 ms (**x163.81**)        | 2.81 ms       | 2.33 ms                 |
| Swagger          | 3.83 s (**x941.46**)           | - (4)                          | - (2)                          | 4.07 ms       | 3.48 ms                 |
| Canada (GeoJSON) | - (1)                          | 77.12 ms (**x99.93**)          | 974.25 ms (**x1262.35**)       | 771.77 µs     | 622.24 µs               |
| CITM Catalog     | - (1)                          | 18.88 ms (**x26.24**)          | 71.64 ms (**x99.56**)          | 719.57 µs     | 532.89 µs               |
| Fast (Valid)     | - (1)                          | 71.93 µs (**x266.93**)         | 29.75 µs (**x110.40**)         | 269.46 ns     | 192.78 ns               |
| Fast (Invalid)   | - (1)                          | - (3)                          | 32.21 µs (**x528.54**)         | 60.94 ns      | 52.42 ns                |
| FHIR             | 490.38 ms (**x71155.02**)      | 2.22 s (**x321740.57**)        | 9.70 ms (**x1406.82**)         | 6.89 µs       | 1.16 µs                 |
| Recursive        | - (1)                          | 3.19 ms (**x245.27**)          | 20.90 s (**x1607720.80**)      | 13.00 µs      | 3.83 µs                 |

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
| OpenAPI          | 2.81 ms | 2.33 ms (**x1.21**) | 2.84 ms | 2.37 ms (**x1.20**) |
| Swagger          | 4.07 ms | 3.48 ms (**x1.17**) | 4.13 ms | 3.57 ms (**x1.16**) |
| Canada (GeoJSON) | 771.77 µs | 622.24 µs (**x1.24**) | 799.91 µs | 522.41 µs (**x1.53**) |
| CITM Catalog     | 719.57 µs | 532.89 µs (**x1.35**) | 729.61 µs | 530.64 µs (**x1.37**) |
| Fast (Valid)     | 269.46 ns | 192.78 ns (**x1.40**) | 334.99 ns | 208.59 ns (**x1.61**) |
| Fast (Invalid)   | 60.94 ns | 52.42 ns (**x1.16**) | 1.98 µs | 734.63 ns (**x2.70**) |
| FHIR             | 6.89 µs | 1.16 µs (**x5.94**) | 6.85 µs | 1.16 µs (**x5.91**) |
| Recursive        | 13.00 µs | 3.83 µs (**x3.39**) | 13.22 µs | 3.79 µs (**x3.49**) |

You can find benchmark code in [bench/](bench/), Ruby version `4.0.1`, Rust version `1.98.0`.

## Contributing

Contributions to improve, expand, or optimize the benchmark suite are welcome. This includes adding new benchmarks, ensuring fair representation of real-world use cases, and optimizing the configuration and usage of benchmarked libraries. Such efforts are highly appreciated as they ensure accurate and meaningful performance comparisons.
