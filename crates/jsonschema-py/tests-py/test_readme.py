import re
import sys
from pathlib import Path

import jsonschema_rs
import pytest

HERE = Path(__file__).absolute()


@pytest.mark.skipif(sys.platform.startswith("win"), reason="Skipping on Windows")
def test_readme():
    with (HERE.parent.parent / "README.md").open() as f:
        readme = f.read()

    code_blocks = re.findall(r"```python\n(.*?)```", readme, re.DOTALL)

    scope = {"jsonschema_rs": jsonschema_rs}
    for i, code_block in enumerate(code_blocks):
        try:
            exec(code_block, scope)
        except Exception as e:
            pytest.fail(f"Code block {i + 1} failed: {str(e)}\n\nCode:\n{code_block}")
