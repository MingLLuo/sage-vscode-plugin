use super::*;

pub(super) fn refresh_materialized_caches(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<()> {
    connection.execute("delete from sage_export_cache", [])?;
    connection.execute("delete from sage_method_cache", [])?;
    refresh_materialized_export_cache(connection, roots)?;
    refresh_materialized_method_cache(connection, roots)?;
    Ok(())
}

pub(super) fn refresh_materialized_caches_from_symbols(
    connection: &Connection,
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
) -> Result<()> {
    connection.execute("delete from sage_export_cache", [])?;
    connection.execute("delete from sage_method_cache", [])?;
    refresh_materialized_export_cache_from_symbols(connection, symbols_by_name)?;
    refresh_materialized_method_cache_from_symbols(connection, symbols_by_name)?;
    Ok(())
}

pub(super) fn refresh_materialized_export_cache_from_symbols(
    connection: &Connection,
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
) -> Result<()> {
    let mut statement = connection.prepare(
        "insert or replace into sage_export_cache(public_name, source_name, import_module, reason, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring) values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
    )?;
    let mut exports_by_module = BTreeMap::<String, BTreeMap<String, SymbolRecord>>::new();
    for import_symbol in symbols_by_name
        .values()
        .flat_map(|symbols| symbols.iter())
        .filter(|symbol| {
            symbol.kind == SymbolKind::Import && module_is_sage_all_export_module(&symbol.module)
        })
    {
        if is_star_import_symbol(import_symbol) || is_all_export_symbol(import_symbol) {
            continue;
        }
        if let Some(record) = resolve_import_symbol_from_symbol_map(
            symbols_by_name,
            import_symbol,
            0,
            &mut BTreeSet::new(),
        ) {
            insert_export_cache_row(
                &mut statement,
                &import_symbol.name,
                &import_symbol.module,
                "indexed sage.all re-export chain",
                &record,
            )?;
            exports_by_module
                .entry(import_symbol.module.clone())
                .or_default()
                .entry(import_symbol.name.clone())
                .or_insert(record);
        }
    }
    let star_edges = sage_all_star_import_edges_from_symbol_map(symbols_by_name);
    populate_star_source_exports_from_symbol_map(
        &mut exports_by_module,
        symbols_by_name,
        &star_edges,
    );
    insert_star_re_exports_from_modules(&mut statement, &mut exports_by_module, &star_edges)?;
    insert_static_sage_export_fallbacks_from_symbol_map(
        &mut statement,
        &mut exports_by_module,
        symbols_by_name,
    )?;
    Ok(())
}

pub(super) fn refresh_materialized_method_cache_from_symbols(
    connection: &Connection,
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
) -> Result<()> {
    let mut statement = connection.prepare(
        "insert or replace into sage_method_cache(owner_type, member, origin, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring) values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
    )?;
    let mut source_derived_keys = BTreeSet::new();
    for (owner_type, member, record) in source_derived_method_records_from_symbols(symbols_by_name)
    {
        source_derived_keys.insert((owner_type, member.clone()));
        insert_method_cache_row(
            &mut statement,
            owner_type,
            &member,
            METHOD_CACHE_ORIGIN_SOURCE_DERIVED,
            &record,
        )?;
    }
    for spec in SAGE_METHOD_SPECS {
        if source_derived_keys.contains(&(spec.owner_type, spec.member.to_string())) {
            continue;
        }
        if let Some(record) = best_symbol_by_name_and_module_from_symbol_map(
            symbols_by_name,
            spec.member,
            spec.module,
        ) {
            insert_method_cache_row(
                &mut statement,
                spec.owner_type,
                spec.member,
                METHOD_CACHE_ORIGIN_STATIC_SPEC,
                &record,
            )?;
        }
    }
    for spec in SAGE_METHOD_ALIAS_SPECS {
        if source_derived_keys.contains(&(spec.owner_type, spec.member.to_string())) {
            continue;
        }
        if let Some(record) = best_symbol_by_name_and_module_from_symbol_map(
            symbols_by_name,
            spec.source_name,
            spec.module,
        ) {
            insert_method_cache_row(
                &mut statement,
                spec.owner_type,
                spec.member,
                METHOD_CACHE_ORIGIN_STATIC_ALIAS,
                &record,
            )?;
        }
    }
    Ok(())
}

pub(super) fn best_symbol_by_name_and_module_from_symbol_map(
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
    name: &str,
    module: &str,
) -> Option<SymbolRecord> {
    symbols_by_name
        .get(&name.to_ascii_lowercase())?
        .iter()
        .filter(|symbol| import_target_definition_matches(symbol, module, name))
        .min_by_key(|symbol| symbol_choice_key(symbol))
        .cloned()
}

pub(super) fn source_derived_method_records_from_symbols(
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
) -> Vec<(SageOwnerType, String, SymbolRecord)> {
    let mut best = BTreeMap::<(SageOwnerType, String), (SageMethodChoiceKey, SymbolRecord)>::new();
    for symbol in symbols_by_name.values().flat_map(|symbols| symbols.iter()) {
        let Some(owner) = source_derived_method_owner_for_symbol(symbol) else {
            continue;
        };
        let key = (owner.owner_type, symbol.name.clone());
        let choice_key = sage_method_choice_key(owner.priority, symbol);
        match best.get(&key) {
            Some((existing_key, _)) if *existing_key <= choice_key => {}
            _ => {
                best.insert(key, (choice_key, symbol.clone()));
            }
        }
    }
    for (owner_type, member, record) in
        source_derived_method_alias_records_from_symbols(symbols_by_name)
    {
        let key = (owner_type, member);
        let choice_key = sage_method_choice_key(0, &record);
        match best.get(&key) {
            Some((existing_key, _)) if *existing_key <= choice_key => {}
            _ => {
                best.insert(key, (choice_key, record));
            }
        }
    }
    best.into_iter()
        .map(|((owner_type, member), (_, record))| (owner_type, member, record))
        .collect()
}

pub(super) fn source_derived_method_alias_records_from_symbols(
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
) -> Vec<(SageOwnerType, String, SymbolRecord)> {
    let mut records: Vec<_> = symbols_by_name
        .values()
        .flat_map(|symbols| symbols.iter())
        .filter(|symbol| symbol.kind == SymbolKind::Import)
        .filter_map(|alias_symbol| {
            let (class_name, alias, target) =
                class_method_alias_detail_parts(&alias_symbol.detail)?;
            let owner_type = sage_owner_type_from_class_name(class_name, &alias_symbol.module)?;
            let target_detail = format!("Method {class_name}.{target}");
            let target_record = symbols_by_name
                .get(&target.to_ascii_lowercase())?
                .iter()
                .filter(|symbol| {
                    symbol.module == alias_symbol.module
                        && symbol.detail == target_detail
                        && is_source_derived_sage_method(symbol)
                })
                .min_by_key(|symbol| symbol_choice_key(symbol))
                .cloned()?;
            Some((owner_type, alias.to_string(), target_record))
        })
        .collect();
    records.extend(
        symbols_by_name
            .values()
            .flat_map(|symbols| symbols.iter())
            .filter(|symbol| symbol.kind == SymbolKind::Import)
            .filter_map(|alias_symbol| {
                let (alias, target) =
                    matrix_constructor_method_alias_detail_parts(&alias_symbol.detail)?;
                let target_record = symbols_by_name
                    .get(&target.to_ascii_lowercase())?
                    .iter()
                    .filter(|symbol| {
                        symbol.module == alias_symbol.module
                            && symbol.name == target
                            && symbol.kind != SymbolKind::Import
                    })
                    .min_by_key(|symbol| symbol_choice_key(symbol))
                    .cloned()?;
                Some((
                    SageOwnerType::MatrixConstructor,
                    alias.to_string(),
                    target_record,
                ))
            }),
    );
    records
}

pub(super) fn sage_all_star_import_edges_from_symbol_map(
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
) -> Vec<(String, String)> {
    symbols_by_name
        .values()
        .flat_map(|symbols| symbols.iter())
        .filter(|symbol| module_is_sage_all_export_module(&symbol.module))
        .filter_map(|symbol| {
            star_import_source_module(symbol)
                .map(|source_module| (symbol.module.clone(), source_module))
        })
        .collect()
}

pub(super) fn sage_all_star_import_edges_from_symbols(
    symbols: &[SymbolRecord],
) -> Vec<(String, String)> {
    symbols
        .iter()
        .filter(|symbol| module_is_sage_all_export_module(&symbol.module))
        .filter_map(|symbol| {
            star_import_source_module(symbol)
                .map(|source_module| (symbol.module.clone(), source_module))
        })
        .collect()
}

pub(super) fn insert_star_re_exports_from_modules(
    statement: &mut rusqlite::Statement<'_>,
    exports_by_module: &mut BTreeMap<String, BTreeMap<String, SymbolRecord>>,
    star_edges: &[(String, String)],
) -> Result<()> {
    for _ in 0..MAX_IMPORT_RESOLUTION_DEPTH {
        let mut pending = Vec::new();
        for (import_module, source_module) in star_edges {
            let Some(source_exports) = exports_by_module.get(source_module) else {
                continue;
            };
            for (public_name, record) in source_exports {
                if exports_by_module
                    .get(import_module)
                    .is_some_and(|exports| exports.contains_key(public_name))
                {
                    continue;
                }
                pending.push((import_module.clone(), public_name.clone(), record.clone()));
            }
        }
        if pending.is_empty() {
            break;
        }
        for (import_module, public_name, record) in pending {
            let inserted = exports_by_module
                .entry(import_module.clone())
                .or_default()
                .insert(public_name.clone(), record.clone())
                .is_none();
            if inserted {
                insert_export_cache_row(
                    statement,
                    &public_name,
                    &import_module,
                    "indexed sage.all star re-export",
                    &record,
                )?;
            }
        }
    }
    Ok(())
}

pub(super) fn populate_star_source_exports_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
    exports_by_module: &mut BTreeMap<String, BTreeMap<String, SymbolRecord>>,
    star_edges: &[(String, String)],
) -> Result<()> {
    for source_module in star_edges.iter().map(|(_, source)| source) {
        if exports_by_module.contains_key(source_module) {
            continue;
        }
        let exports = public_module_exports_from_connection(connection, roots, source_module)?;
        if !exports.is_empty() {
            exports_by_module.insert(source_module.clone(), exports);
        }
    }
    Ok(())
}

pub(super) fn public_module_exports_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
    module: &str,
) -> Result<BTreeMap<String, SymbolRecord>> {
    let symbols = load_symbols_by_module_from_connection(connection, module, roots)?;
    let explicit_names = explicit_all_names_from_symbols(symbols.iter());
    let mut exports = BTreeMap::<String, (SageMethodChoiceKey, SymbolRecord)>::new();
    for symbol in symbols {
        if !is_star_namespace_export_candidate(&symbol, explicit_names.as_ref()) {
            continue;
        }
        let record = if symbol.kind == SymbolKind::Import {
            resolve_import_symbol_from_connection(
                connection,
                &symbol,
                roots,
                0,
                &mut BTreeSet::new(),
            )?
        } else {
            Some(symbol.clone())
        };
        let Some(record) = record else {
            continue;
        };
        let key = sage_method_choice_key(0, &record);
        match exports.get(&symbol.name) {
            Some((existing_key, _)) if *existing_key <= key => {}
            _ => {
                exports.insert(symbol.name.clone(), (key, record));
            }
        }
    }
    Ok(exports
        .into_iter()
        .map(|(public_name, (_, record))| (public_name, record))
        .collect())
}

pub(super) fn populate_star_source_exports_from_symbol_map(
    exports_by_module: &mut BTreeMap<String, BTreeMap<String, SymbolRecord>>,
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
    star_edges: &[(String, String)],
) {
    for source_module in star_edges.iter().map(|(_, source)| source) {
        if exports_by_module.contains_key(source_module) {
            continue;
        }
        let exports = public_module_exports_from_symbol_map(symbols_by_name, source_module);
        if !exports.is_empty() {
            exports_by_module.insert(source_module.clone(), exports);
        }
    }
}

pub(super) fn public_module_exports_from_symbol_map(
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
    module: &str,
) -> BTreeMap<String, SymbolRecord> {
    let module_symbols: Vec<&SymbolRecord> = symbols_by_name
        .values()
        .flat_map(|symbols| symbols.iter())
        .filter(|symbol| symbol.module == module)
        .collect();
    let explicit_names = explicit_all_names_from_symbols(module_symbols.iter().copied());
    let mut exports = BTreeMap::<String, (SageMethodChoiceKey, SymbolRecord)>::new();
    for symbol in module_symbols {
        if !is_star_namespace_export_candidate(symbol, explicit_names.as_ref()) {
            continue;
        }
        let record = if symbol.kind == SymbolKind::Import {
            resolve_import_symbol_from_symbol_map(symbols_by_name, symbol, 0, &mut BTreeSet::new())
        } else {
            Some(symbol.clone())
        };
        let Some(record) = record else {
            continue;
        };
        let key = sage_method_choice_key(0, &record);
        match exports.get(&symbol.name) {
            Some((existing_key, _)) if *existing_key <= key => {}
            _ => {
                exports.insert(symbol.name.clone(), (key, record));
            }
        }
    }
    exports
        .into_iter()
        .map(|(public_name, (_, record))| (public_name, record))
        .collect()
}

pub(super) fn insert_static_sage_export_fallbacks_from_symbol_map(
    statement: &mut rusqlite::Statement<'_>,
    exports_by_module: &mut BTreeMap<String, BTreeMap<String, SymbolRecord>>,
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
) -> Result<()> {
    for target in SAGE_EXPORT_MAP {
        if exports_by_module
            .get(target.import_module)
            .is_some_and(|exports| exports.contains_key(target.name))
        {
            continue;
        }
        let Some(record) = best_symbol_by_name_and_module_from_symbol_map(
            symbols_by_name,
            target.source_name,
            target.source_module,
        ) else {
            continue;
        };
        insert_export_cache_row(
            statement,
            target.name,
            target.import_module,
            "built-in sage.all export fallback",
            &record,
        )?;
        exports_by_module
            .entry(target.import_module.to_string())
            .or_default()
            .insert(target.name.to_string(), record);
    }
    Ok(())
}

pub(super) fn resolve_import_symbol_from_symbol_map(
    symbols_by_name: &HashMap<String, Vec<SymbolRecord>>,
    symbol: &SymbolRecord,
    depth: usize,
    seen: &mut BTreeSet<String>,
) -> Option<SymbolRecord> {
    if symbol.kind != SymbolKind::Import || depth >= MAX_IMPORT_RESOLUTION_DEPTH {
        return None;
    }
    let import_from = symbol.import_from.as_ref()?;
    let (source_module, source_name) =
        import_target_in_context(import_from, &symbol.name, &symbol.module);
    if !seen.insert(format!("{source_module}::{source_name}")) {
        return None;
    }
    let candidates = symbols_by_name.get(&source_name.to_ascii_lowercase())?;
    if let Some(definition) = candidates
        .iter()
        .filter(|candidate| {
            import_target_definition_matches(candidate, &source_module, &source_name)
        })
        .min_by_key(|candidate| symbol_choice_key(candidate))
        .cloned()
    {
        return Some(definition);
    }
    let next_import = candidates
        .iter()
        .filter(|candidate| {
            candidate.kind == SymbolKind::Import
                && candidate.name == source_name
                && module_matches_import(&candidate.module, &source_module)
        })
        .min_by_key(|candidate| symbol_choice_key(candidate))?;
    resolve_import_symbol_from_symbol_map(symbols_by_name, next_import, depth + 1, seen)
        .or_else(|| Some(next_import.clone()))
}

pub(super) fn insert_method_cache_row(
    statement: &mut rusqlite::Statement<'_>,
    owner_type: SageOwnerType,
    member: &str,
    origin: &str,
    record: &SymbolRecord,
) -> Result<()> {
    statement.execute(params![
        owner_type.as_str(),
        member,
        origin,
        record.name.as_str(),
        symbol_kind_as_str(&record.kind),
        record.module.as_str(),
        record.path.display().to_string(),
        record.range.start_line,
        record.range.start_character,
        record.range.end_line,
        record.range.end_character,
        record.detail.as_str(),
        record.import_from.as_deref(),
        record.signature.as_deref(),
        record.docstring.as_deref(),
    ])?;
    Ok(())
}

pub(super) fn insert_export_cache_row(
    statement: &mut rusqlite::Statement<'_>,
    public_name: &str,
    import_module: &str,
    reason: &str,
    record: &SymbolRecord,
) -> Result<()> {
    statement.execute(params![
        public_name,
        record.name.as_str(),
        import_module,
        reason,
        record.name.as_str(),
        symbol_kind_as_str(&record.kind),
        record.module.as_str(),
        record.path.display().to_string(),
        record.range.start_line,
        record.range.start_character,
        record.range.end_line,
        record.range.end_character,
        record.detail.as_str(),
        record.import_from.as_deref(),
        record.signature.as_deref(),
        record.docstring.as_deref(),
    ])?;
    Ok(())
}

pub(super) fn refresh_materialized_export_cache(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<()> {
    let dynamic_imports = load_sage_export_imports_from_connection(connection, roots)?;
    let mut statement = connection.prepare(
        "insert or replace into sage_export_cache(public_name, source_name, import_module, reason, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring) values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
    )?;
    let mut exports_by_module = BTreeMap::<String, BTreeMap<String, SymbolRecord>>::new();
    for import_symbol in &dynamic_imports {
        if is_star_import_symbol(import_symbol) || is_all_export_symbol(import_symbol) {
            continue;
        }
        if let Some(record) = resolve_import_symbol_from_connection(
            connection,
            import_symbol,
            roots,
            0,
            &mut BTreeSet::new(),
        )? {
            insert_export_cache_row(
                &mut statement,
                &import_symbol.name,
                &import_symbol.module,
                "indexed sage.all re-export chain",
                &record,
            )?;
            exports_by_module
                .entry(import_symbol.module.clone())
                .or_default()
                .entry(import_symbol.name.clone())
                .or_insert(record);
        }
    }
    let star_edges = sage_all_star_import_edges_from_symbols(&dynamic_imports);
    populate_star_source_exports_from_connection(
        connection,
        roots,
        &mut exports_by_module,
        &star_edges,
    )?;
    insert_star_re_exports_from_modules(&mut statement, &mut exports_by_module, &star_edges)?;
    insert_static_sage_export_fallbacks_from_connection(
        connection,
        roots,
        &mut statement,
        &mut exports_by_module,
    )?;
    Ok(())
}

pub(super) fn insert_static_sage_export_fallbacks_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
    statement: &mut rusqlite::Statement<'_>,
    exports_by_module: &mut BTreeMap<String, BTreeMap<String, SymbolRecord>>,
) -> Result<()> {
    for target in SAGE_EXPORT_MAP {
        if exports_by_module
            .get(target.import_module)
            .is_some_and(|exports| exports.contains_key(target.name))
        {
            continue;
        }
        let Some(record) = load_best_symbol_by_name_and_module_from_connection(
            connection,
            target.source_name,
            target.source_module,
            roots,
        )?
        else {
            continue;
        };
        insert_export_cache_row(
            statement,
            target.name,
            target.import_module,
            "built-in sage.all export fallback",
            &record,
        )?;
        exports_by_module
            .entry(target.import_module.to_string())
            .or_default()
            .insert(target.name.to_string(), record);
    }
    Ok(())
}

pub(super) fn refresh_materialized_method_cache(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<()> {
    let mut statement = connection.prepare(
        "insert or replace into sage_method_cache(owner_type, member, origin, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring) values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
    )?;
    let mut source_derived_keys = BTreeSet::new();
    for (owner_type, member, record) in
        source_derived_method_records_from_connection(connection, roots)?
    {
        source_derived_keys.insert((owner_type, member.clone()));
        insert_method_cache_row(
            &mut statement,
            owner_type,
            &member,
            METHOD_CACHE_ORIGIN_SOURCE_DERIVED,
            &record,
        )?;
    }
    for spec in SAGE_METHOD_SPECS {
        if source_derived_keys.contains(&(spec.owner_type, spec.member.to_string())) {
            continue;
        }
        if let Some(record) = load_best_symbol_by_name_and_module_from_connection(
            connection,
            spec.member,
            spec.module,
            roots,
        )? {
            insert_method_cache_row(
                &mut statement,
                spec.owner_type,
                spec.member,
                METHOD_CACHE_ORIGIN_STATIC_SPEC,
                &record,
            )?;
        }
    }
    for spec in SAGE_METHOD_ALIAS_SPECS {
        if source_derived_keys.contains(&(spec.owner_type, spec.member.to_string())) {
            continue;
        }
        if let Some(record) = load_best_symbol_by_name_and_module_from_connection(
            connection,
            spec.source_name,
            spec.module,
            roots,
        )? {
            insert_method_cache_row(
                &mut statement,
                spec.owner_type,
                spec.member,
                METHOD_CACHE_ORIGIN_STATIC_ALIAS,
                &record,
            )?;
        }
    }
    Ok(())
}

pub(super) fn source_derived_method_records_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<Vec<(SageOwnerType, String, SymbolRecord)>> {
    let mut best = BTreeMap::<(SageOwnerType, String), (SageMethodChoiceKey, SymbolRecord)>::new();
    for symbol in load_class_context_method_symbols_from_connection(connection, roots)? {
        let Some(owner) = source_derived_method_owner_for_symbol(&symbol) else {
            continue;
        };
        let key = (owner.owner_type, symbol.name.clone());
        let choice_key = sage_method_choice_key(owner.priority, &symbol);
        match best.get(&key) {
            Some((existing_key, _)) if *existing_key <= choice_key => {}
            _ => {
                best.insert(key, (choice_key, symbol));
            }
        }
    }
    for alias_symbol in load_class_method_alias_symbols_from_connection(connection, roots)? {
        let Some((class_name, alias, target)) =
            class_method_alias_detail_parts(&alias_symbol.detail)
        else {
            continue;
        };
        let Some(owner_type) = sage_owner_type_from_class_name(class_name, &alias_symbol.module)
        else {
            continue;
        };
        let Some(record) = load_class_method_alias_target_from_connection(
            connection,
            roots,
            &alias_symbol.module,
            class_name,
            target,
        )?
        else {
            continue;
        };
        let key = (owner_type, alias.to_string());
        let choice_key = sage_method_choice_key(0, &record);
        match best.get(&key) {
            Some((existing_key, _)) if *existing_key <= choice_key => {}
            _ => {
                best.insert(key, (choice_key, record));
            }
        }
    }
    for alias_symbol in
        load_matrix_constructor_method_alias_symbols_from_connection(connection, roots)?
    {
        let Some((alias, target)) =
            matrix_constructor_method_alias_detail_parts(&alias_symbol.detail)
        else {
            continue;
        };
        let Some(record) = load_best_symbol_by_name_and_module_from_connection(
            connection,
            target,
            &alias_symbol.module,
            roots,
        )?
        else {
            continue;
        };
        let key = (SageOwnerType::MatrixConstructor, alias.to_string());
        let choice_key = sage_method_choice_key(0, &record);
        match best.get(&key) {
            Some((existing_key, _)) if *existing_key <= choice_key => {}
            _ => {
                best.insert(key, (choice_key, record));
            }
        }
    }
    for module_spec in SAGE_OWNER_METHOD_MODULES {
        let mut symbols =
            load_method_like_symbols_for_owner_module(connection, module_spec, roots)?;
        for symbol in symbols.drain(..) {
            let Some(owner) = source_derived_method_owner_for_symbol(&symbol) else {
                continue;
            };
            let key = (owner.owner_type, symbol.name.clone());
            let choice_key = sage_method_choice_key(owner.priority, &symbol);
            match best.get(&key) {
                Some((existing_key, _)) if *existing_key <= choice_key => {}
                _ => {
                    best.insert(key, (choice_key, symbol));
                }
            }
        }
    }
    Ok(best
        .into_iter()
        .map(|((owner_type, member), (_, record))| (owner_type, member, record))
        .collect())
}

pub(super) fn load_class_context_method_symbols_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let mut statement = connection.prepare(
        "select s.name, s.kind, s.module, s.path, s.start_line, s.start_character, s.end_line, s.end_character, s.detail, s.import_from, s.signature, d.docstring from symbols s left join docs d on d.path = s.path and d.module = s.module and d.name = s.name and d.detail = s.detail where s.kind != 'Import' and s.signature is not null and s.detail like 'Method %' order by s.module, s.name",
    )?;
    let symbols = collect_symbol_rows(statement.query_map([], symbol_with_doc_from_row)?)?
        .into_iter()
        .filter(|symbol| path_is_under_roots(&symbol.path, roots))
        .filter(|symbol| source_derived_method_owner_for_symbol(symbol).is_some())
        .collect();
    Ok(symbols)
}

pub(super) fn load_class_method_alias_symbols_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where kind = 'Import' and detail like 'MethodAlias %' order by module, name",
    )?;
    let symbols = collect_symbol_rows(statement.query_map([], symbol_from_row)?)?
        .into_iter()
        .filter(|symbol| path_is_under_roots(&symbol.path, roots))
        .collect();
    Ok(symbols)
}

pub(super) fn load_matrix_constructor_method_alias_symbols_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where kind = 'Import' and detail like 'MatrixConstructorMethodAlias %' order by module, name",
    )?;
    let symbols = collect_symbol_rows(statement.query_map([], symbol_from_row)?)?
        .into_iter()
        .filter(|symbol| path_is_under_roots(&symbol.path, roots))
        .collect();
    Ok(symbols)
}

pub(super) fn load_class_method_alias_target_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
    module: &str,
    class_name: &str,
    target: &str,
) -> Result<Option<SymbolRecord>> {
    let target_detail = format!("Method {class_name}.{target}");
    let mut statement = connection.prepare(
        "select s.name, s.kind, s.module, s.path, s.start_line, s.start_character, s.end_line, s.end_character, s.detail, s.import_from, s.signature, d.docstring from symbols s left join docs d on d.path = s.path and d.module = s.module and d.name = s.name and d.detail = s.detail where s.module = ?1 and s.name = ?2 and s.detail = ?3 and s.signature is not null order by s.path, s.start_line, s.start_character",
    )?;
    let symbols = collect_symbol_rows(statement.query_map(
        params![module, target, target_detail],
        symbol_with_doc_from_row,
    )?)?;
    Ok(symbols
        .into_iter()
        .filter(|symbol| {
            path_is_under_roots(&symbol.path, roots) && is_source_derived_sage_method(symbol)
        })
        .min_by_key(symbol_choice_key))
}

pub(super) fn load_method_like_symbols_for_owner_module(
    connection: &Connection,
    module_spec: &SageOwnerModuleSpec,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let mut statement = if module_spec.recursive {
        connection.prepare(
            "select s.name, s.kind, s.module, s.path, s.start_line, s.start_character, s.end_line, s.end_character, s.detail, s.import_from, s.signature, d.docstring from symbols s left join docs d on d.path = s.path and d.module = s.module and d.name = s.name and d.detail = s.detail where s.kind != 'Import' and s.signature is not null and (s.module = ?1 or s.module like ?2) order by s.module, s.name",
        )?
    } else {
        connection.prepare(
            "select s.name, s.kind, s.module, s.path, s.start_line, s.start_character, s.end_line, s.end_character, s.detail, s.import_from, s.signature, d.docstring from symbols s left join docs d on d.path = s.path and d.module = s.module and d.name = s.name and d.detail = s.detail where s.kind != 'Import' and s.signature is not null and s.module = ?1 order by s.module, s.name",
        )?
    };
    let module_pattern = format!("{}.%", module_spec.module);
    let rows = if module_spec.recursive {
        statement.query_map(
            params![module_spec.module, module_pattern],
            symbol_with_doc_from_row,
        )?
    } else {
        statement.query_map(params![module_spec.module], symbol_with_doc_from_row)?
    };
    let symbols = collect_symbol_rows(rows)?
        .into_iter()
        .filter(|symbol| {
            path_is_under_roots(&symbol.path, roots) && is_source_derived_sage_method(symbol)
        })
        .collect();
    Ok(symbols)
}

pub(super) fn load_sage_export_imports_from_connection(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where kind = 'Import' and (module = 'sage.all' or module like 'sage.%.all') order by module, name",
    )?;
    let symbols = collect_symbol_rows(statement.query_map([], symbol_from_row)?)?;
    Ok(filter_symbols_to_roots(symbols, roots))
}

pub(super) fn load_best_symbol_by_name_and_module_from_connection(
    connection: &Connection,
    name: &str,
    module: &str,
    roots: &[PathBuf],
) -> Result<Option<SymbolRecord>> {
    let symbols = load_symbols_by_name_from_connection(connection, name, roots)?;
    Ok(symbols
        .into_iter()
        .filter(|symbol| import_target_definition_matches(symbol, module, name))
        .min_by_key(symbol_choice_key))
}

pub(super) fn load_symbols_by_module_from_connection(
    connection: &Connection,
    module: &str,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let mut statement = connection.prepare(
        "select s.name, s.kind, s.module, s.path, s.start_line, s.start_character, s.end_line, s.end_character, s.detail, s.import_from, s.signature, d.docstring from symbols s left join docs d on d.path = s.path and d.module = s.module and d.name = s.name and d.detail = s.detail where s.module = ?1 order by s.name, s.path, s.start_line, s.start_character",
    )?;
    let symbols =
        collect_symbol_rows(statement.query_map(params![module], symbol_with_doc_from_row)?)?;
    Ok(filter_symbols_to_roots(symbols, roots))
}

pub(super) fn load_symbols_by_name_from_connection(
    connection: &Connection,
    name: &str,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let mut statement = connection.prepare(
        "select s.name, s.kind, s.module, s.path, s.start_line, s.start_character, s.end_line, s.end_character, s.detail, s.import_from, s.signature, d.docstring from symbols s left join docs d on d.path = s.path and d.module = s.module and d.name = s.name and d.detail = s.detail where s.name = ?1 order by s.path, s.start_line, s.start_character",
    )?;
    let symbols =
        collect_symbol_rows(statement.query_map(params![name], symbol_with_doc_from_row)?)?;
    Ok(filter_symbols_to_roots(symbols, roots))
}

pub(super) fn resolve_import_symbol_from_connection(
    connection: &Connection,
    symbol: &SymbolRecord,
    roots: &[PathBuf],
    depth: usize,
    seen: &mut BTreeSet<String>,
) -> Result<Option<SymbolRecord>> {
    if symbol.kind != SymbolKind::Import || depth >= MAX_IMPORT_RESOLUTION_DEPTH {
        return Ok(None);
    }
    let Some(import_from) = symbol.import_from.as_ref() else {
        return Ok(None);
    };
    let (source_module, source_name) =
        import_target_in_context(import_from, &symbol.name, &symbol.module);
    if !seen.insert(format!("{source_module}::{source_name}")) {
        return Ok(None);
    }
    let candidates = load_symbols_by_name_from_connection(connection, &source_name, roots)?;
    if let Some(definition) = candidates
        .iter()
        .filter(|candidate| {
            import_target_definition_matches(candidate, &source_module, &source_name)
        })
        .min_by_key(|candidate| symbol_choice_key(candidate))
        .cloned()
    {
        return Ok(Some(definition));
    }
    let Some(next_import) = candidates
        .iter()
        .filter(|candidate| {
            candidate.kind == SymbolKind::Import
                && candidate.name == source_name
                && module_matches_import(&candidate.module, &source_module)
        })
        .min_by_key(|candidate| symbol_choice_key(candidate))
        .cloned()
    else {
        return Ok(None);
    };
    Ok(
        resolve_import_symbol_from_connection(connection, &next_import, roots, depth + 1, seen)?
            .or(Some(next_import)),
    )
}
