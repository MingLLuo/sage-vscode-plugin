use super::*;

#[test]
fn cache_hydrates_symbols_and_docs() {
    let root = test_root("hydrate");
    let source = root.join("docs.py");
    fs::write(
        &source,
        "def cached_symbol():\n    \"\"\"Cached docs.\"\"\"\n    return 1\n",
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
    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    let symbol = hydrated
        .symbol("cached_symbol")
        .expect("hydrated symbol should exist");
    assert_eq!(symbol.docstring.as_deref(), Some("Cached docs."));
    assert!(hydrated
        .symbols_with_prefix("cached", 10)
        .iter()
        .any(|symbol| symbol.name == "cached_symbol"));
    assert!(hydrated
        .workspace_symbols("cached_symbol", 10)
        .iter()
        .any(|symbol| symbol.name == "cached_symbol"));
    assert!(hydrated
        .file_for_path(&source)
        .expect("hydrated file should exist")
        .symbols
        .iter()
        .any(|symbol| symbol.name == "cached_symbol"));
    assert!(hydrated.status().cache_hit_count > 0);
    fs::remove_dir_all(root).ok();
}

#[test]
fn workspace_symbols_rank_exact_prefix_and_word_boundary_matches() {
    let root = test_root("workspace-symbol-ranking");
    fs::write(
        root.join("constructors.py"),
        [
            "def buildPolynomialRing():",
            "    return None",
            "",
            "def PolynomialRing():",
            "    return None",
            "",
            "def polynomial_ring_factory():",
            "    return None",
            "",
            "def string_helper():",
            "    return None",
        ]
        .join("\n"),
    )
    .unwrap();
    fs::write(
        root.join("uses.py"),
        [
            "from constructors import PolynomialRing",
            "",
            "def use_constructor():",
            "    return PolynomialRing()",
        ]
        .join("\n"),
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

    let exact = index.workspace_symbols("PolynomialRing", 10);
    assert_eq!(
        exact.first().map(|symbol| symbol.name.as_str()),
        Some("PolynomialRing")
    );
    assert!(
        exact.iter().all(|symbol| symbol.name == "PolynomialRing"),
        "{exact:?}"
    );
    assert!(
        exact.iter().all(|symbol| symbol.kind != SymbolKind::Import),
        "workspace symbols should hide import noise when definitions exist: {exact:?}"
    );

    let prefix = index.workspace_symbols("poly", 10);
    assert_eq!(
        prefix.first().map(|symbol| symbol.name.as_str()),
        Some("PolynomialRing")
    );
    assert!(
        prefix
            .iter()
            .filter(|symbol| symbol.name == "PolynomialRing")
            .all(|symbol| symbol.kind != SymbolKind::Import),
        "prefix search should also hide duplicate import entries: {prefix:?}"
    );

    let boundary = index.workspace_symbols("ring", 10);
    assert_eq!(
        boundary.first().map(|symbol| symbol.name.as_str()),
        Some("polynomial_ring_factory")
    );

    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    let hydrated_exact = hydrated.workspace_symbols("PolynomialRing", 10);
    assert_eq!(
        hydrated_exact.first().map(|symbol| symbol.name.as_str()),
        Some("PolynomialRing")
    );
    assert!(
            hydrated_exact
                .iter()
                .all(|symbol| symbol.kind != SymbolKind::Import),
            "hydrated workspace symbols should keep the same import-noise suppression: {hydrated_exact:?}"
        );
    fs::remove_dir_all(root).ok();
}

#[test]
fn cache_metadata_hydrates_counts_without_path_scan() {
    let root = test_root("metadata-hydrate");
    fs::write(
        root.join("docs.py"),
        "def cached_symbol():\n    \"\"\"Cached docs.\"\"\"\n    return 1\n",
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
    let connection = Connection::open(index.db_path()).unwrap();
    let metadata_counts = load_cached_counts_from_metadata(&connection, &index.options().roots)
        .unwrap()
        .expect("root metadata should be persisted");
    assert_eq!(metadata_counts, (1, 2, 1));

    let mut hydrated = WorkspaceIndex::new(options);
    let status = hydrated.hydrate_from_cache().unwrap();
    assert_eq!(status.indexed_file_count, 1);
    assert_eq!(status.symbol_count, 2);
    assert_eq!(status.doc_count, 1);
    assert_eq!(status.last_operation.as_deref(), Some("hydrate"));
    assert!(status.last_hydrate_ms <= status.last_index_ms);
    fs::remove_dir_all(root).ok();
}

#[test]
fn cache_metadata_counts_overlapping_roots_without_double_counting() {
    let root = test_root("metadata-overlap");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        root.join("top_level.py"),
        "def top_level_symbol():\n    return 1\n",
    )
    .unwrap();
    fs::write(
        src.join("nested.py"),
        "def nested_symbol():\n    \"\"\"Nested docs.\"\"\"\n    return 2\n",
    )
    .unwrap();
    let options = IndexOptions {
        roots: vec![root.clone(), src],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    let mut index = WorkspaceIndex::new(options.clone());
    let rebuilt = index.rebuild().unwrap();
    let connection = Connection::open(index.db_path()).unwrap();
    let metadata_counts = load_cached_counts_from_metadata(&connection, &index.options().roots)
        .unwrap()
        .expect("overlapping roots should use the parent root metadata row");
    assert_eq!(
        metadata_counts,
        (
            rebuilt.indexed_file_count,
            rebuilt.symbol_count,
            rebuilt.doc_count,
        ),
    );

    let mut hydrated = WorkspaceIndex::new(options);
    let status = hydrated.hydrate_from_cache().unwrap();
    assert_eq!(status.indexed_file_count, rebuilt.indexed_file_count);
    assert_eq!(status.symbol_count, rebuilt.symbol_count);
    assert_eq!(status.doc_count, rebuilt.doc_count);
    assert_eq!(status.cache_miss_count, 0);
    fs::remove_dir_all(root).ok();
}

#[test]
fn cache_namespace_tracks_source_roots_for_future_sage_updates() {
    let root_a = test_root("cache-root-a");
    let root_b = test_root("cache-root-b");
    let cache_dir = test_root("cache-namespace");
    fs::write(root_a.join("module.py"), "def from_a():\n    return 1\n").unwrap();
    fs::write(root_b.join("module.py"), "def from_b():\n    return 2\n").unwrap();
    let options_a = IndexOptions {
        roots: vec![root_a.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: cache_dir.clone(),
        enable_pyx: true,
    };
    let options_b = IndexOptions {
        roots: vec![root_b.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir,
        enable_pyx: true,
    };

    let mut index_a = WorkspaceIndex::new(options_a);
    let mut index_b = WorkspaceIndex::new(options_b);
    assert_ne!(index_a.db_path(), index_b.db_path());

    index_a.rebuild().unwrap();
    index_b.rebuild().unwrap();

    let mut reopened_a = WorkspaceIndex::new(index_a.options().clone());
    reopened_a.hydrate_from_cache().unwrap();
    assert!(reopened_a.symbol("from_a").is_some());
    assert!(reopened_a.symbol("from_b").is_none());

    let mut reopened_b = WorkspaceIndex::new(index_b.options().clone());
    reopened_b.hydrate_from_cache().unwrap();
    assert!(reopened_b.symbol("from_b").is_some());
    assert!(reopened_b.symbol("from_a").is_none());

    fs::remove_dir_all(root_a).ok();
    fs::remove_dir_all(root_b).ok();
    fs::remove_dir_all(index_a.options().cache_dir.clone()).ok();
}

#[test]
fn status_exposes_cache_namespace_and_source_root_fingerprints() {
    let root = test_root("root-fingerprint");
    fs::create_dir_all(root.join("sage")).unwrap();
    fs::write(root.join("sage").join("version.py"), "version = '10.6'\n").unwrap();
    fs::write(root.join("module.py"), "def exported():\n    return 1\n").unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: vec!["**/__pycache__/**".to_string()],
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    let mut index = WorkspaceIndex::new(options);
    let status = index.rebuild().unwrap();
    assert_eq!(status.cache_namespace.len(), 16);
    assert_eq!(status.source_root_fingerprints.len(), 1);
    assert!(status.source_root_fingerprints[0].exists);
    assert_eq!(status.source_root_fingerprints[0].digest.len(), 16);
    assert!(status.source_root_fingerprints[0]
        .marker
        .as_deref()
        .is_some_and(|marker| marker.ends_with("sage/version.py")));

    let before = status.source_root_fingerprints[0].digest.clone();
    fs::write(root.join("sage").join("version.py"), "version = '10.7'\n").unwrap();
    let after = source_root_fingerprint(&root).digest;
    assert_ne!(before, after);

    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrate_reports_stale_cache_when_source_root_fingerprint_changes() {
    let root = test_root("root-fingerprint-stale");
    fs::create_dir_all(root.join("sage")).unwrap();
    fs::write(root.join("sage").join("version.py"), "version = '10.6'\n").unwrap();
    fs::write(root.join("module.py"), "def exported():\n    return 1\n").unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    let mut index = WorkspaceIndex::new(options.clone());
    let original = index.rebuild().unwrap();
    assert!(!original.cache_stale);

    fs::write(root.join("sage").join("version.py"), "version = '10.7'\n").unwrap();
    let mut hydrated = WorkspaceIndex::new(options);
    let stale = hydrated.hydrate_from_cache().unwrap();
    assert!(stale.cache_stale);
    assert_eq!(stale.stale_source_roots.len(), 1);
    assert_eq!(
        stale.stale_source_roots[0].root,
        normalize_path(root.clone()).display().to_string()
    );
    assert_ne!(
        stale.stale_source_roots[0].cached_digest,
        stale.stale_source_roots[0].current_digest
    );
    assert!(stale.stale_source_roots[0]
        .current_marker
        .as_deref()
        .is_some_and(|marker| marker.ends_with("sage/version.py")));

    let reconciled = hydrated.reconcile_with_cache().unwrap();
    assert!(!reconciled.cache_stale);
    assert!(reconciled.stale_source_roots.is_empty());

    fs::remove_dir_all(root).ok();
}

#[test]
fn root_delete_uses_bulk_path_match_without_touching_other_roots() {
    let root = test_root("bulk-root-delete");
    let outside = test_root("bulk-root-keep");
    let connection = Connection::open_in_memory().unwrap();
    create_schema(&connection).unwrap();
    for path in [
        root.join("a.py"),
        root.join("nested").join("b.py"),
        outside.join("c.py"),
    ] {
        let path_text = path.display().to_string();
        connection
            .execute(
                "insert into files(path, module, fingerprint) values(?1, 'm', 'f')",
                params![path_text],
            )
            .unwrap();
        connection
                .execute(
                    "insert into symbols(name, kind, module, path, start_line, start_character, end_line, end_character, detail) values('x', 'Function', 'm', ?1, 0, 0, 0, 1, 'x()')",
                    params![path_text],
                )
                .unwrap();
        connection
                .execute(
                    "insert into docs(name, module, path, detail, docstring) values('x', 'm', ?1, 'x()', 'docs')",
                    params![path_text],
                )
                .unwrap();
        connection
                .execute(
                    "insert into reference_spans(name, path, start_line, start_character, end_line, end_character) values('x', ?1, 0, 0, 0, 1)",
                    params![path_text],
                )
                .unwrap();
    }

    delete_roots_from_db(&connection, std::slice::from_ref(&root)).unwrap();

    for table in ["files", "symbols", "docs", "reference_spans"] {
        let remaining: i64 = connection
            .query_row(&format!("select count(*) from {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            remaining, 1,
            "{table} should keep only the outside root row"
        );
    }
    fs::remove_dir_all(root).ok();
    fs::remove_dir_all(outside).ok();
}

#[test]
fn editable_reference_cache_hydrates_and_refreshes() {
    let sage_root = test_root("reference-cache-sage");
    let workspace = test_root("reference-cache-workspace");
    fs::write(
        sage_root.join("library.py"),
        "def target():\n    return target\n",
    )
    .unwrap();
    let first = workspace.join("first.py");
    let second = workspace.join("second.py");
    fs::write(&first, "def target():\n    return target()\n").unwrap();
    fs::write(&second, "value = target()\n").unwrap();
    let options = IndexOptions {
        roots: vec![sage_root.clone(), workspace.clone()],
        editable_roots: vec![workspace.clone()],
        exclude_globs: Vec::new(),
        cache_dir: workspace.join(".cache"),
        enable_pyx: true,
    };
    let mut index = WorkspaceIndex::new(options.clone());
    index.rebuild().unwrap();

    let connection = Connection::open(index.db_path()).unwrap();
    let cached_reference_count: usize = connection
        .query_row(
            "select count(*) from reference_spans where name = 'target'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        cached_reference_count, 3,
        "only editable workspace references should be materialized"
    );

    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    assert_eq!(hydrated.editable_references("target").len(), 3);

    fs::write(&first, "def replacement():\n    return 1\n").unwrap();
    fs::write(&second, "value = target() + target()\n").unwrap();
    hydrated
        .refresh_paths(&[first.clone(), second.clone()], &[])
        .unwrap();
    let references = hydrated.editable_references("target");
    assert_eq!(
        references.len(),
        2,
        "file-level upsert should remove stale spans and add changed spans"
    );
    let normalized_second = normalize_path(second);
    assert!(references
        .iter()
        .all(|reference| reference.path == normalized_second));

    fs::remove_dir_all(sage_root).ok();
    fs::remove_dir_all(workspace).ok();
}
