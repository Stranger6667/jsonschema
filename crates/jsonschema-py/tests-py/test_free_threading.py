import subprocess
import sys
import textwrap

import pytest

# Each case runs in a child interpreter: a crash there fails the test instead of the whole run.
PROGRAM = textwrap.dedent(
    """
    import json, sys, threading, time
    import jsonschema_rs, jsonschema_testsuite_pyo3

    backend, mutation = sys.argv[1], sys.argv[2]
    big = 10**30

    if mutation == "list":
        schema = {"items": {"type": "integer"}}
        instance = [big + i for i in range(2000)]

        def mutate(n):
            instance.clear()
            instance.extend(big + n + i for i in range(2000))
    else:
        schema = {"properties": {"a": {"type": "string"}}}
        instance = {f"k{i}": f"value-{i}" * 3 for i in range(2000)}
        instance["a"] = "x"

        def mutate(n):
            instance[f"extra{n}"] = f"v{n}" * 3
            if n % 50 == 0:
                for key in [key for key in instance if key.startswith("extra")]:
                    del instance[key]

    if backend == "runtime":
        check = jsonschema_rs.validator_for(schema).is_valid
    elif backend == "codegen":
        key = json.dumps(schema, sort_keys=True, separators=(",", ":"))
        check = lambda value: jsonschema_testsuite_pyo3.is_valid(key, value)
    else:
        check = lambda value: jsonschema_rs.validator_for({"enum": [value]})

    stop = time.monotonic() + 1

    def validate():
        while time.monotonic() < stop:
            try:
                check(instance)
            except ValueError:
                pass

    def mutator():
        n = 0
        while time.monotonic() < stop:
            n += 1
            mutate(n)

    threads = [threading.Thread(target=validate), threading.Thread(target=mutator)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    """
)


@pytest.mark.parametrize("backend", ["runtime", "codegen", "schema"])
@pytest.mark.parametrize("mutation", ["list", "dict"])
def test_concurrent_mutation_does_not_crash(backend, mutation):
    result = subprocess.run(
        [sys.executable, "-c", PROGRAM, backend, mutation], capture_output=True, text=True, timeout=60
    )
    assert result.returncode == 0, result.stderr
