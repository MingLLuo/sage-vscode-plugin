use super::*;

#[test]
fn full_rebuild_keeps_lookup_indexes_available() {
    let root = test_root("bulk-index-recreate");
    fs::write(root.join("mod.py"), "def indexed_symbol():\n    return 1\n").unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    let mut index = WorkspaceIndex::new(options);
    index.rebuild().unwrap();
    let connection = Connection::open(index.db_path()).unwrap();

    for (table, index_name) in [
        ("symbols", "idx_symbols_name"),
        ("symbols", "idx_symbols_module"),
        ("symbols", "idx_symbols_path"),
        ("docs", "idx_docs_path"),
        ("docs", "idx_docs_symbol"),
        ("sage_export_cache", "idx_sage_export_cache_path"),
        ("sage_method_cache", "idx_sage_method_cache_path"),
    ] {
        assert!(
            sqlite_index_exists(&connection, table, index_name),
            "{index_name} should exist on {table}"
        );
    }
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrate_resolves_materialized_sage_all_export_cache() {
    let root = test_root("materialized-export-cache");
    fs::create_dir_all(root.join("sage/future")).unwrap();
    fs::write(
        root.join("sage/all.py"),
        "from sage.future.all import FutureFactory\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/future/all.py"),
        "from sage.future.module import FutureFactory\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/future/module.py"),
            "def FutureFactory():\n    \"\"\"Build a future factory from cache.\"\"\"\n    return None\n",
        )
        .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import *\nvalue = FutureFactory()\n";
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
    let query =
        hydrated.query_source_symbol(&consumer, source, "FutureFactory", None, None, Vec::new());
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/future/module.py")).as_path())
    );
    assert!(query
        .resolution_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("materialized sage.all export cache")));
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Build a future factory from cache.")
    );
    let workspace_symbols = hydrated.workspace_symbols("FutureFactory", 10);
    assert_eq!(
        workspace_symbols
            .first()
            .map(|symbol| symbol.path.as_path()),
        Some(normalize_path(root.join("sage/future/module.py")).as_path())
    );
    assert!(
            workspace_symbols
                .iter()
                .all(|symbol| symbol.kind != SymbolKind::Import),
            "workspace symbols should reuse materialized exports without import noise: {workspace_symbols:?}"
        );
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrate_materializes_full_sage_export_cache_from_source() {
    let root = test_root("materialized-full-export-cache");
    fs::create_dir_all(root.join("sage/future")).unwrap();
    fs::write(root.join("sage/all.py"), "from sage.future.all import *\n").unwrap();
    let export_count = MAX_DYNAMIC_HOT_EXPORT_NAMES + 8;
    let mut all_source = String::new();
    let mut module_source = String::new();
    for index in 0..export_count {
        let name = format!("FutureFactory{index:03}");
        all_source.push_str(&format!("from sage.future.module import {name}\n"));
        module_source.push_str(&format!(
            "def {name}():\n    \"\"\"Build future factory {index}.\"\"\"\n    return {index}\n\n"
        ));
    }
    fs::write(root.join("sage/future/all.py"), all_source).unwrap();
    fs::write(root.join("sage/future/module.py"), module_source).unwrap();
    let target = format!("FutureFactory{:03}", export_count - 1);
    let consumer = root.join("consumer.py");
    let source = format!("from sage.all import {target}\nvalue = {target}()\n");
    fs::write(&consumer, &source).unwrap();
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
    let query = hydrated.query_source_symbol(&consumer, &source, &target, None, None, Vec::new());
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/future/module.py")).as_path())
    );
    assert!(query
        .resolution_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("materialized sage.all export cache")));
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrate_resolves_materialized_lazy_import_list_exports() {
    let root = test_root("materialized-lazy-list-export-cache");
    fs::create_dir_all(root.join("sage/future")).unwrap();
    fs::write(root.join("sage/all.py"), "from sage.future.all import *\n").unwrap();
    fs::write(
        root.join("sage/future/all.py"),
        "from sage.misc.lazy_import import lazy_import\n\
lazy_import('sage.future.module', ['FutureFactory', 'FutureThing'])\n\
lazy_import(\n\
    'sage.future.aliases',\n\
    ['FutureAliasSource', 'SecondAliasSource'],\n\
    as_=['FutureAlias', 'SecondAlias'],\n\
)\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/future/module.py"),
        "def FutureFactory():\n    \"\"\"Build a future factory.\"\"\"\n    return None\n\n\
def FutureThing():\n    \"\"\"Build a future thing.\"\"\"\n    return None\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/future/aliases.py"),
            "def FutureAliasSource():\n    \"\"\"Build an aliased future object.\"\"\"\n    return None\n\n\
def SecondAliasSource():\n    \"\"\"Build a second aliased future object.\"\"\"\n    return None\n",
        )
        .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import FutureThing, FutureAlias\nthing = FutureThing()\nalias = FutureAlias()\n";
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
    for (name, expected_path, expected_doc) in [
        (
            "FutureThing",
            root.join("sage/future/module.py"),
            "Build a future thing.",
        ),
        (
            "FutureAlias",
            root.join("sage/future/aliases.py"),
            "Build an aliased future object.",
        ),
    ] {
        let query = hydrated.query_source_symbol(&consumer, source, name, None, None, Vec::new());
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(expected_path).as_path()),
            "wrong materialized lazy import target for {name}: {:?}",
            query.definition
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some(expected_doc)
        );
    }
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrate_resolves_materialized_lazy_import_object_assignment() {
    let root = test_root("materialized-lazy-object-export-cache");
    fs::create_dir_all(root.join("sage/future")).unwrap();
    fs::write(root.join("sage/all.py"), "from sage.future.all import *\n").unwrap();
    fs::write(
        root.join("sage/future/all.py"),
        "from sage.misc.lazy_import import LazyImport\n\
FutureCategory = LazyImport(\n\
    'sage.future.categories',\n\
    'FutureCategory',\n\
    at_startup=True,\n\
)\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/future/categories.py"),
        "class FutureCategory:\n    \"\"\"Describe a future category.\"\"\"\n    pass\n",
    )
    .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import FutureCategory\ncategory = FutureCategory()\n";
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
    let query =
        hydrated.query_source_symbol(&consumer, source, "FutureCategory", None, None, Vec::new());
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/future/categories.py")).as_path())
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Describe a future category.")
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrate_resolves_sage_all_alias_assignments_from_indexed_imports() {
    let root = test_root("materialized-alias-export-cache");
    fs::create_dir_all(root.join("sage/rings/finite_rings")).unwrap();
    fs::write(root.join("sage/all.py"), "from sage.rings.all import *\n").unwrap();
    fs::write(
        root.join("sage/rings/all.py"),
        "from sage.rings.finite_rings.all import *\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/rings/finite_rings/all.py"),
        "from sage.rings.finite_rings.constructor import FiniteField\nGF = FiniteField\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/rings/finite_rings/constructor.py"),
            "def FiniteField(order, name=None):\n    \"\"\"Return a finite field.\"\"\"\n    return None\n",
        )
        .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import GF\nfield = GF(2)\n";
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
        Some(normalize_path(root.join("sage/rings/finite_rings/constructor.py")).as_path())
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Return a finite field.")
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrate_resolves_transitive_sage_all_star_reexports_by_module() {
    let root = test_root("materialized-star-reexport-cache");
    fs::create_dir_all(root.join("sage/future")).unwrap();
    fs::create_dir_all(root.join("sage/private")).unwrap();
    fs::write(root.join("sage/all.py"), "from sage.future.all import *\n").unwrap();
    fs::write(
        root.join("sage/future/all.py"),
        "from sage.future.module import FutureOnly\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/future/module.py"),
        "def FutureOnly():\n    \"\"\"Build a future-only public export.\"\"\"\n    return None\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/private/all.py"),
        "from sage.private.module import PrivateOnly\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/private/module.py"),
        "def PrivateOnly():\n    \"\"\"This is not exported by sage.all.\"\"\"\n    return None\n",
    )
    .unwrap();
    let public_consumer = root.join("public_consumer.py");
    let public_source = "from sage.all import FutureOnly\nvalue = FutureOnly()\n";
    fs::write(&public_consumer, public_source).unwrap();
    let private_consumer = root.join("private_consumer.py");
    let private_source = "from sage.all import PrivateOnly\nvalue = PrivateOnly()\n";
    fs::write(&private_consumer, private_source).unwrap();
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
    let public_query = hydrated.query_source_symbol(
        &public_consumer,
        public_source,
        "FutureOnly",
        None,
        None,
        Vec::new(),
    );
    assert_eq!(
        public_query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/future/module.py")).as_path())
    );
    assert!(public_query
        .resolution_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("materialized sage.all export cache")));
    let private_query = hydrated.query_source_symbol(
        &private_consumer,
        private_source,
        "PrivateOnly",
        None,
        None,
        Vec::new(),
    );
    assert_ne!(
        private_query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/private/module.py")).as_path()),
        "module-specific export cache should not treat every sage.*.all name as sage.all"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrate_resolves_star_reexports_from_plain_sage_modules() {
    let root = test_root("materialized-plain-module-star-cache");
    fs::create_dir_all(root.join("sage/categories")).unwrap();
    fs::write(
        root.join("sage/all.py"),
        "from sage.categories.all import *\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/categories/all.py"),
        "from sage.categories.basic import *\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/categories/basic.py"),
        "from sage.categories.posets import Posets\n\
OrderedSets = Posets\n\
\n\
class LocalCategory:\n\
    \"\"\"A category defined in the star-imported module.\"\"\"\n\
    pass\n\
\n\
class _PrivateCategory:\n\
    \"\"\"This private helper should not be star-exported.\"\"\"\n\
    pass\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/categories/posets.py"),
        "class Posets:\n    \"\"\"Category of posets.\"\"\"\n    pass\n",
    )
    .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import OrderedSets, LocalCategory\n\
ordered = OrderedSets()\n\
category = LocalCategory()\n";
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
    let connection = Connection::open(index.db_path()).unwrap();
    refresh_materialized_caches(&connection, &index.options().roots).unwrap();

    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    for (name, expected_path, expected_doc) in [
        (
            "OrderedSets",
            root.join("sage/categories/posets.py"),
            "Category of posets.",
        ),
        (
            "LocalCategory",
            root.join("sage/categories/basic.py"),
            "A category defined in the star-imported module.",
        ),
    ] {
        let query = hydrated.query_source_symbol(&consumer, source, name, None, None, Vec::new());
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(expected_path).as_path()),
            "wrong plain-module star export target for {name}: {:?}",
            query.definition
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some(expected_doc)
        );
    }

    let private_query = hydrated.query_source_symbol(
        &consumer,
        "from sage.all import _PrivateCategory\nvalue = _PrivateCategory()\n",
        "_PrivateCategory",
        None,
        None,
        Vec::new(),
    );
    assert_ne!(
        private_query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/categories/basic.py")).as_path()),
        "private names from a plain star-imported module must not be re-exported"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrate_respects_dunder_all_for_plain_module_star_reexports() {
    let root = test_root("materialized-plain-module-dunder-all-cache");
    fs::create_dir_all(root.join("sage/categories")).unwrap();
    fs::write(
        root.join("sage/all.py"),
        "from sage.categories.all import *\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/categories/all.py"),
        "from sage.categories.basic import *\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/categories/basic.py"),
        "from sage.categories.posets import Posets\n\
VisibleAlias = Posets\n\
__all__ = [\n\
    'VisibleAlias',\n\
]\n\
__all__.append('AppendedCategory')\n\
\n\
class AppendedCategory:\n\
    \"\"\"Category added through __all__.append.\"\"\"\n\
    pass\n\
\n\
class PublicButHidden:\n\
    \"\"\"This public-looking class is intentionally absent from __all__.\"\"\"\n\
    pass\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/categories/posets.py"),
        "class Posets:\n    \"\"\"Category of posets.\"\"\"\n    pass\n",
    )
    .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import VisibleAlias, AppendedCategory\n\
visible = VisibleAlias()\n\
appended = AppendedCategory()\n";
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
    let connection = Connection::open(index.db_path()).unwrap();
    refresh_materialized_caches(&connection, &index.options().roots).unwrap();

    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    for (name, expected_path, expected_doc) in [
        (
            "VisibleAlias",
            root.join("sage/categories/posets.py"),
            "Category of posets.",
        ),
        (
            "AppendedCategory",
            root.join("sage/categories/basic.py"),
            "Category added through __all__.append.",
        ),
    ] {
        let query = hydrated.query_source_symbol(&consumer, source, name, None, None, Vec::new());
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(expected_path).as_path()),
            "wrong __all__ star export target for {name}: {:?}",
            query.definition
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some(expected_doc)
        );
    }

    let hidden_source = "from sage.all import PublicButHidden\nvalue = PublicButHidden()\n";
    let hidden_query = hydrated.query_source_symbol(
        &consumer,
        hidden_source,
        "PublicButHidden",
        None,
        None,
        Vec::new(),
    );
    assert!(
        hidden_query.definition.is_none(),
        "__all__ should prevent fallback to a public-looking but unexported class: {:?}",
        hidden_query.definition
    );
    assert_eq!(
        hidden_query.resolution_confidence.as_deref(),
        Some("ambiguous")
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_prefers_local_symbol_over_sage_all_wildcard_export() {
    let root = test_root("local-shadow-wildcard-export");
    fs::create_dir_all(root.join("sage/matrix")).unwrap();
    fs::write(
        root.join("sage/all.py"),
        "from sage.matrix.constructor import matrix\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/matrix/constructor.py"),
        "def matrix(entries=None):\n    \"\"\"Build a Sage matrix.\"\"\"\n    return entries\n",
    )
    .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import *\n\
def matrix(value):\n\
    \"\"\"Local matrix helper.\"\"\"\n\
    return value\n\
\n\
result = matrix(1)\n";
    fs::write(&consumer, source).unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    let mut index = WorkspaceIndex::new(options);
    index.rebuild().unwrap();

    let (line, character) = position_in_line(source, "result =", "matrix");
    let query =
        index.query_source_at_navigation(&consumer, source, QueryPosition { line, character });
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
        Some("Local matrix helper.")
    );
    assert!(query
        .resolution_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("shadows Sage import/export")));
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_prefers_later_local_symbol_over_explicit_sage_import_for_usage() {
    let root = test_root("local-shadow-explicit-export");
    fs::create_dir_all(root.join("sage/matrix")).unwrap();
    fs::write(
        root.join("sage/all.py"),
        "from sage.matrix.constructor import matrix\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/matrix/constructor.py"),
        "def matrix(entries=None):\n    \"\"\"Build a Sage matrix.\"\"\"\n    return entries\n",
    )
    .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import matrix\n\
def matrix(value):\n\
    \"\"\"Local matrix helper.\"\"\"\n\
    return value\n\
\n\
result = matrix(1)\n";
    fs::write(&consumer, source).unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    let mut index = WorkspaceIndex::new(options);
    index.rebuild().unwrap();

    let (line, character) = position_in_line(source, "result =", "matrix");
    let query =
        index.query_source_at_navigation(&consumer, source, QueryPosition { line, character });
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
        Some("Local matrix helper.")
    );

    let (import_line, import_character) = first_position(source, "matrix");
    let import_query = index.query_source_at_navigation(
        &consumer,
        source,
        QueryPosition {
            line: import_line,
            character: import_character,
        },
    );
    assert_eq!(
        import_query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/matrix/constructor.py")).as_path()),
        "the import binding itself should still navigate to the Sage export"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrate_resolves_materialized_sage_method_cache() {
    let root = test_root("materialized-method-cache");
    fs::create_dir_all(root.join("sage/matrix")).unwrap();
    fs::write(
            root.join("sage/matrix/matrix0.pyx"),
            "def rank(self):\n    \"\"\"Return cached matrix rank without broad lookup.\"\"\"\n    return 0\n",
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
    let (line, character) = member_position(source, "rank");
    let query =
        hydrated.query_source_at_navigation(&consumer, source, QueryPosition { line, character });
    assert_eq!(query.owner_type.as_deref(), Some("Matrix"));
    assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/matrix/matrix0.pyx")).as_path())
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Return cached matrix rank without broad lookup.")
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrate_resolves_source_derived_matrix_constructor_methods() {
    let root = test_root("materialized-matrix-constructor-method-cache");
    fs::create_dir_all(root.join("sage/matrix")).unwrap();
    fs::write(
        root.join("sage/matrix/special.py"),
        "from sage.matrix.constructor import matrix\n\
def matrix_method(func=None, name=None):\n\
    return func\n\
\n\
@matrix_method\n\
def random_matrix(ring, nrows, ncols=None):\n\
    \"\"\"Return a random matrix from a matrix constructor method.\"\"\"\n\
    return matrix([])\n\
\n\
@matrix_method(name='unit')\n\
def identity_matrix(ring, n=0):\n\
    \"\"\"Return an identity matrix from an explicit alias.\"\"\"\n\
    return matrix([])\n",
    )
    .unwrap();
    fs::write(
        root.join("sage/matrix/constructor.py"),
        "def matrix(entries=None):\n    \"\"\"Build a matrix.\"\"\"\n    return entries\n",
    )
    .unwrap();
    let consumer = root.join("consumer.sage");
    let source = "A = matrix.random(GF(2), 3)\nB = matrix.unit(GF(2), 3)\n";
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
    for (needle, expected_doc) in [
        (
            "random",
            "Return a random matrix from a matrix constructor method.",
        ),
        ("unit", "Return an identity matrix from an explicit alias."),
    ] {
        let (line, character) = member_position(source, needle);
        let query = hydrated.query_source_at_navigation(
            &consumer,
            source,
            QueryPosition { line, character },
        );
        assert_eq!(query.owner_type.as_deref(), Some("MatrixConstructor"));
        assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
        assert_eq!(
            query
                .definition
                .as_ref()
                .map(|definition| definition.path.as_path()),
            Some(normalize_path(root.join("sage/matrix/special.py")).as_path())
        );
        assert_eq!(
            query
                .documentation
                .as_ref()
                .map(|documentation| documentation.summary.as_str()),
            Some(expected_doc)
        );
    }
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrate_resolves_source_derived_sage_methods_without_static_spec() {
    let root = test_root("source-derived-method-cache");
    fs::create_dir_all(root.join("sage/graphs")).unwrap();
    fs::write(
            root.join("sage/graphs/generic_graph.py"),
            "class GenericGraph:\n    def chromatic_polynomial(self, algorithm=None):\n        \"\"\"Return the graph chromatic polynomial.\"\"\"\n        return None\n",
        )
        .unwrap();
    let consumer = root.join("consumer.py");
    let source =
        "from sage.all import Graph\nG = Graph()\npoly = G.chromatic_polynomial()\nG.chroma\n";
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
    let rebuilt_status = index.status();
    assert_eq!(
        rebuilt_status.source_derived_method_cache_count, 1,
        "new Sage methods should be counted as source-derived cache rows"
    );
    assert_eq!(rebuilt_status.static_method_cache_count, 0);

    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    let (line, character) = member_position(source, "chromatic_polynomial");
    let query =
        hydrated.query_source_at_navigation(&consumer, source, QueryPosition { line, character });
    assert_eq!(query.owner_type.as_deref(), Some("Graph"));
    assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/graphs/generic_graph.py")).as_path())
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Return the graph chromatic polynomial.")
    );

    let (completion_line, completion_character) = first_position(source, "G.chroma");
    let completion_position = QueryPosition {
        line: completion_line,
        character: completion_character + "G.chroma".len() as u32,
    };
    let labels: Vec<_> = hydrated
        .completion_items_at_source(source, completion_position, 20)
        .into_iter()
        .map(|item| item.label)
        .collect();
    assert!(
        labels.contains(&"chromatic_polynomial".to_string()),
        "source-derived method completion missing: {labels:?}"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrate_resolves_source_derived_class_method_aliases() {
    let root = test_root("source-derived-method-alias-cache");
    fs::create_dir_all(root.join("sage/matrix")).unwrap();
    fs::write(
            root.join("sage/matrix/future.py"),
            "class MatrixFuture:\n    def trace_impl(self, algorithm=None):\n        \"\"\"Return aliased matrix trace docs.\"\"\"\n        return None\n\n    trace_alias = trace_impl\n\n    def helper(self):\n        hidden_alias = trace_impl\n        return hidden_alias()\n",
        )
        .unwrap();
    let consumer = root.join("consumer.py");
    let source =
        "from sage.all import matrix\nmat = matrix([])\nvalue = mat.trace_alias()\nmat.trace_\n";
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
    let (line, character) = member_position(source, "trace_alias");
    let query =
        hydrated.query_source_at_navigation(&consumer, source, QueryPosition { line, character });
    assert_eq!(query.owner_type.as_deref(), Some("Matrix"));
    assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.name.as_str()),
        Some("trace_impl")
    );
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/matrix/future.py")).as_path())
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|documentation| documentation.summary.as_str()),
        Some("Return aliased matrix trace docs.")
    );

    let (completion_line, completion_character) = first_position(source, "mat.trace_");
    let completion_position = QueryPosition {
        line: completion_line,
        character: completion_character + "mat.trace_".len() as u32,
    };
    let completions = hydrated.completion_items_at_source(source, completion_position, 20);
    assert!(
            completions
                .iter()
                .any(|item| item.label == "trace_alias"
                    && item.detail.contains("alias for trace_impl")),
            "source-derived method alias completion missing: {completions:?}"
        );
    assert!(
        completions.iter().all(|item| item.label != "hidden_alias"),
        "function-local aliases must not enter the method cache: {completions:?}"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrate_does_not_cross_classify_sage_classes_outside_owner_modules() {
    assert_eq!(
        sage_owner_type_from_class_name("GenericGraph", "sage.graphs.generic_graph"),
        Some(SageOwnerType::Graph)
    );
    assert_eq!(
        sage_owner_type_from_class_name(
            "EllipticCurve_generic",
            "sage.schemes.elliptic_curves.ell_generic"
        ),
        Some(SageOwnerType::EllipticCurve)
    );
    assert_eq!(
        sage_owner_type_from_class_name(
            "NumberField_generic",
            "sage.rings.number_field.number_field"
        ),
        Some(SageOwnerType::NumberField)
    );
    assert_eq!(
        sage_owner_type_from_class_name("Graphics", "sage.plot.graphics"),
        None
    );
    assert_eq!(
        sage_owner_type_from_class_name(
            "HyperellipticCurve_generic",
            "sage.schemes.hyperelliptic_curves.hyperelliptic_generic"
        ),
        None
    );
    assert_eq!(
        sage_owner_type_from_class_name("FutureGraphAlgorithms", "sage.future.graph_algorithms"),
        None
    );

    let root = test_root("class-derived-method-cache");
    fs::create_dir_all(root.join("sage/future")).unwrap();
    fs::write(
            root.join("sage/future/graph_algorithms.py"),
            "class FutureGraphAlgorithms:\n    def experimental_walks(self, limit=None):\n        \"\"\"Return experimental graph walks.\"\"\"\n        return []\n",
        )
        .unwrap();
    let consumer = root.join("consumer.py");
    let source = "from sage.all import Graph\nG = Graph()\nwalks = G.experimental_walks()\nG.experimental_\n";
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

    let parsed = parse_source(
        "sage.future.graph_algorithms",
        &root.join("sage/future/graph_algorithms.py"),
        &fs::read_to_string(root.join("sage/future/graph_algorithms.py")).unwrap(),
    );
    assert!(parsed.symbols.iter().any(|symbol| {
        symbol.name == "experimental_walks"
            && symbol.detail == "Method FutureGraphAlgorithms.experimental_walks"
    }));

    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    let (line, character) = member_position(source, "experimental_walks");
    let query =
        hydrated.query_source_at_navigation(&consumer, source, QueryPosition { line, character });
    assert_eq!(query.owner_type.as_deref(), Some("Graph"));
    assert_ne!(query.resolution_confidence.as_deref(), Some("high"));
    assert_ne!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(root.join("sage/future/graph_algorithms.py")).as_path())
    );

    let (completion_line, completion_character) = first_position(source, "G.experimental_");
    let completion_position = QueryPosition {
        line: completion_line,
        character: completion_character + "G.experimental_".len() as u32,
    };
    let labels: Vec<_> = hydrated
        .completion_items_at_source(source, completion_position, 20)
        .into_iter()
        .map(|item| item.label)
        .collect();
    assert!(
        labels.iter().all(|label| label != "experimental_walks"),
        "unrelated Sage classes must not pollute Graph completions: {labels:?}"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn connection_refresh_keeps_polyhedron_method_cache_class_scoped() {
    let root = test_root("connection-polyhedron-method-cache");
    let polyhedron_root = root.join("sage/geometry/polyhedron");
    fs::create_dir_all(&polyhedron_root).unwrap();
    fs::write(
        polyhedron_root.join("base0.py"),
        "class Polyhedron_base0:\n    def vertices(self):\n        \"\"\"Return the polyhedron vertices.\"\"\"\n        return ()\n",
    )
    .unwrap();
    fs::write(
        polyhedron_root.join("face.py"),
        "class PolyhedronFace:\n    def as_polyhedron(self):\n        \"\"\"Convert this face, not a polyhedron instance.\"\"\"\n        return None\n",
    )
    .unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    let mut index = WorkspaceIndex::new(options);
    index.rebuild().unwrap();

    let connection = Connection::open(index.db_path()).unwrap();
    refresh_materialized_caches(&connection, &index.options().roots).unwrap();

    let vertices = load_materialized_sage_method_from_db(
        index.db_path(),
        SageOwnerType::Polyhedron,
        "vertices",
        &index.options().roots,
    )
    .unwrap();
    assert_eq!(
        vertices.as_ref().map(|symbol| symbol.path.as_path()),
        Some(normalize_path(polyhedron_root.join("base0.py")).as_path()),
        "connection refresh should retain methods from Polyhedron implementation classes"
    );
    let face_method = load_materialized_sage_method_from_db(
        index.db_path(),
        SageOwnerType::Polyhedron,
        "as_polyhedron",
        &index.options().roots,
    )
    .unwrap();
    assert!(
        face_method.is_none(),
        "PolyhedronFace methods must not enter the Polyhedron method cache: {face_method:?}"
    );

    fs::remove_dir_all(root).ok();
}
