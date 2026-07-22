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

| Benchmark        | json-schema                    | rj_schema                      | json_schemer                   | jsonschema_rs |
|------------------|--------------------------------|--------------------------------|--------------------------------|---------------|
| OpenAPI          | 2.46 s (**x897.40**)           | 450.43 ms (**x164.04**)        | 435.99 ms (**x158.78**)        | 2.75 ms       |
| Swagger          | 3.59 s (**x919.17**)           | - (4)                          | - (2)                          | 3.90 ms       |
| Canada (GeoJSON) | - (1)                          | 78.77 ms (**x154.26**)         | 1.16 s (**x2279.91**)          | 510.62 µs     |
| CITM Catalog     | - (1)                          | 17.62 ms (**x23.34**)          | 70.06 ms (**x92.82**)          | 754.81 µs     |
| Fast (Valid)     | - (1)                          | 66.49 µs (**x277.43**)         | 30.03 µs (**x125.30**)         | 239.67 ns     |
| Fast (Invalid)   | - (1)                          | - (3)                          | 30.59 µs (**x516.18**)         | 59.26 ns      |
| FHIR             | 437.54 ms (**x66979.36**)      | 2.10 s (**x320719.22**)        | 8.41 ms (**x1287.26**)         | 6.53 µs       |
| Recursive        | - (1)                          | 3.05 ms (**x238.10**)          | 21.44 s (**x1673140.33**)      | 12.82 µs      |

Notes:

1. `json-schema` does not support Draft 7 schemas.

2. `json_schemer` fails to resolve the Draft 4 meta-schema reference in the Swagger schema.

3. `rj_schema` uses Draft 4 semantics for `exclusiveMaximum` (boolean, not number), producing incorrect results for this Draft 7 schema.

4. `rj_schema` fails to resolve the Draft 4 meta-schema `$ref` in the Swagger schema.

You can find benchmark code in [bench/](bench/), Ruby version `4.0.1`, Rust version `1.97.1`.

## Contributing

Contributions to improve, expand, or optimize the benchmark suite are welcome. This includes adding new benchmarks, ensuring fair representation of real-world use cases, and optimizing the configuration and usage of benchmarked libraries. Such efforts are highly appreciated as they ensure accurate and meaningful performance comparisons.
