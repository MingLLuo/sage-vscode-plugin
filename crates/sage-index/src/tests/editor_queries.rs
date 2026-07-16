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
fn references_fail_open_when_identifier_filter_lags_external_edit() {
    let root = test_root("stale-identifier-filter");
    let source_path = root.join("demo.py");
    fs::write(&source_path, "existing = 1\n").unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();
    assert!(index.editable_references("external_target").is_empty());

    // Simulate an external edit observed before the asynchronous index refresh is installed.
    fs::write(
        &source_path,
        "existing = 1\nexternal_target = 2\nvalue = external_target\n",
    )
    .unwrap();
    index.mark_paths_pending_refresh(std::slice::from_ref(&source_path), &[]);

    let references = index.references("external_target");
    assert_eq!(references.len(), 2);
    assert!(references
        .iter()
        .all(|reference| reference.path == normalize_path(source_path.clone())));
    assert_eq!(index.editable_references("external_target").len(), 2);

    index
        .refresh_paths(std::slice::from_ref(&source_path), &[])
        .unwrap();
    assert!(index.pending_refresh_path_snapshot().is_empty());

    fs::remove_dir_all(root).ok();
}

#[test]
fn discarded_background_refresh_result_preserves_pending_path() {
    let root = test_root("discarded-background-refresh");
    let source_path = root.join("demo.py");
    fs::write(&source_path, "value = 1\n").unwrap();
    let mut current = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    current.rebuild().unwrap();
    fs::write(&source_path, "value = pending_target\n").unwrap();
    current.mark_paths_pending_refresh(std::slice::from_ref(&source_path), &[]);

    let mut background = current.clone_for_background_work();
    background
        .refresh_paths(std::slice::from_ref(&source_path), &[])
        .unwrap();

    // A failed worker or generation mismatch discards `background` without installing it.
    assert_eq!(current.pending_refresh_path_snapshot().len(), 1);
    assert_eq!(background.pending_refresh_path_snapshot().len(), 1);

    fs::remove_dir_all(root).ok();
}

#[test]
fn background_refresh_install_clears_only_its_captured_event_version() {
    let root = test_root("versioned-background-refresh");
    let source_path = root.join("demo.py");
    fs::write(&source_path, "value = 1\n").unwrap();
    let mut current = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    current.rebuild().unwrap();
    fs::write(&source_path, "value = first_event\n").unwrap();
    current.mark_paths_pending_refresh(std::slice::from_ref(&source_path), &[]);

    let mut first_refresh = current.clone_for_background_work();
    first_refresh
        .refresh_paths(std::slice::from_ref(&source_path), &[])
        .unwrap();

    fs::write(&source_path, "value = second_event\n").unwrap();
    current.mark_paths_pending_refresh(std::slice::from_ref(&source_path), &[]);
    first_refresh.finalize_pending_refresh_install(&current);
    assert_eq!(first_refresh.pending_refresh_path_snapshot().len(), 1);

    let mut second_refresh = first_refresh.clone_for_background_work();
    second_refresh
        .refresh_paths(std::slice::from_ref(&source_path), &[])
        .unwrap();
    second_refresh.finalize_pending_refresh_install(&first_refresh);
    assert!(second_refresh.pending_refresh_path_snapshot().is_empty());

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
fn query_does_not_guess_instance_owner_from_factory_source_path() {
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

    assert!(query.definition.is_none());
    assert_eq!(query.resolution_confidence.as_deref(), Some("ambiguous"));
    assert_eq!(query.definition_candidates.len(), 1);
    assert_eq!(
        query.definition_candidates[0].definition.path,
        normalize_path(provider.clone())
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
fn navigation_uses_enclosing_class_for_self_and_cls_members() {
    let root = test_root("receiver-member-owner");
    let source_path = root.join("receivers.py");
    let source = "class First:\n    def target(self):\n        return 'first'\n\n    def call(self):\n        return self.target()\n\nclass Second:\n    @classmethod\n    def target(cls):\n        return 'second'\n\n    @classmethod\n    def call(cls):\n        return cls.target()\n";
    fs::write(&source_path, source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    for (occurrence, expected_detail, expected_line) in [
        (0, "Method First.target", 1),
        (1, "Method Second.target", 9),
    ] {
        let (line, character) = nth_member_position(source, "target", occurrence);
        let query = index.query_source_at_navigation(
            &source_path,
            source,
            QueryPosition { line, character },
        );
        let definition = query
            .definition
            .unwrap_or_else(|| panic!("receiver occurrence {occurrence} should resolve"));
        assert_eq!(definition.detail, expected_detail);
        assert_eq!(definition.range.start_line, expected_line);
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
    }

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

#[test]
fn parameter_navigation_references_and_rename_stay_in_the_owning_function() {
    let root = test_root("parameter-binding-scope");
    let global_path = root.join("global.py");
    let source_path = root.join("consumer.py");
    let source = "def first(value):\n    return value\n\ndef second(value):\n    return value\n";
    fs::write(&global_path, "value = 'unrelated global'\n").unwrap();
    fs::write(&source_path, source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    let first = index.query_source_at(
        &source_path,
        source,
        QueryPosition {
            line: 1,
            character: 12,
        },
        Some("renamed_value"),
    );
    let first_definition = first.definition.as_ref().expect("first parameter resolves");
    assert_eq!(first_definition.path, normalize_path(source_path.clone()));
    assert_eq!(first_definition.detail, "Local parameter value");
    assert_eq!(
        first_definition.range,
        SourceRange {
            start_line: 0,
            start_character: 10,
            end_line: 0,
            end_character: 15,
        }
    );
    assert_eq!(
        first
            .references
            .iter()
            .map(|reference| (
                reference.path.clone(),
                reference.range.start_line,
                reference.range.start_character,
            ))
            .collect::<Vec<_>>(),
        vec![
            (normalize_path(source_path.clone()), 0, 10),
            (normalize_path(source_path.clone()), 1, 11),
        ]
    );
    assert_eq!(first.rename_preview.len(), 2);
    assert!(first
        .rename_preview
        .iter()
        .all(|edit| edit.path == normalize_path(source_path.clone())
            && edit.new_text == "renamed_value"));

    let declaration = index.query_source_at_navigation(
        &source_path,
        source,
        QueryPosition {
            line: 0,
            character: 11,
        },
    );
    assert_eq!(
        declaration.definition.map(|definition| definition.range),
        Some(first_definition.range.clone())
    );

    let second = index.query_source_at(
        &source_path,
        source,
        QueryPosition {
            line: 4,
            character: 12,
        },
        Some("other_value"),
    );
    assert_eq!(
        second
            .definition
            .as_ref()
            .map(|definition| definition.range.start_line),
        Some(3)
    );
    assert_eq!(
        second
            .references
            .iter()
            .map(|reference| reference.range.start_line)
            .collect::<Vec<_>>(),
        vec![3, 4]
    );
    assert_eq!(second.rename_preview.len(), 2);

    fs::remove_dir_all(root).ok();
}

#[test]
fn parameter_navigation_honors_nested_function_shadowing_and_closures() {
    let root = test_root("nested-parameter-binding");
    let source_path = root.join("live.py");
    let index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });

    let closure_source =
        "def outer(value):\n    def inner():\n        return value\n    return inner()\n";
    let closure = index.query_source_at_navigation(
        &source_path,
        closure_source,
        QueryPosition {
            line: 2,
            character: 16,
        },
    );
    assert_eq!(
        closure.definition.map(|definition| definition.range),
        Some(SourceRange {
            start_line: 0,
            start_character: 10,
            end_line: 0,
            end_character: 15,
        })
    );

    let shadow_source =
        "def outer(value):\n    def inner(value):\n        return value\n    return inner(value)\n";
    let inner = index.query_source_at_navigation(
        &source_path,
        shadow_source,
        QueryPosition {
            line: 2,
            character: 16,
        },
    );
    assert_eq!(
        inner.definition.map(|definition| definition.range),
        Some(SourceRange {
            start_line: 1,
            start_character: 14,
            end_line: 1,
            end_character: 19,
        })
    );

    let receiver_source = "class Example:\n    def read(self):\n        return self\n";
    let receiver = index.query_source_at_navigation(
        &source_path,
        receiver_source,
        QueryPosition {
            line: 2,
            character: 16,
        },
    );
    assert_eq!(
        receiver.definition.map(|definition| definition.range),
        Some(SourceRange {
            start_line: 1,
            start_character: 13,
            end_line: 1,
            end_character: 17,
        })
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn local_import_alias_binding_uses_exact_range_and_lexical_scope() {
    let path = PathBuf::from("/workspace/consumer.py");
    let source = [
        "from package.target_mod import target as t",
        "top = t()",
        "",
        "def first():",
        "    from other import target as t",
        "    return t()",
        "",
        "def second():",
        "    return t()",
    ]
    .join("\n");
    let top = local_import_alias_symbol_from_source(
        "consumer",
        &path,
        &source,
        "t",
        &SourceRange {
            start_line: 1,
            start_character: 6,
            end_line: 1,
            end_character: 7,
        },
    )
    .expect("top-level alias should resolve");
    assert_eq!(top.range.start_line, 0);
    assert_eq!(top.range.start_character, 41);
    assert_eq!(
        top.import_from.as_deref(),
        Some("package.target_mod::target")
    );

    let inner = local_import_alias_symbol_from_source(
        "consumer",
        &path,
        &source,
        "t",
        &SourceRange {
            start_line: 5,
            start_character: 11,
            end_line: 5,
            end_character: 12,
        },
    )
    .expect("function-local alias should shadow the module alias");
    assert_eq!(inner.range.start_line, 4);
    assert_eq!(inner.range.start_character, 32);
    assert_eq!(inner.import_from.as_deref(), Some("other::target"));

    let second = local_import_alias_symbol_from_source(
        "consumer",
        &path,
        &source,
        "t",
        &SourceRange {
            start_line: 8,
            start_character: 11,
            end_line: 8,
            end_character: 12,
        },
    )
    .expect("another function must see the module alias, not first()'s alias");
    assert_eq!(second.range.start_line, 0);

    assert!(local_import_alias_symbol_from_source(
        "consumer",
        &path,
        "from package.target_mod import target\nvalue = target()\n",
        "target",
        &SourceRange {
            start_line: 1,
            start_character: 8,
            end_line: 1,
            end_character: 14,
        },
    )
    .is_none());
}

#[test]
fn local_import_alias_binding_rejects_visible_rebindings() {
    let path = PathBuf::from("/workspace/consumer.py");
    let usage = |line, character| SourceRange {
        start_line: line,
        start_character: character,
        end_line: line,
        end_character: character + 5,
    };

    let reassigned = [
        "from provider import target as alias",
        "before = alias()",
        "alias = replacement",
        "after = alias()",
    ]
    .join("\n");
    assert!(local_import_alias_symbol_from_source(
        "consumer",
        &path,
        &reassigned,
        "alias",
        &usage(1, 9),
    )
    .is_some());
    assert!(local_import_alias_symbol_from_source(
        "consumer",
        &path,
        &reassigned,
        "alias",
        &usage(3, 8),
    )
    .is_none());

    let parameter = [
        "from provider import target as alias",
        "def consume(alias):",
        "    return alias()",
    ]
    .join("\n");
    assert!(local_import_alias_symbol_from_source(
        "consumer",
        &path,
        &parameter,
        "alias",
        &usage(2, 11),
    )
    .is_none());

    let unrelated_parameter = [
        "from provider import target as alias",
        "def consume(value):",
        "    return alias(value)",
    ]
    .join("\n");
    assert!(local_import_alias_symbol_from_source(
        "consumer",
        &path,
        &unrelated_parameter,
        "alias",
        &usage(2, 11),
    )
    .is_some());

    let rebound_before_import = [
        "alias = replacement",
        "from provider import target as alias",
        "value = alias()",
    ]
    .join("\n");
    let resolved = local_import_alias_symbol_from_source(
        "consumer",
        &path,
        &rebound_before_import,
        "alias",
        &usage(2, 8),
    )
    .expect("the later import should replace an earlier binding in the same scope");
    assert_eq!(resolved.range.start_line, 1);
}
