from __future__ import annotations

from dataclasses import dataclass


_CODE = "code"
_SINGLE = "single"
_DOUBLE = "double"
_TRIPLE_SINGLE = "triple_single"
_TRIPLE_DOUBLE = "triple_double"


@dataclass(frozen=True)
class MappedPosition:
    line: int
    character: int


@dataclass(frozen=True)
class MappedRange:
    start: MappedPosition
    end: MappedPosition


@dataclass
class LineMap:
    source_line: str
    generated_line: str
    source_to_generated: list[int]
    generated_to_source: list[int]

    def map_source_character(self, character: int) -> int:
        bounded = min(max(character, 0), len(self.source_to_generated) - 1)
        return self.source_to_generated[bounded]

    def map_generated_character(self, character: int) -> int:
        bounded = min(max(character, 0), len(self.generated_to_source) - 1)
        return self.generated_to_source[bounded]


@dataclass
class PreprocessedDocument:
    original_text: str
    generated_text: str
    line_maps: list[LineMap]
    changed: bool

    def map_source_to_generated(self, line: int, character: int) -> MappedPosition:
        line_index = min(max(line, 0), len(self.line_maps) - 1)
        return MappedPosition(
            line=line_index,
            character=self.line_maps[line_index].map_source_character(character),
        )

    def map_generated_to_source(self, line: int, character: int) -> MappedPosition:
        line_index = min(max(line, 0), len(self.line_maps) - 1)
        return MappedPosition(
            line=line_index,
            character=self.line_maps[line_index].map_generated_character(character),
        )

    def map_generated_range_to_source(
        self,
        start_line: int,
        start_character: int,
        end_line: int,
        end_character: int,
    ) -> MappedRange:
        start = self.map_generated_to_source(start_line, start_character)
        end = self.map_generated_to_source(end_line, end_character)
        return self._normalized_range(start, end)

    def project_generated_error_range(
        self,
        line: int,
        character: int,
        end_character: int,
    ) -> MappedRange:
        return self.map_generated_range_to_source(line, character, line, end_character)

    def _normalized_range(
        self,
        start: MappedPosition,
        end: MappedPosition,
    ) -> MappedRange:
        if start.line != end.line or end.character > start.character:
            return MappedRange(start=start, end=end)

        line_index = min(max(start.line, 0), len(self.line_maps) - 1)
        source_line_length = len(self.line_maps[line_index].source_line)
        if source_line_length == 0:
            return MappedRange(start=MappedPosition(line=line_index, character=0), end=MappedPosition(line=line_index, character=0))

        anchor_character = min(max(start.character, 0), source_line_length)
        if anchor_character >= source_line_length:
            return MappedRange(
                start=MappedPosition(line=line_index, character=max(source_line_length - 1, 0)),
                end=MappedPosition(line=line_index, character=source_line_length),
            )

        return MappedRange(
            start=MappedPosition(line=line_index, character=anchor_character),
            end=MappedPosition(line=line_index, character=min(anchor_character + 1, source_line_length)),
        )


def preprocess_document(uri: str, text: str) -> PreprocessedDocument:
    if uri.lower().endswith(".sage"):
        return preprocess_sage_source(text)
    return _identity_document(text)


def preprocess_sage_source(text: str) -> PreprocessedDocument:
    line_parts = _split_lines(text)
    state = _CODE
    line_maps: list[LineMap] = []
    generated_chunks: list[str] = []
    changed = False

    for source_line, line_ending in line_parts:
        line_map, state, line_changed = _rewrite_line(source_line, state)
        line_maps.append(line_map)
        generated_chunks.append(line_map.generated_line + line_ending)
        changed = changed or line_changed

    return PreprocessedDocument(
        original_text=text,
        generated_text="".join(generated_chunks),
        line_maps=line_maps,
        changed=changed,
    )


def _identity_document(text: str) -> PreprocessedDocument:
    line_maps = [
        LineMap(
            source_line=source_line,
            generated_line=source_line,
            source_to_generated=list(range(len(source_line) + 1)),
            generated_to_source=list(range(len(source_line) + 1)),
        )
        for source_line, _ in _split_lines(text)
    ]
    return PreprocessedDocument(
        original_text=text,
        generated_text=text,
        line_maps=line_maps,
        changed=False,
    )


def _rewrite_line(source_line: str, state: str) -> tuple[LineMap, str, bool]:
    generated_parts: list[str] = []
    source_to_generated = [0]
    generated_to_source = [0]
    generated_length = 0
    changed = False
    index = 0

    while index < len(source_line):
        if state == _CODE:
            if source_line.startswith("'''", index):
                generated_length = _emit_unchanged(
                    "'''", index, generated_parts, source_to_generated, generated_to_source, generated_length
                )
                index += 3
                state = _TRIPLE_SINGLE
                continue
            if source_line.startswith('"""', index):
                generated_length = _emit_unchanged(
                    '"""', index, generated_parts, source_to_generated, generated_to_source, generated_length
                )
                index += 3
                state = _TRIPLE_DOUBLE
                continue

            char = source_line[index]
            next_char = source_line[index + 1] if index + 1 < len(source_line) else ""
            prev_char = source_line[index - 1] if index > 0 else ""

            if char == "#":
                remainder = source_line[index:]
                generated_length = _emit_unchanged(
                    remainder, index, generated_parts, source_to_generated, generated_to_source, generated_length
                )
                index = len(source_line)
                continue
            if char == "'":
                generated_length = _emit_unchanged(
                    char, index, generated_parts, source_to_generated, generated_to_source, generated_length
                )
                index += 1
                state = _SINGLE
                continue
            if char == '"':
                generated_length = _emit_unchanged(
                    char, index, generated_parts, source_to_generated, generated_to_source, generated_length
                )
                index += 1
                state = _DOUBLE
                continue
            if char == "^" and prev_char != "^" and next_char != "^":
                generated_parts.append("**")
                generated_length += 2
                generated_to_source.extend([index, index + 1])
                source_to_generated.append(generated_length)
                index += 1
                changed = True
                continue
            if char == "." and next_char == ".":
                after_next = source_line[index + 2] if index + 2 < len(source_line) else ""
                if prev_char != "." and after_next != ".":
                    generated_parts.append(",")
                    generated_length += 1
                    generated_to_source.append(index + 2)
                    source_to_generated.extend([generated_length, generated_length])
                    index += 2
                    changed = True
                    continue

            generated_length = _emit_unchanged(
                char, index, generated_parts, source_to_generated, generated_to_source, generated_length
            )
            index += 1
            continue

        if state == _SINGLE:
            chunk, advance, next_state = _consume_simple_string(source_line, index, "'")
        elif state == _DOUBLE:
            chunk, advance, next_state = _consume_simple_string(source_line, index, '"')
        elif state == _TRIPLE_SINGLE:
            chunk, advance, next_state = _consume_triple_string(source_line, index, "'''")
        else:
            chunk, advance, next_state = _consume_triple_string(source_line, index, '"""')

        generated_length = _emit_unchanged(
            chunk, index, generated_parts, source_to_generated, generated_to_source, generated_length
        )
        index += advance
        state = next_state

    return (
        LineMap(
            source_line=source_line,
            generated_line="".join(generated_parts),
            source_to_generated=source_to_generated,
            generated_to_source=generated_to_source,
        ),
        state,
        changed,
    )


def _consume_simple_string(source_line: str, index: int, quote: str) -> tuple[str, int, str]:
    if source_line[index] == "\\" and index + 1 < len(source_line):
        return source_line[index : index + 2], 2, _SINGLE if quote == "'" else _DOUBLE
    if source_line[index] == quote:
        return quote, 1, _CODE
    return source_line[index], 1, _SINGLE if quote == "'" else _DOUBLE


def _consume_triple_string(source_line: str, index: int, quote: str) -> tuple[str, int, str]:
    if source_line.startswith(quote, index):
        return quote, 3, _CODE
    return source_line[index], 1, _TRIPLE_SINGLE if quote == "'''" else _TRIPLE_DOUBLE


def _emit_unchanged(
    chunk: str,
    source_start: int,
    generated_parts: list[str],
    source_to_generated: list[int],
    generated_to_source: list[int],
    generated_length: int,
) -> int:
    for offset, char in enumerate(chunk):
        generated_parts.append(char)
        generated_length += 1
        generated_to_source.append(source_start + offset + 1)
        source_to_generated.append(generated_length)
    return generated_length


def _split_lines(text: str) -> list[tuple[str, str]]:
    if text == "":
        return [("", "")]

    raw_lines = text.splitlines(keepends=True)
    result: list[tuple[str, str]] = []

    for raw_line in raw_lines:
        if raw_line.endswith("\r\n"):
            result.append((raw_line[:-2], "\r\n"))
        elif raw_line.endswith("\n") or raw_line.endswith("\r"):
            result.append((raw_line[:-1], raw_line[-1]))
        else:
            result.append((raw_line, ""))

    if text.endswith("\n") or text.endswith("\r"):
        result.append(("", ""))

    return result
