import json
import socket
import subprocess
import sys
import textwrap
import time
import pytest
from jsonschema_rs import (
    Draft4Validator,
    Draft6Validator,
    Draft7Validator,
    Draft201909Validator,
    Draft202012Validator,
    HttpOptions,
    ValidationError,
    is_valid,
    validator_for,
)


def get_free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@pytest.fixture
def schema_server():
    processes = []

    def start(schema=None):
        if schema is None:
            schema = {"type": "string"}
        port = get_free_port()
        schema_json = json.dumps(schema)

        server_code = textwrap.dedent(f"""
            import json
            import http.server
            import socketserver

            class Handler(http.server.BaseHTTPRequestHandler):
                def do_GET(self):
                    response = {schema_json!r}.encode()
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json")
                    self.send_header("Content-Length", str(len(response)))
                    self.send_header("Connection", "close")
                    self.end_headers()
                    self.wfile.write(response)
                def log_message(self, *args):
                    pass

            with socketserver.TCPServer(("127.0.0.1", {port}), Handler) as httpd:
                print("READY", flush=True)
                httpd.serve_forever()
        """)

        proc = subprocess.Popen(
            [sys.executable, "-c", server_code],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        processes.append(proc)

        # Wait for server to be ready
        for line in iter(proc.stdout.readline, ""):
            if "READY" in line:
                break

        time.sleep(0.05)  # Small delay to ensure server is accepting connections
        return f"http://127.0.0.1:{port}"

    yield start

    for proc in processes:
        proc.terminate()
        proc.wait()


def test_http_options_repr():
    opts = HttpOptions(timeout=30.0, connect_timeout=10.0, tls_verify=False, ca_cert="/cert.pem")
    assert repr(opts) == "HttpOptions(timeout=30, connect_timeout=10, tls_verify=False, ca_cert='/cert.pem')"

    opts2 = HttpOptions(tls_verify=True)
    assert repr(opts2) == "HttpOptions(timeout=None, connect_timeout=None, tls_verify=True, ca_cert=None)"


def test_http_options_equality():
    opts1 = HttpOptions(timeout=30.0, connect_timeout=10.0)
    opts2 = HttpOptions(timeout=30.0, connect_timeout=10.0)
    opts3 = HttpOptions(timeout=30.0, connect_timeout=5.0)

    assert opts1 == opts2
    assert opts1 != opts3
    assert opts1 != "not an HttpOptions"


def test_http_options_hashable():
    opts1 = HttpOptions(timeout=30.0)
    opts2 = HttpOptions(timeout=30.0)
    assert len({opts1, opts2}) == 1


@pytest.mark.parametrize("value", [-1.0, float("nan"), float("inf"), float("-inf")])
def test_http_options_invalid_timeout(value):
    with pytest.raises(ValueError, match="non-negative finite"):
        HttpOptions(timeout=value)


@pytest.mark.parametrize("value", [-1.0, float("nan"), float("inf"), float("-inf")])
def test_http_options_invalid_connect_timeout(value):
    with pytest.raises(ValueError, match="non-negative finite"):
        HttpOptions(connect_timeout=value)


def test_http_options_with_validator_for():
    opts = HttpOptions(timeout=30.0)
    validator = validator_for({"type": "string"}, http_options=opts)
    assert validator.is_valid("test")


def test_http_options_with_is_valid():
    opts = HttpOptions(timeout=30.0)
    assert is_valid({"type": "string"}, "test", http_options=opts)


@pytest.mark.parametrize(
    "validator_cls", [Draft4Validator, Draft6Validator, Draft7Validator, Draft201909Validator, Draft202012Validator]
)
def test_http_options_with_draft_validators(validator_cls):
    opts = HttpOptions(timeout=30.0, tls_verify=False)
    validator = validator_cls({"type": "integer"}, http_options=opts)
    assert validator.is_valid(42)
    assert not validator.is_valid("not an integer")


def test_http_options_invalid_cert_path():
    opts = HttpOptions(ca_cert="/nonexistent/path/to/cert.pem")
    with pytest.raises(RuntimeError, match="Failed to configure HTTP options"):
        validator_for({"$ref": "https://example.com/schema.json"}, http_options=opts)


def test_http_fetch_external_schema():
    # Fetches the JSON Schema 2020-12 meta-schema over HTTPS (requires network)
    schema = {"$ref": "https://json-schema.org/draft/2020-12/schema"}
    validator = validator_for(schema)
    # The meta-schema validates objects
    assert validator.is_valid({})
    assert validator.is_valid({"type": "string"})


def test_http_fetch_schema(schema_server):
    base_url = schema_server()
    schema = {"$ref": f"{base_url}/schema.json"}
    opts = HttpOptions(timeout=5.0, connect_timeout=2.0)
    validator = validator_for(schema, http_options=opts)
    assert validator.is_valid("test")
    assert not validator.is_valid(42)


@pytest.fixture
def slow_server():
    # Server that delays response to test timeout behavior
    processes = []

    def start(delay_seconds):
        port = get_free_port()
        schema_json = json.dumps({"type": "string"})

        server_code = textwrap.dedent(f"""
            import time
            import http.server
            import socketserver

            class Handler(http.server.BaseHTTPRequestHandler):
                def do_GET(self):
                    time.sleep({delay_seconds})
                    response = {schema_json!r}.encode()
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json")
                    self.send_header("Content-Length", str(len(response)))
                    self.send_header("Connection", "close")
                    self.end_headers()
                    self.wfile.write(response)
                def log_message(self, *args):
                    pass

            with socketserver.TCPServer(("127.0.0.1", {port}), Handler) as httpd:
                print("READY", flush=True)
                httpd.serve_forever()
        """)

        proc = subprocess.Popen(
            [sys.executable, "-c", server_code],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        processes.append(proc)

        for line in iter(proc.stdout.readline, ""):
            if "READY" in line:
                break

        time.sleep(0.05)
        return f"http://127.0.0.1:{port}"

    yield start

    for proc in processes:
        proc.terminate()
        proc.wait()


def test_http_timeout_triggers(slow_server):
    # Server delays 2 seconds, but we set timeout to 0.5 seconds
    base_url = slow_server(delay_seconds=2)
    schema = {"$ref": f"{base_url}/schema.json"}
    opts = HttpOptions(timeout=0.5)
    with pytest.raises(ValidationError, match="error sending request"):
        validator_for(schema, http_options=opts)
