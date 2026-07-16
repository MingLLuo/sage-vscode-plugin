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
