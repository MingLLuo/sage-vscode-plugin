use super::*;

pub(super) fn persist_file(
    connection: &Connection,
    file: &IndexedFile,
    persist_reference_spans: bool,
) -> Result<()> {
    let path = file.path.display().to_string();
    delete_path_from_db(connection, &path)?;
    let mut file_statement = connection.prepare(
        "insert into files(path, module, fingerprint, identifier_filter) values(?1, ?2, ?3, ?4)",
    )?;
    let mut symbol_statement = connection.prepare(
        "insert into symbols(name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature) values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )?;
    let mut doc_statement = connection.prepare(
        "insert into docs(name, module, path, detail, docstring) values(?1, ?2, ?3, ?4, ?5)",
    )?;
    let mut reference_statement = connection.prepare(
        "insert into reference_spans(name, path, start_line, start_character, end_line, end_character) values(?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    let references = persist_reference_spans.then_some(&mut reference_statement);
    insert_file_rows(
        file,
        &mut file_statement,
        &mut symbol_statement,
        &mut doc_statement,
        references,
    )
}

pub(super) fn insert_file_rows(
    file: &IndexedFile,
    file_statement: &mut rusqlite::Statement<'_>,
    symbol_statement: &mut rusqlite::Statement<'_>,
    doc_statement: &mut rusqlite::Statement<'_>,
    reference_statement: Option<&mut rusqlite::Statement<'_>>,
) -> Result<()> {
    let path = file.path.display().to_string();
    let fingerprint = file_fingerprint(&file.path)?;
    file_statement.execute(params![
        path.as_str(),
        file.module.as_str(),
        fingerprint,
        file.identifier_filter.as_slice(),
    ])?;
    if let Some(docstring) = &file.module_docstring {
        doc_statement.execute(params![
            file.module.as_str(),
            file.module.as_str(),
            path.as_str(),
            "module",
            docstring.as_str()
        ])?;
    }
    for symbol in &file.symbols {
        symbol_statement.execute(params![
            symbol.name.as_str(),
            symbol_kind_as_str(&symbol.kind),
            symbol.module.as_str(),
            path.as_str(),
            symbol.range.start_line,
            symbol.range.start_character,
            symbol.range.end_line,
            symbol.range.end_character,
            symbol.detail.as_str(),
            symbol.import_from.as_deref(),
            symbol.signature.as_deref(),
        ])?;
        if let Some(docstring) = &symbol.docstring {
            doc_statement.execute(params![
                symbol.name.as_str(),
                symbol.module.as_str(),
                path.as_str(),
                symbol.detail.as_str(),
                docstring.as_str()
            ])?;
        }
    }
    if let Some(statement) = reference_statement {
        insert_reference_rows(file, statement)?;
    }
    Ok(())
}

pub(super) fn insert_reference_rows(
    file: &IndexedFile,
    statement: &mut rusqlite::Statement<'_>,
) -> Result<()> {
    let source = fs::read_to_string(&file.path)
        .with_context(|| format!("read references from {}", file.path.display()))?;
    for (name, reference) in reference_spans_in_source(&file.path, &source) {
        statement.execute(params![
            name.as_str(),
            reference.path.display().to_string(),
            reference.range.start_line,
            reference.range.start_character,
            reference.range.end_line,
            reference.range.end_character,
        ])?;
    }
    Ok(())
}

pub(super) fn delete_path_from_db(connection: &Connection, path: &str) -> Result<()> {
    connection.execute("delete from docs where path = ?1", params![path])?;
    connection.execute("delete from reference_spans where path = ?1", params![path])?;
    connection.execute("delete from symbols where path = ?1", params![path])?;
    connection.execute("delete from files where path = ?1", params![path])?;
    Ok(())
}

pub(super) fn delete_roots_from_db(connection: &Connection, roots: &[PathBuf]) -> Result<()> {
    for root in roots {
        let root_path = root.display().to_string();
        let child_path_pattern = like_pattern_for_children(&root_path);
        for table in ["docs", "reference_spans", "symbols", "files"] {
            connection.execute(
                &format!("delete from {table} where path = ?1 or path like ?2 escape '~'"),
                params![root_path, child_path_pattern],
            )?;
        }
    }
    clear_doc_fts(connection)?;
    Ok(())
}

pub(super) fn clear_doc_fts(connection: &Connection) -> Result<()> {
    connection.execute("delete from docs_fts", [])?;
    Ok(())
}

pub(super) fn create_lookup_indexes(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        r#"
        create index if not exists idx_symbols_name on symbols(name);
        create index if not exists idx_symbols_module on symbols(module);
        create index if not exists idx_symbols_path on symbols(path);
        create index if not exists idx_docs_path on docs(path);
        create index if not exists idx_docs_symbol on docs(path, module, name, detail);
        create index if not exists idx_reference_spans_name on reference_spans(name);
        create index if not exists idx_reference_spans_path on reference_spans(path);
        create index if not exists idx_sage_export_cache_path on sage_export_cache(path);
        create index if not exists idx_sage_method_cache_path on sage_method_cache(path);
        "#,
    )?;
    Ok(())
}

pub(super) fn like_pattern_for_children(root_path: &str) -> String {
    let mut value = String::new();
    for character in root_path
        .chars()
        .chain(std::iter::once(std::path::MAIN_SEPARATOR))
    {
        match character {
            '~' | '%' | '_' => {
                value.push('~');
                value.push(character);
            }
            _ => value.push(character),
        }
    }
    value.push('%');
    value
}

pub(super) fn parse_symbol_kind(value: &str) -> SymbolKind {
    match value {
        "Module" => SymbolKind::Module,
        "Class" => SymbolKind::Class,
        "Function" => SymbolKind::Function,
        "Variable" => SymbolKind::Variable,
        "Import" => SymbolKind::Import,
        "CythonDeclaration" => SymbolKind::CythonDeclaration,
        "PreparserGenerator" => SymbolKind::PreparserGenerator,
        _ => SymbolKind::Variable,
    }
}

pub(super) fn symbol_kind_as_str(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Module => "Module",
        SymbolKind::Class => "Class",
        SymbolKind::Function => "Function",
        SymbolKind::Variable => "Variable",
        SymbolKind::Import => "Import",
        SymbolKind::CythonDeclaration => "CythonDeclaration",
        SymbolKind::PreparserGenerator => "PreparserGenerator",
    }
}

pub(super) fn module_matches_import(module: &str, import_from: &str) -> bool {
    module == import_from || module.ends_with(&format!(".{import_from}"))
}

pub(super) fn import_target_definition_matches(
    candidate: &SymbolRecord,
    source_module: &str,
    source_name: &str,
) -> bool {
    if candidate.kind == SymbolKind::Import {
        return false;
    }
    if candidate.kind == SymbolKind::Module {
        return candidate.module == source_module
            || (candidate.name == source_name
                && candidate.module == format!("{source_module}.{source_name}"));
    }
    candidate.name == source_name && module_matches_import(&candidate.module, source_module)
}

pub(super) fn module_basename(module: &str) -> &str {
    module.rsplit('.').next().unwrap_or(module)
}

pub(super) fn is_namespace_owner_record(record: &SymbolRecord) -> bool {
    matches!(record.kind, SymbolKind::Module | SymbolKind::Variable)
}

pub(super) fn namespace_member_matches_owner(
    candidate: &SymbolRecord,
    owner_record: &SymbolRecord,
    member: &str,
) -> bool {
    if candidate.name != member {
        return false;
    }
    if candidate.module == owner_record.module {
        return true;
    }
    candidate.kind == SymbolKind::Module
        && owner_record.kind == SymbolKind::Module
        && candidate.module == format!("{}.{}", owner_record.module, member)
}

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        r#"
        pragma journal_mode = wal;
        create table if not exists files(
          path text primary key,
          module text not null,
          fingerprint text not null,
          identifier_filter blob not null default X''
        );
        create table if not exists symbols(
          name text not null,
          kind text not null,
          module text not null,
          path text not null,
          start_line integer not null,
          start_character integer not null,
          end_line integer not null,
          end_character integer not null,
          detail text not null,
          import_from text,
          signature text
        );
        create table if not exists docs(
          name text not null,
          module text not null,
          path text not null,
          detail text not null,
          docstring text not null
        );
        create table if not exists reference_spans(
          name text not null,
          path text not null,
          start_line integer not null,
          start_character integer not null,
          end_line integer not null,
          end_character integer not null
        );
        create virtual table if not exists docs_fts using fts5(name, module, docstring);
        create table if not exists runtime_docs(
          symbol text primary key,
          name text not null,
          module_name text not null,
          kind text not null,
          detail text not null,
          summary text not null,
          docstring text,
          uri text,
          updated_at integer not null
        );
        create table if not exists index_root_metadata(
          root text primary key,
          file_count integer not null,
          symbol_count integer not null,
          doc_count integer not null,
          updated_at integer not null,
          root_fingerprint text,
          root_marker text
        );
        create table if not exists sage_export_cache(
          public_name text not null,
          source_name text not null,
          import_module text not null,
          reason text not null,
          name text not null,
          kind text not null,
          module text not null,
          path text not null,
          start_line integer not null,
          start_character integer not null,
          end_line integer not null,
          end_character integer not null,
          detail text not null,
          import_from text,
          signature text,
          docstring text,
          primary key(import_module, public_name)
        );
        create table if not exists sage_method_cache(
          owner_type text not null,
          member text not null,
          origin text not null default 'unknown',
          name text not null,
          kind text not null,
          module text not null,
          path text not null,
          start_line integer not null,
          start_character integer not null,
          end_line integer not null,
          end_character integer not null,
          detail text not null,
          import_from text,
          signature text,
          docstring text,
          primary key(owner_type, member)
        );
        "#,
    )?;
    create_lookup_indexes(connection)?;
    ensure_column(connection, "symbols", "import_from", "text")?;
    ensure_column(connection, "symbols", "signature", "text")?;
    ensure_column(
        connection,
        "files",
        "identifier_filter",
        "blob not null default X''",
    )?;
    ensure_column(
        connection,
        "sage_method_cache",
        "origin",
        "text not null default 'unknown'",
    )?;
    ensure_column(
        connection,
        "index_root_metadata",
        "root_fingerprint",
        "text",
    )?;
    ensure_column(connection, "index_root_metadata", "root_marker", "text")?;
    Ok(())
}

pub(super) fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    column_type: &str,
) -> Result<()> {
    let mut statement = connection.prepare(&format!("pragma table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(());
        }
    }
    connection.execute(
        &format!("alter table {table} add column {column} {column_type}"),
        [],
    )?;
    Ok(())
}
