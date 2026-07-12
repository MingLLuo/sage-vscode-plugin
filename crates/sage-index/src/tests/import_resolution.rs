use super::*;

#[test]
fn symbol_lookup_prefers_definitions_over_imports() {
    let path = PathBuf::from("demo.py");
    let import = SymbolRecord {
        name: "target".to_string(),
        kind: SymbolKind::Import,
        module: "consumer".to_string(),
        path: path.clone(),
        range: SourceRange {
            start_line: 0,
            start_character: 0,
            end_line: 0,
            end_character: 6,
        },
        detail: "Import target".to_string(),
        docstring: None,
        import_from: Some("provider".to_string()),
        signature: None,
    };
    let definition = SymbolRecord {
        name: "target".to_string(),
        kind: SymbolKind::Function,
        module: "provider".to_string(),
        path,
        range: SourceRange {
            start_line: 3,
            start_character: 4,
            end_line: 3,
            end_character: 10,
        },
        detail: "Function target".to_string(),
        docstring: Some("Definition docs.".to_string()),
        import_from: None,
        signature: Some("target()".to_string()),
    };
    let mut index = WorkspaceIndex::default();
    index
        .symbols_by_name
        .insert("target".to_string(), vec![import, definition]);

    let symbol = index.symbol("target").expect("symbol should resolve");
    assert_eq!(symbol.kind, SymbolKind::Function);
    assert_eq!(symbol.module, "provider");
}

#[test]
fn import_resolution_prefers_import_source_module() {
    let root = test_root("import-resolution");
    let consumer = root.join("consumer.sage");
    let provider = root.join("provider.py");
    fs::write(&consumer, "from provider import target\nvalue = target()\n").unwrap();
    fs::write(
        &provider,
        "def target():\n    \"\"\"Provider docs.\"\"\"\n    return 1\n",
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
    let resolved = index
        .resolve_symbol("target", Some("consumer"))
        .expect("target should resolve");
    assert_eq!(resolved.module, "provider");
    assert_eq!(resolved.docstring.as_deref(), Some("Provider docs."));
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_explicit_import_target_before_full_index() {
    let root = test_root("cold-explicit-import-target");
    fs::create_dir_all(root.join("sage/future")).unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.future.module import FutureFactory as Future\nvalue = Future()\n";
    fs::write(&consumer, source).unwrap();
    fs::write(
        root.join("sage/future/module.py"),
        "def FutureFactory():\n    \"\"\"Build a future factory.\"\"\"\n    return None\n",
    )
    .unwrap();
    let index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });

    let query = index.query_source_symbol(&consumer, source, "Future", None, None, Vec::new());
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/future/module.py")).as_path())
    );
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.name.as_str()),
        Some("FutureFactory")
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Build a future factory.")
    );
    assert_eq!(
        query.resolution_reason.as_deref(),
        Some("resolved `Future` from explicit import target sage.future.module")
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_relative_import_target_before_full_index() {
    let root = test_root("cold-relative-import-target");
    let package = root.join("sage/future");
    fs::create_dir_all(&package).unwrap();
    let consumer = package.join("consumer.py");
    let source = "from .module import Feature\nvalue = Feature()\n";
    fs::write(&consumer, source).unwrap();
    fs::write(
        package.join("module.py"),
        "def Feature():\n    \"\"\"Build a relative import target.\"\"\"\n    return None\n",
    )
    .unwrap();
    let index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });

    let query = index.query_source_symbol(&consumer, source, "Feature", None, None, Vec::new());
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(package.join("module.py")).as_path())
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Build a relative import target.")
    );
    assert_eq!(
        query.resolution_reason.as_deref(),
        Some("resolved `Feature` from explicit import target sage.future.module")
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_sage_all_fallback_target_before_full_index() {
    let root = test_root("cold-sage-all-fallback-target");
    fs::create_dir_all(root.join("sage/rings")).unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import QQ\nbase = QQ\n";
    fs::write(&consumer, source).unwrap();
    fs::write(
        root.join("sage/rings/rational_field.py"),
        "def RationalField():\n    return object()\n\nQQ = RationalField()\n",
    )
    .unwrap();
    let index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });

    let query = index.query_source_symbol(&consumer, source, "QQ", None, None, Vec::new());
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/rings/rational_field.py")).as_path())
    );
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.name.as_str()),
        Some("QQ")
    );
    assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
    assert!(
        query
            .resolution_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("built-in sage.all export fallback")),
        "{query:?}"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_prefers_source_sage_all_exports_over_static_fallback_before_full_index() {
    let root = test_root("cold-sage-all-source-before-fallback");
    fs::create_dir_all(root.join("sage/future")).unwrap();
    fs::create_dir_all(root.join("sage/rings/finite_rings")).unwrap();
    fs::write(
        root.join("sage/all.py"),
        "from sage.future.factory import GF\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/future/factory.py"),
            "def GF(*args):\n    \"\"\"Construct the moved finite field factory.\"\"\"\n    return args\n",
        )
        .unwrap();
    fs::write(
        root.join("sage/rings/finite_rings/finite_field_constructor.py"),
        "def GF(*args):\n    \"\"\"Outdated static fallback target.\"\"\"\n    return args\n",
    )
    .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import GF\nfield = GF(7)\n";
    fs::write(&consumer, source).unwrap();
    let index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });

    let query = index.query_source_symbol(&consumer, source, "GF", None, None, Vec::new());

    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/future/factory.py")).as_path())
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Construct the moved finite field factory.")
    );
    assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
    assert!(
        query
            .resolution_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("source-derived sage.all export chain")),
        "{query:?}"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrate_prefers_source_sage_all_exports_over_static_fallback_cache_rows() {
    let root = test_root("hydrate-sage-all-source-before-fallback");
    fs::create_dir_all(root.join("sage/future")).unwrap();
    fs::create_dir_all(root.join("sage/rings/finite_rings")).unwrap();
    fs::write(
        root.join("sage/all.py"),
        "from sage.future.factory import GF\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/future/factory.py"),
            "def GF(*args):\n    \"\"\"Construct the moved finite field factory.\"\"\"\n    return args\n",
        )
        .unwrap();
    fs::write(
        root.join("sage/rings/finite_rings/finite_field_constructor.py"),
        "def GF(*args):\n    \"\"\"Outdated static fallback target.\"\"\"\n    return args\n",
    )
    .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import GF\nfield = GF(7)\n";
    fs::write(&consumer, source).unwrap();
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
    let query = hydrated.query_source_symbol(&consumer, source, "GF", None, None, Vec::new());

    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/future/factory.py")).as_path())
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Construct the moved finite field factory.")
    );
    assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrate_prefers_source_sage_all_star_reexports_over_static_fallback_cache_rows() {
    let root = test_root("hydrate-sage-all-star-before-fallback");
    fs::create_dir_all(root.join("sage/future")).unwrap();
    fs::create_dir_all(root.join("sage/rings/finite_rings")).unwrap();
    fs::write(root.join("sage/all.py"), "from sage.future.all import *\n").unwrap();
    fs::write(
        root.join("sage/future/all.py"),
        "__all__ = [\"GF\"]\nfrom sage.future.factory import GF\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/future/factory.py"),
            "def GF(*args):\n    \"\"\"Construct the star-exported finite field factory.\"\"\"\n    return args\n",
        )
        .unwrap();
    fs::write(
        root.join("sage/rings/finite_rings/finite_field_constructor.py"),
        "def GF(*args):\n    \"\"\"Outdated static fallback target.\"\"\"\n    return args\n",
    )
    .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import GF\nfield = GF(7)\n";
    fs::write(&consumer, source).unwrap();
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
    let query = hydrated.query_source_symbol(&consumer, source, "GF", None, None, Vec::new());

    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/future/factory.py")).as_path())
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Construct the star-exported finite field factory.")
    );
    assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_sage_all_source_import_before_full_index_without_fallback_map() {
    let root = test_root("cold-sage-all-source-import");
    fs::create_dir_all(root.join("sage/future")).unwrap();
    fs::write(
        root.join("sage/all.py"),
        "from sage.future.factory import NewConstructor\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/future/factory.py"),
            "def NewConstructor():\n    \"\"\"Construct a source-derived object.\"\"\"\n    return None\n",
        )
        .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import NewConstructor\nvalue = NewConstructor()\n";
    fs::write(&consumer, source).unwrap();
    let index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });

    let query =
        index.query_source_symbol(&consumer, source, "NewConstructor", None, None, Vec::new());
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/future/factory.py")).as_path())
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Construct a source-derived object.")
    );
    assert!(
        query
            .resolution_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("source-derived sage.all export chain")),
        "{query:?}"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_sage_all_star_reexport_before_full_index_without_fallback_map() {
    let root = test_root("cold-sage-all-star-reexport");
    fs::create_dir_all(root.join("sage/future")).unwrap();
    fs::write(root.join("sage/all.py"), "from sage.future.all import *\n").unwrap();
    fs::write(
        root.join("sage/future/all.py"),
        "__all__ = [\"NewFactory\"]\nfrom sage.future.factory import NewFactory\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/future/factory.py"),
        "def NewFactory():\n    \"\"\"Build a star re-exported object.\"\"\"\n    return None\n",
    )
    .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import NewFactory\nvalue = NewFactory()\n";
    fs::write(&consumer, source).unwrap();
    let index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });

    let query = index.query_source_symbol(&consumer, source, "NewFactory", None, None, Vec::new());
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/future/factory.py")).as_path())
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Build a star re-exported object.")
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_implicit_sage_all_name_in_sage_file_before_global_lookup() {
    let root = test_root("implicit-sage-all-name");
    fs::create_dir_all(root.join("sage/combinat")).unwrap();
    fs::write(
        root.join("sage/all.py"),
        "from sage.combinat.all import *\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/combinat/all.py"),
        "__all__ = [\"Combinations\"]\nfrom sage.combinat.combination import Combinations\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/combinat/combination.py"),
            "def Combinations(mset, k=None):\n    \"\"\"Return combinations quickly.\"\"\"\n    return None\n",
        )
        .unwrap();
    let consumer = root.join("consumer.sage");
    let source = "C2_list = Combinations(n, 2).list()\n";
    fs::write(&consumer, source).unwrap();
    let index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });

    let query =
        index.query_source_symbol(&consumer, source, "Combinations", None, None, Vec::new());

    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/combinat/combination.py")).as_path())
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Return combinations quickly.")
    );
    assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
    assert!(
        query.resolution_reason.as_deref().is_some_and(
            |reason| reason.contains("implicit .sage source-derived sage.all export chain")
        ),
        "{query:?}"
    );
    assert_eq!(query.candidate_count, 1);
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_keeps_local_shadow_before_implicit_sage_all_name() {
    let root = test_root("implicit-sage-all-local-shadow");
    fs::create_dir_all(root.join("sage/combinat")).unwrap();
    fs::write(
        root.join("sage/all.py"),
        "from sage.combinat.all import *\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/combinat/all.py"),
        "__all__ = [\"Combinations\"]\nfrom sage.combinat.combination import Combinations\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/combinat/combination.py"),
        "def Combinations(mset, k=None):\n    return None\n",
    )
    .unwrap();
    let consumer = root.join("consumer.sage");
    let source = "def Combinations():\n    \"\"\"Local helper.\"\"\"\n    return []\n\nvalue = Combinations()\n";
    fs::write(&consumer, source).unwrap();
    let index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    let (line, character) = position_in_line(source, "value =", "Combinations");

    let query = index.query_source_at(&consumer, source, QueryPosition { line, character }, None);

    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(consumer.clone()).as_path())
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Local helper.")
    );
    assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
    assert!(
        query
            .resolution_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("shadows Sage import/export")),
        "{query:?}"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_symbols_from_sage_load_spyx_targets() {
    let root = test_root("sage-load-spyx-symbols");
    fs::create_dir_all(root.join("workspace")).unwrap();
    let loaded = root.join("workspace/fast_rank1.spyx");
    fs::write(
            &loaded,
            "def find_rank1_matrices(A, K, int n):\n    \"\"\"Find rank-one matrices in a Cython helper.\"\"\"\n    return []\n",
        )
        .unwrap();
    let consumer = root.join("workspace/full_attack.sage");
    let source = "load(\"fast_rank1.spyx\")\nrank1 = find_rank1_matrices(PK, K, n)\n";
    fs::write(&consumer, source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.join("workspace")],
        editable_roots: vec![root.join("workspace")],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    let (line, character) = position_in_line(source, "rank1 =", "find_rank1_matrices");
    let query =
        index.query_source_at_navigation(&consumer, source, QueryPosition { line, character });

    assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
    assert!(
        query
            .resolution_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("load/attach target")),
        "{query:?}"
    );
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(loaded).as_path())
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Find rank-one matrices in a Cython helper.")
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_prefers_documented_python_constructor_over_pxd_declaration() {
    let root = test_root("python-constructor-over-pxd");
    let package = root.join("sage/rings/number_field");
    fs::create_dir_all(&package).unwrap();
    let declaration = package.join("number_field_base.pxd");
    let cython_base = package.join("number_field_base.pyx");
    let implementation = package.join("number_field.py");
    let consumer = root.join("consumer.sage");
    fs::write(
        &declaration,
        "from sage.rings.ring cimport Field\n\ncdef class NumberField(Field):\n    pass\n",
    )
    .unwrap();
    fs::write(
            &cython_base,
            "from sage.rings.ring cimport Field\n\ncdef class NumberField(Field):\n    \"\"\"Base class docs.\"\"\"\n    pass\n",
        )
        .unwrap();
    fs::write(
            &implementation,
            "def NumberField(polynomial, name=None,\n                check=True, names=None):\n    \"\"\"Return the readable number field constructor.\"\"\"\n    return polynomial\n",
        )
        .unwrap();
    let source = "K = NumberField(poly, \"a\")\n";
    fs::write(&consumer, source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    let query = index.query_source_symbol(&consumer, source, "NumberField", None, None, Vec::new());

    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(implementation.clone()).as_path())
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Return the readable number field constructor.")
    );
    fs::remove_dir_all(root).ok();
}
