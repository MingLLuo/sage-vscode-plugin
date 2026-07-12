use super::*;

#[test]
fn runtime_documentation_writeback_survives_hydrate() {
    let root = test_root("runtime-doc-writeback");
    fs::write(
        root.join("provider.py"),
        "def undocumented():\n    return 1\n",
    )
    .unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    let mut index = WorkspaceIndex::new(options.clone());
    index.rebuild().unwrap();

    assert!(index
        .documentation_for_symbol("undocumented")
        .and_then(|documentation| documentation.docstring)
        .is_none());
    index
        .write_runtime_documentation(
            "undocumented",
            &DocumentationRecord {
                name: "undocumented".to_string(),
                module_name: "runtime.provider".to_string(),
                kind: "function".to_string(),
                detail: "undocumented()".to_string(),
                summary: "Runtime generated docs.".to_string(),
                docstring: Some("Runtime generated docs.\n\nFull runtime body.".to_string()),
                uri: Some(root.join("provider.py").display().to_string()),
                markers: vec!["runtime".to_string()],
                sections: Vec::new(),
            },
        )
        .unwrap();

    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    let documentation = hydrated
        .documentation_for_symbol("undocumented")
        .expect("runtime documentation should be readable after hydrate");
    assert_eq!(documentation.summary, "Runtime generated docs.");
    assert_eq!(
        documentation.docstring.as_deref(),
        Some("Runtime generated docs.\n\nFull runtime body.")
    );
    assert!(documentation
        .markers
        .iter()
        .any(|marker| marker == "runtime-writeback"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn runtime_documentation_writeback_covers_runtime_only_symbols() {
    let root = test_root("runtime-doc-runtime-only");
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    let mut index = WorkspaceIndex::new(options.clone());
    index.rebuild().unwrap();
    index
        .write_runtime_documentation(
            "FutureRuntimeOnly",
            &DocumentationRecord {
                name: "FutureRuntimeOnly".to_string(),
                module_name: "sage.runtime".to_string(),
                kind: "type".to_string(),
                detail: "FutureRuntimeOnly".to_string(),
                summary: "Runtime-only Sage API docs.".to_string(),
                docstring: Some("Runtime-only Sage API docs.".to_string()),
                uri: None,
                markers: vec!["runtime".to_string()],
                sections: Vec::new(),
            },
        )
        .unwrap();

    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    assert_eq!(
        hydrated
            .documentation_for_symbol("FutureRuntimeOnly")
            .map(|documentation| documentation.summary),
        Some("Runtime-only Sage API docs.".to_string())
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn reconcile_prewarms_hot_sage_symbols_and_method_docs() {
    let root = test_root("hot-symbol-hydrate");
    fs::create_dir_all(root.join("sage/rings/polynomial")).unwrap();
    fs::create_dir_all(root.join("sage/matrix")).unwrap();
    fs::write(
            root.join("sage/rings/polynomial/polynomial_ring_constructor.py"),
            "def PolynomialRing(*args):\n    \"\"\"Construct a cached polynomial ring.\"\"\"\n    return args\n",
        )
        .unwrap();
    fs::write(
        root.join("sage/matrix/matrix0.pyx"),
        "def rank(self):\n    \"\"\"Return cached matrix rank.\"\"\"\n    return 0\n",
    )
    .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import matrix\nmat = matrix([])\nvalue = mat.rank()\n";
    fs::write(&consumer, source).unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    let mut index = WorkspaceIndex::new(options.clone());
    index.rebuild().unwrap();
    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    hydrated.reconcile_with_cache().unwrap();

    assert!(
        hydrated.status().hot_symbol_cache_count >= hot_sage_symbol_names().len(),
        "hot cache should be populated after hydrate: {:?}",
        hydrated.status()
    );
    assert_eq!(
        hydrated
            .documentation_for_symbol("PolynomialRing")
            .map(|docs| docs.summary),
        Some("Construct a cached polynomial ring.".to_string())
    );
    let (line, character) = member_position(source, "rank");
    let query =
        hydrated.query_source_at_navigation(&consumer, source, QueryPosition { line, character });
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|docs| docs.summary.as_str()),
        Some("Return cached matrix rank.")
    );
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/matrix/matrix0.pyx")).as_path())
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn fast_reconcile_keeps_indexed_all_py_exports_queryable_for_future_sage_apis() {
    let root = test_root("dynamic-hot-all-exports");
    fs::create_dir_all(root.join("sage/future")).unwrap();
    fs::write(
        root.join("sage/all.py"),
        "from sage.future.all import FuturePublic\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/future/all.py"),
        "from sage.future.module import FutureImplementation as FuturePublic\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/future/module.py"),
            "def FutureImplementation(*args):\n    \"\"\"Future Sage API docs.\"\"\"\n    return args\n",
        )
        .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import FuturePublic\nvalue = FuturePublic()\n";
    fs::write(&consumer, source).unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    let mut index = WorkspaceIndex::new(options.clone());
    index.rebuild().unwrap();
    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    hydrated.reconcile_with_cache().unwrap();

    assert!(
        matches!(
            hydrated.status().last_operation.as_deref(),
            Some("fast-reconcile")
        ),
        "{:?}",
        hydrated.status()
    );

    let query =
        hydrated.query_source_symbol(&consumer, source, "FuturePublic", None, None, Vec::new());
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/future/module.py")).as_path())
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Future Sage API docs.")
    );
    fs::remove_dir_all(root).ok();
}
