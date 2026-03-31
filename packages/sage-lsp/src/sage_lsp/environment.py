from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path


@dataclass(slots=True)
class SageInterpreter:
    """Resolved Sage executable and its derived Python runtime."""

    sage_path: Path
    python_path: Path | None = None
    args: tuple[str, ...] = ()


@dataclass(slots=True)
class AnalysisSettings:
    """Server-side analysis configuration supplied by the editor client."""

    mode: str = "default"
    enable_diagnostics: bool = True
    enable_runtime_introspection: bool = True
    enable_pyx_parsing: bool = True
    extra_paths: tuple[str, ...] = ()
    stub_paths: tuple[str, ...] = ()


@dataclass(slots=True)
class WorkspaceContext:
    """Initialization data that shapes indexing and symbol resolution."""

    root_uri: str | None = None
    workspace_folders: tuple[str, ...] = ()
    source_roots: tuple[str, ...] = ()
    excluded_globs: tuple[str, ...] = ()


@dataclass(slots=True)
class SageEnvironment:
    """Top-level environment snapshot used by the LSP server."""

    interpreter: SageInterpreter | None = None
    analysis: AnalysisSettings = field(default_factory=AnalysisSettings)
    workspace: WorkspaceContext = field(default_factory=WorkspaceContext)

    @classmethod
    def from_initialize_options(cls, options: dict[str, object] | None) -> "SageEnvironment":
        if not options:
            return cls()

        interpreter: SageInterpreter | None = None
        interpreter_config = options.get("interpreter")
        if isinstance(interpreter_config, dict):
            sage_path = interpreter_config.get("path")
            python_path = interpreter_config.get("pythonPath")
            args = interpreter_config.get("args", ())
            if isinstance(sage_path, str) and sage_path:
                interpreter = SageInterpreter(
                    sage_path=Path(sage_path),
                    python_path=Path(python_path) if isinstance(python_path, str) and python_path else None,
                    args=tuple(arg for arg in args if isinstance(arg, str))
                    if isinstance(args, (list, tuple))
                    else (),
                )

        analysis_config = options.get("analysis")
        if isinstance(analysis_config, dict):
            analysis = AnalysisSettings(
                mode=analysis_config.get("mode", "default")
                if isinstance(analysis_config.get("mode"), str)
                else "default",
                enable_diagnostics=bool(analysis_config.get("enableDiagnostics", True)),
                enable_runtime_introspection=bool(
                    analysis_config.get("enableRuntimeIntrospection", True)
                ),
                enable_pyx_parsing=bool(analysis_config.get("enablePyxParsing", True)),
                extra_paths=tuple(
                    value for value in analysis_config.get("extraPaths", ()) if isinstance(value, str)
                )
                if isinstance(analysis_config.get("extraPaths"), (list, tuple))
                else (),
                stub_paths=tuple(
                    value for value in analysis_config.get("stubPaths", ()) if isinstance(value, str)
                )
                if isinstance(analysis_config.get("stubPaths"), (list, tuple))
                else (),
            )
        else:
            analysis = AnalysisSettings()

        workspace_config = options.get("workspace")
        if isinstance(workspace_config, dict):
            workspace = WorkspaceContext(
                root_uri=workspace_config.get("rootUri")
                if isinstance(workspace_config.get("rootUri"), str)
                else None,
                workspace_folders=tuple(
                    value
                    for value in workspace_config.get("folders", ())
                    if isinstance(value, str)
                )
                if isinstance(workspace_config.get("folders"), (list, tuple))
                else (),
                source_roots=tuple(
                    value
                    for value in workspace_config.get("sourceRoots", ())
                    if isinstance(value, str)
                )
                if isinstance(workspace_config.get("sourceRoots"), (list, tuple))
                else (),
                excluded_globs=tuple(
                    value
                    for value in workspace_config.get("exclude", ())
                    if isinstance(value, str)
                )
                if isinstance(workspace_config.get("exclude"), (list, tuple))
                else (),
            )
        else:
            workspace = WorkspaceContext()

        return cls(interpreter=interpreter, analysis=analysis, workspace=workspace)
