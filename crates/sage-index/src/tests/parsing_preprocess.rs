use super::*;

#[test]
fn preprocess_rewrites_caret_outside_strings_and_comments() {
    let result = preprocess_sage_source("x = y^2\ns = '^'\n# z^2\n");
    assert_eq!(result.generated, "x = y**2\ns = '^'\n# z^2\n");
    assert_eq!(result.edits.len(), 1);
}

#[test]
fn preprocess_rewrites_sage_ranges_outside_strings_and_comments() {
    let result = preprocess_sage_source("xs = [1..5]\nys = [1 .. width]\ns = '1..5'\n# [1..5]\n");
    assert_eq!(
        result.generated,
        "xs = [1,5]\nys = [1 , width]\ns = '1..5'\n# [1..5]\n"
    );
    assert_eq!(
        result
            .edits
            .iter()
            .filter(|edit| edit.source_text == ".." && edit.generated_text == ",")
            .count(),
        2
    );
}

#[test]
fn preprocess_rewrites_empty_sage_index_after_ring_owner() {
    let result = preprocess_sage_source("S = Kfun[]\nempty = []\ntext = 'Kfun[]'\n");
    assert_eq!(
        result.generated,
        "S = Kfun[0]\nempty = []\ntext = 'Kfun[]'\n"
    );
    assert_eq!(
        result
            .edits
            .iter()
            .filter(|edit| edit.source_text == "[]" && edit.generated_text == "[0]")
            .count(),
        1
    );
}

#[test]
fn parser_extracts_python_and_preparser_symbols() {
    let file = parse_source(
            "demo",
            Path::new("demo.sage"),
            "R.<x, y> = PolynomialRing(QQ, 2)\nPublicFactory = object()\nclass Solver:\n    pass\n\ndef helper():\n    \"\"\"Return help.\"\"\"\n    return x\n",
        );
    let names: Vec<_> = file
        .symbols
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect();
    assert!(names.contains(&"R"));
    assert!(names.contains(&"x"));
    assert!(names.contains(&"PublicFactory"));
    assert!(names.contains(&"Solver"));
    assert!(names.contains(&"helper"));
    assert_eq!(
        file.symbols
            .iter()
            .find(|symbol| symbol.name == "helper")
            .and_then(|symbol| symbol.docstring.as_deref()),
        Some("Return help.")
    );
}

#[test]
fn parser_handles_non_ascii_text_before_a_definition() {
    let file = parse_source(
        "unicode_demo",
        Path::new("unicode_demo.sage"),
        "π_value = '🚀'\n\ndef target(value):\n    return value\n",
    );
    let target = file
        .symbols
        .iter()
        .find(|symbol| symbol.name == "target")
        .expect("Unicode source should still expose later definitions");
    assert_eq!(target.range.start_line, 2);
}

#[test]
fn parser_extracts_lazy_import_lists_and_aliases() {
    let source = r#"
from sage.misc.lazy_import import lazy_import
from sage.misc.lazy_import import LazyImport

lazy_import("sage.future.module", ["FutureFactory", "FutureThing"])
lazy_import(
    'sage.future.aliases',
    ['FutureAliasSource', 'SecondAliasSource'],
    as_=['FutureAlias', 'SecondAlias'],
)
lazy_import('sage.future.scalar', 'ScalarSource', as_='ScalarAlias')
SymbolicRing = LazyImport('sage.symbolic.ring', 'SymbolicRing')
FiniteGroups = LazyImport(
    'sage.categories.finite_groups',
    'FiniteGroups',
    at_startup=True,
)
"#;
    let file = parse_source("sage.future.all", Path::new("sage/future/all.py"), source);
    let imports: BTreeMap<_, _> = file
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Import)
        .map(|symbol| {
            (
                symbol.name.as_str(),
                symbol.import_from.as_deref().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        imports.get("FutureFactory").copied(),
        Some("sage.future.module::FutureFactory")
    );
    assert_eq!(
        imports.get("FutureThing").copied(),
        Some("sage.future.module::FutureThing")
    );
    assert_eq!(
        imports.get("FutureAlias").copied(),
        Some("sage.future.aliases::FutureAliasSource")
    );
    assert_eq!(
        imports.get("SecondAlias").copied(),
        Some("sage.future.aliases::SecondAliasSource")
    );
    assert_eq!(
        imports.get("ScalarAlias").copied(),
        Some("sage.future.scalar::ScalarSource")
    );
    assert_eq!(
        imports.get("SymbolicRing").copied(),
        Some("sage.symbolic.ring::SymbolicRing")
    );
    assert_eq!(
        imports.get("FiniteGroups").copied(),
        Some("sage.categories.finite_groups::FiniteGroups")
    );
}

#[test]
fn parser_extracts_deprecated_function_aliases() {
    let source = r#"
from sage.misc.superseded import deprecated_function_alias
from sage.future.module import replacement
old_replacement = deprecated_function_alias(12345, replacement)

def local_replacement():
    pass

class Wrapper:
    old_local = deprecated_function_alias(23456, local_replacement)
"#;
    let file = parse_source("sage.future.all", Path::new("sage/future/all.py"), source);
    let imports: BTreeMap<_, _> = file
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Import)
        .map(|symbol| {
            (
                symbol.name.as_str(),
                symbol.import_from.as_deref().unwrap_or_default(),
            )
        })
        .collect();

    assert_eq!(
        imports.get("old_replacement").copied(),
        Some("sage.future.module::replacement")
    );
    assert_eq!(
        imports.get("old_local").copied(),
        Some("sage.future.all::local_replacement")
    );
}

#[test]
fn parser_extracts_top_level_import_member_aliases() {
    let source = r#"
import sage.future.module as future_module
from sage.categories import finite_weyl_groups

class LocalFactory:
    pass

FutureAlias = future_module.FutureFactory
Example = finite_weyl_groups.Example
LocalAlias = LocalFactory

def local():
    Hidden = future_module.HiddenFactory
    LocalHidden = LocalFactory
"#;
    let file = parse_source("sage.future.all", Path::new("sage/future/all.py"), source);
    let imports: BTreeMap<_, _> = file
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Import)
        .map(|symbol| {
            (
                symbol.name.as_str(),
                symbol.import_from.as_deref().unwrap_or_default(),
            )
        })
        .collect();

    assert_eq!(
        imports.get("FutureAlias").copied(),
        Some("sage.future.module::FutureFactory")
    );
    assert_eq!(
        imports.get("Example").copied(),
        Some("sage.categories.finite_weyl_groups::Example")
    );
    assert_eq!(
        imports.get("LocalAlias").copied(),
        Some("sage.future.all::LocalFactory")
    );
    assert!(!imports.contains_key("Hidden"));
    assert!(!imports.contains_key("LocalHidden"));
}

#[test]
fn parser_extracts_class_method_aliases_without_local_assignments() {
    let source = r#"
class MatrixFuture:
    def trace_impl(self):
        """Return a source-derived trace."""
        return 0

    trace_alias = trace_impl
    Element = MatrixFutureElement

    def helper(self):
        hidden_alias = trace_impl
        return hidden_alias()

class MatrixFutureElement:
    pass
"#;
    let file = parse_source(
        "sage.matrix.future",
        Path::new("sage/matrix/future.py"),
        source,
    );
    let imports: BTreeMap<_, _> = file
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Import)
        .map(|symbol| {
            (
                symbol.name.as_str(),
                (
                    symbol.import_from.as_deref().unwrap_or_default(),
                    symbol.detail.as_str(),
                ),
            )
        })
        .collect();

    assert_eq!(
        imports.get("trace_alias").copied(),
        Some((
            "sage.matrix.future::trace_impl",
            "MethodAlias MatrixFuture.trace_alias for trace_impl"
        ))
    );
    assert!(!imports.contains_key("hidden_alias"));
    assert!(!imports.contains_key("Element"));
}

#[test]
fn parser_extracts_matrix_constructor_method_aliases_from_sage_decorators() {
    let source = r#"
from sage.matrix.constructor import matrix

def matrix_method(func=None, name=None):
    return func

@matrix_method
def random_matrix(ring, nrows):
    return matrix([])

@matrix_method(name='unit')
def identity_matrix(ring, n):
    return matrix([])
"#;
    let file = parse_source(
        "sage.matrix.special",
        Path::new("sage/matrix/special.py"),
        source,
    );
    let imports: BTreeMap<_, _> = file
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Import)
        .map(|symbol| {
            (
                symbol.name.as_str(),
                (
                    symbol.import_from.as_deref().unwrap_or_default(),
                    symbol.detail.as_str(),
                ),
            )
        })
        .collect();

    assert_eq!(
        imports.get("random").copied(),
        Some((
            "sage.matrix.special::random_matrix",
            "MatrixConstructorMethodAlias matrix.random for random_matrix"
        ))
    );
    assert_eq!(
        imports.get("unit").copied(),
        Some((
            "sage.matrix.special::identity_matrix",
            "MatrixConstructorMethodAlias matrix.unit for identity_matrix"
        ))
    );
}

#[test]
fn parser_extracts_raw_sage_docstrings() {
    let file = parse_source(
            "demo",
            Path::new("demo.py"),
            "r\"\"\"Module docs.\"\"\"\n\ndef helper():\n    r\"\"\"\n    Return raw docs.\n    \"\"\"\n    return 1\n",
        );

    assert_eq!(file.module_docstring.as_deref(), Some("Module docs."));
    assert_eq!(
        file.symbols
            .iter()
            .find(|symbol| symbol.name == "helper")
            .and_then(|symbol| symbol.docstring.as_deref()),
        Some("Return raw docs.")
    );
}

#[test]
fn preprocess_maps_preparser_assignment() {
    let result = preprocess_sage_source(
            "R.<x, y> = PolynomialRing(QQ, 2)\nK.<i> = NumberField(x^2 + 1)\nS.<Y> = Kfun[]\nxs = [1..5]\nz = x^2\n",
        );
    assert!(result.generated.contains("R = PolynomialRing(QQ, 2)"));
    assert!(result.generated.contains("x = R.gen(0)"));
    assert!(result.generated.contains("K = NumberField(x**2 + 1)"));
    assert!(result.generated.contains("S = Kfun[0]"));
    assert!(result.generated.contains("xs = [1,5]"));
    assert!(result.generated.contains("z = x**2"));
    assert!(result
        .edits
        .iter()
        .any(|edit| edit.generated_text == "preparser-assignment"));
}

#[test]
fn parser_ignores_docstring_examples() {
    let file = parse_source(
            "demo",
            Path::new("demo.py"),
            "\"\"\"\ndef not_real():\n    pass\nR.<a> = PolynomialRing(QQ)\n\"\"\"\ndef real():\n    pass\n",
        );
    let names: Vec<_> = file
        .symbols
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect();
    assert!(!names.contains(&"not_real"));
    assert!(!names.contains(&"R"));
    assert!(!names.contains(&"a"));
    assert!(names.contains(&"real"));
}
