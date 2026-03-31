from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path


@dataclass(frozen=True, slots=True)
class SourcePosition:
    line: int
    character: int


@dataclass(frozen=True, slots=True)
class SourceRange:
    start: SourcePosition
    end: SourcePosition

    @classmethod
    def from_offsets(
        cls,
        start_line: int,
        start_character: int,
        end_line: int | None = None,
        end_character: int | None = None,
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


@dataclass(slots=True)
class SymbolRecord:
    name: str
    kind: str
    module_name: str
    file_path: Path
    source_range: SourceRange
    detail: str = ""
    docstring: str | None = None

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


@dataclass(slots=True)
class ImportBinding:
    alias: str
    module_name: str
    target_name: str | None
    source_range: SourceRange
    is_lazy: bool = False


@dataclass(slots=True)
class ModuleRecord:
    module_name: str
    file_path: Path
    language: str
    source: str
    docstring: str | None = None
    symbols: dict[str, SymbolRecord] = field(default_factory=dict)
    bindings: dict[str, ImportBinding] = field(default_factory=dict)
    star_imports: list[str] = field(default_factory=list)


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
