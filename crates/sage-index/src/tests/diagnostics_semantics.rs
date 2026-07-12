use super::*;

#[test]
fn diagnostics_report_incomplete_sage_caret() {
    let diagnostics = diagnostics_for_source(Path::new("demo.sage"), "value = 2^\n");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "syntax-error");
    assert_eq!(diagnostics[0].severity, "error");
    assert_eq!(diagnostics[0].range.start_character, 9);
}

#[test]
fn diagnostics_warn_for_sage_caret_exponents_in_python_only() {
    let source = [
        "from sage.all import PolynomialRing, QQ",
        "R = PolynomialRing(QQ, 'x')",
        "value = x^2 + 1",
        "text = 'x^2'",
        "# y^3 stays a comment",
    ]
    .join("\n");
    let diagnostics = diagnostics_for_source(Path::new("demo.py"), &source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "sage-python-caret-exponent");
    assert_eq!(diagnostics[0].severity, "warning");
    assert_eq!(diagnostics[0].range.start_line, 2);
    assert_eq!(diagnostics[0].range.start_character, 9);

    let ordinary_python =
        diagnostics_for_source(Path::new("ordinary.py"), "value = flags ^ mask\n");
    assert!(ordinary_python.is_empty(), "{ordinary_python:?}");

    let sage_source = diagnostics_for_source(Path::new("demo.sage"), "value = x^2 + 1\n");
    assert!(sage_source.is_empty(), "{sage_source:?}");
}

#[test]
fn diagnostics_allow_sage_range_syntax() {
    let diagnostics =
        diagnostics_for_source(Path::new("demo.sage"), "values = [n^2 for n in [1..5]]\n");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn diagnostics_allow_preparser_assignment_rhs_operators() {
    let source = "K.<i> = NumberField(w^2 + 1)\nF.<a> = GF(2^8, name=\"a\")\nS.<Y> = Kfun[]\n";
    let diagnostics = diagnostics_for_source(Path::new("demo.sage"), source);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn function_call_context_tracks_keyword_arguments() {
    let source = "result = trace_window(w^2 + 3*w + 1, width=7)\n";
    let character = source.find("width").unwrap() as u32 + 2;
    assert_eq!(
        function_call_at_position(source, 0, character),
        Some(("trace_window".to_string(), 1))
    );
}

#[test]
fn function_call_context_ignores_nested_tuple_commas() {
    let source = "quotient_ring = R.quotient(I, names=(\"xb\", \"yb\", \"zb\"))\n";
    let character = source.find("\"yb\"").unwrap() as u32 + 2;
    assert_eq!(
        function_call_at_position(source, 0, character),
        Some(("quotient".to_string(), 1))
    );
}

#[test]
fn function_call_context_spans_multiline_calls() {
    let source = "result = trace_window(\n    w^2 + 1,\n    width=7,\n)\n";
    let character = source.lines().nth(2).unwrap().find("width").unwrap() as u32 + 2;
    assert_eq!(
        function_call_at_position(source, 2, character),
        Some(("trace_window".to_string(), 1))
    );
}

#[test]
fn cython_declaration_signature_does_not_require_colon() {
    let file = parse_source(
        "native_support",
        Path::new("native_support.pxd"),
        "cpdef int native_step(int value)\n",
    );
    assert_eq!(
        file.symbols
            .iter()
            .find(|symbol| symbol.name == "native_step")
            .and_then(|symbol| symbol.signature.as_deref()),
        Some("native_step(int value)")
    );
}

#[test]
fn references_skip_strings_and_comments() {
    let refs = references_in_source(
        Path::new("demo.py"),
        "target()\ntext = 'target'\n# target\n",
        "target",
    );
    assert_eq!(refs.len(), 1);
}

#[test]
fn code_reference_range_checks_one_token_without_full_reference_scan() {
    let source = "target()\ntext = 'target'\n# target\nother_target()\n";
    assert!(is_code_reference_at_range(
        source,
        "target",
        &SourceRange {
            start_line: 0,
            start_character: 0,
            end_line: 0,
            end_character: 6,
        },
    ));
    assert!(!is_code_reference_at_range(
        source,
        "target",
        &SourceRange {
            start_line: 1,
            start_character: 8,
            end_line: 1,
            end_character: 14,
        },
    ));
    assert!(!is_code_reference_at_range(
        source,
        "target",
        &SourceRange {
            start_line: 3,
            start_character: 6,
            end_line: 3,
            end_character: 12,
        },
    ));
}

#[test]
fn semantic_spans_include_sage_domains() {
    let spans = semantic_spans("R.<x> = PolynomialRing(QQ)\nvalue = PolynomialRing(QQ)\n@cached_method\ndef f():\n    local_value = 2\n    return graphs.PetersenGraph()\n");
    assert!(spans.iter().any(|span| span.token_type == "type"));
    assert!(spans.iter().any(|span| span.token_type == "namespace"));
    assert!(spans.iter().any(|span| span.token_type == "parameter"));
    assert!(spans.iter().any(|span| span.token_type == "decorator"));
    assert!(spans.iter().any(|span| span.line == 1
        && span.start == 0
        && span.length == "value".len() as u32
        && span.token_type == "variable"
        && span
            .modifiers
            .iter()
            .any(|modifier| modifier == "declaration")));
    assert!(spans.iter().any(|span| span.line == 4
        && span.start == 4
        && span.length == "local_value".len() as u32
        && span.token_type == "variable"
        && span
            .modifiers
            .iter()
            .any(|modifier| modifier == "declaration")));
    for pair in spans.windows(2) {
        if pair[0].line == pair[1].line {
            assert!(pair[0].start + pair[0].length <= pair[1].start);
        }
    }
}

#[test]
fn semantic_spans_skip_strings_and_comments() {
    let spans = semantic_spans("text = 'PolynomialRing'\n# graphs\nvalue = PolynomialRing(QQ)\n");
    assert_eq!(
        spans
            .iter()
            .filter(|span| span.token_type == "type")
            .count(),
        1
    );
}
