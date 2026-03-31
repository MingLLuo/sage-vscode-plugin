from sage_lsp.environment import SageEnvironment


def test_environment_from_initialize_options() -> None:
    environment = SageEnvironment.from_initialize_options(
        {
            "interpreter": {
                "path": "/opt/sage/sage",
                "pythonPath": "/opt/sage/python",
                "args": ["-python"],
            },
            "analysis": {
                "mode": "full",
                "enableDiagnostics": False,
                "enableRuntimeIntrospection": True,
                "enablePyxParsing": False,
                "extraPaths": ["/workspace/src"],
                "stubPaths": ["/workspace/stubs"],
            },
            "workspace": {
                "rootUri": "file:///workspace",
                "folders": ["file:///workspace"],
                "sourceRoots": ["/workspace/src"],
                "exclude": ["**/.venv"],
            },
        }
    )

    assert str(environment.interpreter.sage_path) == "/opt/sage/sage"
    assert str(environment.interpreter.python_path) == "/opt/sage/python"
    assert environment.analysis.mode == "full"
    assert environment.analysis.enable_diagnostics is False
    assert environment.analysis.enable_pyx_parsing is False
    assert environment.workspace.root_uri == "file:///workspace"
    assert environment.workspace.source_roots == ("/workspace/src",)
