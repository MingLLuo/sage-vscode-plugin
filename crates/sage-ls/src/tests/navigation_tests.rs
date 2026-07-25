use super::*;

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
        flavor: NavigationQueryCacheFlavor::Definition,
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
fn navigation_cache_entries_are_scoped_to_request_flavor() {
    let mut cache = NavigationQueryCache::default();
    let definition = NavigationQueryCacheKey {
        uri: "file:///workspace/demo.sage".to_string(),
        version: 1,
        content_fingerprint: None,
        line: 2,
        character: 4,
        index_generation: 7,
        flavor: NavigationQueryCacheFlavor::Definition,
    };
    cache.insert(definition.clone(), QueryResult::default());

    for flavor in [
        NavigationQueryCacheFlavor::Hover,
        NavigationQueryCacheFlavor::Declaration,
        NavigationQueryCacheFlavor::Implementation,
    ] {
        assert!(cache
            .get(&NavigationQueryCacheKey {
                flavor,
                ..definition.clone()
            })
            .is_none());
    }
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

    let support = NavigationLinkSupport {
        declaration: true,
        definition: false,
        implementation: true,
    };
    assert!(NavigationRequestKind::Declaration.link_support(support));
    assert!(!NavigationRequestKind::Definition.link_support(support));
    assert!(NavigationRequestKind::Implementation.link_support(support));
    assert_eq!(
        NavigationRequestKind::Declaration.index_role(),
        sage_index::NavigationTargetRole::Declaration
    );
    assert_eq!(
        NavigationRequestKind::Definition.index_role(),
        sage_index::NavigationTargetRole::Definition
    );
    assert_eq!(
        NavigationRequestKind::Implementation.index_role(),
        sage_index::NavigationTargetRole::Implementation
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
    assert!(
        NavigationRequestKind::Implementation.should_defer_python_import(
            &path,
            &source,
            Position::new(2, 8),
            &definition,
        )
    );
    assert!(
        !NavigationRequestKind::Declaration.should_defer_python_import(
            &path,
            &source,
            Position::new(2, 8),
            &definition,
        )
    );
    assert!(
        !NavigationRequestKind::Definition.should_defer_python_import(
            &path,
            &source,
            Position::new(2, 8),
            &definition,
        )
    );
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

fn disk_definition(
    path: PathBuf,
    name: &str,
    detail: &str,
    range: sage_index::SourceRange,
) -> QueryDefinition {
    QueryDefinition {
        name: name.to_string(),
        path,
        range,
        detail: detail.to_string(),
        module: "navigation_validation".to_string(),
    }
}

fn parsed_disk_definition(
    path: PathBuf,
    text: &str,
    name: &str,
    kind: SageSymbolKind,
) -> QueryDefinition {
    let module = "navigation_validation";
    let symbol = parse_source(module, &path, text)
        .symbols
        .into_iter()
        .find(|symbol| symbol.name == name && symbol.kind == kind)
        .expect("test source should contain the requested symbol");
    QueryDefinition {
        name: symbol.name,
        path,
        range: symbol.range,
        detail: symbol.detail,
        module: module.to_string(),
    }
}

fn unique_navigation_test_path(label: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sage-ls-navigation-{label}-{}-{nonce}.py",
        std::process::id()
    ))
}

#[test]
fn unopened_definition_requires_a_readable_current_file() {
    let path = unique_navigation_test_path("missing");
    let definition = disk_definition(
        path,
        "target",
        "Function target",
        sage_index::SourceRange {
            start_line: 0,
            start_character: 4,
            end_line: 0,
            end_character: 10,
        },
    );

    assert!(validated_disk_definition_location(&definition).is_none());
}

#[test]
fn unopened_definition_requires_identity_detail() {
    let path = unique_navigation_test_path("missing-detail");
    std::fs::write(&path, "def target():\n    return 1\n").unwrap();
    let definition = disk_definition(
        path.clone(),
        "target",
        "",
        sage_index::SourceRange {
            start_line: 0,
            start_character: 4,
            end_line: 0,
            end_character: 10,
        },
    );

    assert!(validated_disk_definition_location(&definition).is_none());
    std::fs::remove_file(path).ok();
}

#[test]
fn unopened_definition_relocates_a_stale_index_range() {
    let path = unique_navigation_test_path("moved");
    std::fs::write(
        &path,
        "# definition moved after indexing\n\n\ndef target():\n    return 1\n",
    )
    .unwrap();
    let definition = disk_definition(
        path.clone(),
        "target",
        "Function target",
        sage_index::SourceRange {
            start_line: 0,
            start_character: 4,
            end_line: 0,
            end_character: 10,
        },
    );

    let location =
        validated_disk_definition_location(&definition).expect("moved definition should relocate");
    assert_eq!(location.uri, Url::from_file_path(&path).unwrap());
    assert_eq!(
        location.range,
        Range::new(Position::new(3, 4), Position::new(3, 10))
    );
    std::fs::remove_file(path).ok();
}

#[test]
fn unopened_definition_rejects_a_mismatched_method_owner() {
    let path = unique_navigation_test_path("owner");
    std::fs::write(
        &path,
        "class First:\n    def target(self):\n        return 1\n",
    )
    .unwrap();
    let definition = disk_definition(
        path.clone(),
        "target",
        "Method Second.target",
        sage_index::SourceRange {
            start_line: 1,
            start_character: 8,
            end_line: 1,
            end_character: 14,
        },
    );

    assert!(validated_disk_definition_location(&definition).is_none());
    std::fs::remove_file(path).ok();
}

#[test]
fn stale_definition_does_not_guess_between_duplicate_identities() {
    let path = unique_navigation_test_path("duplicates");
    let text = [
        "def target():",
        "    return 1",
        "",
        "def wrapper():",
        "    def target():",
        "        return 2",
        "    return target()",
        "",
    ]
    .join("\n");
    std::fs::write(&path, &text).unwrap();
    let definition = disk_definition(
        path.clone(),
        "target",
        "Function target",
        sage_index::SourceRange {
            start_line: 20,
            start_character: 4,
            end_line: 20,
            end_character: 10,
        },
    );

    assert!(live_definition_range(&definition, &text).is_none());
    assert!(validated_disk_definition_location(&definition).is_none());
    std::fs::remove_file(path).ok();
}

#[test]
fn unopened_definition_uses_utf16_for_a_verified_unicode_target() {
    let path = unique_navigation_test_path("unicode");
    let text = "def caller(π='🚀', target=1):\n    return target\n";
    std::fs::write(&path, text).unwrap();
    let byte_start = text.find("target").unwrap() as u32;
    let definition = disk_definition(
        path.clone(),
        "target",
        "Local parameter target",
        sage_index::SourceRange {
            start_line: 0,
            start_character: byte_start,
            end_line: 0,
            end_character: byte_start + "target".len() as u32,
        },
    );

    let location =
        validated_disk_definition_location(&definition).expect("unicode target should validate");
    let utf16_start = text[..byte_start as usize].encode_utf16().count() as u32;
    assert_ne!(utf16_start, byte_start);
    assert_eq!(
        location.range,
        Range::new(
            Position::new(0, utf16_start),
            Position::new(0, utf16_start + "target".encode_utf16().count() as u32),
        )
    );
    std::fs::remove_file(path).ok();
}

#[test]
fn unopened_definition_preserves_a_valid_current_target() {
    let path = unique_navigation_test_path("valid");
    std::fs::write(&path, "def target():\n    return 1\n").unwrap();
    let expected = sage_index::SourceRange {
        start_line: 0,
        start_character: 4,
        end_line: 0,
        end_character: 10,
    };
    let definition = disk_definition(path.clone(), "target", "Function target", expected);

    let location =
        validated_disk_definition_location(&definition).expect("current target should validate");
    assert_eq!(
        location.range,
        Range::new(Position::new(0, 4), Position::new(0, 10))
    );
    std::fs::remove_file(path).ok();
}

#[test]
fn unopened_definition_validation_keeps_import_variable_and_cython_targets() {
    let cases = [
        (
            unique_navigation_test_path("import"),
            "from unavailable.provider import external\n",
            "external",
            SageSymbolKind::Import,
        ),
        (
            unique_navigation_test_path("variable"),
            "value = 1\n",
            "value",
            SageSymbolKind::Variable,
        ),
        (
            unique_navigation_test_path("cython").with_extension("pyx"),
            "cpdef int target(int value):\n    return value\n",
            "target",
            SageSymbolKind::CythonDeclaration,
        ),
        (
            unique_navigation_test_path("module"),
            "value = 1\n",
            "navigation_validation",
            SageSymbolKind::Module,
        ),
    ];

    for (path, text, name, kind) in cases {
        std::fs::write(&path, text).unwrap();
        let definition = parsed_disk_definition(path.clone(), text, name, kind);
        let location = validated_disk_definition_location(&definition)
            .expect("verified source symbol should remain navigable");
        assert_eq!(location.uri, Url::from_file_path(&path).unwrap());
        assert_eq!(location.range, lsp_range_for_text(text, &definition.range));
        std::fs::remove_file(path).ok();
    }
}
