from __future__ import annotations

from dataclasses import dataclass
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
from typing import Optional

from .environment import SageEnvironment


RUNTIME_INTROSPECTION_SCRIPT = """
import importlib
import inspect
import json
import sys
from pathlib import Path

try:
    from sage.all import *  # noqa: F401,F403
    from sage.misc.sageinspect import sage_getdef, sage_getdoc, sage_getfile, sage_getsourcelines
except Exception as exc:
    print(json.dumps({"ok": False, "error": str(exc)}))
    raise SystemExit(0)

NAME = sys.argv[1]


def resolve_symbol(name):
    namespace = globals()
    if name in namespace:
        return namespace[name]

    parts = name.split(".")
    target = namespace.get(parts[0])
    if target is None:
        target = importlib.import_module(parts[0])

    for part in parts[1:]:
        target = getattr(target, part)
    return target


def safe(callable_, default=None):
    try:
        return callable_()
    except Exception:
        return default


try:
    obj = resolve_symbol(NAME)
except Exception as exc:
    print(json.dumps({"ok": False, "error": str(exc)}))
    raise SystemExit(0)

if inspect.ismodule(obj):
    kind = "module"
elif inspect.isclass(obj):
    kind = "class"
elif callable(obj):
    kind = "function"
else:
    kind = "variable"

detail = safe(lambda: sage_getdef(obj, obj_name=NAME))
if not isinstance(detail, str) or not detail.strip():
    detail = f"{kind} {NAME}"

docstring = safe(lambda: sage_getdoc(obj, NAME))
if not isinstance(docstring, str) or not docstring.strip():
    docstring = safe(lambda: inspect.getdoc(obj))

file_path = safe(lambda: sage_getfile(obj))
if file_path:
    try:
        file_path = str(Path(file_path).resolve())
    except Exception:
        file_path = str(file_path)

line = None
source_info = safe(lambda: sage_getsourcelines(obj))
if isinstance(source_info, tuple) and len(source_info) >= 2:
    try:
        line = int(source_info[1])
    except Exception:
        line = None

module_name = getattr(obj, "__module__", None)
if inspect.ismodule(obj):
    module_name = getattr(obj, "__name__", module_name)

print(
    json.dumps(
        {
            "ok": True,
            "result": {
                "name": NAME,
                "kind": kind,
                "detail": detail,
                "docstring": docstring,
                "filePath": file_path,
                "line": line,
                "moduleName": module_name or "sage.all",
            },
        }
    )
)
""".strip()

VALID_SYMBOL = re.compile(r"^[A-Za-z_][A-Za-z0-9_\.]*$")


@dataclass(frozen=True)
class RuntimeSymbolResult:
    name: str
    kind: str
    detail: str
    module_name: str
    docstring: Optional[str]
    file_path: Optional[Path]
    line: Optional[int]


class RuntimeIntrospector:
    def __init__(
        self,
        command: Optional[str],
        args: tuple[str, ...] = (),
        enabled: bool = False,
        timeout_seconds: int = 6,
        source_roots: tuple[Path, ...] = (),
    ) -> None:
        self._command = command
        self._args = args
        self._enabled = enabled and bool(command)
        self._timeout_seconds = timeout_seconds
        self._cache: dict[str, Optional[RuntimeSymbolResult]] = {}
        self._runtime_environment = build_runtime_environment(source_roots=source_roots)

    @classmethod
    def from_environment(cls, environment: SageEnvironment) -> "RuntimeIntrospector":
        interpreter = environment.interpreter
        if interpreter is None:
            return cls(command=None, enabled=False)
        source_roots = tuple(Path(value) for value in environment.workspace.source_roots if value)
        if interpreter.python_path is not None and looks_like_local_sage_checkout(interpreter.sage_path):
            return cls(
                command=str(interpreter.python_path),
                enabled=environment.analysis.enable_runtime_introspection,
                source_roots=source_roots,
            )
        return cls(
            command=str(interpreter.sage_path),
            args=interpreter.args,
            enabled=environment.analysis.enable_runtime_introspection,
            source_roots=source_roots,
        )

    def lookup(self, name: str) -> Optional[RuntimeSymbolResult]:
        if name in self._cache:
            return self._cache[name]

        result = self._lookup_uncached(name)
        self._cache[name] = result
        return result

    def _lookup_uncached(self, name: str) -> Optional[RuntimeSymbolResult]:
        if not self._enabled or not self._command or not VALID_SYMBOL.match(name):
            return None

        invocation = self._build_invocation(name)
        if invocation is None:
            return None

        try:
            completed = subprocess.run(
                invocation,
                check=False,
                capture_output=True,
                text=True,
                timeout=self._timeout_seconds,
                env=self._runtime_environment,
            )
        except OSError:
            return None
        except subprocess.TimeoutExpired:
            return None

        payload = parse_runtime_introspection_output(completed.stdout)
        if payload is None:
            return None
        return payload

    def _build_invocation(self, name: str) -> Optional[list[str]]:
        if not self._command:
            return None

        command = Path(self._command).name.lower()
        if command.startswith("python"):
            return [self._command, *self._args, "-c", RUNTIME_INTROSPECTION_SCRIPT, name]
        if command.startswith("sage") or self._command == "sage":
            return [self._command, *self._args, "-python", "-c", RUNTIME_INTROSPECTION_SCRIPT, name]
        return None


def parse_runtime_introspection_output(stdout: str) -> Optional[RuntimeSymbolResult]:
    lines = [line.strip() for line in stdout.splitlines() if line.strip()]
    if not lines:
        return None

    try:
        payload = json.loads(lines[-1])
    except json.JSONDecodeError:
        return None

    if not isinstance(payload, dict) or payload.get("ok") is not True:
        return None

    result = payload.get("result")
    if not isinstance(result, dict):
        return None

    name = result.get("name")
    kind = result.get("kind")
    detail = result.get("detail")
    module_name = result.get("moduleName")

    if not all(isinstance(value, str) for value in (name, kind, detail, module_name)):
        return None

    docstring = result.get("docstring")
    file_path = result.get("filePath")
    line = result.get("line")

    return RuntimeSymbolResult(
        name=name,
        kind=kind,
        detail=detail,
        module_name=module_name,
        docstring=docstring if isinstance(docstring, str) else None,
        file_path=Path(file_path) if isinstance(file_path, str) and file_path else None,
        line=line if isinstance(line, int) else None,
    )


def build_runtime_environment(
    base_environment: Optional[dict[str, str]] = None,
    source_roots: tuple[Path, ...] = (),
) -> dict[str, str]:
    environment = dict(base_environment or os.environ)
    runtime_home = Path(tempfile.gettempdir()) / "sage-lsp-runtime-home"
    dot_sage = runtime_home / ".sage"
    runtime_home.mkdir(parents=True, exist_ok=True)
    dot_sage.mkdir(parents=True, exist_ok=True)
    environment["HOME"] = str(runtime_home)
    environment["DOT_SAGE"] = str(dot_sage)
    environment.setdefault("XDG_CACHE_HOME", str(runtime_home / ".cache"))
    python_paths = [str(root) for root in expand_runtime_import_roots(source_roots)]
    if python_paths:
        existing_pythonpath = environment.get("PYTHONPATH")
        if existing_pythonpath:
            python_paths.append(existing_pythonpath)
        environment["PYTHONPATH"] = os.pathsep.join(python_paths)
    return environment


def looks_like_local_sage_checkout(candidate: Path) -> bool:
    runtime_root = candidate.resolve().parent
    return (runtime_root / "src" / "bin" / "sage").exists() or (runtime_root / "src" / "sage").exists()


def expand_runtime_import_roots(source_roots: tuple[Path, ...]) -> tuple[Path, ...]:
    discovered: list[Path] = []
    seen: set[Path] = set()

    for source_root in source_roots:
        resolved_root = source_root.resolve()
        if resolved_root not in seen and (resolved_root / "sage").exists():
            seen.add(resolved_root)
            discovered.append(resolved_root)

        repo_root = resolved_root.parent
        for build_src in repo_root.glob("builddir*/src"):
            if (build_src / "sage").exists() and build_src.resolve() not in seen:
                seen.add(build_src.resolve())
                discovered.append(build_src.resolve())

    return tuple(discovered)
