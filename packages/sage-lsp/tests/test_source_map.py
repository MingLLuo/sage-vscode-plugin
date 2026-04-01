from sage_lsp.source_map import preprocess_document, preprocess_sage_source


def test_rewrites_standalone_caret_and_maps_columns() -> None:
    document = preprocess_sage_source("a^2\nb = c^d")

    assert document.generated_text == "a**2\nb = c**d"
    assert document.changed is True

    mapped = document.map_source_to_generated(0, 2)
    assert (mapped.line, mapped.character) == (0, 3)

    original = document.map_generated_to_source(0, 2)
    assert (original.line, original.character) == (0, 1)


def test_skips_comments_and_strings_including_triple_quoted_blocks() -> None:
    text = "\"x^y\" # keep^comment\n'''a^b\nstill^string'''\nc^d"

    document = preprocess_sage_source(text)

    assert document.generated_text == "\"x^y\" # keep^comment\n'''a^b\nstill^string'''\nc**d"
    assert document.line_maps[0].generated_line == "\"x^y\" # keep^comment"
    assert document.line_maps[1].generated_line == "'''a^b"
    assert document.line_maps[2].generated_line == "still^string'''"
    assert document.line_maps[3].generated_line == "c**d"


def test_non_sage_documents_are_left_unchanged() -> None:
    document = preprocess_document("file:///tmp/example.py", "value = a^b\n")

    assert document.generated_text == "value = a^b\n"
    assert document.changed is False
    assert document.map_source_to_generated(0, 9).character == 9


def test_preserves_trailing_newline_line_count() -> None:
    document = preprocess_sage_source("x^2\n")

    assert document.generated_text == "x**2\n"
    assert len(document.line_maps) == 2
    assert document.line_maps[1].source_line == ""
    assert document.map_source_to_generated(1, 0).character == 0


def test_projects_generated_error_spans_back_to_source_positions() -> None:
    document = preprocess_sage_source("value = 2^\n")

    projected = document.project_generated_error_range(0, 11, 12)

    assert (projected.start.line, projected.start.character) == (0, 9)
    assert (projected.end.line, projected.end.character) == (0, 10)
