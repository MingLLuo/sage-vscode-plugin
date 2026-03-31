from __future__ import annotations

import sys
from dataclasses import dataclass
from pathlib import Path

from sage_lsp.environment import SageEnvironment
from sage_lsp.index import WorkspaceIndex
from sage_lsp.runtime_introspection import RuntimeIntrospector
from sage_lsp.server import _documentation_for_request, create_server


REPOSITORY_ROOT = Path(__file__).resolve().parent.parent
WORKSPACE_ROOT = REPOSITORY_ROOT / "examples" / "manual-smoke-workspace" / "src"
DEFAULT_NATIVE_SOURCE_ROOT = REPOSITORY_ROOT.parent / "sage" / "src"
DEFAULT_SAGE_EXECUTABLE = REPOSITORY_ROOT.parent / "sage" / "sage"

SMOKE_SOURCE = """\
G = graphs.PetersenGraph()
R.<x, y> = PolynomialRing(QQ, 2)
E = EllipticCurve([0, 0, 1, -1, 0])
M = matrix(QQ, [[1, 2], [3, 4]])
T.<t> = QQ[]
K.<a> = NumberField(t^2 + 1)
parts = Partitions(4)
"""


@dataclass(frozen=True)
class NativeSymbolExpectation:
    name: str
    summary_fragment: str
    path_suffix: str


EXPECTATIONS = (
    NativeSymbolExpectation(
        name="graphs.PetersenGraph",
        summary_fragment="Petersen Graph",
        path_suffix="sage/graphs/generators/smallgraphs.py",
    ),
    NativeSymbolExpectation(
        name="PolynomialRing",
        summary_fragment="polynomial ring",
        path_suffix="sage/rings/polynomial/polynomial_ring_constructor.py",
    ),
    NativeSymbolExpectation(
        name="EllipticCurve",
        summary_fragment="elliptic curve",
        path_suffix="sage/schemes/elliptic_curves/constructor.py",
    ),
    NativeSymbolExpectation(
        name="matrix",
        summary_fragment="Create a matrix.",
        path_suffix="sage/matrix/constructor.pyx",
    ),
    NativeSymbolExpectation(
        name="NumberField",
        summary_fragment="number field",
        path_suffix="sage/rings/number_field/number_field.py",
    ),
    NativeSymbolExpectation(
        name="Partitions",
        summary_fragment="integer partitions",
        path_suffix="sage/combinat/partition.py",
    ),
)


def main() -> int:
    native_source_root = DEFAULT_NATIVE_SOURCE_ROOT
    if not native_source_root.exists():
        print(f"SKIP native sage smoke: missing source root {native_source_root}")
        return 0

    sage_executable = DEFAULT_SAGE_EXECUTABLE if DEFAULT_SAGE_EXECUTABLE.exists() else None
    if not WORKSPACE_ROOT.exists():
        print(f"FAIL native sage smoke: missing workspace root {WORKSPACE_ROOT}")
        return 1

    index = WorkspaceIndex(
        source_roots=[WORKSPACE_ROOT, native_source_root],
        excluded_globs=(),
        enable_pyx=True,
    )
    index.build()

    server = create_server()
    server.environment = SageEnvironment.from_initialize_options(
        {
            "interpreterPath": str(sage_executable) if sage_executable is not None else None,
            "analysisSourceRoots": [str(WORKSPACE_ROOT), str(native_source_root)],
            "enableRuntimeIntrospection": True,
            "showDocsOnHover": True,
        }
    )
    server.workspace_index = index
    server.runtime_introspector = RuntimeIntrospector(
        command=str(sage_executable) if sage_executable is not None else None,
        enabled=sage_executable is not None,
    )
    record = index.parse_document("file:///tmp/native-sage-smoke.sage", SMOKE_SOURCE, "sagemath")

    failures: list[str] = []
    print("Native Sage smoke")
    print(f"  source root: {native_source_root}")
    print(f"  sage executable: {sage_executable if sage_executable is not None else 'unavailable'}")

    for expectation in EXPECTATIONS:
        documentation = _documentation_for_request(
            server,
            record,
            expectation.name,
            expectation.name,
        )
        symbol = index.resolve_symbol(record, expectation.name)
        summary = normalize_whitespace(documentation.summary if documentation is not None else "")
        path = symbol.file_path if symbol is not None else None
        path_text = str(path) if path is not None else "<missing>"
        print(f"- {expectation.name}")
        print(f"  summary: {summary or '<missing>'}")
        print(f"  path: {path_text}")

        if expectation.summary_fragment.casefold() not in summary.casefold():
            failures.append(
                f"{expectation.name}: expected summary containing {expectation.summary_fragment!r}, got {summary!r}"
            )
        if path is None or not path_text.endswith(expectation.path_suffix):
            failures.append(
                f"{expectation.name}: expected definition path ending with {expectation.path_suffix!r}, got {path_text!r}"
            )

    if failures:
        print("FAIL native sage smoke")
        for failure in failures:
            print(f"  - {failure}")
        return 1

    print("PASS native sage smoke")
    return 0


def normalize_whitespace(value: str) -> str:
    return " ".join(value.split())


if __name__ == "__main__":
    raise SystemExit(main())
