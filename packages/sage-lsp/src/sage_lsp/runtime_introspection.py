from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path
import re
import subprocess
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
    ) -> None:
        self._command = command
        self._args = args
        self._enabled = enabled and bool(command)
        self._cache: dict[str, Optional[RuntimeSymbolResult]] = {}

    @classmethod
    def from_environment(cls, environment: SageEnvironment) -> "RuntimeIntrospector":
        interpreter = environment.interpreter
        if interpreter is None:
            return cls(command=None, enabled=False)
        return cls(
            command=str(interpreter.sage_path),
            args=interpreter.args,
            enabled=environment.analysis.enable_runtime_introspection,
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
                invocation[0],
                invocation[1],
                check=False,
                capture_output=True,
                text=True,
                timeout=3,
            )
        except OSError:
            return None
        except subprocess.TimeoutExpired:
            return None

        payload = parse_runtime_introspection_output(completed.stdout)
        if payload is None:
            return None
        return payload

    def _build_invocation(self, name: str) -> Optional[tuple[str, list[str]]]:
        if not self._command:
            return None

        command = Path(self._command).name.lower()
        if command.startswith("python"):
            return (
                self._command,
                [*self._args, "-c", RUNTIME_INTROSPECTION_SCRIPT, name],
            )
        if command.startswith("sage") or self._command == "sage":
            return (
                self._command,
                [*self._args, "-python", "-c", RUNTIME_INTROSPECTION_SCRIPT, name],
            )
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
