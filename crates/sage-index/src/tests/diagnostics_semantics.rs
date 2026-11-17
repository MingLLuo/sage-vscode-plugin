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
fn pep_695_sage_homset_headers_are_indexed_without_false_diagnostics() {
    let source = "def Hom[DomainElementT: Parent, CodomainElementT: Parent](\n\
    X: DomainElementT,\n\
    Y: CodomainElementT,\n\
) -> Homset[DomainElementT, CodomainElementT]:\n\
    \"\"\"Create a homset.\"\"\"\n\
    return Homset(X, Y)\n\
\n\
class Homset[DomainElementT: Parent, CodomainElementT: Parent](Set_generic):\n\
    \"\"\"The class for collections of morphisms.\"\"\"\n\
    pass\n";

    let parsed = parse_source("sage.categories.homset", Path::new("homset.py"), source);
    let hom = parsed
        .symbols
        .iter()
        .find(|symbol| symbol.name == "Hom")
        .expect("PEP 695 generic function should be indexed");
    assert_eq!(hom.range.start_line, 0);
    assert!(hom.signature.as_deref().is_some_and(|signature| {
        signature.starts_with("Hom(")
            && signature.contains("X: DomainElementT")
            && signature.contains("Y: CodomainElementT")
    }));
    assert!(parsed
        .symbols
        .iter()
        .any(|symbol| symbol.name == "Homset" && symbol.range.start_line == 7));

    let diagnostics = diagnostics_for_source(Path::new("homset.py"), source);
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
    let context =
        function_call_context_at_position(source, 0, source.find('7').unwrap() as u32 + 1)
            .expect("keyword call context");
    assert_eq!(context.keyword.as_deref(), Some("width"));
    assert!(!context.qualified);
    assert_eq!(
        active_parameter_for_keyword(
            "trace_window(poly, base_ring=QQ, *, width=5, normalize=True)",
            "width",
        ),
        Some(3),
    );
}

#[test]
fn call_context_distinguishes_qualified_and_unicode_names() {
    let source = "结果 = 模块.计算(值, 精度=7)\n";
    let character = source.find('7').unwrap() as u32 + 1;
    let context = function_call_context_at_position(source, 0, character)
        .expect("Unicode qualified call context");
    assert_eq!(context.name, "计算");
    assert_eq!(context.keyword.as_deref(), Some("精度"));
    assert!(context.qualified);
    assert_eq!(
        active_parameter_for_keyword("计算(值, *, 精度=5)", "精度"),
        Some(2)
    );
    assert_eq!(
        active_parameter_for_keyword("计算(值, *, 精度=5)", "missing"),
        None
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

#[test]
fn semantic_spans_do_not_label_shadowed_names_as_sage_builtins() {
    let source = "outside = plot(1)\ndef render(plot):\n    return plot(2)\n\ndef other():\n    plot = lambda x: x\n    return plot(3)\n\nplot = lambda x: x\nafter = plot(4)\n";
    let spans = semantic_spans(source);
    let library_plot_lines: Vec<_> = spans
        .iter()
        .filter(|span| {
            span.token_type == "function"
                && span
                    .modifiers
                    .iter()
                    .any(|modifier| modifier == "defaultLibrary")
        })
        .map(|span| span.line)
        .collect();
    assert_eq!(library_plot_lines, vec![0]);
}

#[test]
fn semantic_spans_distinguish_user_decorators_and_unicode_declarations() {
    let source = "@自定义\ndef 变量():\n    return 1\n\n@cached_method\ndef 方法(self):\n    return 变量()\n";
    let spans = semantic_spans(source);
    assert!(spans.iter().any(|span| span.line == 0
        && span.token_type == "decorator"
        && span.length == "自定义".len() as u32
        && span.modifiers.is_empty()));
    assert!(spans.iter().any(|span| span.line == 1
        && span.token_type == "function"
        && span.length == "变量".len() as u32));
    assert!(spans.iter().any(|span| span.line == 4
        && span.token_type == "decorator"
        && span
            .modifiers
            .iter()
            .any(|modifier| modifier == "defaultLibrary")));
}

#[test]
fn semantic_spans_keep_explicit_sage_imports_colored_as_library_names() {
    let source = "from sage.all import QQ\nbase = QQ\n\ndef local():\n    from sage.all import QQ\n    return QQ\n\nfrom sage.misc.cachefunc import cached_method\n@cached_method\ndef cached(self):\n    return QQ\n";
    let spans = semantic_spans(source);
    for line in [1, 5, 10] {
        assert!(
            spans.iter().any(|span| span.line == line
                && span.token_type == "variable"
                && span
                    .modifiers
                    .iter()
                    .any(|modifier| modifier == "defaultLibrary")),
            "line {line}"
        );
    }
    assert!(spans.iter().any(|span| span.line == 8
        && span.token_type == "decorator"
        && span
            .modifiers
            .iter()
            .any(|modifier| modifier == "defaultLibrary")));
}

#[test]
fn local_document_highlights_follow_the_function_binding() {
    let source = "def first():\n    变量 = 1\n    return 变量\n\ndef second():\n    变量 = 2\n    return 变量\n# 变量\n";
    let first = local_document_highlight_ranges(source, "变量", 2, 11).unwrap();
    assert_eq!(
        first
            .iter()
            .map(|range| range.start_line)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    let second = local_document_highlight_ranges(source, "变量", 6, 11).unwrap();
    assert_eq!(
        second
            .iter()
            .map(|range| range.start_line)
            .collect::<Vec<_>>(),
        vec![5, 6]
    );
    assert!(local_document_highlight_ranges(source, "first", 0, 4).is_none());
    let shared_name = "def plot(plot):\n    return plot\n";
    assert!(local_document_highlight_ranges(shared_name, "plot", 0, 4).is_none());
    assert_eq!(
        local_document_highlight_ranges(shared_name, "plot", 0, 9)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn diagnostics_ignore_carets_in_comments_and_multiline_strings() {
    let source = "# documentation ^\ntext = \"\"\"example ^\nstill text ^\n\"\"\"\n";
    assert!(diagnostics_for_source(Path::new("demo.sage"), source).is_empty());
    assert!(diagnostics_for_source(Path::new("demo.py"), source).is_empty());
}

#[test]
fn diagnostics_locate_multiple_errors_without_hiding_following_lines() {
    let source = "value = 2^\nvalid = 3\nbroken = )\n";
    let diagnostics = diagnostics_for_source(Path::new("demo.sage"), source);
    assert!(
        diagnostics
            .iter()
            .any(|d| d.range.start_line == 0 && d.range.end_character >= 7),
        "{diagnostics:?}"
    );
    assert!(
        diagnostics.iter().any(|d| d.range.start_line == 2),
        "{diagnostics:?}"
    );
}

#[test]
fn diagnostics_map_sage_operators_and_generator_prefixes() {
    for line in [
        "value = 2^3 + )",
        "K.<i> = NumberField(2^3 + )",
        "结果 = 2^3 + )",
    ] {
        let source = format!("valid = 1\n{line}\n");
        let diagnostics = diagnostics_for_source(Path::new("demo.sage"), &source);
        assert!(!diagnostics.is_empty(), "{source}");
        for diagnostic in diagnostics {
            assert_eq!(diagnostic.range.start_line, 1, "{diagnostic:?}");
            assert!(diagnostic.range.end_character > 0, "{diagnostic:?}");
            assert!(
                diagnostic.range.end_character <= line.len() as u32,
                "{diagnostic:?}"
            );
        }
    }
}

#[test]
fn diagnostics_locate_errors_on_cr_lines() {
    let source = "valid = 1\rdef broken()\r    pass\r";
    let diagnostics = diagnostics_for_source(Path::new("demo.py"), source);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].range.start_line, 1);
    assert_eq!(diagnostics[0].range.end_line, 2);
}

#[test]
fn diagnostics_allow_exponents_continued_on_the_next_line() {
    let source = "value = (2^\n    3)\n";
    let diagnostics = diagnostics_for_source(Path::new("demo.sage"), source);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn crlf_sources_with_non_ascii_text_keep_exact_line_offsets() {
    // Line offsets used to assume a one-byte terminator, so every CRLF line shifted later slices
    // by one byte until a slice landed inside a multi-byte character and panicked.
    let lf = "def outer(value):\n    label = 'é'\n    other = 'ü'\n    def inner():\n        return value\n    return label\n\nresult = outer(1)\n";
    let crlf = lf.replace('\n', "\r\n");
    let lf_spans = semantic_spans(lf);
    let crlf_spans = semantic_spans(&crlf);
    assert!(!lf_spans.is_empty());
    assert_eq!(
        crlf_spans
            .iter()
            .map(|span| (span.line, span.start, span.length, span.token_type.clone()))
            .collect::<Vec<_>>(),
        lf_spans
            .iter()
            .map(|span| (span.line, span.start, span.length, span.token_type.clone()))
            .collect::<Vec<_>>(),
        "CRLF line endings must not change semantic spans"
    );
    assert_eq!(
        references_in_source(Path::new("demo.py"), &crlf, "value").len(),
        references_in_source(Path::new("demo.py"), lf, "value").len()
    );
}

#[test]
fn function_body_lines_are_sliced_exactly_in_crlf_sources() {
    let source = "def outer(value):\r\n    label = 'é'\r\n    other = 'üü'\r\n    return label\r\n";
    let scopes = LexicalScopeMap::new(source);
    let lines: Vec<(u32, &str)> = scopes.direct_function_source_lines(source, 0).collect();
    assert_eq!(
        lines,
        vec![
            (1, "    label = 'é'"),
            (2, "    other = 'üü'"),
            (3, "    return label"),
        ],
        "body lines must match the source text regardless of the line terminator"
    );
}
