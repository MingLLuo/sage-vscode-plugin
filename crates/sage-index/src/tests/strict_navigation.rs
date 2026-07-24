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
