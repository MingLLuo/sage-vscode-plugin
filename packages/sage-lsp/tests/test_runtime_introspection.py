from pathlib import Path
import subprocess

from sage_lsp.runtime_introspection import (
    RuntimeIntrospector,
    RuntimeSymbolResult,
    parse_runtime_introspection_output,
)


def test_parse_runtime_introspection_output_accepts_valid_payload() -> None:
    result = parse_runtime_introspection_output(
        '{"ok": true, "result": {"name": "sqrt", "kind": "function", "detail": "sqrt(x)", "docstring": "Return the principal square root.", "filePath": "/tmp/sqrt.pyx", "line": 12, "moduleName": "sage.misc.functional"}}'
    )

    assert result == RuntimeSymbolResult(
        name="sqrt",
        kind="function",
        detail="sqrt(x)",
        module_name="sage.misc.functional",
        docstring="Return the principal square root.",
        file_path=Path("/tmp/sqrt.pyx"),
        line=12,
    )


def test_parse_runtime_introspection_output_rejects_failures() -> None:
    assert parse_runtime_introspection_output('{"ok": false, "error": "boom"}') is None
    assert parse_runtime_introspection_output("not json") is None


def test_runtime_introspector_builds_python_invocation() -> None:
    introspector = RuntimeIntrospector(command="/opt/bin/python3", args=("-X", "utf8"), enabled=True)

    invocation = introspector._build_invocation("graphs.PetersenGraph")  # noqa: SLF001 - unit test

    assert invocation is not None
    assert invocation[:2] == ["/opt/bin/python3", "-X"]
    assert invocation[2] == "utf8"
    assert invocation[-1] == "graphs.PetersenGraph"


def test_runtime_introspector_lookup_invokes_subprocess_with_argv(monkeypatch) -> None:
    introspector = RuntimeIntrospector(command="/opt/bin/sage", args=("--nodotsage",), enabled=True)
    calls: list[dict[str, object]] = []
    expected_argv = introspector._build_invocation("graphs.PetersenGraph")  # noqa: SLF001 - unit test
    assert expected_argv is not None

    def fake_run(argv, *, check, capture_output, text, timeout):
        calls.append(
            {
                "argv": argv,
                "check": check,
                "capture_output": capture_output,
                "text": text,
                "timeout": timeout,
            }
        )
        return subprocess.CompletedProcess(
            argv,
            0,
            stdout='{"ok": true, "result": {"name": "PetersenGraph", "kind": "function", "detail": "graphs.PetersenGraph()", "docstring": "Build the Petersen graph.", "filePath": "/tmp/graphs.py", "line": 8, "moduleName": "sage.graphs.graph_generators"}}\n',
            stderr="",
        )

    monkeypatch.setattr("sage_lsp.runtime_introspection.subprocess.run", fake_run)

    result = introspector.lookup("graphs.PetersenGraph")

    assert result is not None
    assert result.name == "PetersenGraph"
    assert calls == [
        {
            "argv": expected_argv,
            "check": False,
            "capture_output": True,
            "text": True,
            "timeout": 3,
        }
    ]
