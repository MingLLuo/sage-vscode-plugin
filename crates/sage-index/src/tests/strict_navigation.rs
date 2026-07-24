use super::*;

fn strict_navigation_index(name: &str, files: &[(&str, &str)]) -> (PathBuf, WorkspaceIndex) {
    let root = test_root(name);
    for (relative, source) in files {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, source).unwrap();
    }
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();
    (root, index)
}

#[test]
fn strict_owner_inference_understands_preparser_generator_assignments() {
    let source = "R.<x, y, z> = PolynomialRing(QQ, 3)\nP.<w> = PolynomialRing(QQ)\nF.<a> = GF(7)\nK.<b> = NumberField(w^2 + 1)\nS.<t> = QQ[]\nL.<l> = LaurentPolynomialRing(QQ)\nT.<q> = PowerSeriesRing(QQ)\nB.<u, v> = BooleanPolynomialRing(2)\nr = R.gen()\nd = x.degree()\nu = w.degree()\np = a.parent()\nn = b.polynomial()\ns = S.gen()\ne = t.degree()\n";

    assert_eq!(
        infer_owner_type_before_strict(source, "R", "gen", 5),
        Some(SageOwnerType::MultivariatePolynomialRing)
    );
    assert_eq!(
        infer_owner_type_before_strict(source, "P", "gen", 5),
        Some(SageOwnerType::UnivariatePolynomialRing)
    );
    assert_eq!(
        infer_owner_type_before_strict(source, "x", "degree", 6),
        Some(SageOwnerType::PolynomialElement)
    );
    assert!(
        infer_owner_type_before_strict(source, "w", "degree", 7).is_none(),
        "one preparser generator does not prove a multivariate polynomial implementation"
    );
    assert_eq!(
        infer_owner_type_before_strict(source, "F", "gen", 8),
        Some(SageOwnerType::Field)
    );
    assert_eq!(
        infer_owner_type_before_strict(source, "a", "parent", 8),
        Some(SageOwnerType::FieldElement)
    );
    assert_eq!(
        infer_owner_type_before_strict(source, "b", "polynomial", 9),
        Some(SageOwnerType::NumberFieldElement)
    );
    assert_eq!(
        infer_owner_type_before_strict(source, "S", "gen", 10),
        Some(SageOwnerType::UnivariatePolynomialRing)
    );
    assert!(
        infer_owner_type_before_strict(source, "t", "degree", 11).is_none(),
        "one bracket-syntax generator is still representation-ambiguous"
    );
    for owner in ["L", "T", "B"] {
        assert_eq!(
            infer_owner_type_before_strict(source, owner, "gen", 13),
            Some(SageOwnerType::PolynomialRing),
            "{owner} must not be classified as an ordinary polynomial-ring subtype"
        );
    }

    let rebound = "R.<x, y> = PolynomialRing(QQ, 2)\nx = unknown()\nvalue = x.degree()\n";
    assert!(
        infer_owner_type_before_strict(rebound, "x", "degree", 2).is_none(),
        "an unknown rebinding must clear the preparser generator type"
    );

    let local_return = "def generator():\n    R.<x, y> = PolynomialRing(QQ, 2)\n    return x\n\nvalue = generator()\nresult = value.degree()\n";
    assert_eq!(
        infer_owner_type_before_strict(local_return, "value", "degree", 5),
        Some(SageOwnerType::PolynomialElement)
    );
}

#[test]
fn strict_method_return_inference_depends_on_receiver_type() {
    let source = "v = vector(ZZ, [1, 2])\nw = v.change_ring(QQ)\nF = GF(7)\na = F.gen()\nR.<x, y> = PolynomialRing(ZZ, 2)\np = R.gen()\nB = R.base_ring()\nxs = R.gens()\nvector_norm = w.norm()\nfield_order = a.multiplicative_order()\npoly_degree = p.degree()\nwrong_ring_order = B.order()\nwrong_tuple_factor = xs.factor()\n";

    assert_eq!(
        infer_owner_type_before_strict(source, "w", "norm", 8),
        Some(SageOwnerType::Vector)
    );
    assert_eq!(
        infer_owner_type_before_strict(source, "a", "multiplicative_order", 9),
        Some(SageOwnerType::FieldElement)
    );
    assert_eq!(
        infer_owner_type_before_strict(source, "p", "degree", 10),
        Some(SageOwnerType::PolynomialElement)
    );
    assert!(
        infer_owner_type_before_strict(source, "B", "order", 11).is_none(),
        "base_ring() is not necessarily a finite Field"
    );
    assert!(
        infer_owner_type_before_strict(source, "xs", "factor", 12).is_none(),
        "gens() returns a tuple, not a PolynomialElement"
    );
}

#[test]
fn chained_sage_methods_keep_only_receiver_proven_high_confidence_types() {
    let consumer = "from sage.all import GF, PolynomialRing, vector\nv = vector(ZZ, [1, 2])\nw = v.change_ring(QQ)\nF = GF(7)\na = F.gen()\nR.<x, y> = PolynomialRing(ZZ, 2)\np = R.gen()\nB = R.base_ring()\nxs = R.gens()\nvector_norm = w.norm()\nfield_order = a.multiplicative_order()\npoly_degree = p.degree()\nwrong_ring_order = B.order()\nwrong_tuple_factor = xs.factor()\n";
    let (root, index) = strict_navigation_index(
        "receiver-aware-method-returns",
        &[
            (
                "sage/all.py",
                "from sage.modules.free_module_element import vector\nfrom sage.rings.finite_rings.finite_field_constructor import GF\nfrom sage.rings.polynomial.polynomial_ring_constructor import PolynomialRing\n",
            ),
            (
                "sage/modules/free_module_element.pyx",
                "def vector(ring, entries):\n    return entries\n\nclass FreeModuleElement:\n    def change_ring(self, ring):\n        return self\n\n    def norm(self):\n        return 0\n",
            ),
            (
                "sage/rings/finite_rings/finite_field_constructor.py",
                "def GF(order):\n    return order\n",
            ),
            (
                "sage/rings/finite_rings/element_base.pyx",
                "class FiniteFieldElement:\n    def multiplicative_order(self):\n        return 1\n",
            ),
            (
                "sage/rings/finite_rings/finite_field_base.pyx",
                "class FiniteField:\n    def order(self):\n        return 1\n",
            ),
            (
                "sage/rings/polynomial/polynomial_ring_constructor.py",
                "def PolynomialRing(base, *args):\n    return base\n",
            ),
            (
                "sage/rings/polynomial/multi_polynomial.pyx",
                "class MPolynomial:\n    def degree(self):\n        return 0\n\n    def factor(self):\n        return []\n",
            ),
            ("consumer.py", consumer),
        ],
    );
    let consumer_path = root.join("consumer.py");

    for (member, expected_owner, expected_path) in [
        (
            "norm",
            "Vector",
            root.join("sage/modules/free_module_element.pyx"),
        ),
        (
            "multiplicative_order",
            "FieldElement",
            root.join("sage/rings/finite_rings/element_base.pyx"),
        ),
        (
            "degree",
            "PolynomialElement",
            root.join("sage/rings/polynomial/multi_polynomial.pyx"),
        ),
    ] {
        let (line, character) = member_position(consumer, member);
        let query = index.query_source_at_navigation(
            &consumer_path,
            consumer,
            QueryPosition { line, character },
        );
        assert_eq!(query.owner_type.as_deref(), Some(expected_owner));
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(expected_path).as_path()),
            "wrong receiver-aware target for {member}: {query:?}"
        );
    }

    for (member, forbidden_path) in [
        (
            "order",
            root.join("sage/rings/finite_rings/finite_field_base.pyx"),
        ),
        (
            "factor",
            root.join("sage/rings/polynomial/multi_polynomial.pyx"),
        ),
    ] {
        let (line, character) = member_position(consumer, member);
        let query = index.query_source_at_navigation(
            &consumer_path,
            consumer,
            QueryPosition { line, character },
        );
        assert_ne!(query.resolution_confidence.as_deref(), Some("high"));
        assert_ne!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(forbidden_path).as_path()),
            "unproven return types must not force a wrong target for {member}: {query:?}"
        );
    }

    fs::remove_dir_all(root).ok();
}

#[test]
fn strict_owner_inference_does_not_leak_across_sibling_functions() {
    let source = "def build_value():\n    value = matrix(QQ, 1, 1)\n\ndef use_value():\n    return value.rank()\n";

    assert!(infer_owner_type_before_strict(source, "value", "rank", 4).is_none());
    assert_eq!(assignment_constructor_before_line(source, "value", 4), None);
}

#[test]
fn strict_owner_inference_respects_function_wide_local_bindings() {
    for (name, later_binding) in [
        ("assignment", "value = replacement"),
        ("import", "import replacement as value"),
        ("loop", "for value in replacements:\n        pass"),
        ("with", "with resource() as value:\n        pass"),
        ("except", "except Error as value:\n        pass"),
        ("delete", "del value"),
        ("function", "def value():\n        pass"),
        ("class", "class value:\n        pass"),
    ] {
        let source = format!(
            "value = matrix(QQ, 1, 1)\ndef use_value():\n    result = value.rank()\n    {later_binding}\n"
        );
        assert!(
            infer_owner_type_before_strict(&source, "value", "rank", 2).is_none(),
            "{name} declared after the use must still shadow the outer binding"
        );
        assert_eq!(
            assignment_constructor_before_line(&source, "value", 2),
            None,
            "{name} declared after the use retained an outer constructor"
        );
    }

    let global =
        "value = matrix(QQ, 1, 1)\ndef use_value():\n    global value\n    return value.rank()\n";
    assert!(infer_owner_type_before_strict(global, "value", "rank", 3).is_some());
    let nonlocal = "def outer():\n    value = matrix(QQ, 1, 1)\n    def inner():\n        nonlocal value\n        return value.rank()\n";
    assert!(infer_owner_type_before_strict(nonlocal, "value", "rank", 4).is_some());
}

#[test]
fn strict_owner_inference_is_cleared_by_unknown_rebindings() {
    for (name, rebinding) in [
        ("none", "value = None"),
        ("ordinary-expression", "value = replacement"),
        ("augmented", "value += replacement"),
        ("loop-target", "for value in replacements:\n    pass"),
        ("import-alias", "import replacement as value"),
        ("function", "def value():\n    return None"),
        ("class", "class value:\n    pass"),
        ("destructuring", "value, other = replacement"),
    ] {
        let source = format!("value = matrix(QQ, 1, 1)\n{rebinding}\nresult = value.rank()\n");
        let target_line = source.lines().count().saturating_sub(1) as u32;

        assert!(
            infer_owner_type_before_strict(&source, "value", "rank", target_line).is_none(),
            "{name} rebinding retained a stale owner type"
        );
        assert_eq!(
            assignment_constructor_before_line(&source, "value", target_line),
            None,
            "{name} rebinding retained a stale constructor"
        );
    }
}

#[test]
fn strict_owner_inference_keeps_a_constructor_in_the_same_scope() {
    let source = "def use_value():\n    value = matrix(QQ, 1, 1)\n    return value.rank()\n";

    assert!(infer_owner_type_before_strict(source, "value", "rank", 2).is_some());
    assert_eq!(
        assignment_constructor_before_line(source, "value", 2).as_deref(),
        Some("matrix")
    );
}

#[test]
fn strict_owner_inference_scopes_local_function_returns() {
    let sibling = "def left():\n    def make_value():\n        return matrix(QQ, 1, 1)\n\ndef right():\n    value = make_value()\n    return value.rank()\n";
    assert!(infer_owner_type_before_strict(sibling, "value", "rank", 6).is_none());

    let nested_only = "def make_value():\n    def nested():\n        return matrix(QQ, 1, 1)\n\nvalue = make_value()\nresult = value.rank()\n";
    assert!(infer_owner_type_before_strict(nested_only, "value", "rank", 5).is_none());

    let stale_local = "def make_value():\n    value = matrix(QQ, 1, 1)\n    value = None\n    return value\n\nresult = make_value()\nanswer = result.rank()\n";
    assert!(infer_owner_type_before_strict(stale_local, "result", "rank", 6).is_none());

    let duplicate = "def make_value():\n    return matrix(QQ, 1, 1)\n\ndef make_value():\n    return None\n\nvalue = make_value()\nresult = value.rank()\n";
    assert!(infer_owner_type_before_strict(duplicate, "value", "rank", 7).is_none());

    let conditional_return = "def make_value(flag):\n    if flag:\n        return matrix(QQ, 1, 1)\n\nvalue = make_value(flag)\nresult = value.rank()\n";
    assert!(infer_owner_type_before_strict(conditional_return, "value", "rank", 5).is_none());

    let conflicting_return = "def make_value(flag):\n    if flag:\n        return Graph()\n    return matrix(QQ, 1, 1)\n\nvalue = make_value(flag)\nresult = value.rank()\n";
    assert!(infer_owner_type_before_strict(conflicting_return, "value", "rank", 6).is_none());

    let matching_return = "def make_value(flag):\n    if flag:\n        return matrix(QQ, 1, 1)\n    return matrix(QQ, 2, 2)\n\nvalue = make_value(flag)\nresult = value.rank()\n";
    assert!(infer_owner_type_before_strict(matching_return, "value", "rank", 6).is_some());

    let conditional_rebinding = "def make_value(flag):\n    value = matrix(QQ, 1, 1)\n    if flag:\n        value = None\n    return value\n\nresult = make_value(flag)\nanswer = result.rank()\n";
    assert!(infer_owner_type_before_strict(conditional_rebinding, "result", "rank", 7).is_none());

    let conditional_definition = "def make_value():\n    return matrix(QQ, 1, 1)\n\nif flag:\n    def make_value():\n        return None\n\nvalue = make_value()\nresult = value.rank()\n";
    assert!(infer_owner_type_before_strict(conditional_definition, "value", "rank", 8).is_none());

    let unconditional_return = "def make_value():\n    value = matrix(QQ, 1, 1)\n    return value\n\nresult = make_value()\nanswer = result.rank()\n";
    assert!(infer_owner_type_before_strict(unconditional_return, "result", "rank", 5).is_some());
}

#[test]
fn strict_owner_inference_requires_control_flow_dominance() {
    let outside = "if flag:\n    value = matrix(QQ, 1, 1)\nresult = value.rank()\n";
    assert!(infer_owner_type_before_strict(outside, "value", "rank", 2).is_none());
    assert_eq!(
        assignment_constructor_before_line(outside, "value", 2),
        None
    );

    let conditional_rebinding =
        "value = matrix(QQ, 1, 1)\nif flag:\n    value = None\nresult = value.rank()\n";
    assert!(infer_owner_type_before_strict(conditional_rebinding, "value", "rank", 3).is_none());
    assert_eq!(
        assignment_constructor_before_line(conditional_rebinding, "value", 3),
        None
    );

    let sibling_branch =
        "if flag:\n    value = matrix(QQ, 1, 1)\nelse:\n    result = value.rank()\n";
    assert!(infer_owner_type_before_strict(sibling_branch, "value", "rank", 3).is_none());
    assert_eq!(
        assignment_constructor_before_line(sibling_branch, "value", 3),
        None
    );

    let mutually_exclusive =
        "value = matrix(QQ, 1, 1)\nif flag:\n    value = None\nelse:\n    result = value.rank()\n";
    assert!(infer_owner_type_before_strict(mutually_exclusive, "value", "rank", 4).is_some());
    assert_eq!(
        assignment_constructor_before_line(mutually_exclusive, "value", 4).as_deref(),
        Some("matrix")
    );

    let independent_sibling_if = "value = matrix(QQ, 1, 1)\nif reset:\n    value = None\nif inspect:\n    result = value.rank()\n";
    assert!(infer_owner_type_before_strict(independent_sibling_if, "value", "rank", 4).is_none());
    assert_eq!(
        assignment_constructor_before_line(independent_sibling_if, "value", 4),
        None
    );

    let same_branch = "if flag:\n    value = matrix(QQ, 1, 1)\n    result = value.rank()\n";
    assert!(infer_owner_type_before_strict(same_branch, "value", "rank", 2).is_some());
    assert_eq!(
        assignment_constructor_before_line(same_branch, "value", 2).as_deref(),
        Some("matrix")
    );
}

#[test]
fn strict_owner_inference_respects_expression_local_bindings() {
    for (name, source, target_line) in [
        (
            "lambda-parameter",
            "value = matrix(QQ, 1, 1)\nresult = (lambda value: value.rank())(graph)\n",
            1,
        ),
        (
            "comprehension-target",
            "value = matrix(QQ, 1, 1)\nranks = [value.rank() for value in graphs]\n",
            1,
        ),
        (
            "walrus-target",
            "value = matrix(QQ, 1, 1)\nresult = ((value := graph), value.rank())\n",
            1,
        ),
        (
            "multiline-lambda-parameter",
            "value = matrix(QQ, 1, 1)\nresult = (\n    lambda value:\n        value.rank()\n)(graph)\n",
            3,
        ),
        (
            "multiline-comprehension-target",
            "value = matrix(QQ, 1, 1)\nranks = [\n    value.rank()\n    for value in graphs\n]\n",
            2,
        ),
        (
            "preceding-walrus-rebinding",
            "value = matrix(QQ, 1, 1)\nreplacement = (value := graph)\nresult = value.rank()\n",
            2,
        ),
        (
            "conditional-walrus-rebinding",
            "value = matrix(QQ, 1, 1)\nif value := graph:\n    result = value.rank()\n",
            2,
        ),
    ] {
        assert!(
            infer_owner_type_before_strict(source, "value", "rank", target_line).is_none(),
            "{name} inherited the outer Matrix owner type"
        );
        assert_eq!(
            assignment_constructor_before_line(source, "value", target_line),
            None,
            "{name} retained the outer matrix constructor"
        );
    }

    let after_comprehension = "value = matrix(QQ, 1, 1)\nranks = [value.rank() for value in graphs]\nresult = value.rank()\n";
    assert!(
        infer_owner_type_before_strict(after_comprehension, "value", "rank", 2).is_some(),
        "a comprehension target must not rebind the outer value after the expression"
    );
    assert_eq!(
        assignment_constructor_before_line(after_comprehension, "value", 2).as_deref(),
        Some("matrix")
    );
}

#[test]
fn expression_and_independent_if_bindings_never_produce_high_navigation() {
    let provider = "def rank(self):\n    return 0\n";
    let comprehension = "from sage.all import matrix\nvalue = matrix(QQ, 1, 1)\ngraphs = []\nranks = [value.rank() for value in graphs]\n";
    let independent_if = "from sage.all import matrix\nvalue = matrix(QQ, 1, 1)\nif reset:\n    value = None\nif inspect:\n    result = value.rank()\n";
    let (root, index) = strict_navigation_index(
        "strict-expression-and-independent-if-bindings",
        &[
            ("sage/matrix/matrix0.pyx", provider),
            ("comprehension.py", comprehension),
            ("independent_if.py", independent_if),
        ],
    );

    for (file, source, line, character) in [
        ("comprehension.py", comprehension, 3, 16),
        ("independent_if.py", independent_if, 5, 21),
    ] {
        let query = index.query_source_at_navigation(
            &root.join(file),
            source,
            QueryPosition { line, character },
        );
        assert!(
            query.definition.is_none(),
            "{file} incorrectly produced a single definition"
        );
        assert_ne!(
            query.resolution_confidence.as_deref(),
            Some("high"),
            "{file} incorrectly produced high-confidence navigation"
        );
        assert_eq!(
            query.owner_type, None,
            "{file} leaked the Matrix owner type"
        );
    }

    fs::remove_dir_all(root).ok();
}

#[test]
fn strict_owner_inference_respects_parameter_shadowing() {
    let direct = "value = matrix(QQ, 1, 1)\n\ndef inspect(value):\n    return value.rank()\n";
    assert!(infer_owner_type_before_strict(direct, "value", "rank", 3).is_none());
    assert_eq!(assignment_constructor_before_line(direct, "value", 3), None);

    let closure = "value = matrix(QQ, 1, 1)\n\ndef outer(value):\n    def inner():\n        return value.rank()\n";
    assert!(infer_owner_type_before_strict(closure, "value", "rank", 4).is_none());
    assert_eq!(
        assignment_constructor_before_line(closure, "value", 4),
        None
    );

    let rebound = "value = matrix(QQ, 1, 1)\n\ndef inspect(value):\n    value = matrix(QQ, 2, 2)\n    return value.rank()\n";
    assert!(infer_owner_type_before_strict(rebound, "value", "rank", 4).is_some());
    assert_eq!(
        assignment_constructor_before_line(rebound, "value", 4).as_deref(),
        Some("matrix")
    );

    let cython_memoryview =
        "value = matrix(QQ, 1, 1)\n\ncdef inspect(double[:] value):\n    return value.rank()\n";
    assert!(infer_owner_type_before_strict(cython_memoryview, "value", "rank", 3).is_none());
    assert_eq!(
        assignment_constructor_before_line(cython_memoryview, "value", 3),
        None
    );
}

#[test]
fn method_name_only_produces_ranked_candidates_without_a_wrong_jump() {
    let provider = "class MatrixLike:\n    def rank(self):\n        \"\"\"Matrix-like rank docs.\"\"\"\n        return 1\n\nclass CurveLike:\n    def rank(self):\n        \"\"\"Curve-like rank docs.\"\"\"\n        return 2\n";
    let consumer = "result = unknown.rank()\n";
    let (root, index) = strict_navigation_index(
        "strict-member-name",
        &[("provider.py", provider), ("consumer.py", consumer)],
    );

    let query = index.query_source_at_navigation(
        &root.join("consumer.py"),
        consumer,
        QueryPosition {
            line: 0,
            character: 17,
        },
    );

    assert!(query.definition.is_none());
    assert_eq!(query.resolution_confidence.as_deref(), Some("ambiguous"));
    assert_eq!(query.owner_type, None);
    assert_eq!(query.candidate_count, 2);
    assert_eq!(query.definition_candidates.len(), 2);
    let details: BTreeSet<_> = query
        .definition_candidates
        .iter()
        .map(|candidate| candidate.definition.detail.as_str())
        .collect();
    assert_eq!(
        details,
        BTreeSet::from(["Method CurveLike.rank", "Method MatrixLike.rank"])
    );
    assert!(query
        .documentation
        .as_ref()
        .is_some_and(|docs| docs.kind == "AmbiguousMember" && docs.sections.len() == 2));

    fs::remove_dir_all(root).ok();
}

#[test]
fn same_path_and_module_do_not_choose_between_multiple_member_owners() {
    let provider = "def build():\n    return object()\n\nclass First:\n    def target(self):\n        return 1\n\nclass Second:\n    def target(self):\n        return 2\n";
    let consumer = "from provider import build\nvalue = build()\nresult = value.target()\n";
    let (root, index) = strict_navigation_index(
        "strict-same-module",
        &[("provider.py", provider), ("consumer.py", consumer)],
    );

    let query = index.query_source_at_navigation(
        &root.join("consumer.py"),
        consumer,
        QueryPosition {
            line: 2,
            character: 15,
        },
    );

    assert!(query.definition.is_none());
    assert_eq!(query.resolution_confidence.as_deref(), Some("ambiguous"));
    assert_eq!(query.definition_candidates.len(), 2);
    assert!(query.resolution_reason.as_deref().is_some_and(|reason| {
        reason.contains("no exact owner match") && reason.contains("ranking signals only")
    }));

    fs::remove_dir_all(root).ok();
}

#[test]
fn same_path_and_module_do_not_promote_a_single_unowned_member() {
    let provider = "def build():\n    return object()\n\nclass OnlyClass:\n    def target(self):\n        return 1\n";
    let consumer = "from provider import build\nvalue = build()\nresult = value.target()\n";
    let (root, index) = strict_navigation_index(
        "strict-single-same-module",
        &[("provider.py", provider), ("consumer.py", consumer)],
    );

    let query = index.query_source_at_navigation(
        &root.join("consumer.py"),
        consumer,
        QueryPosition {
            line: 2,
            character: 15,
        },
    );

    assert!(query.definition.is_none());
    assert_eq!(query.definition_candidates.len(), 1);
    assert_eq!(query.resolution_confidence.as_deref(), Some("ambiguous"));

    fs::remove_dir_all(root).ok();
}

#[test]
fn explicit_constructor_identity_overrides_weak_member_name_hints() {
    let provider = "class MatrixLike:\n    def rank(self):\n        return 1\n\nclass CurveLike:\n    def rank(self):\n        return 2\n";
    let consumer = "from provider import CurveLike\nvalue = CurveLike()\nresult = value.rank()\n";
    let (root, index) = strict_navigation_index(
        "strict-constructor",
        &[("provider.py", provider), ("consumer.py", consumer)],
    );

    let query = index.query_source_at_navigation(
        &root.join("consumer.py"),
        consumer,
        QueryPosition {
            line: 2,
            character: 15,
        },
    );

    let definition = query.definition.expect("constructor owner should be exact");
    assert_eq!(definition.detail, "Method CurveLike.rank");
    assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
    assert!(query.definition_candidates.is_empty());

    fs::remove_dir_all(root).ok();
}

#[test]
fn constructor_class_name_does_not_cross_module_boundaries() {
    let first_provider = "class Shared:\n    pass\n";
    let second_provider = "class Shared:\n    def target(self):\n        return 'wrong module'\n";
    let consumer = "from first_provider import Shared\nvalue = Shared()\nresult = value.target()\n";
    let (root, index) = strict_navigation_index(
        "strict-constructor-module-identity",
        &[
            ("first_provider.py", first_provider),
            ("second_provider.py", second_provider),
            ("consumer.py", consumer),
        ],
    );

    let query = index.query_source_at_navigation(
        &root.join("consumer.py"),
        consumer,
        QueryPosition {
            line: 2,
            character: 16,
        },
    );

    assert!(query.definition.is_none());
    assert_eq!(query.resolution_confidence.as_deref(), Some("ambiguous"));
    assert_eq!(query.definition_candidates.len(), 1);
    assert_eq!(
        query.definition_candidates[0].definition.path,
        normalize_path(root.join("second_provider.py"))
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn duplicate_constructor_reexports_never_become_a_high_confidence_owner() {
    let base = test_root("strict-duplicate-constructor-reexport");
    let first_root = base.join("first");
    let second_root = base.join("second");
    for (root, implementation, value) in [(&first_root, "impl_a", 1), (&second_root, "impl_b", 2)] {
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::write(
            root.join("pkg/api.py"),
            format!("from {implementation} import Shared\n"),
        )
        .unwrap();
        fs::write(
            root.join(format!("{implementation}.py")),
            format!("class Shared:\n    def target(self):\n        return {value}\n"),
        )
        .unwrap();
    }
    let consumer = "from pkg.api import Shared\nvalue = Shared()\nresult = value.target()\n";
    let consumer_path = first_root.join("consumer.py");
    fs::write(&consumer_path, consumer).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![first_root.clone(), second_root.clone()],
        editable_roots: vec![first_root.clone(), second_root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: base.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    let query = index.query_source_at_navigation(
        &consumer_path,
        consumer,
        QueryPosition {
            line: 2,
            character: 16,
        },
    );

    assert!(query.definition.is_none());
    assert_eq!(query.resolution_confidence.as_deref(), Some("ambiguous"));
    assert_eq!(query.definition_candidates.len(), 2);
    let candidate_paths: BTreeSet<_> = query
        .definition_candidates
        .iter()
        .map(|candidate| candidate.definition.path.clone())
        .collect();
    assert_eq!(
        candidate_paths,
        BTreeSet::from([
            normalize_path(first_root.join("impl_a.py")),
            normalize_path(second_root.join("impl_b.py")),
        ])
    );

    fs::remove_dir_all(base).ok();
}

#[test]
fn shadowed_sage_constructor_binding_is_not_high_confidence() {
    let matrix_method = "def rank(self):\n    return 0\n";
    let consumer = "from sage.all import matrix\n\ndef custom_matrix(*args):\n    return object()\n\nmatrix = custom_matrix\nvalue = matrix(None, 1, 1)\nresult = value.rank()\n";
    let (root, index) = strict_navigation_index(
        "strict-shadowed-sage-constructor-binding",
        &[
            ("sage/matrix/matrix0.pyx", matrix_method),
            ("consumer.py", consumer),
        ],
    );

    let query = index.query_source_at_navigation(
        &root.join("consumer.py"),
        consumer,
        QueryPosition {
            line: 7,
            character: 16,
        },
    );

    assert!(query.definition.is_none());
    assert_eq!(query.resolution_confidence.as_deref(), Some("ambiguous"));
    assert_eq!(query.owner_type, None);

    fs::remove_dir_all(root).ok();
}

#[test]
fn qualified_sage_constructor_requires_the_active_namespace_import() {
    let matrix_method = "def rank(self):\n    return 0\n";
    let free_module_method = "def basis_matrix(self):\n    return None\n";
    let consumer = "import sage.all as sage\nbefore = sage.matrix(None, 1, 1)\ngood = before.rank()\n\nclass FakeSage:\n    def matrix(self, *args):\n        return object()\n\nsage = FakeSage()\nafter = sage.matrix(None, 1, 1)\nbad = after.rank()\nkernel = after.right_kernel()\nnested_bad = kernel.basis_matrix()\n";
    let (root, index) = strict_navigation_index(
        "strict-active-sage-namespace-binding",
        &[
            ("sage/matrix/matrix0.pyx", matrix_method),
            ("sage/modules/free_module.py", free_module_method),
            ("consumer.py", consumer),
        ],
    );

    let before_shadow = index.query_source_at_navigation(
        &root.join("consumer.py"),
        consumer,
        QueryPosition {
            line: 2,
            character: 15,
        },
    );
    assert!(before_shadow.definition.is_some());
    assert_eq!(before_shadow.resolution_confidence.as_deref(), Some("high"));

    let after_shadow = index.query_source_at_navigation(
        &root.join("consumer.py"),
        consumer,
        QueryPosition {
            line: 10,
            character: 13,
        },
    );
    assert!(after_shadow.definition.is_none());
    assert_eq!(
        after_shadow.resolution_confidence.as_deref(),
        Some("ambiguous")
    );
    assert_eq!(after_shadow.owner_type, None);

    let nested_after_shadow = index.query_source_at_navigation(
        &root.join("consumer.py"),
        consumer,
        QueryPosition {
            line: 12,
            character: 21,
        },
    );
    assert!(nested_after_shadow.definition.is_none());
    assert_eq!(
        nested_after_shadow.resolution_confidence.as_deref(),
        Some("ambiguous")
    );
    assert_eq!(nested_after_shadow.owner_type, None);

    fs::remove_dir_all(root).ok();
}

#[test]
fn local_constructor_shadow_does_not_resolve_as_a_sage_builtin_type() {
    let source = "class Custom:\n    def rank(self):\n        return 1\n\ndef matrix():\n    return Custom()\n\nmat = matrix()\nresult = mat.rank()\n";
    let (root, index) =
        strict_navigation_index("strict-shadowed-constructor", &[("consumer.py", source)]);

    let query = index.query_source_at_navigation(
        &root.join("consumer.py"),
        source,
        QueryPosition {
            line: 8,
            character: 13,
        },
    );

    assert!(query.definition.is_none());
    assert_eq!(query.owner_type, None);
    assert_eq!(query.definition_candidates.len(), 1);
    assert_eq!(
        query.definition_candidates[0].definition.detail,
        "Method Custom.rank"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn type_definition_requires_a_reliable_sage_constructor_binding() {
    let source = "graph = Graph([(0, 1)])\nvalue = graph.vertices()\n";
    let (root, index) =
        strict_navigation_index("strict-type-definition-binding", &[("consumer.py", source)]);

    assert!(index
        .type_definition_at_source(
            &root.join("consumer.py"),
            source,
            QueryPosition {
                line: 1,
                character: 9,
            },
        )
        .is_none());

    fs::remove_dir_all(root).ok();
}

#[test]
fn unbound_sage_namespace_returns_candidates_instead_of_a_wrong_jump() {
    let provider = "def PetersenGraph():\n    \"\"\"Build a graph.\"\"\"\n    return None\n";
    let consumer = "value = graphs.PetersenGraph()\n";
    let (root, index) = strict_navigation_index(
        "strict-namespace-binding",
        &[
            ("sage/graphs/graph_generators.py", provider),
            ("consumer.py", consumer),
        ],
    );

    let query = index.query_source_at_navigation(
        &root.join("consumer.py"),
        consumer,
        QueryPosition {
            line: 0,
            character: 16,
        },
    );
    assert!(query.definition.is_none());
    assert_eq!(query.resolution_confidence.as_deref(), Some("ambiguous"));
    assert_eq!(query.definition_candidates.len(), 1);

    fs::remove_dir_all(root).ok();
}

#[test]
fn ambiguous_member_never_produces_references_or_rename_edits() {
    let provider = "class First:\n    def rank(self):\n        return 1\n\nclass Second:\n    def rank(self):\n        return 2\n";
    let consumer = "first = unknown.rank()\nsecond = unknown.rank()\n";
    let (root, index) = strict_navigation_index(
        "strict-mutation",
        &[("provider.py", provider), ("consumer.py", consumer)],
    );

    let query = index.query_source_at(
        &root.join("consumer.py"),
        consumer,
        QueryPosition {
            line: 0,
            character: 16,
        },
        Some("renamed"),
    );

    assert!(query.definition.is_none());
    assert_eq!(query.definition_candidates.len(), 2);
    assert!(query.references.is_empty());
    assert!(query.rename_preview.is_empty());

    fs::remove_dir_all(root).ok();
}

#[test]
fn duplicate_explicit_import_targets_return_multiple_definitions() {
    let base = test_root("strict-duplicate-import-target");
    let first_root = base.join("first");
    let second_root = base.join("second");
    for (root, value) in [(&first_root, 1), (&second_root, 2)] {
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::write(
            root.join("pkg/provider.py"),
            format!("def target():\n    return {value}\n"),
        )
        .unwrap();
    }
    let consumer = "from pkg.provider import target\nvalue = target()\n";
    let consumer_path = first_root.join("consumer.py");
    fs::write(&consumer_path, consumer).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![first_root.clone(), second_root.clone()],
        editable_roots: vec![first_root.clone(), second_root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: base.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    let query = index.query_source_at_navigation(
        &consumer_path,
        consumer,
        QueryPosition {
            line: 1,
            character: 9,
        },
    );
    assert!(query.definition.is_none());
    assert_eq!(query.resolution_confidence.as_deref(), Some("ambiguous"));
    assert_eq!(query.definition_candidates.len(), 2);

    fs::remove_dir_all(base).ok();
}

#[test]
fn duplicate_namespace_members_return_candidates_instead_of_a_high_confidence_target() {
    let base = test_root("strict-duplicate-namespace-member");
    let first_root = base.join("first");
    let second_root = base.join("second");
    for (root, value) in [(&first_root, 1), (&second_root, 2)] {
        fs::create_dir_all(root.join("sage/graphs")).unwrap();
        fs::write(
            root.join("sage/graphs/graph_generators.py"),
            format!(
                "class GraphGenerators:\n    pass\n\ngraphs = GraphGenerators()\n\ndef PetersenGraph():\n    return {value}\n"
            ),
        )
        .unwrap();
    }
    let consumer =
        "from sage.graphs.graph_generators import graphs\nvalue = graphs.PetersenGraph()\n";
    let consumer_path = first_root.join("consumer.py");
    fs::write(&consumer_path, consumer).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![first_root.clone(), second_root.clone()],
        editable_roots: vec![first_root.clone(), second_root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: base.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    let query = index.query_source_at_navigation(
        &consumer_path,
        consumer,
        QueryPosition {
            line: 1,
            character: 16,
        },
    );

    assert!(query.definition.is_none());
    assert_eq!(query.resolution_confidence.as_deref(), Some("ambiguous"));
    assert_eq!(query.definition_candidates.len(), 2);
    let paths: BTreeSet<_> = query
        .definition_candidates
        .iter()
        .map(|candidate| candidate.definition.path.clone())
        .collect();
    assert_eq!(
        paths,
        BTreeSet::from([
            normalize_path(first_root.join("sage/graphs/graph_generators.py")),
            normalize_path(second_root.join("sage/graphs/graph_generators.py")),
        ])
    );

    fs::remove_dir_all(base).ok();
}

#[test]
fn unbound_global_name_returns_all_indexed_definitions_without_arbitrary_docs() {
    let consumer = "value = target()\n";
    let (root, index) = strict_navigation_index(
        "strict-duplicate-global",
        &[
            (
                "provider_a.py",
                "def target():\n    \"\"\"Provider A.\"\"\"\n    return 1\n",
            ),
            (
                "provider_b.py",
                "def target():\n    \"\"\"Provider B.\"\"\"\n    return 2\n",
            ),
            ("consumer.py", consumer),
        ],
    );

    let query = index.query_source_at_navigation(
        &root.join("consumer.py"),
        consumer,
        QueryPosition {
            line: 0,
            character: 10,
        },
    );

    assert!(query.definition.is_none());
    assert_eq!(query.resolution_confidence.as_deref(), Some("ambiguous"));
    assert_eq!(query.candidate_count, 2);
    assert_eq!(query.definition_candidates.len(), 2);
    assert!(query.resolution_reason.as_deref().is_some_and(|reason| {
        reason.contains("no reliable binding") && reason.contains("explicit selection")
    }));
    assert!(query.documentation.as_ref().is_some_and(|documentation| {
        documentation.kind == "AmbiguousMember"
            && documentation.docstring.as_deref().is_none_or(|docstring| {
                !docstring.contains("Provider A") && !docstring.contains("Provider B")
            })
    }));

    fs::remove_dir_all(root).ok();
}

#[test]
fn generator_navigation_keeps_number_field_and_polynomial_implementations_separate() {
    let consumer = "from sage.all import NumberField, PolynomialRing, QQ\n\
P.<u> = PolynomialRing(QQ)\n\
single_parent_gen = P.gen()\n\
single_degree = u.degree()\n\
Q = PolynomialRing(QQ, 'q')\n\
q = Q.gen()\n\
direct_single_degree = q.degree()\n\
R.<x, y, z> = PolynomialRing(QQ, 3)\n\
multi_parent_gen = R.gen()\n\
multi_degree = x.degree()\n\
K.<a> = NumberField(x^2 + 1)\n\
number_field_polynomial = a.polynomial()\n\
b = K.gen()\n\
number_field_call_polynomial = b.polynomial()\n";
    let (root, index) = strict_navigation_index(
        "strict-generator-representations",
        &[
            (
                "sage/all.py",
                "from sage.rings.number_field.number_field import NumberField\n\
from sage.rings.polynomial.polynomial_ring_constructor import PolynomialRing\n\
from sage.rings.rational_field import QQ\n",
            ),
            (
                "sage/rings/number_field/number_field.py",
                "def NumberField(polynomial, name=None):\n\
    return polynomial\n\
\n\
class NumberField_generic:\n\
    def gen(self):\n\
        return None\n",
            ),
            (
                "sage/rings/number_field/number_field_element.pyx",
                "cdef class NumberFieldElement:\n\
    def polynomial(self, var='x'):\n\
        return None\n",
            ),
            (
                "sage/rings/polynomial/polynomial_ring_constructor.py",
                "def PolynomialRing(base, *args):\n\
    return base\n",
            ),
            (
                "sage/rings/polynomial/polynomial_ring.py",
                "class PolynomialRing_generic:\n\
    def gen(self, n=0):\n\
        return None\n",
            ),
            (
                "sage/rings/polynomial/multi_polynomial_libsingular.pyx",
                "class MPolynomialRing_libsingular:\n\
    def gen(self, n=0):\n\
        return None\n",
            ),
            (
                "sage/rings/polynomial/polynomial_element.pyx",
                "cdef class Polynomial:\n\
    def degree(self, gen=None):\n\
        return 0\n",
            ),
            (
                "sage/rings/polynomial/multi_polynomial_element.py",
                "class MPolynomial:\n\
    def degree(self, x=None):\n\
        return 0\n",
            ),
            ("sage/rings/rational_field.py", "QQ = object()\n"),
            ("consumer.sage", consumer),
        ],
    );
    let consumer_path = root.join("consumer.sage");
    let univariate_ring_path =
        normalize_path(root.join("sage/rings/polynomial/polynomial_ring.py"));
    let multi_ring_path =
        normalize_path(root.join("sage/rings/polynomial/multi_polynomial_libsingular.pyx"));
    let multi_element_path =
        normalize_path(root.join("sage/rings/polynomial/multi_polynomial_element.py"));
    let number_field_element_path =
        normalize_path(root.join("sage/rings/number_field/number_field_element.pyx"));

    for occurrence in 0..2 {
        let (line, character) = nth_member_position(consumer, "gen", occurrence);
        let query = index.query_source_at_navigation(
            &consumer_path,
            consumer,
            QueryPosition { line, character },
        );
        assert_eq!(
            query.owner_type.as_deref(),
            Some("UnivariatePolynomialRing")
        );
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(univariate_ring_path.as_path()),
            "proven single-variable rings should select their exact implementation"
        );
    }

    for (member, occurrence) in [("degree", 0), ("degree", 1)] {
        let (line, character) = nth_member_position(consumer, member, occurrence);
        let query = index.query_source_at_navigation(
            &consumer_path,
            consumer,
            QueryPosition { line, character },
        );
        assert_ne!(
            query.resolution_confidence.as_deref(),
            Some("high"),
            "single-variable polynomial evidence must not choose a multivariate target: {query:?}"
        );
        assert_ne!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(if member == "gen" {
                multi_ring_path.as_path()
            } else {
                multi_element_path.as_path()
            }),
            "single-variable polynomial navigation selected a multivariate implementation"
        );
    }

    for (member, occurrence, expected_path) in [
        ("gen", 2, multi_ring_path.as_path()),
        ("degree", 2, multi_element_path.as_path()),
    ] {
        let (line, character) = nth_member_position(consumer, member, occurrence);
        let query = index.query_source_at_navigation(
            &consumer_path,
            consumer,
            QueryPosition { line, character },
        );
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(expected_path),
            "explicit multivariate preparser evidence should retain an exact target: {query:?}"
        );
    }

    for occurrence in 0..2 {
        let (line, character) = nth_member_position(consumer, "polynomial", occurrence);
        let query = index.query_source_at_navigation(
            &consumer_path,
            consumer,
            QueryPosition { line, character },
        );
        assert_eq!(
            query.owner_type.as_deref(),
            Some("NumberFieldElement"),
            "NumberField generators need a distinct element owner: {query:?}"
        );
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(number_field_element_path.as_path()),
            "NumberField element polynomial() resolved outside its implementation hierarchy"
        );
    }

    fs::remove_dir_all(root).ok();
}

#[test]
fn polyhedron_navigation_uses_constructor_identity_and_exact_base_modules() {
    let consumer = "from sage.all import Polyhedron\n\
from sage.geometry.all import Polyhedron as GeometryPolyhedron\n\
from sage.geometry.polyhedron.all import Polyhedron as ModulePolyhedron\n\
poly = Polyhedron(vertices=[[0, 0], [1, 0], [0, 1]])\n\
vertices = poly.vertices()\n\
dimension = poly.dim()\n\
facets = poly.facets()\n\
volume = poly.volume()\n\
contained = poly.contains([0, 0])\n\
meet = poly.intersection(poly)\n\
dual = poly.polar()\n\
picture = poly.plot()\n\
future = poly.future_invariant()\n\
dual_vertices = dual.vertices()\n\
meet_volume = meet.volume()\n";
    let (root, index) = strict_navigation_index(
        "strict-polyhedron-navigation",
        &[
            ("sage/all.py", "from sage.geometry.all import *\n"),
            (
                "sage/geometry/all.py",
                "from sage.geometry.polyhedron.all import *\n",
            ),
            (
                "sage/geometry/polyhedron/all.py",
                "from sage.misc.lazy_import import lazy_import\n\
lazy_import('sage.geometry.polyhedron.constructor', 'Polyhedron')\n",
            ),
            (
                "sage/geometry/polyhedron/constructor.py",
                "def Polyhedron(vertices=None, **kwds):\n\
    \"\"\"Construct a polyhedron.\"\"\"\n\
    return vertices\n",
            ),
            (
                "sage/geometry/polyhedron/base0.py",
                "class Polyhedron_base0:\n\
    def vertices(self):\n\
        \"\"\"Return the vertices.\"\"\"\n\
        return ()\n",
            ),
            (
                "sage/geometry/polyhedron/base1.py",
                "class Polyhedron_base1:\n\
    def dim(self):\n\
        \"\"\"Return the dimension.\"\"\"\n\
        return 0\n\
\n\
    def contains(self, point):\n\
        \"\"\"Test point containment.\"\"\"\n\
        return False\n",
            ),
            (
                "sage/geometry/polyhedron/base3.py",
                "class Polyhedron_base3:\n\
    def facets(self):\n\
        \"\"\"Return the facets.\"\"\"\n\
        return ()\n",
            ),
            (
                "sage/geometry/polyhedron/base4.py",
                "class Polyhedron_base4:\n\
    def future_invariant(self):\n\
        \"\"\"Return a source-derived future invariant.\"\"\"\n\
        return 0\n",
            ),
            (
                "sage/geometry/polyhedron/base5.py",
                "class Polyhedron_base5:\n\
    def polar(self, in_affine_span=False):\n\
        \"\"\"Return the polar polyhedron.\"\"\"\n\
        return self\n\
\n\
    def intersection(self, other):\n\
        \"\"\"Return the intersection polyhedron.\"\"\"\n\
        return self\n",
            ),
            (
                "sage/geometry/polyhedron/base6.py",
                "class Polyhedron_base6:\n\
    def plot(self, **kwds):\n\
        \"\"\"Plot the polyhedron.\"\"\"\n\
        return None\n",
            ),
            (
                "sage/geometry/polyhedron/base7.py",
                "class Polyhedron_base7:\n\
    def volume(self, measure='ambient', engine='auto', **kwds):\n\
        \"\"\"Return the volume.\"\"\"\n\
        return 0\n",
            ),
            (
                "sage/geometry/polyhedron/base_ZZ.py",
                "class Polyhedron_ZZ:\n\
    def polar(self):\n\
        \"\"\"Return the integer-specialized polar.\"\"\"\n\
        return self\n",
            ),
            (
                "sage/geometry/polyhedron/face.py",
                "class PolyhedronFace:\n\
    def vertices(self):\n\
        \"\"\"Return face vertices, not polyhedron vertices.\"\"\"\n\
        return ()\n",
            ),
            ("consumer.py", consumer),
        ],
    );

    let constructor_path = normalize_path(root.join("sage/geometry/polyhedron/constructor.py"));
    for line_needle in [
        "from sage.all import",
        "from sage.geometry.all import",
        "from sage.geometry.polyhedron.all import",
    ] {
        let (line, character) = position_in_line(consumer, line_needle, "Polyhedron");
        let query = index.query_source_at_navigation(
            &root.join("consumer.py"),
            consumer,
            QueryPosition { line, character },
        );
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(constructor_path.as_path()),
            "wrong Polyhedron export target for {line_needle}: {:?}",
            query.definition
        );
    }

    for (member, occurrence, expected_module) in [
        ("vertices", 0, "base0.py"),
        ("dim", 0, "base1.py"),
        ("facets", 0, "base3.py"),
        ("volume", 0, "base7.py"),
        ("contains", 0, "base1.py"),
        ("intersection", 0, "base5.py"),
        ("polar", 0, "base5.py"),
        ("plot", 0, "base6.py"),
        ("future_invariant", 0, "base4.py"),
        ("vertices", 1, "base0.py"),
        ("volume", 1, "base7.py"),
    ] {
        let (line, character) = nth_member_position(consumer, member, occurrence);
        let query = index.query_source_at_navigation(
            &root.join("consumer.py"),
            consumer,
            QueryPosition { line, character },
        );
        assert_eq!(query.owner_type.as_deref(), Some("Polyhedron"));
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(
                normalize_path(root.join("sage/geometry/polyhedron").join(expected_module))
                    .as_path()
            ),
            "wrong Polyhedron method target for {member}: {:?}",
            query.definition
        );
    }

    let (completion_line, completion_character) = first_position(consumer, "poly.future_");
    let completions = index.completion_items_at_source(
        consumer,
        QueryPosition {
            line: completion_line,
            character: completion_character + "poly.future_".len() as u32,
        },
        20,
    );
    assert!(
        completions
            .iter()
            .any(|completion| completion.label == "future_invariant"),
        "recursive Polyhedron method cache did not contribute completion: {completions:?}"
    );

    fs::remove_dir_all(root).ok();
}
