def pytest_configure(config):
    config.addinivalue_line("markers", "data(schema, instance): add data for benchmarking")
    config.addinivalue_line("markers", "codegen(name): compile-time validator in jsonschema_bench_pyo3")
