use serde_json::Value;
use std::sync::LazyLock;
use strum::{EnumIter, IntoEnumIterator};

pub static OPEN_API: &[u8] = include_bytes!("../data/openapi.json");
pub static SWAGGER: &[u8] = include_bytes!("../data/swagger.json");
pub static GEOJSON: &[u8] = include_bytes!("../data/geojson.json");
pub static CITM_SCHEMA: &[u8] = include_bytes!("../data/citm_catalog_schema.json");
pub static FAST_SCHEMA: &[u8] = include_bytes!("../data/fast_schema.json");

static ZUORA: &[u8] = include_bytes!("../data/zuora.json");
static KUBERNETES: &[u8] = include_bytes!("../data/kubernetes.json");
static CANADA: &[u8] = include_bytes!("../data/canada.json");
static CITM: &[u8] = include_bytes!("../data/citm_catalog.json");
static FAST_VALID: &[u8] = include_bytes!("../data/fast_valid.json");
static FAST_INVALID: &[u8] = include_bytes!("../data/fast_invalid.json");

pub fn read_json(slice: &[u8]) -> Value {
    serde_json::from_slice(slice).expect("Invalid JSON")
}

#[derive(Debug, Clone, Copy, EnumIter)]
pub enum Benchmark {
    OpenAPI,
    Swagger,
    GeoJSON,
    CITM,
    Fast,
}

type BenchFunc<'a> = dyn FnMut(&str, &Value, &[BenchInstance]) + 'a;

impl Benchmark {
    pub fn iter() -> impl Iterator<Item = Benchmark> {
        <Benchmark as IntoEnumIterator>::iter()
    }
    pub fn run(self, bench: &mut BenchFunc) {
        BENCHMARK_SUITE.run(self, bench)
    }
}

struct BenchData {
    name: &'static str,
    schema: Value,
    instances: Vec<BenchInstance>,
}

#[derive(Debug)]
pub struct BenchInstance {
    pub name: String,
    pub data: Value,
}

pub struct BenchmarkSuite {
    benchmarks: [LazyLock<BenchData>; 5],
}

impl BenchmarkSuite {
    fn new() -> Self {
        Self {
            benchmarks: [
                LazyLock::new(|| BenchData {
                    name: "Open API",
                    schema: read_json(OPEN_API),
                    instances: vec![BenchInstance {
                        name: "Zuora".to_string(),
                        data: read_json(ZUORA),
                    }],
                }),
                LazyLock::new(|| BenchData {
                    name: "Swagger",
                    schema: read_json(SWAGGER),
                    instances: vec![BenchInstance {
                        name: "Kubernetes".to_string(),
                        data: read_json(KUBERNETES),
                    }],
                }),
                LazyLock::new(|| BenchData {
                    name: "GeoJSON",
                    schema: read_json(GEOJSON),
                    instances: vec![BenchInstance {
                        name: "Canada".to_string(),
                        data: read_json(CANADA),
                    }],
                }),
                LazyLock::new(|| BenchData {
                    name: "CITM",
                    schema: read_json(CITM_SCHEMA),
                    instances: vec![BenchInstance {
                        name: "Catalog".to_string(),
                        data: read_json(CITM),
                    }],
                }),
                LazyLock::new(|| BenchData {
                    name: "Fast",
                    schema: read_json(FAST_SCHEMA),
                    instances: vec![
                        BenchInstance {
                            name: "Valid".to_string(),
                            data: read_json(FAST_VALID),
                        },
                        BenchInstance {
                            name: "Invalid".to_string(),
                            data: read_json(FAST_INVALID),
                        },
                    ],
                }),
            ],
        }
    }

    fn run(&self, bench_type: Benchmark, bench: &mut BenchFunc) {
        let index = bench_type as usize;
        let data = &self.benchmarks[index];
        bench(data.name, &data.schema, &data.instances);
    }
}

static BENCHMARK_SUITE: LazyLock<BenchmarkSuite> = LazyLock::new(BenchmarkSuite::new);

#[derive(serde::Deserialize)]
pub struct KeywordBenchmark {
    pub name: String,
    pub schema: Value,
    #[serde(default)]
    pub valid: Vec<Value>,
    #[serde(default)]
    pub invalid: Vec<Value>,
}

static KEYWORDS: &[u8] = include_bytes!("../data/keywords.json");
static KEYWORD_BENCHMARKS: LazyLock<Vec<KeywordBenchmark>> =
    LazyLock::new(|| serde_json::from_slice(KEYWORDS).expect("Invalid JSON"));

pub fn run_keyword_benchmarks(bench: &mut BenchFunc) {
    for kb in KEYWORD_BENCHMARKS.iter() {
        for (prefix, values) in [("valid", &kb.valid), ("invalid", &kb.invalid)] {
            let instances: Vec<_> = values
                .iter()
                .enumerate()
                .map(|(idx, instance)| BenchInstance {
                    name: format!("{}/{}", prefix, idx),
                    data: instance.clone(),
                })
                .collect();
            bench(&kb.name, &kb.schema, &instances);
        }
    }
}

#[derive(serde::Deserialize)]
pub struct ErrorBenchmark {
    pub name: String,
    pub schema: Value,
    pub instance: Value,
}

static ERROR_CASES: &[u8] = include_bytes!("../data/errors.json");
static ERROR_BENCHMARKS: LazyLock<Vec<ErrorBenchmark>> =
    LazyLock::new(|| serde_json::from_slice(ERROR_CASES).expect("Invalid JSON"));

pub fn run_error_formatting_benchmarks(bench_fn: &mut dyn FnMut(&str, &Value, &Value)) {
    for case in ERROR_BENCHMARKS.iter() {
        bench_fn(&case.name, &case.schema, &case.instance);
    }
}
