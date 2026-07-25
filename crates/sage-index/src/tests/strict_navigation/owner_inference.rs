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
