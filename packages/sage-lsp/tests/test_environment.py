from sage_lsp.environment import ServerSettings


def test_server_settings_defaults() -> None:
    settings = ServerSettings.from_initialization_options({})

    assert settings.interpreter_path == "python"
    assert settings.analysis_source_roots == []
    assert settings.log_level == "info"
    assert settings.workspace_trust_mode == "restricted"


def test_server_settings_normalizes_values() -> None:
    settings = ServerSettings.from_initialization_options(
        {
            "interpreterPath": "sage -python",
            "analysisSourceRoots": ["src", 3],
            "logLevel": "debug",
            "workspaceTrustMode": "trusted",
        }
    )

    assert settings.interpreter_path == "sage -python"
    assert settings.analysis_source_roots == ["src", "3"]
    assert settings.log_level == "debug"
    assert settings.workspace_trust_mode == "trusted"

