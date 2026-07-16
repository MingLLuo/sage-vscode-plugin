use super::*;

#[test]
fn completion_items_are_owner_aware_for_sage_member_access() {
    let root = test_root("owner-aware-completion");
    fs::create_dir_all(root.join("sage/matrix")).unwrap();
    fs::write(
        root.join("sage/matrix/matrix0.pyx"),
        "def rank(self):\n    \"\"\"Return matrix rank.\"\"\"\n    return 0\n",
    )
    .unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();
    let source = "def demo(A, field, value, signature):\n    A.ra\n    field.o\n    value.integer_\n    signature.ba\n    text = 'A.ra'\n";

    let labels_at = |needle: &str| -> Vec<String> {
        let (line, character) = first_position(source, needle);
        index
            .completion_items_at_source(
                source,
                QueryPosition {
                    line,
                    character: character + needle.len() as u32,
                },
                20,
            )
            .into_iter()
            .map(|completion| completion.label)
            .collect()
    };

    let matrix_labels = labels_at("A.ra");
    assert!(
        matrix_labels.contains(&"rank".to_string()),
        "{matrix_labels:?}"
    );
    assert!(
        matrix_labels
            .iter()
            .all(|label| label.starts_with("ra") || label.starts_with("r")),
        "{matrix_labels:?}"
    );
    let (rank_line, rank_character) = first_position(source, "A.ra");
    let rank_completion = index
        .completion_items_at_source(
            source,
            QueryPosition {
                line: rank_line,
                character: rank_character + "A.ra".len() as u32,
            },
            20,
        )
        .into_iter()
        .find(|completion| completion.label == "rank")
        .expect("rank completion should exist");
    assert_eq!(rank_completion.signature.as_deref(), Some("rank(self)"));
    assert!(
        rank_completion
            .documentation
            .as_deref()
            .is_some_and(|docs| docs.contains("Return matrix rank.")),
        "{rank_completion:?}"
    );

    let field_labels = labels_at("field.o");
    assert!(
        field_labels.contains(&"order".to_string()),
        "{field_labels:?}"
    );

    let field_element_labels = labels_at("value.integer_");
    assert!(
        field_element_labels.contains(&"integer_representation".to_string()),
        "{field_element_labels:?}"
    );

    let vector_labels = labels_at("signature.ba");
    assert!(
        vector_labels.contains(&"base_ring".to_string()),
        "{vector_labels:?}"
    );

    let (line, character) = first_position(source, "'A.ra");
    let string_labels = index.completion_items_at_source(
        source,
        QueryPosition {
            line,
            character: character + "'A.ra".len() as u32,
        },
        20,
    );
    assert!(
        string_labels.is_empty(),
        "string literal member completions should be suppressed: {string_labels:?}"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn completion_items_are_local_first_for_open_documents() {
    let root = test_root("local-first-completion");
    fs::write(
        root.join("indexed.py"),
        "def kernel_archive():\n    pass\n\ndef scratch_global():\n    pass\n",
    )
    .unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    let source = "from sage.all import matrix\n\n\
def kernel_columns(A):\n    \"\"\"Return a basis matrix for the right kernel.\"\"\"\n    return A.right_kernel().basis_matrix()\n\n\
def demo(A, field):\n    scratch_matrix = matrix([])\n    ker\n    scra\n    fi\n";
    let (kernel_line, kernel_character) = first_position(source, "    ker");
    let kernel_completions = index.completion_items_at_source(
        source,
        QueryPosition {
            line: kernel_line,
            character: kernel_character + "    ker".len() as u32,
        },
        20,
    );
    let kernel_labels: Vec<_> = kernel_completions
        .iter()
        .map(|completion| completion.label.clone())
        .collect();
    assert_eq!(
        kernel_labels.first().map(String::as_str),
        Some("kernel_columns")
    );
    let kernel_completion = kernel_completions
        .first()
        .expect("kernel_columns completion should exist");
    assert_eq!(
        kernel_completion.signature.as_deref(),
        Some("kernel_columns(A)")
    );
    assert!(
        kernel_completion
            .documentation
            .as_deref()
            .is_some_and(|docs| docs.contains("right kernel")),
        "{kernel_completion:?}"
    );
    assert!(
        kernel_labels.contains(&"kernel_archive".to_string()),
        "{kernel_labels:?}"
    );

    let (scratch_line, scratch_character) = first_position(source, "    scra");
    let scratch_labels: Vec<_> = index
        .completion_items_at_source(
            source,
            QueryPosition {
                line: scratch_line,
                character: scratch_character + "    scra".len() as u32,
            },
            20,
        )
        .into_iter()
        .map(|completion| completion.label)
        .collect();
    assert_eq!(
        scratch_labels.first().map(String::as_str),
        Some("scratch_matrix"),
        "{scratch_labels:?}"
    );

    let (field_line, field_character) = first_position(source, "    fi");
    let parameter_labels: Vec<_> = index
        .completion_items_at_source(
            source,
            QueryPosition {
                line: field_line,
                character: field_character + "    fi".len() as u32,
            },
            20,
        )
        .into_iter()
        .map(|completion| completion.label)
        .collect();
    assert!(
        parameter_labels.contains(&"field".to_string()),
        "{parameter_labels:?}"
    );

    let multiline_source = "def collect_spectral_kernel(\n    Qs,\n    Q0inv,\n    n,\n    max_candidates=None,\n    search_seed=1,\n):\n    max_\n    search_\n";
    let body_position = |line_text: &str, prefix: &str| -> QueryPosition {
        let (line, character) = multiline_source
            .lines()
            .enumerate()
            .find_map(|(line, line_source)| {
                if line_source == line_text {
                    Some((line as u32, line_source.find(prefix).unwrap() as u32))
                } else {
                    None
                }
            })
            .unwrap();
        QueryPosition {
            line,
            character: character + prefix.len() as u32,
        }
    };
    let multiline_parameter_labels: Vec<_> = index
        .completion_items_at_source(multiline_source, body_position("    max_", "max_"), 20)
        .into_iter()
        .map(|completion| completion.label)
        .collect();
    assert!(
        multiline_parameter_labels.contains(&"max_candidates".to_string()),
        "{multiline_parameter_labels:?}"
    );
    let second_parameter_labels: Vec<_> = index
        .completion_items_at_source(
            multiline_source,
            body_position("    search_", "search_"),
            20,
        )
        .into_iter()
        .map(|completion| completion.label)
        .collect();
    assert!(
        second_parameter_labels.contains(&"search_seed".to_string()),
        "{second_parameter_labels:?}"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_lazy_import_alias_to_source_definition() {
    let root = test_root("lazy-query");
    let consumer = root.join("consumer.sage");
    let provider = root.join("external_series.py");
    fs::write(
            &consumer,
            "def lazy_import(module, names, as_=None, *, at_startup=False):\n    pass\n\nlazy_import('external_series', 'alternating_square_sum', 'alt_square_sum')\nvalue = alt_square_sum(5)\n",
        )
        .unwrap();
    fs::write(
            &provider,
            "def alternating_square_sum(n):\n    \"\"\"Return an alternating square sum.\"\"\"\n    return n\n",
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
    let query =
        index.query_source_symbol(&consumer, &source, "alt_square_sum", None, None, Vec::new());

    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.name.as_str()),
        Some("alternating_square_sum")
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|docs| docs.summary.as_str()),
        Some("Return an alternating square sum.")
    );
    assert_eq!(
        query
            .signature
            .as_ref()
            .map(|signature| signature.label.as_str()),
        Some("alternating_square_sum(n)")
    );
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(provider.clone()).as_path())
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_deprecated_function_alias_to_replacement() {
    let root = test_root("deprecated-alias-query");
    let source_path = root.join("consumer.sage");
    let source = "from sage.misc.superseded import deprecated_function_alias\n\n\
def replacement(n):\n    \"\"\"Return the replacement value.\"\"\"\n    return n\n\n\
old_replacement = deprecated_function_alias(12345, replacement)\nvalue = old_replacement(5)\n";
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
        "old_replacement",
        None,
        None,
        Vec::new(),
    );

    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.name.as_str()),
        Some("replacement")
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|docs| docs.summary.as_str()),
        Some("Return the replacement value.")
    );
    assert_eq!(
        query
            .signature
            .as_ref()
            .map(|signature| signature.label.as_str()),
        Some("replacement(n)")
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_import_member_alias_to_source_definition() {
    let root = test_root("member-alias-query");
    let provider_dir = root.join("sage/future");
    fs::create_dir_all(&provider_dir).unwrap();
    let all_path = provider_dir.join("all.py");
    let provider_path = provider_dir.join("module.py");
    let consumer_path = root.join("consumer.sage");
    fs::write(
        &all_path,
        "import sage.future.module as future_module\nFutureAlias = future_module.FutureFactory\n",
    )
    .unwrap();
    fs::write(
        &provider_path,
        "class FutureFactory:\n    \"\"\"Build a source-owned future factory.\"\"\"\n    pass\n",
    )
    .unwrap();
    fs::write(
        &consumer_path,
        "from sage.future.all import FutureAlias\nvalue = FutureAlias()\n",
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
    let source = fs::read_to_string(&consumer_path).unwrap();

    let query = index.query_source_symbol(
        &consumer_path,
        &source,
        "FutureAlias",
        None,
        None,
        Vec::new(),
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
            .map(|docs| docs.summary.as_str()),
        Some("Build a source-owned future factory.")
    );
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(normalize_path(provider_path.clone()).as_path())
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn query_resolves_local_definition_alias_to_source_definition() {
    let root = test_root("local-definition-alias-query");
    let all_path = root.join("module.py");
    let consumer_path = root.join("consumer.sage");
    fs::write(
            &all_path,
            "class Replacement:\n    \"\"\"Replacement class docs.\"\"\"\n    pass\n\nAlias = Replacement\n",
        )
        .unwrap();
    fs::write(
        &consumer_path,
        "from module import Alias\nvalue = Alias()\n",
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
    let source = fs::read_to_string(&consumer_path).unwrap();

    let query = index.query_source_symbol(&consumer_path, &source, "Alias", None, None, Vec::new());

    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.name.as_str()),
        Some("Replacement")
    );
    assert_eq!(
        query
            .documentation
            .as_ref()
            .map(|docs| docs.summary.as_str()),
        Some("Replacement class docs.")
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn ambiguous_dotted_member_returns_explainable_docs_without_wrong_definition() {
    let root = test_root("ambiguous-member-docs");
    fs::create_dir_all(root.join("sage/categories")).unwrap();
    fs::write(
        root.join("sage/factory.py"),
        "def WeylGroup(data=None):\n    \"\"\"Build a Weyl group.\"\"\"\n    return object()\n",
    )
    .unwrap();
    fs::write(
            root.join("sage/categories/coxeter.py"),
            "class ParentMethods:\n    def simple_reflections(self):\n        \"\"\"Return Coxeter simple reflections.\"\"\"\n        return {}\n",
        )
        .unwrap();
    fs::write(
            root.join("sage/other.py"),
            "class Other:\n    def simple_reflections(self):\n        \"\"\"Return another simple-reflection implementation.\"\"\"\n        return {}\n",
        )
        .unwrap();
    let consumer = root.join("consumer.py");
    let source =
        "from sage.factory import WeylGroup\nW = WeylGroup(['A', 2])\ns = W.simple_reflections()\n";
    fs::write(&consumer, source).unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();

    let (line, character) = member_position(source, "simple_reflections");
    let query =
        index.query_source_at_navigation(&consumer, source, QueryPosition { line, character });
    assert!(
        query.definition.is_none(),
        "ambiguous member resolution must not jump to an arbitrary candidate"
    );
    assert_eq!(query.resolution_confidence.as_deref(), Some("ambiguous"));
    assert_eq!(query.candidate_count, 2);
    assert!(query
        .fallback_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("no exact owner match")));
    let docs = query
        .documentation
        .as_ref()
        .expect("ambiguous member should provide explainable documentation");
    assert_eq!(docs.kind, "AmbiguousMember");
    assert!(docs.summary.contains("multiple-definition preview"));
    assert!(docs.markers.iter().any(|marker| marker == "ambiguous"));
    assert_eq!(docs.sections.len(), 2);
    assert!(query
        .hover
        .as_ref()
        .is_some_and(|hover| hover.markdown.contains("Top indexed candidates")));
    fs::remove_dir_all(root).ok();
}
