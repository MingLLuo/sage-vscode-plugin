use super::*;

#[test]
fn root_aware_cache_reuses_same_root_set_and_isolates_other_projects() {
    let root = test_root("shared-cache");
    let cache_dir = root.join(".cache");
    let shared = root.join("shared-sage-root");
    let project_a = root.join("project-a");
    let project_b = root.join("project-b");
    fs::create_dir_all(&shared).unwrap();
    fs::create_dir_all(&project_a).unwrap();
    fs::create_dir_all(&project_b).unwrap();
    fs::write(
        shared.join("common.py"),
        "def common_symbol():\n    \"\"\"Shared docs.\"\"\"\n    return 1\n",
    )
    .unwrap();
    fs::write(
        project_a.join("a.sage"),
        "from common import common_symbol\n",
    )
    .unwrap();
    fs::write(
        project_b.join("b.sage"),
        "from common import common_symbol\n",
    )
    .unwrap();

    let mut first = WorkspaceIndex::new(IndexOptions {
        roots: vec![project_a.clone(), shared.clone()],
        editable_roots: vec![project_a.clone()],
        exclude_globs: Vec::new(),
        cache_dir: cache_dir.clone(),
        enable_pyx: true,
    });
    first.reconcile_with_cache().unwrap();
    assert_eq!(first.status().cache_miss_count, 2);

    let mut same_roots = WorkspaceIndex::new(IndexOptions {
        roots: vec![project_a.clone(), shared.clone()],
        editable_roots: vec![project_a.clone()],
        exclude_globs: Vec::new(),
        cache_dir: cache_dir.clone(),
        enable_pyx: true,
    });
    assert_eq!(first.db_path(), same_roots.db_path());
    same_roots.hydrate_from_cache().unwrap();
    same_roots.reconcile_with_cache().unwrap();
    let same_status = same_roots.status();
    assert!(same_status.cache_hit_count >= 2, "{same_status:?}");
    assert_eq!(same_status.cache_miss_count, 0, "{same_status:?}");
    assert!(same_roots.symbol("common_symbol").is_some());
    assert!(same_roots
        .file_for_path(&project_a.join("a.sage"))
        .is_some());

    let mut other_project = WorkspaceIndex::new(IndexOptions {
        roots: vec![project_b.clone(), shared.clone()],
        editable_roots: vec![project_b.clone()],
        exclude_globs: Vec::new(),
        cache_dir,
        enable_pyx: true,
    });
    assert_ne!(first.db_path(), other_project.db_path());
    let cold_hydrate = other_project.hydrate_from_cache().unwrap();
    assert_eq!(cold_hydrate.peer_seed_file_count, 0, "{cold_hydrate:?}");
    assert_eq!(cold_hydrate.cache_miss_count, 1, "{cold_hydrate:?}");
    other_project.reconcile_with_cache().unwrap();

    let status = other_project.status();
    assert!(status.cache_hit_count >= 1, "{status:?}");
    assert_eq!(status.cache_miss_count, 2, "{status:?}");
    assert!(status.peer_seed_file_count >= 1, "{status:?}");
    assert!(other_project.symbol("common_symbol").is_some());
    assert!(other_project
        .file_for_path(&project_a.join("a.sage"))
        .is_none());
    assert!(other_project
        .file_for_path(&project_b.join("b.sage"))
        .is_some());
    fs::remove_dir_all(root).ok();
}

#[test]
fn cached_lookup_survives_single_file_refresh_overlay() {
    let root = test_root("cache-overlay");
    let src = root.join("src");
    fs::create_dir_all(&src).unwrap();
    let bridge = src.join("cythonish_bridge.pyx");
    let declaration = src.join("native_support.pxd");
    let source = "from native_support cimport NativeAccumulator\n\ncdef class StepCounter(NativeAccumulator):\n    pass\n";
    fs::write(&bridge, source).unwrap();
    fs::write(&declaration, "cdef class NativeAccumulator:\n    pass\n").unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    WorkspaceIndex::new(options.clone())
        .reconcile_with_cache()
        .unwrap();

    let mut reopened = WorkspaceIndex::new(options);
    reopened.hydrate_from_cache().unwrap();
    reopened
        .refresh_paths(std::slice::from_ref(&bridge), &[])
        .unwrap();

    let query =
        reopened.query_source_symbol(&bridge, source, "NativeAccumulator", None, None, Vec::new());
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(declaration.clone()).as_path())
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn cache_falls_back_when_configured_cache_dir_is_unusable() {
    let root = test_root("cache-fallback");
    let source = root.join("docs.py");
    fs::write(&source, "def fallback_symbol():\n    return 1\n").unwrap();
    let unusable_cache_path = root.join("not-a-directory");
    fs::write(&unusable_cache_path, "blocks cache directory creation").unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: unusable_cache_path.clone(),
        enable_pyx: true,
    });

    let status = index.rebuild().unwrap();

    assert!(status.last_error.is_none());
    assert_ne!(
        Path::new(&status.cache_path).parent(),
        Some(unusable_cache_path.as_path())
    );
    assert!(Path::new(&status.cache_path).exists());
    assert!(index.symbol("fallback_symbol").is_some());
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrated_incremental_fallback_rebuilds_complete_index() {
    let root = test_root("hydrated-incremental-fallback");
    let changed_source = root.join("changed.py");
    let unchanged_source = root.join("unchanged.py");
    fs::write(&changed_source, "def before():\n    return 1\n").unwrap();
    fs::write(&unchanged_source, "def unchanged():\n    return 2\n").unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    let mut initial = WorkspaceIndex::new(options.clone());
    initial.rebuild().unwrap();
    let primary_db = initial.db_path().to_path_buf();
    drop(initial);

    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    assert!(hydrated.symbol("before").is_some());
    assert!(hydrated.symbol("unchanged").is_some());

    fs::write(&changed_source, "def after():\n    return 3\n").unwrap();
    fs::remove_file(&primary_db).unwrap();
    fs::create_dir(&primary_db).unwrap();

    let status = hydrated
        .refresh_paths(std::slice::from_ref(&changed_source), &[])
        .unwrap();
    assert!(status.last_error.is_none(), "{:?}", status.last_error);
    assert_ne!(hydrated.db_path(), primary_db.as_path());
    assert!(hydrated.symbol("before").is_none());
    assert!(hydrated.symbol("after").is_some());
    assert!(hydrated.symbol("unchanged").is_some());

    let fallback_db = hydrated.db_path().to_path_buf();
    let fallback_options = hydrated.options().clone();
    let mut reopened = WorkspaceIndex::new(fallback_options);
    reopened.hydrate_from_cache().unwrap();
    assert!(reopened.symbol("before").is_none());
    assert!(reopened.symbol("after").is_some());
    assert!(reopened.symbol("unchanged").is_some());

    fs::remove_file(fallback_db).ok();
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrated_refresh_rebuilds_complete_primary_cache_when_database_disappears() {
    let root = test_root("hydrated-refresh-missing-database");
    let changed_source = root.join("changed.py");
    let unchanged_source = root.join("unchanged.py");
    fs::write(&changed_source, "def before():\n    return 1\n").unwrap();
    fs::write(&unchanged_source, "def unchanged():\n    return 2\n").unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    WorkspaceIndex::new(options.clone()).rebuild().unwrap();

    let mut hydrated = WorkspaceIndex::new(options.clone());
    hydrated.hydrate_from_cache().unwrap();
    let primary_db = hydrated.db_path().to_path_buf();
    assert!(hydrated.symbol("unchanged").is_some());

    fs::write(&changed_source, "def after():\n    return 3\n").unwrap();
    fs::remove_file(&primary_db).unwrap();
    let status = hydrated
        .refresh_paths(std::slice::from_ref(&changed_source), &[])
        .unwrap();

    assert_eq!(status.last_operation.as_deref(), Some("rebuild"));
    assert!(status.last_error.is_none(), "{:?}", status.last_error);
    assert_eq!(hydrated.db_path(), primary_db.as_path());
    assert!(hydrated.symbol("before").is_none());
    assert!(hydrated.symbol("after").is_some());
    assert!(hydrated.symbol("unchanged").is_some());

    let mut reopened = WorkspaceIndex::new(options);
    reopened.hydrate_from_cache().unwrap();
    assert!(reopened.symbol("before").is_none());
    assert!(reopened.symbol("after").is_some());
    assert!(reopened.symbol("unchanged").is_some());

    fs::remove_dir_all(root).ok();
}

#[test]
fn reconcile_after_hydrate_rebuilds_when_cached_rows_are_truncated() {
    let root = test_root("hydrate-truncated-cache");
    let source = root.join("module.py");
    fs::write(
        &source,
        "def restored_symbol():\n    \"\"\"Restored documentation.\"\"\"\n    return 1\n",
    )
    .unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    for table in ["files", "symbols", "docs"] {
        let mut initial = WorkspaceIndex::new(options.clone());
        initial.rebuild().unwrap();
        let connection = Connection::open(initial.db_path()).unwrap();
        connection
            .execute(&format!("delete from {table}"), [])
            .unwrap();
        drop(connection);

        let mut hydrated = WorkspaceIndex::new(options.clone());
        let hydrated_status = hydrated.hydrate_from_cache().unwrap();
        assert_eq!(hydrated_status.last_operation.as_deref(), Some("hydrate"));
        let status = hydrated.reconcile_with_cache().unwrap();
        assert_eq!(
            status.last_operation.as_deref(),
            Some("rebuild"),
            "truncating {table} must trigger recovery"
        );
        assert!(status.cache_miss_count >= 1, "{status:?}");
        assert!(hydrated.symbol("restored_symbol").is_some());
        assert_eq!(
            hydrated
                .documentation_for_symbol("restored_symbol")
                .and_then(|record| record.docstring),
            Some("Restored documentation.".to_string())
        );
    }
    fs::remove_dir_all(root).ok();
}

#[test]
fn fast_reconcile_rebuilds_when_cache_is_truncated_after_hydrate() {
    let root = test_root("reconcile-truncated-cache");
    let source = root.join("module.py");
    fs::write(&source, "def restored_symbol():\n    return 1\n").unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    WorkspaceIndex::new(options.clone()).rebuild().unwrap();
    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    let connection = Connection::open(hydrated.db_path()).unwrap();
    connection.execute("delete from symbols", []).unwrap();
    drop(connection);

    let status = hydrated.reconcile_with_cache().unwrap();
    assert_eq!(status.last_operation.as_deref(), Some("rebuild"));
    assert!(hydrated.symbol("restored_symbol").is_some());
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrated_refresh_rebuilds_when_unchanged_cache_rows_are_truncated() {
    let root = test_root("refresh-truncated-cache");
    let changed_source = root.join("changed.py");
    let unchanged_source = root.join("unchanged.py");
    fs::write(&changed_source, "def before():\n    return 1\n").unwrap();
    fs::write(&unchanged_source, "def unchanged():\n    return 2\n").unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    WorkspaceIndex::new(options.clone()).rebuild().unwrap();
    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    let connection = Connection::open(hydrated.db_path()).unwrap();
    connection
        .execute(
            "delete from symbols where path = ?1",
            params![normalize_path(unchanged_source.clone())
                .display()
                .to_string()],
        )
        .unwrap();
    drop(connection);
    fs::write(&changed_source, "def after():\n    return 3\n").unwrap();

    let status = hydrated
        .refresh_paths(std::slice::from_ref(&changed_source), &[])
        .unwrap();
    assert_eq!(status.last_operation.as_deref(), Some("rebuild"));
    assert!(hydrated.symbol("before").is_none());
    assert!(hydrated.symbol("after").is_some());
    assert!(hydrated.symbol("unchanged").is_some());
    fs::remove_dir_all(root).ok();
}

#[test]
fn reconcile_checks_editable_file_fingerprints_before_fast_path() {
    let root = test_root("reconcile-editable-fingerprint");
    let source = root.join("module.py");
    fs::write(&source, "def before():\n    return 1\n").unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    WorkspaceIndex::new(options.clone()).rebuild().unwrap();
    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    fs::write(
        &source,
        "def replacement_with_different_size():\n    return 2\n",
    )
    .unwrap();

    let status = hydrated.reconcile_with_cache().unwrap();
    assert_ne!(status.last_operation.as_deref(), Some("fast-reconcile"));
    assert!(hydrated.symbol("before").is_none());
    assert!(hydrated.symbol("replacement_with_different_size").is_some());
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrate_missing_configured_cache_stays_on_configured_path() {
    let root = test_root("cache-hydrate-missing-primary");
    let configured_cache = root.join(".cache");
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: configured_cache.clone(),
        enable_pyx: true,
    });

    let status = index.hydrate_from_cache().unwrap();

    assert_eq!(
        Path::new(&status.cache_path).parent(),
        Some(configured_cache.as_path())
    );
    assert_eq!(status.cache_miss_count, 1);
    assert!(configured_cache.exists());
    fs::remove_dir_all(root).ok();
}

#[test]
fn refresh_paths_updates_and_deletes_files() {
    let root = test_root("refresh");
    let source = root.join("module.py");
    fs::write(&source, "def before():\n    return 1\n").unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();
    fs::write(&source, "def after():\n    return 2\n").unwrap();
    index
        .refresh_paths(std::slice::from_ref(&source), &[])
        .unwrap();
    assert_eq!(index.cached_symbol_count, 0);
    assert!(index.symbol("after").is_some());
    assert!(index.symbol("before").is_none());
    fs::remove_file(&source).unwrap();
    index
        .refresh_paths(&[], std::slice::from_ref(&source))
        .unwrap();
    assert!(index.symbol("after").is_none());
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrated_refresh_updates_overlapping_root_metadata() {
    let root = test_root("hydrated-refresh-overlapping-roots");
    let nested_root = root.join("src");
    fs::create_dir_all(&nested_root).unwrap();
    let top_level = root.join("top_level.py");
    let nested = nested_root.join("nested.py");
    fs::write(&top_level, "def top_level_symbol():\n    return 1\n").unwrap();
    fs::write(&nested, "def nested_symbol():\n    return 2\n").unwrap();
    let options = IndexOptions {
        roots: vec![root.clone(), nested_root],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    WorkspaceIndex::new(options.clone()).rebuild().unwrap();

    let mut hydrated = WorkspaceIndex::new(options.clone());
    hydrated.hydrate_from_cache().unwrap();
    assert!(hydrated.symbol("nested_symbol").is_some());
    fs::remove_file(&nested).unwrap();

    let refreshed = hydrated
        .refresh_paths(&[], std::slice::from_ref(&nested))
        .unwrap();
    assert!(hydrated.symbol("nested_symbol").is_none());
    assert_eq!(refreshed.indexed_file_count, 1);
    assert_eq!(refreshed.symbol_count, 2);

    let mut reopened = WorkspaceIndex::new(options);
    let rehydrated = reopened.hydrate_from_cache().unwrap();
    assert_eq!(rehydrated.indexed_file_count, 1);
    assert_eq!(rehydrated.symbol_count, 2);
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrated_refresh_invalidates_renamed_and_deleted_symbol_caches() {
    let root = test_root("hydrated-refresh-cache");
    let renamed_source = root.join("renamed.py");
    let deleted_source = root.join("deleted.py");
    fs::write(&renamed_source, "def before():\n    return 1\n").unwrap();
    fs::write(&deleted_source, "def removed():\n    return 2\n").unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    WorkspaceIndex::new(options.clone()).rebuild().unwrap();

    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    assert!(hydrated.symbol("before").is_some());
    assert!(hydrated.symbol("removed").is_some());

    fs::write(&renamed_source, "def after():\n    return 3\n").unwrap();
    fs::remove_file(&deleted_source).unwrap();
    let status = hydrated
        .refresh_paths(
            std::slice::from_ref(&renamed_source),
            std::slice::from_ref(&deleted_source),
        )
        .unwrap();

    assert!(hydrated.symbol("before").is_none());
    assert!(hydrated.symbol("removed").is_none());
    assert!(hydrated.symbol("after").is_some());
    assert_eq!(status.indexed_file_count, 1);
    assert_eq!(status.symbol_count, 2);
    fs::remove_dir_all(root).ok();
}

#[test]
fn replacement_index_generation_advances_past_previous_index() {
    let mut index = WorkspaceIndex::default();
    index.ensure_generation_after(7);
    assert_eq!(index.status().generation, 8);
    index.ensure_generation_after(3);
    assert_eq!(index.status().generation, 8);
}

#[test]
fn preload_paths_warms_symbols_without_advancing_generation() {
    let root = test_root("preload");
    let source = root.join("local_docs.py");
    fs::write(&source, "class PolynomialNotebook:\n    pass\n").unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    let generation = index.status().generation;

    assert_eq!(index.preload_paths(std::slice::from_ref(&source)), 1);

    assert_eq!(index.status().generation, generation);
    assert!(index.symbol("PolynomialNotebook").is_some());
    let canonical_source = source.canonicalize().unwrap();
    assert_eq!(
        index.source_path_for_module("local_docs").as_deref(),
        Some(canonical_source.as_path())
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn collect_indexable_paths_limits_installed_site_packages_to_sage_package() {
    let root = test_root("site-packages-scan");
    let site_packages = root.join("local/lib/python3.14/site-packages");
    let sage_module = site_packages.join("sage/matrix/matrix0.pyx");
    let dependency_module = site_packages.join("numpy/core.py");
    fs::create_dir_all(sage_module.parent().unwrap()).unwrap();
    fs::create_dir_all(dependency_module.parent().unwrap()).unwrap();
    fs::write(&sage_module, "def rank(self):\n    pass\n").unwrap();
    fs::write(&dependency_module, "def array():\n    pass\n").unwrap();

    let paths = collect_indexable_paths(&IndexOptions {
        roots: vec![site_packages],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });

    assert_eq!(paths, vec![sage_module]);
    fs::remove_dir_all(root).ok();
}

#[test]
fn sage_prewarm_modules_cover_sage_heavy_method_paths() {
    let modules = sage_prewarm_modules_for_source(
            "from sage.all import GF, Polyhedron, PolynomialRing, matrix\nmat = matrix(GF(7), 2, 2)\nrank = mat.rank()\nR = PolynomialRing(GF(7), 'x')\nI = R.ideal([])\npoly = Polyhedron(vertices=[])\nvolume = poly.volume()\n",
        );

    assert!(modules.contains(&"sage.matrix.matrix0"));
    assert!(modules.contains(&"sage.matrix.matrix2"));
    assert!(modules.contains(&"sage.rings.polynomial.multi_polynomial_libsingular"));
    assert!(modules.contains(&"sage.rings.polynomial.multi_polynomial_ideal"));
    assert!(modules.contains(&"sage.geometry.polyhedron.base0"));
    assert!(modules.contains(&"sage.geometry.polyhedron.base7"));
}

#[test]
fn prewarmed_sage_method_modules_resolve_before_full_rebuild() {
    let root = test_root("method-prewarm");
    let site_packages = root.join("local/lib/python3.14/site-packages");
    let matrix0 = site_packages.join("sage/matrix/matrix0.pyx");
    let consumer = root.join("workspace/demo.py");
    fs::create_dir_all(matrix0.parent().unwrap()).unwrap();
    fs::create_dir_all(consumer.parent().unwrap()).unwrap();
    fs::write(
        &matrix0,
        "def rank(self):\n    \"\"\"Return the rank of this matrix.\"\"\"\n    return 0\n",
    )
    .unwrap();
    let source = "from sage.all import matrix\nmat = matrix([])\nvalue = mat.rank()\n";
    fs::write(&consumer, source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![consumer.parent().unwrap().to_path_buf(), site_packages],
        editable_roots: vec![consumer.parent().unwrap().to_path_buf()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    let prewarm_targets: Vec<_> = sage_prewarm_modules_for_source(source)
        .into_iter()
        .filter_map(|module| index.source_path_for_module(module))
        .collect();

    index.preload_paths(&prewarm_targets);
    let (line, character) = member_position(source, "rank");
    let query =
        index.query_source_at_navigation(&consumer, source, QueryPosition { line, character });

    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|docs| docs.summary.as_str()),
        Some("Return the rank of this matrix.")
    );
    assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
    fs::remove_dir_all(root).ok();
}
