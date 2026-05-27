from __future__ import annotations

from contextlib import contextmanager
from dataclasses import dataclass
import logging
import sys
import time
from typing import Iterator


LEVELS = {
    "error": logging.ERROR,
    "warn": logging.WARNING,
    "warning": logging.WARNING,
    "info": logging.INFO,
    "debug": logging.DEBUG,
}


@dataclass(frozen=True)
class TraceEvent:
    component: str
    message: str
    fields: dict[str, object]


class TraceLogger:
    def __init__(self, level: str = "info", component: str = "lsp") -> None:
        self.component = component
        self.recent_events: list[TraceEvent] = []
        self._logger = logging.getLogger("sage_lsp")
        self._logger.propagate = False
        if not self._logger.handlers:
            handler = logging.StreamHandler(sys.stderr)
            handler.setFormatter(logging.Formatter("[%(levelname)s] [%(name)s] %(message)s"))
            self._logger.addHandler(handler)
        self.set_level(level)

    def set_level(self, level: str) -> None:
        self.level = level if level in LEVELS else "info"
        self._logger.setLevel(LEVELS[self.level])

    def debug(self, message: str, **fields: object) -> None:
        self._log(logging.DEBUG, message, fields)

    def info(self, message: str, **fields: object) -> None:
        self._log(logging.INFO, message, fields)

    def warning(self, message: str, **fields: object) -> None:
        self._log(logging.WARNING, message, fields)

    def error(self, message: str, **fields: object) -> None:
        self._log(logging.ERROR, message, fields)

    @contextmanager
    def span(self, message: str, **fields: object) -> Iterator[None]:
        start = time.perf_counter()
        self.debug(f"{message}.start", **fields)
        try:
            yield
        except Exception as error:
            elapsed_ms = round((time.perf_counter() - start) * 1000, 3)
            self.error(
                f"{message}.error",
                **fields,
                elapsed_ms=elapsed_ms,
                error=type(error).__name__,
            )
            raise
        else:
            elapsed_ms = round((time.perf_counter() - start) * 1000, 3)
            self.debug(f"{message}.end", **fields, elapsed_ms=elapsed_ms)

    def _log(self, level: int, message: str, fields: dict[str, object]) -> None:
        if not self._logger.isEnabledFor(level):
            return
        self.recent_events.append(TraceEvent(self.component, message, dict(fields)))
        del self.recent_events[:-200]
        formatted = " ".join(
            f"{key}={_format_value(value)}"
            for key, value in sorted(fields.items())
            if value is not None
        )
        suffix = f" {formatted}" if formatted else ""
        self._logger.log(level, f"[{self.component}] {message}{suffix}")


def _format_value(value: object) -> str:
    text = str(value)
    if any(char.isspace() for char in text):
        return repr(text)
    return text
