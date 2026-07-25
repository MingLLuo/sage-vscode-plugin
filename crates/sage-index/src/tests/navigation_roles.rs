use super::*;

fn navigation_role_index(root: &Path) -> WorkspaceIndex {
    let mut index = WorkspaceIndex::new(IndexOptions {
        roots: vec![root.to_path_buf()],
        editable_roots: vec![root.to_path_buf()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    });
    index.rebuild().unwrap();
    index
}

fn navigation_role_snapshot(query: QueryResult) -> (Option<String>, Vec<(PathBuf, SourceRange)>) {
    let mut targets = query
        .definition
        .into_iter()
        .map(|definition| (definition.path, definition.range))
        .collect::<Vec<_>>();
    targets.extend(
        query
            .definition_candidates
            .into_iter()
            .map(|candidate| (candidate.definition.path, candidate.definition.range)),
    );
    (query.resolution_confidence, targets)
}

#[test]
fn exact_cython_method_pairs_have_role_specific_navigation_targets() {
    let root = test_root("navigation-role-method-pair");
    let declaration = root.join("provider.pxd");
    let implementation = root.join("provider.pyx");
    fs::write(
        &declaration,
        "cdef class Example:\n    cpdef target(self)\n    cpdef call(self)\n",
    )
    .unwrap();
    let source = "cdef class Example:\n    cpdef target(self):\n        return 1\n\n    cpdef call(self):\n        return self.target()\n";
    fs::write(&implementation, source).unwrap();
    let index = navigation_role_index(&root);
    let (line, character) = member_position(source, "target");

    let definition = index.query_source_at_navigation_for_role(
        &implementation,
        source,
        QueryPosition { line, character },
        NavigationTargetRole::Definition,
    );
    let declaration_query = index.query_source_at_navigation_for_role(
        &implementation,
        source,
        QueryPosition { line, character },
        NavigationTargetRole::Declaration,
    );
    let implementation_query = index.query_source_at_navigation_for_role(
        &implementation,
        source,
        QueryPosition { line, character },
        NavigationTargetRole::Implementation,
    );

    assert_eq!(definition.resolution_confidence.as_deref(), Some("high"));
    assert_eq!(
        definition
            .definition
            .as_ref()
            .map(|target| target.path.as_path()),
        Some(normalize_path(implementation.clone()).as_path())
    );
    assert_eq!(
        declaration_query.resolution_confidence.as_deref(),
        Some("high")
    );
    assert_eq!(
        declaration_query
            .definition
            .as_ref()
            .map(|target| target.path.as_path()),
        Some(normalize_path(declaration.clone()).as_path())
    );
    assert_eq!(declaration_query.candidate_count, 1);
    assert!(declaration_query
        .resolution_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("exact sibling Cython declaration")));
    assert_eq!(
        implementation_query
            .definition
            .as_ref()
            .map(|target| target.path.as_path()),
        Some(normalize_path(implementation.clone()).as_path())
    );
    assert_eq!(implementation_query.candidate_count, 1);
    assert!(declaration_query.definition_candidates.is_empty());
    assert!(implementation_query.definition_candidates.is_empty());

    fs::remove_dir_all(root).ok();
}

#[test]
fn explicit_cimport_pair_is_one_role_specific_logical_identity() {
    let root = test_root("navigation-role-cimport-pair");
    let declaration = root.join("provider.pxd");
    let implementation = root.join("provider.pyx");
    let consumer = root.join("consumer.pyx");
    fs::write(&declaration, "cdef class Example:\n    pass\n").unwrap();
    fs::write(&implementation, "cdef class Example:\n    pass\n").unwrap();
    let source = "from provider cimport Example\n\ncdef Example value\n";
    fs::write(&consumer, source).unwrap();
    let index = navigation_role_index(&root);
    let (line, character) = position_in_line(source, "cdef Example", "Example");

    let definition = index.query_source_at_navigation_for_role(
        &consumer,
        source,
        QueryPosition { line, character },
        NavigationTargetRole::Definition,
    );
    let declaration_query = index.query_source_at_navigation_for_role(
        &consumer,
        source,
        QueryPosition { line, character },
        NavigationTargetRole::Declaration,
    );
    let implementation_query = index.query_source_at_navigation_for_role(
        &consumer,
        source,
        QueryPosition { line, character },
        NavigationTargetRole::Implementation,
    );

    assert_eq!(
        definition.resolution_confidence.as_deref(),
        Some("ambiguous")
    );
    assert_eq!(definition.definition_candidates.len(), 2);
    assert_eq!(
        declaration_query.resolution_confidence.as_deref(),
        Some("high")
    );
    assert_eq!(
        declaration_query
            .definition
            .as_ref()
            .map(|target| target.path.as_path()),
        Some(normalize_path(declaration.clone()).as_path())
    );
    assert_eq!(declaration_query.candidate_count, 1);
    assert_eq!(
        implementation_query.resolution_confidence.as_deref(),
        Some("high")
    );
    assert_eq!(
        implementation_query
            .definition
            .as_ref()
            .map(|target| target.path.as_path()),
        Some(normalize_path(implementation.clone()).as_path())
    );
    assert_eq!(implementation_query.candidate_count, 1);

    fs::remove_dir_all(root).ok();
}

#[test]
fn duplicate_proven_declarations_remain_ordered_candidates() {
    let root = test_root("navigation-role-duplicate-declarations");
    let declaration = root.join("provider.pxd");
    let implementation = root.join("provider.pyx");
    fs::write(
        &declaration,
        "cdef class Example:\n    cpdef target(self)\n\ncdef class Example:\n    cpdef target(self)\n",
    )
    .unwrap();
    let source = "cdef class Example:\n    cpdef target(self):\n        return 1\n\n    cpdef call(self):\n        return self.target()\n";
    fs::write(&implementation, source).unwrap();
    let index = navigation_role_index(&root);
    let (line, character) = member_position(source, "target");

    let query = index.query_source_at_navigation_for_role(
        &implementation,
        source,
        QueryPosition { line, character },
        NavigationTargetRole::Declaration,
    );

    assert!(query.definition.is_none());
    assert_eq!(query.resolution_confidence.as_deref(), Some("ambiguous"));
    assert_eq!(query.definition_candidates.len(), 2);
    assert_eq!(
        query
            .definition_candidates
            .iter()
            .map(|candidate| candidate.definition.range.start_line)
            .collect::<Vec<_>>(),
        vec![1, 4]
    );
    assert!(query
        .definition_candidates
        .iter()
        .all(|candidate| candidate.definition.path == normalize_path(declaration.clone())));

    fs::remove_dir_all(root).ok();
}

#[test]
fn owner_mismatches_and_top_level_cython_functions_are_not_paired() {
    let root = test_root("navigation-role-conservative-boundaries");
    let owner_declaration = root.join("owner_provider.pxd");
    let owner_implementation = root.join("owner_provider.pyx");
    fs::write(
        &owner_declaration,
        "cdef class Other:\n    cpdef target(self)\n",
    )
    .unwrap();
    let owner_source = "cdef class Example:\n    cpdef target(self):\n        return 1\n\n    cpdef call(self):\n        return self.target()\n";
    fs::write(&owner_implementation, owner_source).unwrap();

    let function_declaration = root.join("function_provider.pxd");
    let function_implementation = root.join("function_provider.pyx");
    fs::write(&function_declaration, "cpdef target(int value)\n").unwrap();
    let function_source =
        "cpdef target(int value):\n    return value\n\ncpdef call():\n    return target(1)\n";
    fs::write(&function_implementation, function_source).unwrap();
    let index = navigation_role_index(&root);

    let (owner_line, owner_character) = member_position(owner_source, "target");
    let owner_query = index.query_source_at_navigation_for_role(
        &owner_implementation,
        owner_source,
        QueryPosition {
            line: owner_line,
            character: owner_character,
        },
        NavigationTargetRole::Declaration,
    );
    assert_eq!(owner_query.resolution_confidence.as_deref(), Some("high"));
    assert_eq!(
        owner_query
            .definition
            .as_ref()
            .map(|target| target.path.as_path()),
        Some(normalize_path(owner_implementation.clone()).as_path())
    );

    let (function_line, function_character) =
        position_in_line(function_source, "return target", "target");
    let function_query = index.query_source_at_navigation_for_role(
        &function_implementation,
        function_source,
        QueryPosition {
            line: function_line,
            character: function_character,
        },
        NavigationTargetRole::Declaration,
    );
    assert_eq!(
        function_query.resolution_confidence.as_deref(),
        Some("high")
    );
    assert_eq!(
        function_query
            .definition
            .as_ref()
            .map(|target| target.path.as_path()),
        Some(normalize_path(function_implementation.clone()).as_path())
    );
    assert!(function_query.definition_candidates.is_empty());

    fs::remove_dir_all(root).ok();
}

#[test]
fn mismatched_method_signatures_are_not_paired() {
    let root = test_root("navigation-role-signature-mismatch");
    let declaration = root.join("provider.pxd");
    let implementation = root.join("provider.pyx");
    fs::write(
        &declaration,
        "cdef class Example:\n    cpdef target(self, value)\n",
    )
    .unwrap();
    let source = "cdef class Example:\n    cpdef target(self):\n        return 1\n\n    cpdef call(self):\n        return self.target()\n";
    fs::write(&implementation, source).unwrap();
    let index = navigation_role_index(&root);
    let (line, character) = member_position(source, "target");

    let query = index.query_source_at_navigation_for_role(
        &implementation,
        source,
        QueryPosition { line, character },
        NavigationTargetRole::Declaration,
    );

    assert_eq!(query.resolution_confidence.as_deref(), Some("high"));
    assert_eq!(
        query
            .definition
            .as_ref()
            .map(|target| target.path.as_path()),
        Some(normalize_path(implementation.clone()).as_path())
    );
    assert!(query
        .resolution_reason
        .as_deref()
        .is_none_or(|reason| !reason.contains("exact sibling Cython")));

    fs::remove_dir_all(root).ok();
}

#[test]
fn hydrated_index_preserves_role_specific_targets_and_order() {
    let root = test_root("navigation-role-hydration");
    let declaration = root.join("provider.pxd");
    let implementation = root.join("provider.pyx");
    let consumer = root.join("consumer.pyx");
    fs::write(
        &declaration,
        "cdef class Example:\n    cpdef target(self)\n",
    )
    .unwrap();
    fs::write(
        &implementation,
        "cdef class Example:\n    cpdef target(self):\n        return 1\n",
    )
    .unwrap();
    let source = "from provider cimport Example\n\ncdef Example value\n";
    fs::write(&consumer, source).unwrap();
    let options = IndexOptions {
        roots: vec![root.clone()],
        editable_roots: vec![root.clone()],
        exclude_globs: Vec::new(),
        cache_dir: root.join(".cache"),
        enable_pyx: true,
    };
    let mut rebuilt = WorkspaceIndex::new(options.clone());
    rebuilt.rebuild().unwrap();
    let (line, character) = position_in_line(source, "cdef Example", "Example");
    let position = QueryPosition { line, character };
    let roles = [
        NavigationTargetRole::Definition,
        NavigationTargetRole::Declaration,
        NavigationTargetRole::Implementation,
    ];
    let rebuilt_targets = roles
        .into_iter()
        .map(|role| {
            navigation_role_snapshot(
                rebuilt.query_source_at_navigation_for_role(&consumer, source, position, role),
            )
        })
        .collect::<Vec<_>>();

    let mut hydrated = WorkspaceIndex::new(options);
    hydrated.hydrate_from_cache().unwrap();
    let hydrated_targets = roles
        .into_iter()
        .map(|role| {
            navigation_role_snapshot(
                hydrated.query_source_at_navigation_for_role(&consumer, source, position, role),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(hydrated_targets, rebuilt_targets);
    fs::remove_dir_all(root).ok();
}
