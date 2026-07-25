use super::*;

#[test]
fn navigation_candidate_limit_counts_only_records_with_locations() {
    let root = test_root("navigation-candidate-location-limit");
    let source_path = root.join("consumer.py");
    let source = "result = crowded()\n";
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    let mut records = vec![SymbolRecord {
        name: "crowded".to_string(),
        kind: SymbolKind::Function,
        module: "missing".to_string(),
        path: PathBuf::new(),
        range: SourceRange::default(),
        detail: "Function crowded".to_string(),
        docstring: Some("Ranks before undocumented records but has no location.".to_string()),
        import_from: None,
        signature: None,
    }];
    records.extend((0..6).map(|index| SymbolRecord {
        name: "crowded".to_string(),
        kind: SymbolKind::Function,
        module: format!("provider_{index}"),
        path: root.join(format!("provider_{index}.py")),
        range: SourceRange {
            start_line: index,
            start_character: 4,
            end_line: index,
            end_character: 11,
        },
        detail: "Function crowded".to_string(),
        docstring: None,
        import_from: None,
        signature: None,
    }));
    index.symbols_by_name.insert("crowded".to_string(), records);

    let (line, character) = first_position(source, "crowded");
    let query =
        index.query_source_at_navigation(&source_path, source, QueryPosition { line, character });

    assert_eq!(query.resolution_confidence.as_deref(), Some("ambiguous"));
    assert_eq!(query.candidate_count, 7);
    assert_eq!(
        query.definition_candidates.len(),
        5,
        "a higher-ranked record without a path must not consume the candidate display limit"
    );
    assert!(query
        .definition_candidates
        .iter()
        .all(|candidate| !candidate.definition.path.as_os_str().is_empty()));
    assert_eq!(
        query
            .definition_candidates
            .iter()
            .map(|candidate| candidate.definition.module.as_str())
            .collect::<Vec<_>>(),
        vec![
            "provider_0",
            "provider_1",
            "provider_2",
            "provider_3",
            "provider_4"
        ]
    );

    fs::remove_dir_all(root).ok();
}
