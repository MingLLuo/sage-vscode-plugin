use super::*;
use crate::call_hierarchy::call_ranges_in_range;

#[test]
fn background_index_results_require_latest_job_and_unchanged_index() {
    assert!(index_job_result_is_current(2, 2, 7, 7));
    assert!(!index_job_result_is_current(3, 2, 7, 7));
    assert!(!index_job_result_is_current(2, 2, 8, 7));
}

#[test]
fn navigation_cache_entries_are_scoped_to_index_generation() {
    let mut cache = NavigationQueryCache::default();
    let base = NavigationQueryCacheKey {
        uri: "file:///workspace/demo.sage".to_string(),
        version: 1,
        content_fingerprint: None,
        line: 2,
        character: 4,
        index_generation: 7,
    };
    cache.insert(base.clone(), QueryResult::default());

    assert!(cache.get(&base).is_some());
    assert!(cache
        .get(&NavigationQueryCacheKey {
            index_generation: 8,
            ..base
        })
        .is_none());
}

#[test]
fn on_disk_navigation_cache_identity_tracks_unicode_source_content() {
    assert_ne!(
        source_text_fingerprint("value = π\n"),
        source_text_fingerprint("value = 🚀\n")
    );
    assert_eq!(
        source_text_fingerprint("value = 🚀\n"),
        source_text_fingerprint("value = 🚀\n")
    );
}

#[test]
fn navigation_link_support_is_negotiated_per_request_kind() {
    let capabilities: ClientCapabilities = serde_json::from_value(json!({
        "textDocument": {
            "declaration": { "linkSupport": true },
            "definition": { "linkSupport": false },
            "implementation": { "linkSupport": true },
            "typeDefinition": { "linkSupport": true }
        }
    }))
    .unwrap();
    assert_eq!(
        NavigationLinkSupport::from_client_capabilities(&capabilities),
        NavigationLinkSupport {
            declaration: true,
            definition: false,
            implementation: true,
        }
    );
    assert_eq!(
        NavigationLinkSupport::from_client_capabilities(&ClientCapabilities::default()),
        NavigationLinkSupport::default()
    );
}

#[test]
fn navigation_links_fall_back_to_ordered_exact_locations() {
    let origin = Range::new(Position::new(4, 10), Position::new(4, 16));
    let first_uri = Url::parse("file:///workspace/first.py").unwrap();
    let second_uri = Url::parse("file:///workspace/second.py").unwrap();
    let first_range = Range::new(Position::new(1, 8), Position::new(1, 14));
    let second_range = Range::new(Position::new(7, 4), Position::new(7, 10));
    let links = vec![
        LocationLink {
            origin_selection_range: Some(origin),
            target_uri: first_uri.clone(),
            target_range: Range::new(Position::new(1, 0), Position::new(2, 0)),
            target_selection_range: first_range,
        },
        LocationLink {
            origin_selection_range: Some(origin),
            target_uri: second_uri.clone(),
            target_range: Range::new(Position::new(7, 0), Position::new(8, 0)),
            target_selection_range: second_range,
        },
    ];

    assert_eq!(
        navigation_response_for_links(links.clone(), true),
        GotoDefinitionResponse::Link(links)
    );
    assert_eq!(
        navigation_response_for_links(
            vec![
                LocationLink {
                    origin_selection_range: Some(origin),
                    target_uri: first_uri.clone(),
                    target_range: Range::default(),
                    target_selection_range: first_range,
                },
                LocationLink {
                    origin_selection_range: Some(origin),
                    target_uri: second_uri.clone(),
                    target_range: Range::default(),
                    target_selection_range: second_range,
                },
            ],
            false,
        ),
        GotoDefinitionResponse::Array(vec![
            Location {
                uri: first_uri,
                range: first_range,
            },
            Location {
                uri: second_uri,
                range: second_range,
            },
        ])
    );

    let implementation_response: GotoImplementationResponse = navigation_response_for_links(
        vec![LocationLink {
            origin_selection_range: Some(origin),
            target_uri: Url::parse("file:///workspace/implementation.py").unwrap(),
            target_range: Range::new(Position::new(3, 0), Position::new(4, 0)),
            target_selection_range: Range::new(Position::new(3, 4), Position::new(3, 10)),
        }],
        false,
    );
    assert!(matches!(
        implementation_response,
        GotoImplementationResponse::Array(_)
    ));
}

#[test]
fn call_hierarchy_only_falls_back_to_the_enclosing_item_without_a_target_word() {
    let source = "def caller():\n    result = unknown.shared()\n    return result\n";

    assert!(
        !may_fallback_to_enclosing_call_hierarchy(source, Position::new(1, 23)),
        "an unresolved member token must not be replaced by the enclosing function"
    );
    assert!(
        !may_fallback_to_enclosing_call_hierarchy(source, Position::new(1, 14)),
        "an unresolved owner token is still an explicit navigation target"
    );
    assert!(
        may_fallback_to_enclosing_call_hierarchy(source, Position::new(2, 2)),
        "ordinary indentation has no explicit target and may use the enclosing function"
    );
}

#[test]
fn reference_candidates_are_scoped_to_the_resolved_definition() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sage-ls-reference-scope-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let first_path = root.join("first.py");
    let second_path = root.join("second.py");
    let first_consumer_path = root.join("first_consumer.py");
    let shared_consumer_path = root.join("shared_consumer.py");
    let second_consumer_path = root.join("second_consumer.py");
    let first_definition = "def target():\n    return 1\n\nunknown.target()\n";
    let second_definition = "def target():\n    return 2\n";
    let first_consumer = "from first import target\nvalue = target()\n";
    let shared_consumer = "from first import target\nother = target()\n";
    let second_consumer = "from second import target\nvalue = target()\n";
    std::fs::write(&first_path, first_definition).unwrap();
    std::fs::write(&second_path, second_definition).unwrap();
    std::fs::write(&first_consumer_path, first_consumer).unwrap();
    std::fs::write(&shared_consumer_path, shared_consumer).unwrap();
    std::fs::write(&second_consumer_path, second_consumer).unwrap();

    let root = root.canonicalize().unwrap();
    let first_path = first_path.canonicalize().unwrap();
    let second_path = second_path.canonicalize().unwrap();
    let first_consumer_path = first_consumer_path.canonicalize().unwrap();
    let shared_consumer_path = shared_consumer_path.canonicalize().unwrap();
    let second_consumer_path = second_consumer_path.canonicalize().unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join("cache"),
        enable_pyx: true,
    });
    index.preload_indexed_files(vec![
        parse_source("first", &first_path, first_definition),
        parse_source("second", &second_path, second_definition),
    ]);

    let definition = index
        .query_source_at_navigation(
            &first_consumer_path,
            first_consumer,
            QueryPosition {
                line: 1,
                character: 10,
            },
        )
        .definition
        .expect("first import should resolve");
    let target = ResolvedReferenceTarget {
        word: "target".to_string(),
        range: Range::default(),
        definition_ranges: vec![definition.range.clone()],
        definition,
        declaration: None,
        local_import_alias: None,
    };
    let usage_range = |path: &Path, source: &str| {
        sage_index::references_in_source(path, source, "target")
            .into_iter()
            .find(|reference| reference.range.start_line == 1)
            .unwrap()
            .range
    };

    assert!(reference_candidate_matches_target(
        &index,
        &first_consumer_path,
        first_consumer,
        &usage_range(&first_consumer_path, first_consumer),
        &target,
    ));
    assert!(reference_candidate_matches_target(
        &index,
        &shared_consumer_path,
        shared_consumer,
        &usage_range(&shared_consumer_path, shared_consumer),
        &target,
    ));
    assert!(!reference_candidate_matches_target(
        &index,
        &second_consumer_path,
        second_consumer,
        &usage_range(&second_consumer_path, second_consumer),
        &target,
    ));
    let unresolved_same_file =
        sage_index::references_in_source(&first_path, first_definition, "target")
            .into_iter()
            .find(|reference| reference.range.start_line == 3)
            .unwrap();
    assert!(
        !reference_candidate_matches_target(
            &index,
            &first_path,
            first_definition,
            &unresolved_same_file.range,
            &target,
        ),
        "an unresolved same-file occurrence must not enter a rename edit"
    );
    assert!(same_definition_identity(
        &target.definition,
        &index
            .query_source_at_navigation(
                &first_path,
                first_definition,
                QueryPosition {
                    line: 0,
                    character: 5,
                },
            )
            .definition
            .unwrap(),
    ));
    assert!(!same_definition_identity(
        &target.definition,
        &index
            .query_source_at_navigation(
                &second_path,
                second_definition,
                QueryPosition {
                    line: 0,
                    character: 5,
                },
            )
            .definition
            .unwrap(),
    ));

    std::fs::remove_dir_all(root).ok();
}

#[test]
fn parameter_reference_candidates_do_not_cross_function_or_file_bindings() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sage-ls-parameter-reference-scope-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let global_path = root.join("global.py");
    let consumer_path = root.join("consumer.py");
    let global_source = "value = 'unrelated global'\n";
    let consumer_source =
        "def first(value):\n    return value\n\ndef second(value):\n    return value\n";
    std::fs::write(&global_path, global_source).unwrap();
    std::fs::write(&consumer_path, consumer_source).unwrap();

    let root = root.canonicalize().unwrap();
    let global_path = global_path.canonicalize().unwrap();
    let consumer_path = consumer_path.canonicalize().unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join("cache"),
        enable_pyx: true,
    });
    index.preload_indexed_files(vec![parse_source("global", &global_path, global_source)]);

    let definition = index
        .query_source_at_navigation(
            &consumer_path,
            consumer_source,
            QueryPosition {
                line: 1,
                character: 12,
            },
        )
        .definition
        .expect("first parameter should resolve locally");
    let target = ResolvedReferenceTarget {
        word: "value".to_string(),
        range: Range::default(),
        definition_ranges: vec![definition.range.clone()],
        definition,
        declaration: None,
        local_import_alias: None,
    };
    let references = sage_index::references_in_source(&consumer_path, consumer_source, "value");
    for line in [0, 1] {
        let reference = references
            .iter()
            .find(|reference| reference.range.start_line == line)
            .unwrap();
        assert!(reference_candidate_matches_target(
            &index,
            &consumer_path,
            consumer_source,
            &reference.range,
            &target,
        ));
    }
    for line in [3, 4] {
        let reference = references
            .iter()
            .find(|reference| reference.range.start_line == line)
            .unwrap();
        assert!(!reference_candidate_matches_target(
            &index,
            &consumer_path,
            consumer_source,
            &reference.range,
            &target,
        ));
    }
    let global_reference = sage_index::references_in_source(&global_path, global_source, "value")
        .into_iter()
        .next()
        .unwrap();
    assert!(!reference_candidate_matches_target(
        &index,
        &global_path,
        global_source,
        &global_reference.range,
        &target,
    ));

    std::fs::remove_dir_all(root).ok();
}

#[test]
fn import_alias_rename_target_uses_local_binding_instead_of_source_definition() {
    let path = PathBuf::from("/workspace/consumer.py");
    let uri = Url::from_file_path(&path).unwrap();
    let source = "from provider import target as alias\nvalue = alias()\n";
    let usage_range = Range::new(Position::new(1, 8), Position::new(1, 13));
    let target = local_import_alias_rename_target(&uri, &path, source, "alias", usage_range)
        .expect("explicit alias should have a local rename target");

    assert_eq!(target.word, "alias");
    assert_eq!(target.range, usage_range);
    assert_eq!(target.declaration.uri, uri);
    assert_eq!(
        target.declaration.range,
        Range::new(Position::new(0, 31), Position::new(0, 36))
    );
    assert_eq!(target.definition.path, path);
    assert_eq!(target.definition.range.start_character, 31);
    assert_eq!(
        target
            .local_import_alias
            .as_ref()
            .and_then(|alias| alias.import_from.as_deref()),
        Some("provider::target")
    );

    let unaliased = "from provider import target\nvalue = target()\n";
    assert!(local_import_alias_rename_target(
        &uri,
        &path,
        unaliased,
        "target",
        Range::new(Position::new(1, 8), Position::new(1, 14)),
    )
    .is_none());

    let module_alias = "import pkg.mod as module_alias\nvalue = module_alias.Factory()\n";
    let module_target = local_import_alias_rename_target(
        &uri,
        &path,
        module_alias,
        "module_alias",
        Range::new(Position::new(1, 8), Position::new(1, 20)),
    )
    .expect("plain import alias should have a local rename target");
    assert_eq!(
        module_target
            .local_import_alias
            .as_ref()
            .and_then(|alias| alias.import_from.as_deref()),
        Some("pkg.mod::mod")
    );
}

#[test]
fn import_alias_rename_and_references_stop_at_visible_rebindings() {
    let path = PathBuf::from("/workspace/consumer.py");
    let uri = Url::from_file_path(&path).unwrap();
    let source = [
        "from provider import target as alias",
        "before = alias()",
        "alias = replacement",
        "after = alias()",
    ]
    .join("\n");
    let rename = local_import_alias_rename_target(
        &uri,
        &path,
        &source,
        "alias",
        Range::new(Position::new(1, 9), Position::new(1, 14)),
    )
    .expect("the alias should be renameable before it is rebound");
    assert!(local_import_alias_rename_target(
        &uri,
        &path,
        &source,
        "alias",
        Range::new(Position::new(3, 8), Position::new(3, 13)),
    )
    .is_none());

    let target = ResolvedReferenceTarget {
        word: rename.word,
        range: rename.range,
        definition: rename.definition,
        definition_ranges: rename.definition_ranges,
        declaration: Some(rename.declaration),
        local_import_alias: rename.local_import_alias,
    };
    let references = sage_index::references_in_source(&path, &source, "alias");
    let before = references
        .iter()
        .find(|reference| reference.range.start_line == 1)
        .unwrap();
    let after = references
        .iter()
        .find(|reference| reference.range.start_line == 3)
        .unwrap();
    assert!(reference_candidate_matches_target(
        &WorkspaceIndex::default(),
        &path,
        &source,
        &before.range,
        &target,
    ));
    assert!(!reference_candidate_matches_target(
        &WorkspaceIndex::default(),
        &path,
        &source,
        &after.range,
        &target,
    ));

    let parameter_source = [
        "from provider import target as alias",
        "def consume(alias):",
        "    return alias()",
    ]
    .join("\n");
    assert!(local_import_alias_rename_target(
        &uri,
        &path,
        &parameter_source,
        "alias",
        Range::new(Position::new(2, 11), Position::new(2, 16)),
    )
    .is_none());
}

#[test]
fn nested_relative_import_alias_rename_keeps_the_workspace_module_identity() {
    let path = PathBuf::from("/workspace/pkg/sub/consumer.py");
    let uri = Url::from_file_path(&path).unwrap();
    let source = "from .provider import target as alias\nvalue = alias()\n";
    let symbols = parse_source("pkg.sub.consumer", &path, source).symbols;
    let usage_range = Range::new(Position::new(1, 8), Position::new(1, 13));
    let rename =
        local_import_alias_rename_target_with_symbols(&uri, source, "alias", usage_range, &symbols)
            .expect("nested relative alias should have a local rename target");
    assert_eq!(rename.definition.module, "pkg.sub.consumer");
    assert_eq!(
        rename
            .local_import_alias
            .as_ref()
            .and_then(|alias| alias.import_from.as_deref()),
        Some(".provider::target")
    );

    let target = ResolvedReferenceTarget {
        word: rename.word,
        range: rename.range,
        definition: rename.definition,
        definition_ranges: rename.definition_ranges,
        declaration: Some(rename.declaration),
        local_import_alias: rename.local_import_alias,
    };
    let reference = sage_index::references_in_source(&path, source, "alias")
        .into_iter()
        .find(|reference| reference.range.start_line == 1)
        .unwrap();
    assert!(reference_candidate_matches_target_with_symbols(
        &WorkspaceIndex::default(),
        &path,
        source,
        &reference.range,
        &target,
        Some(&symbols),
    ));
}

#[test]
fn import_alias_rename_candidates_stay_in_the_alias_binding_scope() {
    let path = PathBuf::from("/workspace/consumer.py");
    let uri = Url::from_file_path(&path).unwrap();
    let source = [
        "def first():",
        "    from provider import target as alias",
        "    return alias()",
        "",
        "def second():",
        "    return alias()",
    ]
    .join("\n");
    let rename = local_import_alias_rename_target(
        &uri,
        &path,
        &source,
        "alias",
        Range::new(Position::new(2, 11), Position::new(2, 16)),
    )
    .expect("function-local alias should be renameable");
    let target = ResolvedReferenceTarget {
        word: rename.word,
        range: rename.range,
        definition: rename.definition,
        definition_ranges: rename.definition_ranges,
        declaration: Some(rename.declaration),
        local_import_alias: rename.local_import_alias,
    };
    let index = WorkspaceIndex::default();
    let references = sage_index::references_in_source(&path, &source, "alias");
    for line in [1, 2] {
        let reference = references
            .iter()
            .find(|reference| reference.range.start_line == line)
            .unwrap();
        assert!(reference_candidate_matches_target(
            &index,
            &path,
            &source,
            &reference.range,
            &target,
        ));
    }
    let unrelated_function = references
        .iter()
        .find(|reference| reference.range.start_line == 5)
        .unwrap();
    assert!(!reference_candidate_matches_target(
        &index,
        &path,
        &source,
        &unrelated_function.range,
        &target,
    ));

    let other_path = PathBuf::from("/workspace/other.py");
    let other_source = "from provider import target as alias\nvalue = alias()\n";
    let other_reference = sage_index::references_in_source(&other_path, other_source, "alias")
        .into_iter()
        .last()
        .unwrap();
    assert!(!reference_candidate_matches_target(
        &index,
        &other_path,
        other_source,
        &other_reference.range,
        &target,
    ));
}

#[test]
fn source_definition_references_and_rename_include_aliased_import_source_names_only() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sage-ls-aliased-source-reference-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let provider_path = root.join("provider.py");
    let other_provider_path = root.join("other_provider.py");
    let first_consumer_path = root.join("first_consumer.py");
    let second_consumer_path = root.join("second_consumer.py");
    let unrelated_consumer_path = root.join("unrelated_consumer.py");
    let provider_source = "def target():\n    return 1\n";
    let other_provider_source = "def target():\n    return 2\n";
    let first_consumer_source =
        "from provider import target as first_alias\nvalue = first_alias()\n";
    let second_consumer_source = [
        "from provider import (",
        "    target as second_alias,",
        ")",
        "value = second_alias()",
    ]
    .join("\n");
    let unrelated_consumer_source =
        "from other_provider import target as unrelated_alias\nvalue = unrelated_alias()\n";
    for (path, source) in [
        (&provider_path, provider_source),
        (&other_provider_path, other_provider_source),
        (&first_consumer_path, first_consumer_source),
        (&second_consumer_path, second_consumer_source.as_str()),
        (&unrelated_consumer_path, unrelated_consumer_source),
    ] {
        std::fs::write(path, source).unwrap();
    }

    let root = root.canonicalize().unwrap();
    let provider_path = provider_path.canonicalize().unwrap();
    let other_provider_path = other_provider_path.canonicalize().unwrap();
    let first_consumer_path = first_consumer_path.canonicalize().unwrap();
    let second_consumer_path = second_consumer_path.canonicalize().unwrap();
    let unrelated_consumer_path = unrelated_consumer_path.canonicalize().unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join("cache"),
        enable_pyx: true,
    });
    index.preload_indexed_files(vec![
        parse_source("provider", &provider_path, provider_source),
        parse_source(
            "other_provider",
            &other_provider_path,
            other_provider_source,
        ),
        parse_source(
            "first_consumer",
            &first_consumer_path,
            first_consumer_source,
        ),
        parse_source(
            "second_consumer",
            &second_consumer_path,
            &second_consumer_source,
        ),
        parse_source(
            "unrelated_consumer",
            &unrelated_consumer_path,
            unrelated_consumer_source,
        ),
    ]);

    let definition_query = index.query_source_at_navigation(
        &provider_path,
        provider_source,
        QueryPosition {
            line: 0,
            character: 5,
        },
    );
    assert_eq!(
        definition_query.resolution_confidence.as_deref(),
        Some("high")
    );
    let definition = definition_query
        .definition
        .expect("definition should resolve");
    let target = ResolvedReferenceTarget {
        word: "target".to_string(),
        range: Range::new(Position::new(0, 4), Position::new(0, 10)),
        definition_ranges: vec![definition.range.clone()],
        definition,
        declaration: None,
        local_import_alias: None,
    };
    let open_paths = BTreeSet::new();
    let collect_closed_locations = |mode| {
        indexed_reference_locations(&index, &target, mode, &open_paths)
            .into_iter()
            .map(|location| {
                (
                    uri_to_path(&location.uri).unwrap(),
                    location.range.start.line,
                    location.range.start.character,
                )
            })
            .collect::<Vec<_>>()
    };
    let accepted = collect_closed_locations(ReferenceCollectionMode::References);
    let rename_accepted = collect_closed_locations(ReferenceCollectionMode::Rename);

    assert_eq!(rename_accepted, accepted);
    assert_eq!(
        accepted,
        vec![
            (first_consumer_path.clone(), 0, 21),
            (provider_path.clone(), 0, 4),
            (second_consumer_path.clone(), 1, 4),
        ]
    );
    assert!(accepted
        .iter()
        .all(|(path, _, _)| path != &other_provider_path && path != &unrelated_consumer_path));
    assert!(!accepted.iter().any(|(path, line, character)| {
        path == &first_consumer_path && *line == 0 && *character == 31
    }));
    assert!(!accepted
        .iter()
        .any(|(path, line, _)| path == &first_consumer_path && *line == 1));

    std::fs::remove_dir_all(root).ok();
}

#[test]
fn rename_reference_collection_rejects_non_editable_live_paths() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sage-ls-reference-editability-{}-{nonce}",
        std::process::id()
    ));
    let editable = root.join("workspace");
    let external = root.join("external");
    std::fs::create_dir_all(&editable).unwrap();
    std::fs::create_dir_all(&external).unwrap();
    let editable = editable.canonicalize().unwrap();
    let external = external.canonicalize().unwrap();
    let index = WorkspaceIndex::new(IndexOptions {
        roots: vec![editable.clone(), external.clone()],
        editable_roots: vec![editable.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join("cache"),
        enable_pyx: true,
    });

    assert!(reference_path_is_collectible(
        &index,
        &editable.join("open.py"),
        ReferenceCollectionMode::Rename,
    ));
    assert!(!reference_path_is_collectible(
        &index,
        &external.join("open.py"),
        ReferenceCollectionMode::Rename,
    ));
    assert!(reference_path_is_collectible(
        &index,
        &external.join("open.py"),
        ReferenceCollectionMode::References,
    ));
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn method_definition_identity_keeps_owner_specific_targets_separate() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sage-ls-method-owner-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("owners.py");
    let source = [
        "class First:",
        "    def target(self):",
        "        return 1",
        "",
        "class Second:",
        "    def target(self):",
        "        return 2",
        "",
        "value = Second()",
        "value.target()",
    ]
    .join("\n");
    std::fs::write(&path, &source).unwrap();
    let root = root.canonicalize().unwrap();
    let path = path.canonicalize().unwrap();
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join("cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();
    let definitions: Vec<_> = parse_source("owners", &path, &source)
        .symbols
        .into_iter()
        .filter(|symbol| symbol.name == "target")
        .map(|symbol| QueryDefinition {
            name: symbol.name,
            path: symbol.path,
            range: symbol.range,
            detail: symbol.detail,
            module: symbol.module,
        })
        .collect();
    let first = definitions
        .iter()
        .find(|definition| definition.detail == "Method First.target")
        .unwrap();
    let second = definitions
        .iter()
        .find(|definition| definition.detail == "Method Second.target")
        .unwrap();

    assert!(!same_definition_identity(second, first));
    assert!(same_definition_identity(second, second));

    let query = index.query_source_at_navigation(
        &path,
        &source,
        QueryPosition {
            line: 9,
            character: 8,
        },
    );
    let target = query
        .definition
        .expect("unified navigation should resolve the member owner");
    assert_eq!(target.detail, "Method Second.target");
    let uri = Url::from_file_path(&path).unwrap();
    let item = call_hierarchy_item_for_local_definition(&uri, &path, &source, &target)
        .expect("call hierarchy should reuse the owner-aware navigation target");
    assert_eq!(item.selection_range.start, Position::new(5, 8));

    std::fs::remove_dir_all(root).ok();
}

#[test]
fn definition_identity_separates_same_path_nested_definitions() {
    let path = PathBuf::from("/workspace/nested.py");
    let first = QueryDefinition {
        name: "target".to_string(),
        path: path.clone(),
        range: sage_index::SourceRange {
            start_line: 1,
            start_character: 8,
            end_line: 1,
            end_character: 14,
        },
        detail: "Function target".to_string(),
        module: "nested".to_string(),
    };
    let second = QueryDefinition {
        range: sage_index::SourceRange {
            start_line: 5,
            start_character: 8,
            end_line: 5,
            end_character: 14,
        },
        ..first.clone()
    };

    assert!(same_definition_owner_identity(&first, &second));
    assert!(!same_definition_identity(&first, &second));
}

#[test]
fn scoped_reference_locations_honor_include_declaration() {
    let uri = Url::parse("file:///workspace/demo.py").unwrap();
    let declaration = Location {
        uri: uri.clone(),
        range: Range::new(Position::new(0, 4), Position::new(0, 10)),
    };
    let usage = Location {
        uri,
        range: Range::new(Position::new(3, 0), Position::new(3, 6)),
    };
    let mut locations = Vec::new();
    let mut seen = BTreeSet::new();

    push_scoped_reference_location(
        &mut locations,
        &mut seen,
        declaration.clone(),
        Some(&declaration),
        false,
    );
    push_scoped_reference_location(
        &mut locations,
        &mut seen,
        usage.clone(),
        Some(&declaration),
        false,
    );
    assert_eq!(locations, vec![usage]);
}

#[test]
fn sage_inlay_hints_infer_common_constructor_assignments() {
    let source = [
        "F = GF(7)",
        "R = PolynomialRing(F, 'x')",
        "M = matrix(F, 2, 2)",
        "v = vector(F, [1, 2])",
        "G = Graph([(0, 1)])",
        "E = EllipticCurve(F, [0, 1])",
        "K = NumberField(x^2 + 1, 'a')",
        "I = R.ideal(x^2 + 1)",
        "g = R.gen()",
    ]
    .join("\n");
    let hints = sage_inlay_hints(&source, full_range());
    let labels: Vec<_> = hints.iter().filter_map(hint_label).collect();

    assert_eq!(
        labels,
        vec![
            ": Field",
            ": PolynomialRing",
            ": Matrix",
            ": Vector",
            ": Graph",
            ": EllipticCurve",
            ": NumberField",
            ": Ideal",
            ": PolynomialElement",
        ]
    );
    assert_eq!(hints[0].position, Position::new(0, 1));
    assert_eq!(hints[1].position, Position::new(1, 1));
}

#[test]
fn sage_inlay_hints_cover_preparser_assignments_and_skip_comments_strings() {
    let source = [
        "R.<x, y> = PolynomialRing(QQ, 2)",
        "text = 'PolynomialRing(QQ)'",
        "# M = matrix(QQ, 2)",
        "comparison == matrix(QQ, 2)",
        "A = zero_matrix(QQ, 2) # matrix comment",
    ]
    .join("\n");
    let hints = sage_inlay_hints(&source, full_range());
    let labels: Vec<_> = hints.iter().filter_map(hint_label).collect();

    assert_eq!(labels, vec![": PolynomialRing", ": Matrix"]);
    assert_eq!(hints[0].position, Position::new(0, 1));
    assert_eq!(hints[1].position, Position::new(4, 1));
}

#[test]
fn sage_inlay_hints_respect_requested_line_range() {
    let source = [
        "F = GF(7)",
        "R = PolynomialRing(F, 'x')",
        "M = matrix(F, 2, 2)",
    ]
    .join("\n");
    let hints = sage_inlay_hints(
        &source,
        Range::new(Position::new(1, 0), Position::new(1, 200)),
    );
    let labels: Vec<_> = hints.iter().filter_map(hint_label).collect();

    assert_eq!(labels, vec![": PolynomialRing"]);
    assert_eq!(hints[0].position, Position::new(1, 1));
}

#[test]
fn code_actions_offer_sage_exponent_quick_fixes() {
    let uri = Url::parse("file:///demo.sage").unwrap();
    let diagnostic = Diagnostic {
        range: Range::new(Position::new(0, 9), Position::new(0, 10)),
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(NumberOrString::String("syntax-error".to_string())),
        source: Some("sage-ls".to_string()),
        message: "Syntax error: incomplete Sage exponentiation".to_string(),
        ..Diagnostic::default()
    };
    let actions = code_actions_for_diagnostics(uri.clone(), std::slice::from_ref(&diagnostic));

    assert_eq!(actions.len(), 2);
    let titles: Vec<_> = actions
        .iter()
        .filter_map(|action| match action {
            CodeActionOrCommand::CodeAction(action) => Some(action.title.as_str()),
            CodeActionOrCommand::Command(_) => None,
        })
        .collect();
    assert_eq!(
        titles,
        vec![
            "Remove incomplete Sage exponent operator",
            "Insert exponent placeholder",
        ]
    );

    let first_edit = match &actions[0] {
        CodeActionOrCommand::CodeAction(action) => action
            .edit
            .as_ref()
            .and_then(|edit| edit.changes.as_ref())
            .and_then(|changes| changes.get(&uri))
            .and_then(|edits| edits.first())
            .expect("first action should include a text edit"),
        CodeActionOrCommand::Command(_) => panic!("expected code action"),
    };
    assert_eq!(first_edit.range, diagnostic.range);
    assert_eq!(first_edit.new_text, "");

    let unrelated = Diagnostic {
        message: "Syntax error: source could not be parsed".to_string(),
        ..diagnostic
    };
    assert!(code_actions_for_diagnostics(uri, &[unrelated]).is_empty());
}

#[test]
fn code_actions_replace_python_sage_caret_exponents() {
    let uri = Url::parse("file:///demo.py").unwrap();
    let diagnostic = Diagnostic {
        range: Range::new(Position::new(2, 9), Position::new(2, 10)),
        severity: Some(DiagnosticSeverity::WARNING),
        code: Some(NumberOrString::String(
            "sage-python-caret-exponent".to_string(),
        )),
        source: Some("sage-ls".to_string()),
        message: "Sage-style exponent operator `^` has Python XOR semantics in `.py`; use `**`."
            .to_string(),
        ..Diagnostic::default()
    };
    let actions = code_actions_for_diagnostics(uri.clone(), std::slice::from_ref(&diagnostic));

    assert_eq!(actions.len(), 1);
    let action = match &actions[0] {
        CodeActionOrCommand::CodeAction(action) => action,
        CodeActionOrCommand::Command(_) => panic!("expected code action"),
    };
    assert_eq!(action.title, "Replace Sage-style ^ with Python exponent **");
    let edit = action
        .edit
        .as_ref()
        .and_then(|edit| edit.changes.as_ref())
        .and_then(|changes| changes.get(&uri))
        .and_then(|edits| edits.first())
        .expect("action should include a text edit");
    assert_eq!(edit.range, diagnostic.range);
    assert_eq!(edit.new_text, "**");
}

#[test]
fn initialization_options_parse_diagnostics_switch() {
    let disabled = parse_initialization_options(Some(json!({
        "analysis": {
            "enableDiagnostics": false,
            "enableRuntimeIntrospection": false,
            "enablePyxParsing": false
        }
    })));
    assert!(!disabled.analysis.enable_diagnostics);
    assert!(!disabled.analysis.enable_runtime_introspection);
    assert!(!disabled.analysis.enable_pyx_parsing);

    let defaults = parse_initialization_options(None);
    assert!(defaults.analysis.enable_diagnostics);
    assert!(defaults.analysis.enable_runtime_introspection);
    assert!(defaults.analysis.enable_pyx_parsing);
}

#[test]
fn initialization_options_parse_and_validate_analysis_mode_independently() {
    for (configured, expected, limit) in [
        ("light", AnalysisMode::Light, 50),
        ("default", AnalysisMode::Default, 200),
        ("full", AnalysisMode::Full, 1_000),
    ] {
        let options = parse_initialization_options(Some(json!({
            "analysis": { "mode": configured }
        })));
        assert_eq!(options.analysis.mode.effective(), expected);
        assert_eq!(
            options.analysis.mode.effective().workspace_symbol_limit(),
            limit
        );
        assert!(options.analysis.mode.invalid_value().is_none());
    }

    let invalid = parse_initialization_options(Some(json!({
        "analysis": {
            "mode": "maximum",
            "enableDiagnostics": false,
            "sourceRoots": ["/configured/source"]
        }
    })));
    assert_eq!(invalid.analysis.mode.effective(), AnalysisMode::Default);
    assert_eq!(invalid.analysis.mode.invalid_value(), Some("maximum"));
    assert!(!invalid.analysis.enable_diagnostics);
    assert_eq!(invalid.analysis.source_roots, vec!["/configured/source"]);
}

#[test]
fn initialization_options_parse_documentation_hover_switch() {
    let disabled = parse_initialization_options(Some(json!({
        "documentation": {
            "preferredSource": "runtime",
            "showOnHover": false
        }
    })));

    assert_eq!(disabled.documentation.preferred_source, "runtime");
    assert!(!disabled.documentation.show_on_hover);

    let defaults = parse_initialization_options(None);
    assert_eq!(defaults.documentation.preferred_source, "auto");
    assert!(defaults.documentation.show_on_hover);
}

#[test]
fn documentation_preferred_source_parses_known_values() {
    assert_eq!(
        DocumentationPreferredSource::from_config("auto"),
        DocumentationPreferredSource::Auto
    );
    assert_eq!(
        DocumentationPreferredSource::from_config("workspace"),
        DocumentationPreferredSource::Workspace
    );
    assert_eq!(
        DocumentationPreferredSource::from_config("runtime"),
        DocumentationPreferredSource::Runtime
    );
    assert_eq!(
        DocumentationPreferredSource::from_config("reference"),
        DocumentationPreferredSource::Reference
    );
    assert_eq!(
        DocumentationPreferredSource::from_config("unexpected"),
        DocumentationPreferredSource::Auto
    );
}

#[test]
fn documentation_source_position_covers_external_definition_files() {
    let path = PathBuf::from("/workspace/sage/combinat/combination.py");
    let source = [
        "def Combinations(mset, k=None, *, as_tuples=False):",
        "    \"\"\"",
        "    Return the combinatorial class of combinations of the multiset.",
        "",
        "    EXAMPLES::",
        "",
        "        sage: C = Combinations(range(4)); C",
        "    \"\"\"",
        "    return []",
    ]
    .join("\n");
    let index = WorkspaceIndex::new(IndexOptions {
        roots: vec![PathBuf::from("/workspace")],
        editable_roots: Vec::new(),
        exclude_globs: Vec::new(),
        cache_dir: PathBuf::from("/tmp/sage-ls-doc-position-test"),
        enable_pyx: true,
    });

    let record = documentation_record_for_source_position(
        &index,
        &path,
        &source,
        QueryPosition {
            line: 0,
            character: 8,
        },
    )
    .expect("definition source position should produce docs");

    assert_eq!(record.name, "Combinations");
    assert_eq!(
        record.summary,
        "Return the combinatorial class of combinations of the multiset."
    );
    assert!(record
        .docstring
        .as_deref()
        .is_some_and(|doc| doc.contains("EXAMPLES::")));
}

#[test]
fn runtime_docs_query_policy_respects_preferred_source() {
    let placeholder = DocumentationRecord {
        name: "PolynomialRing".to_string(),
        docstring: Some(
            "Known Sage symbol. Runtime documentation worker can provide details.".to_string(),
        ),
        ..DocumentationRecord::default()
    };
    let strong_static = DocumentationRecord {
        name: "PolynomialRing".to_string(),
        docstring: Some("Construct a polynomial ring.".to_string()),
        ..DocumentationRecord::default()
    };
    let query = QueryResult {
        target: Some(sage_index::QueryTarget {
            symbol: "PolynomialRing".to_string(),
            dotted_symbol: Some("sage.all.PolynomialRing".to_string()),
            range: sage_index::SourceRange::default(),
        }),
        documentation: Some(placeholder),
        ..QueryResult::default()
    };
    assert_eq!(
        runtime_docs_symbol_for_query(&query, DocumentationPreferredSource::Auto),
        Some("sage.all.PolynomialRing")
    );
    assert_eq!(
        runtime_docs_symbol_for_query(&query, DocumentationPreferredSource::Runtime),
        Some("sage.all.PolynomialRing")
    );
    assert_eq!(
        runtime_docs_symbol_for_query(&query, DocumentationPreferredSource::Workspace),
        None
    );
    assert_eq!(
        runtime_docs_symbol_for_query(&query, DocumentationPreferredSource::Reference),
        None
    );

    let strong_query = QueryResult {
        documentation: Some(strong_static),
        ..query
    };
    assert_eq!(
        runtime_docs_symbol_for_query(&strong_query, DocumentationPreferredSource::Auto),
        None
    );
    assert_eq!(
        runtime_docs_symbol_for_query(&strong_query, DocumentationPreferredSource::Runtime),
        Some("sage.all.PolynomialRing")
    );
}

#[test]
fn hover_markdown_respects_documentation_preview_setting() {
    let markdown = [
        "```sage",
        "PolynomialRing(base_ring, names)",
        "```",
        "",
        "Module: `sage.rings.polynomial.polynomial_ring_constructor`",
        "",
        "Return a polynomial ring over the given base ring.",
    ]
    .join("\n");

    assert_eq!(hover_markdown_for_hover_setting(&markdown, true), markdown);

    let compact = hover_markdown_for_hover_setting(&markdown, false);
    assert!(compact.contains("PolynomialRing(base_ring, names)"));
    assert!(compact.contains("Module: `sage.rings.polynomial.polynomial_ring_constructor`"));
    assert!(!compact.contains("Return a polynomial ring"));
}

#[test]
fn sage_folding_ranges_cover_python_sage_cython_and_comments() {
    let source = [
        "def kernel_columns(A):",
        "    if A.ncols() == 0:",
        "        return A",
        "    return A",
        "",
        "# region setup",
        "R = PolynomialRing(QQ, 'x')",
        "# endregion",
        "",
        "# first note",
        "# second note",
        "cdef class NativeThing:",
        "    cpdef rank(self):",
        "        return 1",
        "text = 'def fake():'",
    ]
    .join("\n");

    let ranges = sage_folding_ranges(&source);
    assert!(ranges
        .iter()
        .any(|range| range.start_line == 0 && range.end_line == 3 && range.kind.is_none()));
    assert!(ranges
        .iter()
        .any(|range| range.start_line == 1 && range.end_line == 2 && range.kind.is_none()));
    assert!(ranges.iter().any(|range| {
        range.start_line == 5 && range.end_line == 7 && range.kind == Some(FoldingRangeKind::Region)
    }));
    assert!(ranges.iter().any(|range| {
        range.start_line == 9
            && range.end_line == 10
            && range.kind == Some(FoldingRangeKind::Comment)
    }));
    assert!(ranges
        .iter()
        .any(|range| range.start_line == 11 && range.end_line == 13 && range.kind.is_none()));
    assert!(ranges
        .iter()
        .all(|range| range.start_line != 14 && range.end_line != 14));
}

#[test]
fn sage_selection_ranges_expand_symbol_line_blocks_and_document() {
    let source = [
        "def kernel_columns(A):",
        "    if A.ncols() == 0:",
        "        return A",
        "    return A",
        "",
        "value = kernel_columns(M)",
    ]
    .join("\n");

    let chain = selection_chain_ranges(sage_selection_range(&source, Position::new(2, 15)));
    assert!(chain.len() >= 5, "{chain:?}");
    assert_eq!(
        chain[0],
        Range::new(Position::new(2, 15), Position::new(2, 16))
    );
    assert_eq!(
        chain[1],
        Range::new(Position::new(2, 8), Position::new(2, 16))
    );
    assert_eq!(
        chain[2],
        Range::new(Position::new(1, 0), Position::new(2, 16))
    );
    assert_eq!(
        chain[3],
        Range::new(Position::new(0, 0), Position::new(3, 12))
    );
    assert_eq!(
        chain.last().copied(),
        Some(Range::new(Position::new(0, 0), Position::new(5, 25)))
    );

    let leading_space_chain =
        selection_chain_ranges(sage_selection_range(&source, Position::new(1, 2)));
    assert_eq!(
        leading_space_chain[0],
        Range::new(Position::new(1, 2), Position::new(1, 2))
    );
    assert_eq!(
        leading_space_chain[1],
        Range::new(Position::new(1, 0), Position::new(1, 22))
    );
}

#[test]
fn document_symbols_nest_classes_functions_and_locals() {
    let path = PathBuf::from("/workspace/demo.sage");
    let source = [
        "class Solver:",
        "    def build(self):",
        "        R = PolynomialRing(QQ, 'x')",
        "        return helper(R)",
        "",
        "def helper(R):",
        "    return R",
        "",
        "R.<x, y> = PolynomialRing(QQ, 2)",
    ]
    .join("\n");
    let parsed = parse_source(module_name_for_path(&path), &path, &source);
    let symbols = document_symbols_for_source(&source, &parsed.symbols);
    let names = symbols
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect::<Vec<_>>();

    assert!(names.contains(&"Solver"), "{names:?}");
    assert!(names.contains(&"helper"), "{names:?}");

    let solver = symbols
        .iter()
        .find(|symbol| symbol.name == "Solver")
        .expect("class should be a top-level outline entry");
    let children = solver
        .children
        .as_ref()
        .expect("class should contain nested method symbols");
    assert!(children.iter().any(|symbol| symbol.name == "build"));
    assert_eq!(
        solver.range,
        Range::new(Position::new(0, 0), Position::new(3, 24))
    );
    assert_eq!(
        solver.selection_range,
        Range::new(Position::new(0, 6), Position::new(0, 12))
    );
}

#[test]
fn document_symbols_hide_module_and_import_metadata() {
    let path = PathBuf::from("/workspace/demo.py");
    let source = [
        "\"\"\"Module-level documentation.\"\"\"",
        "from sage.all import PolynomialRing",
        "",
        "def build_ring():",
        "    return PolynomialRing(QQ, 'x')",
    ]
    .join("\n");
    let parsed = parse_source(module_name_for_path(&path), &path, &source);
    let symbols = document_symbols_for_source(&source, &parsed.symbols);
    let names = symbols
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["build_ring"]);
}

#[test]
fn python_sage_import_items_are_detected_for_duplicate_navigation_suppression() {
    let path = PathBuf::from("/workspace/demo.py");
    let source = [
        "from sage.all import (",
        "    GF,",
        "    PolynomialRing,",
        ")",
        "",
        "R = PolynomialRing(GF(7), 'x')",
    ]
    .join("\n");
    let definition = QueryDefinition {
        name: "PolynomialRing".to_string(),
        path: PathBuf::from("/sage/sage/rings/polynomial/polynomial_ring_constructor.py"),
        range: sage_index::SourceRange {
            start_line: 60,
            start_character: 4,
            end_line: 60,
            end_character: 18,
        },
        detail: "def PolynomialRing(...)".to_string(),
        module: "sage.rings.polynomial.polynomial_ring_constructor".to_string(),
    };

    assert!(should_defer_python_import_definition_to_python_provider(
        &path,
        &source,
        Position::new(2, 8),
        &definition,
    ));
    assert!(!should_defer_python_import_definition_to_python_provider(
        &path,
        &source,
        Position::new(5, 6),
        &definition,
    ));
}

#[test]
fn document_symbol_provider_has_visible_vscode_label() {
    assert_eq!(
        sage_document_symbol_options().label.as_deref(),
        Some("Sage")
    );
}

#[test]
fn semantic_token_range_filters_to_requested_lines() {
    let source = [
        "# class IgnoredComment:",
        "class Solver:",
        "    def build(self):",
        "        R = PolynomialRing(QQ, 'x')",
        "text = 'def hidden():'",
    ]
    .join("\n");

    let class_tokens = encode_semantic_tokens_for_range(
        &source,
        Range::new(Position::new(1, 0), Position::new(2, 0)),
    );
    assert_eq!(class_tokens.len(), 1);
    assert_eq!(class_tokens[0].delta_line, 1);
    assert_eq!(class_tokens[0].delta_start, 6);
    assert_eq!(class_tokens[0].length, 6);
    assert_eq!(class_tokens[0].token_type, token_type_index("class"));

    let comment_tokens = encode_semantic_tokens_for_range(
        &source,
        Range::new(Position::new(0, 0), Position::new(1, 0)),
    );
    assert!(comment_tokens.is_empty());
}

#[test]
fn incremental_text_change_applies_single_line_insert() {
    let mut source = "value = kernel_col\n".to_string();
    apply_text_document_change(
        &mut source,
        &TextDocumentContentChangeEvent {
            range: Some(Range::new(Position::new(0, 18), Position::new(0, 18))),
            range_length: None,
            text: "umns".to_string(),
        },
    )
    .unwrap();

    assert_eq!(source, "value = kernel_columns\n");
}

#[test]
fn incremental_text_change_replaces_multiline_range() {
    let mut source = "def build():\n    pass\nvalue = 1\n".to_string();
    apply_text_document_change(
        &mut source,
        &TextDocumentContentChangeEvent {
            range: Some(Range::new(Position::new(1, 4), Position::new(2, 9))),
            range_length: None,
            text: "return 2".to_string(),
        },
    )
    .unwrap();

    assert_eq!(source, "def build():\n    return 2\n");
}

#[test]
fn incremental_text_change_handles_utf16_positions() {
    let mut source = "text = \"😀\"\nvalue = π\n".to_string();
    apply_text_document_change(
        &mut source,
        &TextDocumentContentChangeEvent {
            range: Some(Range::new(Position::new(0, 8), Position::new(0, 10))),
            range_length: None,
            text: "theta".to_string(),
        },
    )
    .unwrap();
    apply_text_document_change(
        &mut source,
        &TextDocumentContentChangeEvent {
            range: Some(Range::new(Position::new(1, 8), Position::new(1, 9))),
            range_length: None,
            text: "pi".to_string(),
        },
    )
    .unwrap();

    assert_eq!(source, "text = \"theta\"\nvalue = pi\n");
}

#[test]
fn incremental_text_change_rejects_split_surrogate_positions() {
    let mut source = "text = \"😀\"\n".to_string();
    let result = apply_text_document_change(
        &mut source,
        &TextDocumentContentChangeEvent {
            range: Some(Range::new(Position::new(0, 9), Position::new(0, 10))),
            range_length: None,
            text: "broken".to_string(),
        },
    );

    assert!(result.is_err());
    assert_eq!(source, "text = \"😀\"\n");
}

#[test]
fn lsp_utf16_positions_resolve_symbols_after_unicode_prefixes() {
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap_or_else(|_| std::env::temp_dir());
    let definition_path = root.join("sage-ls-utf16-defs.py");
    let consumer_path = root.join("sage-ls-utf16-consumer.py");
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join("sage-ls-utf16-cache"),
        enable_pyx: true,
    });
    index.preload_indexed_files(vec![parse_source(
        "defs",
        &definition_path,
        "def target():\n    return 1\n",
    )]);

    let source = "from defs import target\nπ = 1; 😀 target()\n";
    let usage_line = source.lines().nth(1).unwrap();
    let byte_start = usage_line.find("target").unwrap();
    let utf16_start = usage_line[..byte_start].encode_utf16().count() as u32;
    assert_ne!(utf16_start, byte_start as u32);

    let lsp_position = Position::new(1, utf16_start);
    let query_position = query_position_from_lsp(source, lsp_position).unwrap();
    assert_eq!(query_position.character, byte_start as u32);
    let (word, word_range) = word_at_position(source, lsp_position).unwrap();
    assert_eq!(word, "target");
    assert_eq!(
        word_range,
        Range::new(
            Position::new(1, utf16_start),
            Position::new(1, utf16_start + "target".encode_utf16().count() as u32),
        )
    );

    let query = index.query_source_at_navigation(&consumer_path, source, query_position);
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|definition| definition.path.as_path()),
        Some(definition_path.as_path())
    );
    let target_range = query
        .target
        .as_ref()
        .map(|target| lsp_range_for_text(source, &target.range));
    assert_eq!(target_range, Some(word_range));

    let usage_reference = sage_index::references_in_source(&consumer_path, source, "target")
        .into_iter()
        .find(|reference| reference.range.start_line == 1)
        .expect("usage reference should be indexed");
    assert_eq!(
        lsp_range_for_text(source, &usage_reference.range),
        word_range
    );
}

#[test]
fn semantic_tokens_use_utf16_columns_after_astral_characters() {
    let tokens = encode_semantic_tokens("😀 matrix(QQ, 1, 1)\n");
    let matrix = tokens
        .iter()
        .find(|token| token.token_type == token_type_index("function"))
        .expect("matrix should be classified as a function");
    assert_eq!(matrix.delta_line, 0);
    assert_eq!(matrix.delta_start, 3);
    assert_eq!(matrix.length, 6);
}

#[test]
fn call_hierarchy_scanner_includes_members_and_skips_non_code_calls() {
    let source = [
        "def main(A):",
        "    helper(A)",
        "    A.rank()",
        "    text = \"fake()\"",
        "    '''hidden()'''",
        "    # ignored()",
        "    R = PolynomialRing(QQ, 'x')",
        "    return zero_matrix(QQ, 1, 1)",
    ]
    .join("\n");
    let calls = call_ranges_in_range(
        &source,
        Range::new(Position::new(0, 0), Position::new(7, 35)),
    );
    let names = calls
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec!["helper", "rank", "PolynomialRing", "zero_matrix"]
    );
}

#[test]
fn call_hierarchy_scanner_skips_calls_in_multiline_strings() {
    let source = [
        "def main():",
        "    description = \"\"\"",
        "    hidden_call()",
        "    and_hidden()",
        "    \"\"\"",
        "    return visible_call()",
    ]
    .join("\n");
    let calls = call_ranges_in_range(
        &source,
        Range::new(Position::new(0, 0), Position::new(5, 25)),
    );
    assert_eq!(
        calls
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        vec!["visible_call"]
    );
}

#[test]
fn call_hierarchy_enclosing_item_finds_nearest_function_block() {
    let path = PathBuf::from("/workspace/demo.sage");
    let uri = Url::from_file_path(&path).unwrap();
    let source = [
        "def helper(A):",
        "    return A",
        "",
        "def main(A):",
        "    if A:",
        "        return helper(A)",
    ]
    .join("\n");

    let item = enclosing_call_hierarchy_item(&uri, &path, &source, Position::new(5, 18)).unwrap();
    assert_eq!(item.name, "main");
    assert_eq!(item.selection_range.start, Position::new(3, 4));
    assert_eq!(
        item.range,
        Range::new(Position::new(3, 0), Position::new(5, 24))
    );
}

#[test]
fn call_hierarchy_local_fast_path_only_matches_declarations() {
    let path = PathBuf::from("/workspace/demo.py");
    let uri = Url::from_file_path(&path).unwrap();
    let source = [
        "from sage.all import zero_matrix",
        "",
        "def kernel_columns(A):",
        "    if A.ncols() == 0:",
        "        return zero_matrix(A.base_ring(), 0, 0)",
        "    return A",
        "",
        "def caller(M):",
        "    return kernel_columns(M)",
    ]
    .join("\n");

    let item =
        call_hierarchy_item_for_local_symbol_at_position(&uri, &path, &source, Position::new(2, 8))
            .expect("local declaration should use the live-document fast path");

    assert_eq!(item.name, "kernel_columns");
    assert_eq!(item.uri, uri);
    assert_eq!(
        item.selection_range,
        Range::new(Position::new(2, 4), Position::new(2, 18))
    );
    assert_eq!(
        item.range,
        Range::new(Position::new(2, 0), Position::new(5, 12))
    );
    assert!(call_hierarchy_item_for_local_symbol_at_position(
        &uri,
        &path,
        &source,
        Position::new(8, 13),
    )
    .is_none());
}

#[test]
fn sage_document_links_cover_load_attach_and_cython_include() {
    let path = PathBuf::from("/workspace/project/src/demo.sage");
    let source = [
        "load(\"helpers/setup.sage\")",
        "attach('../shared/tools.sage')",
        "# load('ignored.sage')",
        "text = \"load('ignored.sage')\"",
        "include \"native_include.pxi\"",
        "    include 'native_support.pxd'",
    ]
    .join("\n");

    let links = sage_document_links(&source, &path);
    let targets = links
        .iter()
        .filter_map(|link| link.target.as_ref())
        .map(|uri| uri.to_file_path().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(links.len(), 4);
    assert!(targets.contains(&PathBuf::from("/workspace/project/src/helpers/setup.sage")));
    assert!(targets.contains(&PathBuf::from("/workspace/project/shared/tools.sage")));
    assert!(targets.contains(&PathBuf::from("/workspace/project/src/native_include.pxi")));
    assert!(targets.contains(&PathBuf::from("/workspace/project/src/native_support.pxd")));
    assert_eq!(
        links[0].range,
        Range::new(Position::new(0, 6), Position::new(0, 24))
    );
}

#[test]
fn import_modules_for_prewarm_extracts_local_targets() {
    let path = PathBuf::from("/workspace/project/src/demo.sage");
    let source = [
        "from local_docs import PolynomialNotebook",
        "from package_demo import named_polynomial, AffineNote",
        "from external_series import EXTERNAL_LABEL as label_value",
    ]
    .join("\n");

    assert_eq!(
        import_modules_for_prewarm(&path, &source),
        vec![
            "external_series".to_string(),
            "local_docs".to_string(),
            "package_demo".to_string(),
        ]
    );
}

#[test]
fn document_highlights_cover_code_references_only() {
    let path = PathBuf::from("/workspace/demo.sage");
    let source = [
        "def kernel_columns(A):",
        "    return A",
        "N = kernel_columns(M)",
        "text = 'kernel_columns(M)'",
        "# kernel_columns(comment)",
        "K = kernel_columns(N)",
    ]
    .join("\n");
    let highlights = document_highlights_for_source(
        &path,
        &source,
        "kernel_columns",
        Range::new(Position::new(0, 4), Position::new(0, 18)),
    );

    assert_eq!(highlights.len(), 3);
    assert_eq!(
        highlights
            .iter()
            .map(|highlight| highlight.range.start.line)
            .collect::<Vec<_>>(),
        vec![0, 2, 5]
    );
    assert!(highlights
        .iter()
        .all(|highlight| highlight.kind == Some(DocumentHighlightKind::TEXT)));

    let comment_range = Range::new(Position::new(4, 2), Position::new(4, 16));
    assert!(
        document_highlights_for_source(&path, &source, "kernel_columns", comment_range,).is_empty()
    );
}

#[test]
fn signature_information_extracts_parameter_offsets() {
    let label = "trace_window(poly, base_ring=QQ, *, width=5, normalize=True)".to_string();
    let info = signature_information(label.clone(), Some("docs".to_string()), 1);
    assert_eq!(info.active_parameter, Some(1));
    assert_eq!(
        info.documentation,
        Some(Documentation::String("docs".to_string()))
    );
    let parameters = info.parameters.expect("parameters should be present");
    let labels = parameters
        .iter()
        .map(|parameter| match parameter.label {
            ParameterLabel::LabelOffsets([start, end]) => &label[start as usize..end as usize],
            ParameterLabel::Simple(_) => panic!("expected offset labels"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        labels,
        vec!["poly", "base_ring=QQ", "*", "width=5", "normalize=True"]
    );
}

#[test]
fn signature_parameters_ignore_nested_commas_and_strings() {
    let label = "foo(a, data=(1, 2), names='x,y', options={\"k\": [1, 2]})";
    let labels = signature_parameter_offsets(label)
        .into_iter()
        .map(|[start, end]| &label[start as usize..end as usize])
        .collect::<Vec<_>>();
    assert_eq!(
        labels,
        vec!["a", "data=(1, 2)", "names='x,y'", "options={\"k\": [1, 2]}"]
    );
    assert!(signature_parameter_offsets("foo()").is_empty());
}

#[test]
fn signature_parameter_offsets_use_utf16_code_units() {
    let label = "λ(α, rocket='🚀', γ=π)";
    let labels = signature_parameter_offsets(label)
        .into_iter()
        .map(|[start, end]| {
            let start = utf16_character_to_byte_offset(label, start).unwrap();
            let end = utf16_character_to_byte_offset(label, end).unwrap();
            &label[start..end]
        })
        .collect::<Vec<_>>();
    assert_eq!(labels, vec!["α", "rocket='🚀'", "γ=π"]);
    assert_eq!(signature_parameter_offsets(label)[0], [2, 3]);
}

#[test]
fn open_document_definition_range_reparses_live_unicode_text() {
    let path = PathBuf::from("/workspace/live_definition.py");
    let definition = QueryDefinition {
        name: "target".to_string(),
        path,
        range: sage_index::SourceRange {
            start_line: 0,
            start_character: 4,
            end_line: 0,
            end_character: 10,
        },
        detail: "Function target".to_string(),
        module: "live_definition".to_string(),
    };
    let text = "π = 3\n\ndef target(value='🚀'):\n    return value\n";
    let live = live_definition_range(&definition, text).expect("live definition should parse");
    assert_eq!(live.start_line, 2);
    assert_eq!(lsp_range_for_text(text, &live).start, Position::new(2, 4));
    assert!(live_definition_range(&definition, "value = 1\n").is_none());
}

#[test]
fn open_document_definition_range_preserves_verified_live_parameter() {
    let path = PathBuf::from("/workspace/live_parameter.py");
    let definition = QueryDefinition {
        name: "value".to_string(),
        path,
        range: sage_index::SourceRange {
            start_line: 0,
            start_character: 11,
            end_line: 0,
            end_character: 16,
        },
        detail: "Local parameter value".to_string(),
        module: "live_parameter".to_string(),
    };
    let text = "def caller(value):\n    return value\n";
    assert_eq!(
        live_definition_range(&definition, text),
        Some(definition.range.clone())
    );
    assert!(live_definition_range(&definition, "def caller(other):\n    return other\n").is_none());
}

#[test]
fn open_document_definition_range_uses_method_owner_detail() {
    let path = PathBuf::from("/workspace/live_methods.py");
    let definition = QueryDefinition {
        name: "target".to_string(),
        path,
        range: sage_index::SourceRange {
            start_line: 1,
            start_character: 8,
            end_line: 1,
            end_character: 14,
        },
        detail: "Method Second.target".to_string(),
        module: "live_methods".to_string(),
    };
    let text = [
        "class First:",
        "    def target(self):",
        "        return 1",
        "",
        "class Second:",
        "    def target(self):",
        "        return 2",
        "",
    ]
    .join("\n");
    let live = live_definition_range(&definition, &text).expect("owned method should parse");
    assert_eq!(live.start_line, 5);
}

fn selection_chain_ranges(selection_range: SelectionRange) -> Vec<Range> {
    let mut ranges = Vec::new();
    let mut current = Some(selection_range);
    while let Some(selection) = current {
        ranges.push(selection.range);
        current = selection.parent.map(|parent| *parent);
    }
    ranges
}

fn full_range() -> Range {
    Range::new(Position::new(0, 0), Position::new(u32::MAX, u32::MAX))
}

fn hint_label(hint: &InlayHint) -> Option<&str> {
    match &hint.label {
        InlayHintLabel::String(label) => Some(label.as_str()),
        InlayHintLabel::LabelParts(_) => None,
    }
}
