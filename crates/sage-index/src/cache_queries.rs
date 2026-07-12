use super::*;

pub(super) fn load_file_paths_from_db(db_path: &Path, roots: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare("select path from files order by path")?;
    let rows = statement.query_map([], |row| Ok(PathBuf::from(row.get::<_, String>(0)?)))?;
    let paths = rows
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|path| path_is_under_roots(path, roots))
        .collect();
    Ok(paths)
}

pub(super) fn load_reference_spans_from_db(
    db_path: &Path,
    name: &str,
    roots: &[PathBuf],
) -> Result<Vec<ReferenceRecord>> {
    if name.is_empty() || !db_path.exists() {
        return Ok(Vec::new());
    }
    let connection = Connection::open(db_path)?;
    create_schema(&connection)?;
    let mut statement = connection.prepare(
        "select path, start_line, start_character, end_line, end_character
         from reference_spans
         where name = ?1
         order by path, start_line, start_character",
    )?;
    let rows = statement.query_map(params![name], |row| {
        Ok(ReferenceRecord {
            path: PathBuf::from(row.get::<_, String>(0)?),
            range: SourceRange {
                start_line: row.get(1)?,
                start_character: row.get(2)?,
                end_line: row.get(3)?,
                end_character: row.get(4)?,
            },
        })
    })?;
    let mut references = Vec::new();
    for row in rows {
        let reference = row?;
        if path_is_under_roots(&reference.path, roots) {
            references.push(reference);
        }
    }
    Ok(references)
}

pub(super) fn load_file_from_db(db_path: &Path, path: &Path) -> Result<IndexedFile> {
    let connection = Connection::open(db_path)?;
    let path_text = path.display().to_string();
    let module = connection
        .query_row(
            "select module from files where path = ?1",
            params![path_text],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .with_context(|| format!("indexed file not found {}", path.display()))?;
    Ok(IndexedFile {
        module,
        path: path.to_path_buf(),
        symbols: Vec::new(),
        module_docstring: None,
    })
}

pub(super) fn load_symbols_by_name_from_db(
    db_path: &Path,
    name: &str,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where name = ?1 order by path, start_line, start_character",
    )?;
    let mut symbols = filter_symbols_to_roots(
        collect_symbol_rows(statement.query_map(params![name], symbol_from_row)?)?,
        roots,
    );
    attach_docstrings(&connection, &mut symbols)?;
    Ok(symbols)
}

pub(super) fn load_symbols_by_name_from_db_without_docs(
    db_path: &Path,
    name: &str,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where name = ?1 order by path, start_line, start_character",
    )?;
    let symbols = collect_symbol_rows(statement.query_map(params![name], symbol_from_row)?)?;
    Ok(filter_symbols_to_roots(symbols, roots))
}

pub(super) fn load_symbols_by_name_and_module_from_db(
    db_path: &Path,
    name: &str,
    module: &str,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare(
        "select s.name, s.kind, s.module, s.path, s.start_line, s.start_character, s.end_line, s.end_character, s.detail, s.import_from, s.signature, d.docstring from symbols s left join docs d on d.path = s.path and d.module = s.module and d.name = s.name and d.detail = s.detail where s.name = ?1 and s.module = ?2 order by s.path, s.start_line, s.start_character",
    )?;
    let symbols =
        collect_symbol_rows(statement.query_map(params![name, module], symbol_with_doc_from_row)?)?;
    Ok(filter_symbols_to_roots(symbols, roots))
}

pub(super) fn load_symbols_by_name_and_module_from_db_without_docs(
    db_path: &Path,
    name: &str,
    module: &str,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where name = ?1 and module = ?2 order by path, start_line, start_character",
    )?;
    let symbols =
        collect_symbol_rows(statement.query_map(params![name, module], symbol_from_row)?)?;
    Ok(filter_symbols_to_roots(symbols, roots))
}

pub(super) fn load_materialized_sage_export_groups_by_names_from_db(
    db_path: &Path,
    import_module: &str,
    names: &[String],
    roots: &[PathBuf],
) -> Result<HashMap<String, Vec<SymbolRecord>>> {
    if names.is_empty() {
        return Ok(HashMap::new());
    }
    let connection = Connection::open(db_path)?;
    let mut grouped = HashMap::new();
    for chunk in names.chunks(128) {
        let placeholders = std::iter::repeat_n("?", chunk.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "select public_name, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from sage_export_cache where import_module = ? and public_name in ({placeholders}) order by public_name"
        );
        let mut statement = connection.prepare(&sql)?;
        let params = std::iter::once(import_module).chain(chunk.iter().map(String::as_str));
        let rows = statement.query_map(params_from_iter(params), |row| {
            let public_name = row.get::<_, String>(0)?;
            let symbol = symbol_with_doc_from_row_offset(row, 1)?;
            Ok((public_name, symbol))
        })?;
        insert_export_rows_into_groups(rows, roots, &mut grouped)?;
    }
    Ok(grouped)
}

pub(super) fn load_hot_sage_export_groups_from_db(
    db_path: &Path,
    import_module: &str,
    roots: &[PathBuf],
) -> Result<HashMap<String, Vec<SymbolRecord>>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare(
        "select public_name, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from sage_export_cache where import_module = ?1 order by public_name limit ?2",
    )?;
    let rows = statement.query_map(
        params![import_module, MAX_DYNAMIC_HOT_EXPORT_NAMES as i64],
        |row| {
            let public_name = row.get::<_, String>(0)?;
            let symbol = symbol_with_doc_from_row_offset(row, 1)?;
            Ok((public_name, symbol))
        },
    )?;
    let mut grouped = HashMap::new();
    insert_export_rows_into_groups(rows, roots, &mut grouped)?;
    Ok(grouped)
}

pub(super) fn insert_export_rows_into_groups<I>(
    rows: I,
    roots: &[PathBuf],
    grouped: &mut HashMap<String, Vec<SymbolRecord>>,
) -> Result<()>
where
    I: IntoIterator<Item = rusqlite::Result<(String, SymbolRecord)>>,
{
    for row in rows {
        let (public_name, symbol) = row?;
        if !path_is_under_roots(&symbol.path, roots) {
            continue;
        }
        grouped
            .entry(public_name.to_ascii_lowercase())
            .or_default()
            .push(symbol.clone());
        grouped
            .entry(symbol.name.to_ascii_lowercase())
            .or_default()
            .push(symbol);
    }
    Ok(())
}

pub(super) fn filter_symbols_to_roots(
    symbols: Vec<SymbolRecord>,
    roots: &[PathBuf],
) -> Vec<SymbolRecord> {
    symbols
        .into_iter()
        .filter(|symbol| path_is_under_roots(&symbol.path, roots))
        .collect()
}

pub(super) fn load_symbols_for_path_from_db(
    db_path: &Path,
    path: &Path,
) -> Result<Vec<SymbolRecord>> {
    let connection = Connection::open(db_path)?;
    let path = path.display().to_string();
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where path = ?1 order by start_line, start_character",
    )?;
    let mut symbols = collect_symbol_rows(statement.query_map(params![path], symbol_from_row)?)?;
    attach_docstrings(&connection, &mut symbols)?;
    Ok(symbols)
}

pub(super) fn load_lookup_names_for_paths_from_db(
    db_path: &Path,
    paths: &BTreeSet<PathBuf>,
) -> Result<BTreeSet<String>> {
    if paths.is_empty() {
        return Ok(BTreeSet::new());
    }
    let connection = Connection::open(db_path)?;
    let mut statement =
        connection.prepare("select name, module, import_from from symbols where path = ?1")?;
    let mut names = BTreeSet::new();
    for path in paths {
        let path = path.display().to_string();
        let rows = statement.query_map(params![path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (name, module, import_from) = row?;
            names.insert(name.to_ascii_lowercase());
            if let Some(import_from) = import_from.as_deref() {
                let (_module, source_name) = import_target_in_context(import_from, &name, &module);
                names.insert(source_name.to_ascii_lowercase());
            }
        }
    }
    Ok(names)
}

pub(super) fn load_symbols_with_prefix_from_db(
    db_path: &Path,
    prefix: &str,
    limit: usize,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let connection = Connection::open(db_path)?;
    let fetch_limit = limit.saturating_mul(8).max(limit).max(64) as i64;
    let mut symbols = Vec::new();
    if prefix.is_empty() {
        let mut statement = connection.prepare(
            "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols order by name, module limit ?1",
        )?;
        symbols.extend(collect_symbol_rows(
            statement.query_map(params![fetch_limit], symbol_from_row)?,
        )?);
    } else {
        let mut range_prefixes = vec![prefix.to_string()];
        if let Some(title_prefix) = ascii_titlecase_first(prefix) {
            if title_prefix != prefix {
                range_prefixes.push(title_prefix);
            }
        }
        for range_prefix in range_prefixes {
            if let Some(upper_bound) = prefix_upper_bound(&range_prefix) {
                let mut statement = connection.prepare(
                    "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where name >= ?1 and name < ?2 order by name, module limit ?3",
                )?;
                symbols.extend(collect_symbol_rows(statement.query_map(
                    params![range_prefix, upper_bound, fetch_limit],
                    symbol_from_row,
                )?)?);
            } else {
                let mut statement = connection.prepare(
                    "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from symbols where name >= ?1 order by name, module limit ?2",
                )?;
                symbols.extend(collect_symbol_rows(
                    statement.query_map(params![range_prefix, fetch_limit], symbol_from_row)?,
                )?);
            }
        }
    }
    let prefix_lower = prefix.to_ascii_lowercase();
    let mut symbols = filter_symbols_to_roots(dedupe_symbol_records(symbols), roots)
        .into_iter()
        .filter(|symbol| {
            prefix_lower.is_empty() || symbol.name.to_ascii_lowercase().starts_with(&prefix_lower)
        })
        .collect::<Vec<_>>();
    attach_docstrings(&connection, &mut symbols)?;
    Ok(dedupe_best_symbols(symbols, limit))
}

pub(super) fn prefix_upper_bound(prefix: &str) -> Option<String> {
    let mut bytes = prefix.as_bytes().to_vec();
    for index in (0..bytes.len()).rev() {
        if bytes[index] < u8::MAX {
            bytes[index] = bytes[index].saturating_add(1);
            bytes.truncate(index + 1);
            return String::from_utf8(bytes).ok();
        }
    }
    None
}

pub(super) fn ascii_titlecase_first(prefix: &str) -> Option<String> {
    let mut chars = prefix.chars();
    let first = chars.next()?;
    if !first.is_ascii_lowercase() {
        return None;
    }
    let mut result = String::new();
    result.push(first.to_ascii_uppercase());
    result.push_str(chars.as_str());
    Some(result)
}

pub(super) fn load_workspace_symbols_from_db(
    db_path: &Path,
    query: &str,
    limit: usize,
    roots: &[PathBuf],
) -> Result<Vec<SymbolRecord>> {
    let connection = Connection::open(db_path)?;
    let query_lower = query.to_ascii_lowercase();
    let prefix_pattern = format!("{query_lower}%");
    let contains_pattern = format!("%{query_lower}%");
    let fetch_limit = limit.max(1);
    let sql_limit = fetch_limit as i64;
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature
         from symbols
         where ?1 = ''
            or lower(name) = ?1
            or lower(name) like ?2
            or lower(name) like ?3
            or lower(module) like ?3
         order by
            case
              when ?1 = '' then 3
              when lower(name) = ?1 then 0
              when lower(name) like ?2 then 1
              when lower(name) like ?3 then 3
              when lower(module) like ?3 then 4
              else 5
            end,
            case kind
              when 'Class' then 0
              when 'Function' then 0
              when 'CythonDeclaration' then 0
              when 'PreparserGenerator' then 1
              when 'Variable' then 1
              when 'Module' then 2
              else 3
            end,
            length(name),
            name,
            module
         limit ?4",
    )?;
    let mut symbols = filter_symbols_to_roots(
        collect_symbol_rows(statement.query_map(
            params![query_lower, prefix_pattern, contains_pattern, sql_limit],
            symbol_from_row,
        )?)?,
        roots,
    );
    symbols.truncate(fetch_limit);
    attach_docstrings(&connection, &mut symbols)?;
    Ok(symbols)
}

pub(super) fn collect_symbol_rows<I>(rows: I) -> Result<Vec<SymbolRecord>>
where
    I: IntoIterator<Item = rusqlite::Result<SymbolRecord>>,
{
    let mut symbols = Vec::new();
    for row in rows {
        symbols.push(row?);
    }
    Ok(symbols)
}

pub(super) fn symbol_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SymbolRecord> {
    symbol_from_row_offset(row, 0)
}

pub(super) fn symbol_from_row_offset(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<SymbolRecord> {
    let path = PathBuf::from(row.get::<_, String>(offset + 3)?);
    Ok(SymbolRecord {
        name: row.get(offset)?,
        kind: parse_symbol_kind(&row.get::<_, String>(offset + 1)?),
        module: row.get(offset + 2)?,
        path,
        range: SourceRange {
            start_line: row.get(offset + 4)?,
            start_character: row.get(offset + 5)?,
            end_line: row.get(offset + 6)?,
            end_character: row.get(offset + 7)?,
        },
        detail: row.get(offset + 8)?,
        docstring: None,
        import_from: row.get(offset + 9)?,
        signature: row.get(offset + 10)?,
    })
}

pub(super) fn symbol_with_doc_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SymbolRecord> {
    symbol_with_doc_from_row_offset(row, 0)
}

pub(super) fn symbol_with_doc_from_row_offset(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<SymbolRecord> {
    let mut symbol = symbol_from_row_offset(row, offset)?;
    symbol.docstring = row.get(offset + 11)?;
    Ok(symbol)
}

pub(super) fn attach_docstrings(
    connection: &Connection,
    symbols: &mut [SymbolRecord],
) -> Result<()> {
    let mut statement = connection.prepare(
        "select docstring from docs where path = ?1 and module = ?2 and name = ?3 and detail = ?4 limit 1",
    )?;
    for symbol in symbols {
        symbol.docstring = statement
            .query_row(
                params![
                    symbol.path.display().to_string(),
                    symbol.module,
                    symbol.name,
                    symbol.detail
                ],
                |row| row.get(0),
            )
            .optional()?;
    }
    Ok(())
}

pub(super) fn load_runtime_documentation_from_db(
    db_path: &Path,
    symbol: &str,
) -> Result<Option<DocumentationRecord>> {
    if !db_path.exists() {
        return Ok(None);
    }
    let connection = Connection::open(db_path)?;
    create_schema(&connection)?;
    let mut statement = connection.prepare(
        "select name, module_name, kind, detail, summary, docstring, uri from runtime_docs where symbol = ?1",
    )?;
    let record = statement
        .query_row(params![symbol], |row| {
            Ok(DocumentationRecord {
                name: row.get(0)?,
                module_name: row.get(1)?,
                kind: row.get(2)?,
                detail: row.get(3)?,
                summary: row.get(4)?,
                docstring: row.get(5)?,
                uri: row.get(6)?,
                markers: vec!["runtime-writeback".to_string()],
                sections: Vec::new(),
            })
        })
        .optional()?;
    Ok(record)
}

pub(super) fn upsert_runtime_documentation(
    connection: &Connection,
    symbol: &str,
    record: &DocumentationRecord,
) -> Result<()> {
    let now = UNIX_EPOCH.elapsed().unwrap_or_default().as_secs() as i64;
    connection.execute(
        "insert or replace into runtime_docs(symbol, name, module_name, kind, detail, summary, docstring, uri, updated_at) values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            symbol,
            record.name,
            record.module_name,
            record.kind,
            record.detail,
            record.summary,
            record.docstring,
            record.uri,
            now,
        ],
    )?;
    Ok(())
}

pub(super) fn load_materialized_sage_export_from_db(
    db_path: &Path,
    import_module: &str,
    name: &str,
    roots: &[PathBuf],
) -> Result<Option<SageExportResolution>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from sage_export_cache where import_module = ?1 and public_name = ?2",
    )?;
    let symbol = statement
        .query_row(params![import_module, name], symbol_with_doc_from_row)
        .optional()?;
    Ok(symbol
        .filter(|symbol| path_is_under_roots(&symbol.path, roots))
        .map(|record| SageExportResolution {
            record,
            reason: "materialized sage.all export cache",
        }))
}

pub(super) fn load_materialized_sage_method_from_db(
    db_path: &Path,
    owner_type: SageOwnerType,
    member: &str,
    roots: &[PathBuf],
) -> Result<Option<SymbolRecord>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from sage_method_cache where owner_type = ?1 and member = ?2",
    )?;
    let symbol = statement
        .query_row(
            params![owner_type.as_str(), member],
            symbol_with_doc_from_row,
        )
        .optional()?;
    Ok(symbol.filter(|symbol| path_is_under_roots(&symbol.path, roots)))
}

pub(super) fn load_materialized_sage_methods_from_db(
    db_path: &Path,
    keys: &[(SageOwnerType, &'static str)],
    roots: &[PathBuf],
) -> Result<Vec<(SageOwnerType, String, SymbolRecord)>> {
    let connection = Connection::open(db_path)?;
    let mut statement = connection.prepare(
        "select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from sage_method_cache where owner_type = ?1 and member = ?2",
    )?;
    let mut records = Vec::new();
    for (owner_type, member) in keys {
        let symbol = statement
            .query_row(
                params![owner_type.as_str(), member],
                symbol_with_doc_from_row,
            )
            .optional()?;
        if let Some(symbol) = symbol.filter(|symbol| path_is_under_roots(&symbol.path, roots)) {
            records.push((*owner_type, (*member).to_string(), symbol));
        }
    }
    Ok(records)
}

pub(super) fn load_materialized_sage_method_completions_from_db(
    db_path: &Path,
    owner_type: SageOwnerType,
    prefix: &str,
    roots: &[PathBuf],
    limit: usize,
) -> Result<Vec<(String, SymbolRecord)>> {
    let connection = Connection::open(db_path)?;
    let like_pattern = format!("{prefix}%");
    let mut statement = connection.prepare(
        "select member, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from sage_method_cache where owner_type = ?1 and member like ?2 order by member limit ?3",
    )?;
    let rows = statement.query_map(
        params![owner_type.as_str(), like_pattern, limit.saturating_mul(2)],
        |row| {
            let member: String = row.get(0)?;
            let symbol = symbol_with_doc_from_row_offset(row, 1)?;
            Ok((member, symbol))
        },
    )?;
    let mut completions = Vec::new();
    for row in rows {
        let (member, symbol) = row?;
        if path_is_under_roots(&symbol.path, roots) {
            completions.push((member, symbol));
        }
        if completions.len() >= limit {
            break;
        }
    }
    Ok(completions)
}

pub(super) fn load_sage_method_cache_stats_from_db(db_path: &Path) -> Result<SageMethodCacheStats> {
    let connection = Connection::open(db_path)?;
    let mut statement =
        connection.prepare("select coalesce(origin, 'unknown'), count(*) from sage_method_cache group by coalesce(origin, 'unknown')")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
    })?;
    let mut stats = SageMethodCacheStats::default();
    for row in rows {
        let (origin, count) = row?;
        stats.total += count;
        match origin.as_str() {
            METHOD_CACHE_ORIGIN_SOURCE_DERIVED => stats.source_derived += count,
            _ => stats.static_fallback += count,
        }
    }
    Ok(stats)
}
