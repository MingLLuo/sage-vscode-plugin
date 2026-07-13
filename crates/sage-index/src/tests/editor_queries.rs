use super::*;

#[test]
fn query_returns_hover_docs_definition_references_rename_and_signature() {
    let root = test_root("query-api");
    let source_path = root.join("demo.sage");
    let source = "def make_demo_matrix(n, scale=1):\n    \"\"\"Build a demo matrix.\"\"\"\n    return n * scale\n\nvalue = make_demo_matrix(2, scale=3)\n";
    fs::write(&source_path, source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    let query = index.query_source_symbol(
        &source_path,
        source,
        "make_demo_matrix",
        None,
        Some("renamed_demo_matrix"),
        Vec::new(),
    );

    assert!(query
        .hover
        .as_ref()
        .is_some_and(|hover| hover.markdown.contains("Build a demo matrix.")));
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|docs| docs.summary.as_str()),
        Some("Build a demo matrix.")
    );
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.name.as_str()),
        Some("make_demo_matrix")
    );
    assert!(query.references.len() >= 2);
    assert_eq!(query.rename_preview.len(), query.references.len());
    assert_eq!(
        query
            .signature
            .as_ref()
            .map(|signature| signature.label.as_str()),
        Some("make_demo_matrix(n, scale=1)")
    );
    let completion_labels: Vec<_> = query
        .completions
        .iter()
        .map(|completion| completion.label.as_str())
        .collect();
    assert_eq!(
        completion_labels.len(),
        completion_labels
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn navigation_query_skips_expensive_lists_but_keeps_editor_payload() {
    let root = test_root("query-navigation-api");
    let source_path = root.join("demo.sage");
    let source = "def make_demo_matrix(n, scale=1):\n    \"\"\"Build a demo matrix.\"\"\"\n    return n * scale\n\nvalue = make_demo_matrix(2, scale=3)\n";
    fs::write(&source_path, source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    let query = index.query_source_symbol_with_options(
        &source_path,
        source,
        "make_demo_matrix",
        None,
        QueryExecutionOptions {
            rename_to: Some("renamed_demo_matrix"),
            diagnostics: Vec::new(),
            features: QueryFeatures::navigation(),
        },
    );

    assert!(query.hover.is_some());
    assert!(query.documentation.is_some());
    assert!(query.definition.is_some());
    assert!(query.signature.is_some());
    assert!(query.completions.is_empty());
    assert!(query.references.is_empty());
    assert!(query.rename_preview.is_empty());
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_hover_infers_assignment_value_kind_for_variables() {
    let root = test_root("assignment-hover-detail");
    let source_path = root.join("demo.sage");
    let source = "poly_preview = named_polynomial(\"x\")\nnotebook = PolynomialNotebook()\ncount = 5\nannotated: Matrix = make_demo_matrix()\n";
    fs::write(&source_path, source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    for (name, expected_detail) in [
        (
            "poly_preview",
            "Variable poly_preview: result of named_polynomial(...)",
        ),
        ("notebook", "Variable notebook: PolynomialNotebook"),
        ("count", "Variable count: Integer"),
        ("annotated", "Variable annotated: Matrix"),
    ] {
        let query = index.query_source_symbol(&source_path, source, name, None, None, Vec::new());
        assert!(
            query
                .hover
                .as_ref()
                .is_some_and(|hover| hover.markdown.contains(expected_detail)),
            "expected hover for {name} to contain {expected_detail:?}: {:?}",
            query.hover
        );
    }

    fs::remove_dir_all(root).ok();
}

#[test]
fn query_prefers_current_module_variable_over_same_name_elsewhere() {
    let root = test_root("current-module-variable");
    let other_path = root.join("a_other.sage");
    let current_path = root.join("z_current.sage");
    let current_source = "E = EllipticCurve(GF(431), [0, 1])\npoints = E.points()\n";
    fs::write(&other_path, "E = 1\nother_points = E\n").unwrap();
    fs::write(&current_path, current_source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    let query = index.query_source_symbol(
        &current_path,
        current_source,
        "E",
        None,
        Some("current_curve"),
        Vec::new(),
    );

    assert!(query
        .hover
        .as_ref()
        .is_some_and(|hover| hover.markdown.contains("Variable E: EllipticCurve")));
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(current_path.clone()).as_path())
    );
    let current_path = normalize_path(current_path);
    assert!(query
        .references
        .iter()
        .all(|reference| reference.path == current_path));
    assert_eq!(query.rename_preview.len(), query.references.len());

    fs::remove_dir_all(root).ok();
}

#[test]
fn query_rename_preview_filters_to_editable_roots() {
    let root = test_root("editable-roots");
    let workspace = root.join("workspace");
    let library = root.join("sage-src");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&library).unwrap();
    let source_path = workspace.join("demo.sage");
    let library_path = library.join("lib.py");
    let source = "def shared_symbol():\n    return 1\n\nvalue = shared_symbol()\n";
    fs::write(&source_path, source).unwrap();
    fs::write(
        &library_path,
        "def shared_symbol():\n    return 2\n\nother = shared_symbol()\n",
    )
    .unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![workspace.clone(), library.clone()],
        editable_roots: vec![workspace.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    assert!(
        index.references("shared_symbol").len() > index.editable_references("shared_symbol").len()
    );
    let query = index.query_source_symbol(
        &source_path,
        source,
        "shared_symbol",
        None,
        Some("renamed_symbol"),
        Vec::new(),
    );

    let workspace = normalize_path(workspace);
    assert!(query
        .rename_preview
        .iter()
        .all(|edit| edit.path.starts_with(&workspace)));
    assert!(query
        .references
        .iter()
        .all(|reference| reference.path.starts_with(&workspace)));
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_skips_reference_scan_for_read_only_definitions() {
    let root = test_root("read-only-query-target");
    let workspace = root.join("workspace");
    let library = root.join("sage-src");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&library).unwrap();
    let source_path = workspace.join("demo.py");
    let library_path = library.join("lib.py");
    let source = "from lib import ExternalThing\n\nvalue = ExternalThing()\n";
    fs::write(&source_path, source).unwrap();
    fs::write(
        &library_path,
        "def ExternalThing():\n    \"\"\"External constructor.\"\"\"\n    return object()\n",
    )
    .unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![workspace.clone(), library.clone()],
        editable_roots: vec![workspace],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    let query = index.query_source_symbol(
        &source_path,
        source,
        "ExternalThing",
        None,
        Some("RenamedThing"),
        Vec::new(),
    );

    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(library_path).as_path())
    );
    assert!(query.references.is_empty());
    assert!(query.rename_preview.is_empty());
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_keeps_hover_compact_while_preserving_full_docs() {
    let root = test_root("compact-hover");
    let source_path = root.join("demo.sage");
    let long_doc = (0..40)
        .map(|index| format!("Documentation line {index}."))
        .collect::<Vec<_>>()
        .join("\n");
    let indented_doc = long_doc.replace('\n', "\n    ");
    let source = format!(
            "def verbose_symbol():\n    \"\"\"{indented_doc}\"\"\"\n    return 1\n\nvalue = verbose_symbol()\n"
        );
    fs::write(&source_path, &source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    let query = index.query_source_symbol(
        &source_path,
        &source,
        "verbose_symbol",
        None,
        None,
        Vec::new(),
    );

    let hover = query.hover.as_ref().expect("hover should exist");
    assert!(hover.markdown.contains("Documentation line 0."));
    assert!(!hover.markdown.contains("Documentation line 39."));
    assert!(hover.markdown.contains("full docstring"));
    let docs = query.documentation.as_ref().expect("docs should exist");
    assert_eq!(docs.summary, "Documentation line 0.");
    assert!(docs
        .docstring
        .as_ref()
        .is_some_and(|docstring| docstring.contains("Documentation line 39.")));
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_instance_method_from_constructor_assignment() {
    let root = test_root("method-resolution");
    let consumer = root.join("consumer.sage");
    let provider = root.join("provider.py");
    fs::write(
        &consumer,
        "from provider import RingFactory\nR = RingFactory()\nvalue = R.rank()\n",
    )
    .unwrap();
    fs::write(
            &provider,
            "def RingFactory():\n    \"\"\"Build a ring.\"\"\"\n    return None\n\nclass Ring:\n    def rank(self):\n        \"\"\"Return the ring rank.\"\"\"\n        return 1\n",
        )
        .unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();
    let source = fs::read_to_string(&consumer).unwrap();

    let query = index.query_source_symbol(&consumer, &source, "rank", None, None, Vec::new());

    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(provider.clone()).as_path())
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Return the ring rank.")
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn navigation_uses_constructor_class_for_same_file_method_candidates() {
    let root = test_root("same-file-method-owner");
    let source_path = root.join("methods.py");
    let source = "class First:\n    def target(self):\n        return 'first'\n\nclass Second:\n    def target(self):\n        return 'second'\n\nvalue = Second()\nresult = value.target()\n";
    fs::write(&source_path, source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();
    let (line, character) = member_position(source, "target");

    let query =
        index.query_source_at_navigation(&source_path, source, QueryPosition { line, character });
    let definition = query.definition.expect("Second.target should resolve");
    assert_eq!(definition.detail, "Method Second.target");
    assert_eq!(definition.path, normalize_path(source_path.clone()));
    assert_eq!(
        definition.range,
        SourceRange {
            start_line: 5,
            start_character: 8,
            end_line: 5,
            end_character: 14,
        }
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn navigation_resolves_unindexed_local_symbols_with_lexical_scope() {
    let root = test_root("unindexed-local-navigation");
    let source_path = root.join("live.py");
    let source = "def target():\n    return 1\n\ndef caller():\n    return target()\n";
    let index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });

    let query = index.query_source_at_navigation(
        &source_path,
        source,
        QueryPosition {
            line: 4,
            character: 11,
        },
    );
    let definition = query
        .definition
        .expect("live local function should resolve");
    assert_eq!(definition.name, "target");
    assert_eq!(definition.path, normalize_path(source_path.clone()));
    assert_eq!(definition.range.start_line, 0);

    let isolated_source = "def outer_a():\n    def nested():\n        return 1\n    return nested()\n\ndef outer_b():\n    return nested()\n";
    let isolated = index.query_source_at_navigation(
        &source_path,
        isolated_source,
        QueryPosition {
            line: 6,
            character: 11,
        },
    );
    assert!(
        isolated.definition.is_none(),
        "a nested definition from another lexical scope must not be selected"
    );

    let class_source =
        "class Example:\n    class_value = 1\n\n    def read(self):\n        return class_value\n";
    let class_lookup = index.query_source_at_navigation(
        &source_path,
        class_source,
        QueryPosition {
            line: 4,
            character: 15,
        },
    );
    assert!(
        class_lookup.definition.is_none(),
        "class namespace bindings are not lexical locals inside methods"
    );
    fs::remove_dir_all(root).ok();
}
