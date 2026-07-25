use super::*;

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
