import os
import sys
# Keep the protocol on a private descriptor. Both Python prints and native
# writes to fd 1 now go to stderr, even during Sage import and introspection.
protocol = os.fdopen(os.dup(sys.stdout.fileno()), "w", buffering=1)
os.dup2(sys.stderr.fileno(), sys.stdout.fileno())
sys.stdout = sys.stderr
import inspect
import json
import traceback

def send(payload):
    protocol.write(json.dumps(payload) + "\n")
    protocol.flush()

try:
    from sage.all import *  # noqa: F401,F403
    from sage.misc.sageinspect import sage_getdef, sage_getdoc, sage_getfile, sage_getsourcelines
    # Workspace paths must never precede Sage or the standard library.
    sys.path.extend(json.loads(os.environ.get("SAGE_LS_SOURCE_ROOTS", "[]")))
    send({"ready": True})
except Exception as exc:
    send({"ready": False, "error": traceback.format_exc()})
    sys.exit(0)

def resolve_symbol(name):
    obj = None
    namespace = globals()
    for part in name.split("."):
        if not part:
            raise KeyError(name)
        if obj is None:
            obj = namespace[part]
        else:
            obj = getattr(obj, part)
    return obj

def one_line(value):
    value = (value or "").strip()
    for line in value.splitlines():
        line = line.strip()
        if line:
            return line
    return ""

for line in sys.stdin:
    request = {}
    name = ""
    try:
        request = json.loads(line)
        name = request.get("symbol", "")
        try:
            obj = resolve_symbol(name)
        except (KeyError, AttributeError):
            send({"id": request.get("id"), "name": name, "ok": False, "missing": True})
            continue
        try:
            # Embedded formatting preserves docs without probing optional packages
            # (e.g. starting Maxima merely to format sin's doctest tags).
            doc = sage_getdoc(obj, obj_name=name, embedded=True) or ""
        except TypeError:
            doc = sage_getdoc(obj) or ""
        if not doc:
            doc = inspect.getdoc(obj) or ""
        try:
            detail = sage_getdef(obj, name) or ""
        except Exception:
            detail = ""
        try:
            path = sage_getfile(obj) or ""
        except Exception:
            path = ""
        source_line = None
        source_text = None
        try:
            source_lines, source_line = sage_getsourcelines(obj)
            source_text = source_lines[0].rstrip() if source_lines else None
        except Exception:
            pass
        module = getattr(obj, "__module__", "") or "sage.runtime"
        kind = type(obj).__name__
        send({
            "id": request.get("id"),
            "ok": True,
            "name": name,
            "module_name": module,
            "kind": kind,
            "detail": detail,
            "summary": one_line(doc) or detail or name,
            "docstring": doc,
            "uri": path,
            "source_line": source_line,
            "source_text": source_text,
        })
    except Exception as exc:
        send({
            "id": request.get("id"),
            "ok": False,
            "name": name,
            "error": str(exc),
            "traceback": traceback.format_exc(limit=2),
        })
