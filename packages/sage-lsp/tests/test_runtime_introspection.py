from pathlib import Path

from sage_lsp.runtime_introspection import RuntimeSymbolResult, parse_runtime_introspection_output


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
