from __future__ import annotations

from dataclasses import dataclass
from typing import Any


@dataclass(slots=True)
class ServerSettings:
    interpreter_path: str
    analysis_source_roots: list[str]
    log_level: str
    workspace_trust_mode: str

    @classmethod
    def from_initialization_options(cls, raw: Any) -> "ServerSettings":
        if not isinstance(raw, dict):
            raw = {}

        source_roots = raw.get("analysisSourceRoots", [])
        if not isinstance(source_roots, list):
            source_roots = []

        return cls(
            interpreter_path=str(raw.get("interpreterPath", "python")),
            analysis_source_roots=[str(entry) for entry in source_roots],
            log_level=str(raw.get("logLevel", "info")),
            workspace_trust_mode=str(raw.get("workspaceTrustMode", "restricted"))
        )

