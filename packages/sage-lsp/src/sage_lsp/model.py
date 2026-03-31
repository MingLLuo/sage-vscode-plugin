from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional


@dataclass(frozen=True)
class SourcePosition:
    line: int
    character: int


@dataclass(frozen=True)
class SourceRange:
    start: SourcePosition
    end: SourcePosition

    @classmethod
    def from_offsets(
        cls,
        start_line: int,
        start_character: int,
        end_line: Optional[int] = None,
        end_character: Optional[int] = None,
    ) -> "SourceRange":
        final_end_line = start_line if end_line is None else end_line
        final_end_character = start_character if end_character is None else end_character
        return cls(
            start=SourcePosition(line=max(0, start_line - 1), character=max(0, start_character)),
            end=SourcePosition(line=max(0, final_end_line - 1), character=max(0, final_end_character)),
        )

    def to_lsp(self) -> dict[str, dict[str, int]]:
        return {
            "start": {"line": self.start.line, "character": self.start.character},
            "end": {"line": self.end.line, "character": self.end.character},
        }


@dataclass
class SymbolRecord:
    name: str
    kind: str
    module_name: str
    file_path: Path
    source_range: SourceRange
    detail: str = ""
    docstring: Optional[str] = None

    def location(self) -> dict[str, object]:
        return {
            "uri": self.file_path.as_uri(),
            "range": self.source_range.to_lsp(),
        }

    def completion_item(self) -> dict[str, object]:
        return {
            "label": self.name,
            "kind": completion_kind(self.kind),
            "detail": self.detail or self.module_name,
        }


@dataclass
class ImportBinding:
    alias: str
    module_name: str
    target_name: Optional[str]
    source_range: SourceRange
    is_lazy: bool = False


@dataclass
class ModuleRecord:
    module_name: str
    file_path: Path
    language: str
    source: str
    docstring: Optional[str] = None
    symbols: dict[str, SymbolRecord] = field(default_factory=dict)
    bindings: dict[str, ImportBinding] = field(default_factory=dict)
    star_imports: list[str] = field(default_factory=list)
    member_symbols: dict[str, dict[str, SymbolRecord]] = field(default_factory=dict)
    member_bindings: dict[str, dict[str, ImportBinding]] = field(default_factory=dict)
    instance_types: dict[str, str] = field(default_factory=dict)
    diagnostics: list[dict[str, object]] = field(default_factory=list)


def completion_kind(kind: str) -> int:
    mapping = {
        "module": 9,
        "class": 7,
        "function": 3,
        "variable": 6,
        "constant": 21,
    }
    return mapping.get(kind, 6)


def document_symbol_kind(kind: str) -> int:
    mapping = {
        "module": 2,
        "class": 5,
        "function": 12,
        "variable": 13,
        "constant": 14,
    }
    return mapping.get(kind, 13)
