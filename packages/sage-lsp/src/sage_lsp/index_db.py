from __future__ import annotations

import json
from pathlib import Path
import sqlite3
from typing import Optional


INDEX_DB_SCHEMA_VERSION = 1


class IndexDatabase:
    def __init__(self, path: Path) -> None:
        self.path = path

    def load_entries(self) -> tuple[bool, dict[str, dict[str, object]]]:
        if not self.path.exists():
            return False, {}
        try:
            with self._connect() as connection:
                self._ensure_schema(connection)
                complete = self._read_complete(connection)
                entries: dict[str, dict[str, object]] = {}
                for path, module_name, mtime_ns, size, summary_json in connection.execute(
                    """
                    select path, module_name, mtime_ns, size, summary_json
                    from files
                    order by path
                    """
                ):
                    entry = self._entry_from_row(
                        module_name,
                        mtime_ns,
                        size,
                        summary_json,
                    )
                    if entry is not None:
                        entries[path] = entry
        except (OSError, sqlite3.DatabaseError):
            return False, {}
        return complete, entries

    def query_entries(self, needle: str) -> tuple[bool, dict[str, dict[str, object]]]:
        if not self.path.exists() or not needle:
            return False, {}
        like_pattern = f"%{needle.casefold()}%"
        try:
            with self._connect() as connection:
                self._ensure_schema(connection)
                complete = self._read_complete(connection)
                rows = connection.execute(
                    """
                    select distinct f.path, f.module_name, f.mtime_ns, f.size, f.summary_json
                    from files f
                    left join symbols s on s.path = f.path
                    left join exports e on e.path = f.path
                    where lower(f.module_name) like ?
                       or lower(s.name) like ?
                       or lower(s.container_name) like ?
                       or lower(e.name) like ?
                    order by f.path
                    """,
                    (like_pattern, like_pattern, like_pattern, like_pattern),
                )
                entries: dict[str, dict[str, object]] = {}
                for path, module_name, mtime_ns, size, summary_json in rows:
                    entry = self._entry_from_row(
                        module_name,
                        mtime_ns,
                        size,
                        summary_json,
                    )
                    if entry is not None:
                        entries[path] = entry
        except (OSError, sqlite3.DatabaseError):
            return False, {}
        return complete, entries

    def write_entries(
        self,
        complete: bool,
        entries: dict[str, dict[str, object]],
    ) -> bool:
        try:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            with self._connect() as connection:
                self._ensure_schema(connection)
                with connection:
                    connection.execute("delete from metadata")
                    connection.execute("delete from exports")
                    connection.execute("delete from symbols")
                    connection.execute("delete from files")
                    connection.executemany(
                        "insert into metadata(key, value) values(?, ?)",
                        (
                            ("schemaVersion", str(INDEX_DB_SCHEMA_VERSION)),
                            ("complete", "1" if complete else "0"),
                        ),
                    )
                    for path, entry in sorted(entries.items()):
                        self._insert_entry(connection, path, entry)
        except (OSError, sqlite3.DatabaseError):
            return False
        return True

    def _connect(self) -> sqlite3.Connection:
        connection = sqlite3.connect(self.path)
        connection.execute("pragma journal_mode=wal")
        connection.execute("pragma synchronous=normal")
        return connection

    def _ensure_schema(self, connection: sqlite3.Connection) -> None:
        connection.executescript(
            """
            create table if not exists metadata (
              key text primary key,
              value text not null
            );
            create table if not exists files (
              path text primary key,
              module_name text not null,
              mtime_ns integer not null,
              size integer not null,
              summary_json text not null
            );
            create table if not exists symbols (
              path text not null,
              name text not null,
              kind text not null,
              container_name text not null,
              start_line integer not null,
              start_character integer not null,
              end_line integer not null,
              end_character integer not null,
              primary key(path, name, container_name, start_line, start_character)
            );
            create table if not exists exports (
              path text not null,
              name text not null,
              primary key(path, name)
            );
            create index if not exists idx_symbols_name on symbols(name);
            create index if not exists idx_exports_name on exports(name);
            create index if not exists idx_files_module_name on files(module_name);
            """
        )
        schema_version = connection.execute(
            "select value from metadata where key = 'schemaVersion'"
        ).fetchone()
        if schema_version is not None and schema_version[0] != str(INDEX_DB_SCHEMA_VERSION):
            connection.executescript(
                """
                delete from metadata;
                delete from exports;
                delete from symbols;
                delete from files;
                """
            )

    def _read_complete(self, connection: sqlite3.Connection) -> bool:
        row = connection.execute("select value from metadata where key = 'complete'").fetchone()
        return bool(row and row[0] == "1")

    def _insert_entry(
        self,
        connection: sqlite3.Connection,
        path: str,
        entry: dict[str, object],
    ) -> None:
        module_name = entry.get("moduleName")
        fingerprint = entry.get("fingerprint")
        summary = entry.get("summary")
        if not isinstance(module_name, str) or not isinstance(fingerprint, dict) or not isinstance(summary, dict):
            return
        mtime_ns = fingerprint.get("mtimeNs")
        size = fingerprint.get("size")
        if not isinstance(mtime_ns, int) or not isinstance(size, int):
            return
        summary_json = json.dumps(summary, separators=(",", ":"), sort_keys=True)
        connection.execute(
            """
            insert into files(path, module_name, mtime_ns, size, summary_json)
            values(?, ?, ?, ?, ?)
            """,
            (path, module_name, mtime_ns, size, summary_json),
        )
        exports = summary.get("exports")
        if isinstance(exports, list):
            connection.executemany(
                "insert or ignore into exports(path, name) values(?, ?)",
                ((path, export) for export in exports if isinstance(export, str)),
            )
        symbols = summary.get("symbols")
        if isinstance(symbols, list):
            connection.executemany(
                """
                insert or ignore into symbols(
                  path, name, kind, container_name,
                  start_line, start_character, end_line, end_character
                ) values(?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    row
                    for symbol in symbols
                    if (row := _symbol_row(path, symbol)) is not None
                ),
            )

    def _entry_from_row(
        self,
        module_name: object,
        mtime_ns: object,
        size: object,
        summary_json: object,
    ) -> Optional[dict[str, object]]:
        if not isinstance(module_name, str) or not isinstance(mtime_ns, int) or not isinstance(size, int):
            return None
        if not isinstance(summary_json, str):
            return None
        try:
            summary = json.loads(summary_json)
        except json.JSONDecodeError:
            return None
        if not isinstance(summary, dict):
            return None
        return {
            "moduleName": module_name,
            "fingerprint": {"mtimeNs": mtime_ns, "size": size},
            "summary": summary,
        }


def _symbol_row(path: str, symbol: object) -> Optional[tuple[str, str, str, str, int, int, int, int]]:
    if not isinstance(symbol, dict):
        return None
    name = symbol.get("name")
    kind = symbol.get("kind")
    container_name = symbol.get("containerName", "")
    source_range = symbol.get("sourceRange")
    if not isinstance(name, str) or not isinstance(kind, str) or not isinstance(container_name, str):
        return None
    if not isinstance(source_range, dict):
        return None
    return (
        path,
        name,
        kind,
        container_name,
        int(source_range.get("startLine", 0)),
        int(source_range.get("startCharacter", 0)),
        int(source_range.get("endLine", 0)),
        int(source_range.get("endCharacter", 0)),
    )
