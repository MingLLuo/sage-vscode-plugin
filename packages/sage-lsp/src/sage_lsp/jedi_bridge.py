from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Optional


JEDI_COMPLETION_KIND = {
    "class": 7,
    "function": 3,
    "module": 9,
    "instance": 6,
    "param": 6,
    "path": 17,
    "keyword": 14,
    "property": 10,
    "statement": 6,
}


@dataclass(frozen=True)
class JediCompletion:
    label: str
    kind: int
    detail: str

    def to_completion_item(self) -> dict[str, object]:
        return {
            "label": self.label,
            "kind": self.kind,
            "detail": self.detail,
        }


class JediBridge:
    def __init__(self, enabled: bool = True) -> None:
        self.enabled = enabled
        self._jedi = _load_jedi() if enabled else None

    @property
    def available(self) -> bool:
        return self._jedi is not None

    def completion_items(
        self,
        source: str,
        file_path: Path,
        line: int,
        character: int,
        prefix: str = "",
    ) -> list[dict[str, object]]:
        completions = self._complete(source, file_path, line, character)
        if not completions:
            return []
        items: list[dict[str, object]] = []
        seen: set[str] = set()
        for completion in completions:
            if prefix and not completion.label.startswith(prefix):
                continue
            if completion.label in seen:
                continue
            seen.add(completion.label)
            items.append(completion.to_completion_item())
        return items

    def _complete(
        self,
        source: str,
        file_path: Path,
        line: int,
        character: int,
    ) -> list[JediCompletion]:
        if self._jedi is None:
            return []
        try:
            script = self._jedi.Script(code=source, path=str(file_path))
            completions = script.complete(line + 1, character)
        except Exception:
            return []
        return [
            JediCompletion(
                label=completion.name,
                kind=JEDI_COMPLETION_KIND.get(completion.type, 6),
                detail=f"jedi {completion.type}",
            )
            for completion in completions
            if completion.name
        ]


def _load_jedi() -> Optional[object]:
    try:
        import jedi  # type: ignore[import-not-found]
    except Exception:
        return None
    return jedi
