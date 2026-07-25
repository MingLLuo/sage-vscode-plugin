use super::*;

#[test]
fn multiline_preparser_preprocessing_binds_generators_after_the_closing_line() {
    let source = "def build():\n    K.<a> = QuadraticField(\n        2^2 +\n        1)  # field\n    return a\n";
    let result = preprocess_sage_source(source);

    assert_eq!(
        result.generated,
        "def build():\n    K = QuadraticField(\n        2**2 +\n        1); a = K.gen(0)  # field\n    return a\n"
    );
    assert_eq!(
        result.generated.lines().count(),
        source.lines().count(),
        "preprocessing must preserve physical line mapping"
    );
    assert!(result.edits.iter().any(|edit| {
        edit.line == 2
            && edit.source_text == "^"
            && edit.generated_text == "**"
            && edit.source_character == 9
            && edit.generated_character == 9
    }));
    assert!(
        diagnostics_for_source(Path::new("demo.sage"), source).is_empty(),
        "a complete multiline preparser assignment must remain valid after preprocessing"
    );
}

#[test]
fn incomplete_multiline_preparser_does_not_insert_a_generator_binding() {
    let source = "K.<a> = QuadraticField(\n    2 + 1\n";
    let result = preprocess_sage_source(source);

    assert_eq!(result.generated, source);
    assert!(!result.generated.contains("a = K.gen"));
    assert!(!result
        .edits
        .iter()
        .any(|edit| edit.generated_text == "preparser-assignment"));
}

#[test]
fn strict_inference_waits_for_the_complete_multiline_constructor() {
    let source = "K.<a> = QuadraticField(\n    2 +\n    1)  # field\nvalue = a.polynomial()\n";

    assert!(
        infer_owner_type_before_strict(source, "K", "gen", 1).is_none(),
        "an unclosed constructor prefix must not produce an exact owner"
    );
    assert_eq!(assignment_constructor_before_line(source, "K", 1), None);
    assert_eq!(
        infer_owner_type_before_strict(source, "K", "gen", 3),
        Some(SageOwnerType::NumberField)
    );
    assert_eq!(
        infer_owner_type_before_strict(source, "a", "polynomial", 3),
        Some(SageOwnerType::NumberFieldElement)
    );
    assert_eq!(
        assignment_constructor_before_line(source, "K", 3).as_deref(),
        Some("QuadraticField")
    );
    assert_eq!(
        assignment_constructor_before_line(source, "a", 3).as_deref(),
        Some("K.gen")
    );
}

#[test]
fn brackets_inside_strings_and_comments_do_not_close_the_statement() {
    let source =
        "K.<a> = QuadraticField(\n    \")\",  # ) ] }\n    2 + 1\n)\nvalue = a.polynomial()\n";

    assert!(
        infer_owner_type_before_strict(source, "K", "gen", 1).is_none(),
        "string and comment delimiters are not structural evidence"
    );
    assert_eq!(
        infer_owner_type_before_strict(source, "K", "gen", 4),
        Some(SageOwnerType::NumberField)
    );
    let generated = preprocess_sage_source(source).generated;
    assert!(generated.contains("\n); a = K.gen(0)\n"));
    assert_eq!(generated.lines().count(), source.lines().count());
}

#[test]
fn multiline_polynomial_preparser_preserves_proven_subtype_evidence() {
    let source = "R.<x, y> = PolynomialRing(\n    QQ,\n    2\n)\nvalue = x.degree()\n";

    assert_eq!(
        infer_owner_type_before_strict(source, "R", "gen", 4),
        Some(SageOwnerType::MultivariatePolynomialRing)
    );
    assert_eq!(
        infer_owner_type_before_strict(source, "x", "degree", 4),
        Some(SageOwnerType::PolynomialElement)
    );
}

#[test]
fn local_return_inference_understands_multiline_preparser_assignments() {
    let source = "def generator():\n    K.<a> = QuadraticField(\n        2 +\n        1)\n    return a\n\nvalue = generator()\nresult = value.polynomial()\n";

    assert_eq!(
        infer_owner_type_before_strict(source, "value", "polynomial", 7),
        Some(SageOwnerType::NumberFieldElement)
    );
}

#[test]
fn dedented_closing_delimiter_preserves_function_local_preparser_inference() {
    let source = "def generator():\n    K.<a> = QuadraticField(\n        2 +\n        1\n)\n    return a\n\nvalue = generator()\nresult = value.polynomial()\n";

    assert_eq!(
        infer_owner_type_before_strict(source, "value", "polynomial", 8),
        Some(SageOwnerType::NumberFieldElement),
        "a closing delimiter may be physically dedented without ending the function suite"
    );
}

#[test]
fn unclosed_multiline_preparser_never_leaks_constructor_confidence() {
    let source = "K.<a> = QuadraticField(\n    2 + 1\nvalue = a.polynomial()\nresult = K.gen()\n";

    assert!(infer_owner_type_before_strict(source, "K", "gen", 3).is_none());
    assert!(infer_owner_type_before_strict(source, "a", "polynomial", 2).is_none());
    assert_eq!(assignment_constructor_before_line(source, "K", 3), None);
    assert_eq!(assignment_constructor_before_line(source, "a", 3), None);
}

#[test]
fn continuation_walrus_invalidates_old_owner_and_constructor_bindings() {
    let source = "R = matrix(QQ, 1, 1)\nK.<a> = QuadraticField(\n    (R := 2)\n)\nrank = R.rank()\npolynomial = a.polynomial()\n";

    assert!(
        infer_owner_type_before_strict(source, "R", "rank", 4).is_none(),
        "a continuation-line walrus must clear the old Matrix owner"
    );
    assert_eq!(
        assignment_constructor_before_line(source, "R", 4),
        None,
        "a continuation-line walrus must clear the old matrix constructor"
    );
    assert_eq!(
        infer_owner_type_before_strict(source, "K", "gen", 5),
        Some(SageOwnerType::NumberField)
    );
    assert_eq!(
        infer_owner_type_before_strict(source, "a", "polynomial", 5),
        Some(SageOwnerType::NumberFieldElement)
    );
}

#[test]
fn continuation_walrus_invalidates_local_function_return_inference() {
    let source = "def changed():\n    R = matrix(QQ, 1, 1)\n    K.<a> = QuadraticField(\n        (R := 2)\n    )\n    return R\n\nvalue = changed()\nrank = value.rank()\n";

    assert!(
        infer_owner_type_before_strict(source, "value", "rank", 8).is_none(),
        "a local return must not retain the owner replaced by a continuation walrus"
    );
}

#[test]
fn complete_preparser_rhs_still_sees_an_unmodified_old_parent_binding() {
    let source = "R = PolynomialRing(QQ, 2)\nR.<x, y> = R.change_ring(QQ)\nvalue = x.degree()\n";

    assert_eq!(
        infer_owner_type_before_strict(source, "R", "gen", 2),
        Some(SageOwnerType::MultivariatePolynomialRing)
    );
    assert_eq!(
        infer_owner_type_before_strict(source, "x", "degree", 2),
        Some(SageOwnerType::PolynomialElement)
    );
}

#[test]
fn escaped_quotes_keep_carets_in_strings_and_release_following_code() {
    let source = r#"text = "escaped quote: \" ^ stays"
K.<a> = QuadraticField(
    "escaped quote: \" ^ stays",
    x^2
)
value = x^3
"#;
    let result = preprocess_sage_source(source);

    assert!(result
        .generated
        .contains(r#"text = "escaped quote: \" ^ stays""#));
    assert!(result
        .generated
        .contains(r#"    "escaped quote: \" ^ stays","#));
    assert!(result.generated.contains("    x**2"));
    assert!(result.generated.contains("); a = K.gen(0)"));
    assert!(result.generated.contains("value = x**3"));
    let caret_lines: Vec<_> = result
        .edits
        .iter()
        .filter(|edit| edit.source_text == "^")
        .map(|edit| edit.line)
        .collect();
    assert_eq!(caret_lines, vec![3, 5]);
}
