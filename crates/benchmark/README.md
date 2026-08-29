# benchmark

A helper crate for running JSON Schema validation benchmarks across multiple libraries.

## Features

- Predefined set of JSON schemas and instances for benchmarking
- Easy-to-use API for running benchmarks uniformly across different libraries

## Usage

```rust
use benchmark::Benchmark;

for benchmark in Benchmark::iter() {
    benchmark.run(&mut |schema_name, instance_name, schema, instance| {
        // Your benchmarking code here
    });
}
```
