use super::*;

pub(super) fn tune_cache_connection(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        r#"
        pragma synchronous = off;
        pragma temp_store = memory;
        pragma cache_size = -200000;
        "#,
    )?;
    Ok(())
}

pub(super) fn cached_counts_for_roots(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<(usize, usize, usize)> {
    if let Some(counts) = load_cached_counts_from_metadata_partial(connection, roots)? {
        return Ok(counts);
    }
    cached_counts_for_roots_by_path_scan(connection, roots)
}

pub(super) fn verified_cached_counts_for_roots(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<(usize, usize, usize)> {
    let metadata_counts = load_cached_counts_from_metadata_partial(connection, roots)?;
    let actual_counts = actual_cached_counts_for_roots(connection, roots)?;
    if let Some(metadata_counts) = metadata_counts {
        if metadata_counts != actual_counts {
            bail!(
                "cache metadata counts {metadata_counts:?} do not match stored rows {actual_counts:?}"
            );
        }
    }
    Ok(actual_counts)
}

pub(super) fn actual_cached_counts_for_roots(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<(usize, usize, usize)> {
    let mut counts = (0usize, 0usize, 0usize);
    for root in metadata_count_roots(roots) {
        let root_counts = count_rows_under_root(connection, &root.display().to_string())?;
        counts.0 = counts.0.saturating_add(root_counts.0);
        counts.1 = counts.1.saturating_add(root_counts.1);
        counts.2 = counts.2.saturating_add(root_counts.2);
    }
    Ok(counts)
}

pub(super) fn cached_counts_for_roots_by_path_scan(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<(usize, usize, usize)> {
    let files = load_file_fingerprints_from_db(connection, roots)?;
    let paths: BTreeSet<String> = files
        .keys()
        .map(|path| path.display().to_string())
        .collect();
    if paths.is_empty() {
        return Ok((0, 0, 0));
    }
    let symbol_count = count_paths(connection, "symbols", &paths)?;
    let doc_count = count_docs_for_paths(connection, &paths)?;
    Ok((paths.len(), symbol_count, doc_count))
}

pub(super) fn load_cached_counts_from_metadata(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<Option<(usize, usize, usize)>> {
    let roots = metadata_count_roots(roots);
    if roots.is_empty() {
        return Ok(None);
    }
    let Ok(mut statement) = connection.prepare(
        "select file_count, symbol_count, doc_count from index_root_metadata where root = ?1",
    ) else {
        return Ok(None);
    };
    let mut file_count = 0usize;
    let mut symbol_count = 0usize;
    let mut doc_count = 0usize;
    for root in &roots {
        let root_text = root.display().to_string();
        let Some((files, symbols, docs)) = statement
            .query_row(params![root_text], |row| {
                Ok((
                    row.get::<_, usize>(0)?,
                    row.get::<_, usize>(1)?,
                    row.get::<_, usize>(2)?,
                ))
            })
            .optional()?
        else {
            return Ok(None);
        };
        file_count = file_count.saturating_add(files);
        symbol_count = symbol_count.saturating_add(symbols);
        doc_count = doc_count.saturating_add(docs);
    }
    Ok(Some((file_count, symbol_count, doc_count)))
}

pub(super) fn load_cached_counts_from_metadata_partial(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<Option<(usize, usize, usize)>> {
    let roots = metadata_count_roots(roots);
    if roots.is_empty() {
        return Ok(None);
    }
    let Ok(mut statement) = connection.prepare(
        "select file_count, symbol_count, doc_count from index_root_metadata where root = ?1",
    ) else {
        return Ok(None);
    };
    let mut file_count = 0usize;
    let mut symbol_count = 0usize;
    let mut doc_count = 0usize;
    let mut missing_roots = Vec::new();
    let mut found_metadata = false;
    for root in &roots {
        let root_text = root.display().to_string();
        let Some((files, symbols, docs)) = statement
            .query_row(params![root_text], |row| {
                Ok((
                    row.get::<_, usize>(0)?,
                    row.get::<_, usize>(1)?,
                    row.get::<_, usize>(2)?,
                ))
            })
            .optional()?
        else {
            missing_roots.push(root.clone());
            continue;
        };
        found_metadata = true;
        file_count = file_count.saturating_add(files);
        symbol_count = symbol_count.saturating_add(symbols);
        doc_count = doc_count.saturating_add(docs);
    }
    if !missing_roots.is_empty() {
        if !found_metadata {
            return Ok(None);
        }
        let (files, symbols, docs) =
            cached_counts_for_roots_by_path_scan(connection, &missing_roots)?;
        file_count = file_count.saturating_add(files);
        symbol_count = symbol_count.saturating_add(symbols);
        doc_count = doc_count.saturating_add(docs);
    }
    Ok(Some((file_count, symbol_count, doc_count)))
}

pub(super) fn metadata_count_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut count_roots = Vec::<PathBuf>::new();
    for root in normalize_paths(roots.to_vec()) {
        if count_roots.iter().any(|kept| root.starts_with(kept)) {
            continue;
        }
        count_roots.retain(|kept| !kept.starts_with(&root));
        count_roots.push(root);
    }
    count_roots
}

pub(super) fn peer_cache_paths(cache_dir: &Path, current_db_path: &Path) -> Result<Vec<PathBuf>> {
    let current_name = current_db_path.file_name().and_then(|name| name.to_str());
    let mut paths = Vec::new();
    for entry in fs::read_dir(cache_dir)
        .with_context(|| format!("read cache dir {}", cache_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if current_name == Some(name) {
            continue;
        }
        if name.starts_with("sage-index-") && name.ends_with(".sqlite") {
            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok();
            paths.push((path, modified));
        }
    }
    paths.sort_by(|(left_path, left_modified), (right_path, right_modified)| {
        right_modified
            .cmp(left_modified)
            .then_with(|| left_path.cmp(right_path))
    });
    Ok(paths.into_iter().map(|(path, _)| path).collect())
}

pub(super) fn seed_shared_roots_from_peer_cache(
    connection: &mut Connection,
    peer_path: &Path,
    roots: &[PathBuf],
) -> Result<usize> {
    let peer_path_text = peer_path.display().to_string();
    connection.execute("attach database ?1 as peer_seed", params![peer_path_text])?;
    let result = seed_shared_roots_from_attached_peer(connection, roots);
    let detach_result = connection.execute("detach database peer_seed", []);
    result.and_then(|imported| {
        detach_result?;
        Ok(imported)
    })
}

pub(super) fn seed_shared_roots_from_attached_peer(
    connection: &mut Connection,
    roots: &[PathBuf],
) -> Result<usize> {
    let tx = connection.transaction()?;
    let mut imported = 0usize;
    for root in roots {
        let current_fingerprint = source_root_fingerprint(root);
        if metadata_matches_current_root(
            root_metadata_for_schema(&tx, "", root)?,
            &current_fingerprint,
        ) {
            continue;
        }
        let peer_metadata = root_metadata_for_schema(&tx, "peer_seed.", root)?;
        if !metadata_matches_current_root(peer_metadata, &current_fingerprint) {
            continue;
        }
        imported = imported.saturating_add(copy_root_from_attached_peer(&tx, root)?);
    }
    tx.commit()?;
    Ok(imported)
}

pub(super) fn root_metadata_for_schema(
    connection: &Connection,
    schema_prefix: &str,
    root: &Path,
) -> Result<Option<(usize, Option<String>)>> {
    let sql = format!(
        "select file_count, root_fingerprint from {schema_prefix}index_root_metadata where root = ?1"
    );
    let root_text = root.display().to_string();
    connection
        .query_row(&sql, params![root_text], |row| {
            Ok((row.get::<_, usize>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .optional()
        .map_err(Into::into)
}

pub(super) fn schema_table_has_column(
    connection: &Connection,
    schema_prefix: &str,
    table: &str,
    column: &str,
) -> Result<bool> {
    let sql = match schema_prefix.strip_suffix('.') {
        Some(schema) if !schema.is_empty() => format!("pragma {schema}.table_info({table})"),
        _ => format!("pragma table_info({table})"),
    };
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn shared_roots_are_seeded(connection: &Connection, roots: &[PathBuf]) -> Result<bool> {
    for root in roots {
        let current_fingerprint = source_root_fingerprint(root);
        if !metadata_matches_current_root(
            root_metadata_for_schema(connection, "", root)?,
            &current_fingerprint,
        ) {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn metadata_matches_current_root(
    metadata: Option<(usize, Option<String>)>,
    current: &SourceRootFingerprint,
) -> bool {
    let Some((file_count, cached_digest)) = metadata else {
        return false;
    };
    if file_count == 0 {
        return false;
    }
    cached_digest
        .filter(|digest| !digest.is_empty())
        .is_none_or(|digest| digest == current.digest)
}

pub(super) fn copy_root_from_attached_peer(connection: &Connection, root: &Path) -> Result<usize> {
    let root_text = root.display().to_string();
    let child_pattern = like_pattern_for_children(&root_text);
    for table in ["docs", "reference_spans", "symbols", "files"] {
        connection.execute(
            &format!("delete from {table} where path = ?1 or path like ?2 escape '~'"),
            params![root_text.as_str(), child_pattern.as_str()],
        )?;
    }
    connection.execute(
        "delete from sage_export_cache where path = ?1 or path like ?2 escape '~'",
        params![root_text.as_str(), child_pattern.as_str()],
    )?;
    connection.execute(
        "delete from sage_method_cache where path = ?1 or path like ?2 escape '~'",
        params![root_text.as_str(), child_pattern.as_str()],
    )?;

    connection.execute(
        "insert into files(path, module, fingerprint)
         select path, module, fingerprint from peer_seed.files
         where path = ?1 or path like ?2 escape '~'",
        params![root_text.as_str(), child_pattern.as_str()],
    )?;
    connection.execute(
        "insert into symbols(name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature)
         select name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature from peer_seed.symbols
         where path = ?1 or path like ?2 escape '~'",
        params![root_text.as_str(), child_pattern.as_str()],
    )?;
    connection.execute(
        "insert into docs(name, module, path, detail, docstring)
         select name, module, path, detail, docstring from peer_seed.docs
         where path = ?1 or path like ?2 escape '~'",
        params![root_text.as_str(), child_pattern.as_str()],
    )?;
    connection.execute(
        "insert into reference_spans(name, path, start_line, start_character, end_line, end_character)
         select name, path, start_line, start_character, end_line, end_character from peer_seed.reference_spans
         where path = ?1 or path like ?2 escape '~'",
        params![root_text.as_str(), child_pattern.as_str()],
    )?;
    connection.execute(
        "insert or replace into sage_export_cache(public_name, source_name, import_module, reason, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring)
         select public_name, source_name, import_module, reason, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from peer_seed.sage_export_cache
         where path = ?1 or path like ?2 escape '~'",
        params![root_text.as_str(), child_pattern.as_str()],
    )?;
    if schema_table_has_column(connection, "peer_seed.", "sage_method_cache", "origin")? {
        connection.execute(
            "insert or replace into sage_method_cache(owner_type, member, origin, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring)
             select owner_type, member, origin, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from peer_seed.sage_method_cache
             where path = ?1 or path like ?2 escape '~'",
            params![root_text.as_str(), child_pattern.as_str()],
        )?;
    } else {
        connection.execute(
            "insert or replace into sage_method_cache(owner_type, member, origin, name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring)
             select owner_type, member, 'unknown', name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature, docstring from peer_seed.sage_method_cache
             where path = ?1 or path like ?2 escape '~'",
            params![root_text.as_str(), child_pattern.as_str()],
        )?;
    }
    connection.execute(
        "insert or replace into index_root_metadata(root, file_count, symbol_count, doc_count, updated_at, root_fingerprint, root_marker)
         select root, file_count, symbol_count, doc_count, updated_at, root_fingerprint, root_marker from peer_seed.index_root_metadata
         where root = ?1",
        params![root_text.as_str()],
    )?;

    count_files_under_root(connection, root)
}

pub(super) fn count_files_under_root(connection: &Connection, root: &Path) -> Result<usize> {
    let root_text = root.display().to_string();
    let child_pattern = like_pattern_for_children(&root_text);
    connection
        .query_row(
            "select count(*) from files where path = ?1 or path like ?2 escape '~'",
            params![root_text, child_pattern],
            |row| row.get::<_, usize>(0),
        )
        .map_err(Into::into)
}

pub(super) fn load_root_fingerprint_mismatches_from_metadata(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<Vec<StaleSourceRootFingerprint>> {
    load_root_fingerprint_status_from_metadata(connection, roots).map(|(_, mismatches)| mismatches)
}

pub(super) fn load_root_fingerprint_status_from_metadata(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<(Vec<SourceRootFingerprint>, Vec<StaleSourceRootFingerprint>)> {
    if roots.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let Ok(mut statement) = connection
        .prepare("select root_fingerprint, root_marker from index_root_metadata where root = ?1")
    else {
        return Ok((source_root_fingerprints_for_roots(roots), Vec::new()));
    };
    let mut fingerprints = Vec::new();
    let mut mismatches = Vec::new();
    for root in roots {
        let root_text = root.display().to_string();
        let current = source_root_fingerprint(root);
        fingerprints.push(current.clone());
        let Some((cached_digest, cached_marker)) = statement
            .query_row(params![root_text], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            })
            .optional()?
        else {
            continue;
        };
        let Some(cached_digest) = cached_digest.filter(|digest| !digest.is_empty()) else {
            continue;
        };
        if cached_digest != current.digest {
            mismatches.push(StaleSourceRootFingerprint {
                root: root_text,
                cached_digest,
                current_digest: current.digest,
                cached_marker,
                current_marker: current.marker,
            });
        }
    }
    Ok((fingerprints, mismatches))
}

pub(super) fn update_root_metadata(connection: &Connection, roots: &[PathBuf]) -> Result<()> {
    let now = UNIX_EPOCH.elapsed().unwrap_or_default().as_secs() as i64;
    let mut statement = connection.prepare(
        "insert or replace into index_root_metadata(root, file_count, symbol_count, doc_count, updated_at, root_fingerprint, root_marker) values(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    for root in roots {
        let root_text = root.display().to_string();
        let (file_count, symbol_count, doc_count) = count_rows_under_root(connection, &root_text)?;
        let fingerprint = source_root_fingerprint(root);
        statement.execute(params![
            root_text,
            file_count as i64,
            symbol_count as i64,
            doc_count as i64,
            now,
            fingerprint.digest,
            fingerprint.marker,
        ])?;
    }
    Ok(())
}

pub(super) fn metadata_deltas_for_path_refresh(
    connection: &Connection,
    changed: &[IndexedFile],
    deleted: &[PathBuf],
    roots: &[PathBuf],
) -> Result<BTreeMap<String, (i64, i64, i64)>> {
    let mut deltas = BTreeMap::<String, (i64, i64, i64)>::new();
    for path in deleted {
        let old = count_rows_for_path(connection, path)?;
        for root in roots.iter().filter(|root| path.starts_with(root)) {
            add_metadata_delta(&mut deltas, root, -old.0, -old.1, -old.2);
        }
    }
    for file in changed {
        let old = count_rows_for_path(connection, &file.path)?;
        let new = counts_for_indexed_file(file);
        for root in roots.iter().filter(|root| file.path.starts_with(root)) {
            add_metadata_delta(
                &mut deltas,
                root,
                new.0 - old.0,
                new.1 - old.1,
                new.2 - old.2,
            );
        }
    }
    Ok(deltas)
}

pub(super) fn update_root_metadata_with_deltas(
    connection: &Connection,
    roots: &[PathBuf],
    deltas: BTreeMap<String, (i64, i64, i64)>,
) -> Result<()> {
    if deltas.is_empty() {
        return Ok(());
    }
    let now = UNIX_EPOCH.elapsed().unwrap_or_default().as_secs() as i64;
    let mut update_statement = connection.prepare(
        "update index_root_metadata set file_count = max(file_count + ?2, 0), symbol_count = max(symbol_count + ?3, 0), doc_count = max(doc_count + ?4, 0), updated_at = ?5, root_fingerprint = ?6, root_marker = ?7 where root = ?1",
    )?;
    for (root_text, (file_delta, symbol_delta, doc_delta)) in deltas {
        let root = roots
            .iter()
            .find(|candidate| candidate.display().to_string() == root_text);
        let Some(root) = root else {
            continue;
        };
        let fingerprint = source_root_fingerprint(root);
        let changed = update_statement.execute(params![
            root_text,
            file_delta,
            symbol_delta,
            doc_delta,
            now,
            fingerprint.digest,
            fingerprint.marker,
        ])?;
        if changed == 0 {
            update_root_metadata(connection, std::slice::from_ref(root))?;
        }
    }
    Ok(())
}

pub(super) fn add_metadata_delta(
    deltas: &mut BTreeMap<String, (i64, i64, i64)>,
    root: &Path,
    file_delta: i64,
    symbol_delta: i64,
    doc_delta: i64,
) {
    let entry = deltas
        .entry(root.display().to_string())
        .or_insert((0, 0, 0));
    entry.0 += file_delta;
    entry.1 += symbol_delta;
    entry.2 += doc_delta;
}

pub(super) fn count_rows_for_path(connection: &Connection, path: &Path) -> Result<(i64, i64, i64)> {
    let path = path.display().to_string();
    let file_count = connection.query_row(
        "select count(*) from files where path = ?1",
        params![path.as_str()],
        |row| row.get::<_, i64>(0),
    )?;
    let symbol_count = connection.query_row(
        "select count(*) from symbols where path = ?1",
        params![path.as_str()],
        |row| row.get::<_, i64>(0),
    )?;
    let doc_count = connection.query_row(
        "select count(*) from docs where path = ?1 and detail != 'module'",
        params![path.as_str()],
        |row| row.get::<_, i64>(0),
    )?;
    Ok((file_count, symbol_count, doc_count))
}

pub(super) fn counts_for_indexed_file(file: &IndexedFile) -> (i64, i64, i64) {
    (
        1,
        file.symbols.len() as i64,
        file.symbols
            .iter()
            .filter(|symbol| symbol.docstring.as_ref().is_some_and(|doc| !doc.is_empty()))
            .count() as i64,
    )
}

pub(super) fn count_rows_under_root(
    connection: &Connection,
    root: &str,
) -> Result<(usize, usize, usize)> {
    let (child_start, child_end) = child_path_range(root);
    let file_count = connection.query_row(
        "select
           (select count(*) from files where path = ?1) +
           (select count(*) from files where path >= ?2 and path < ?3 and path != ?1)",
        params![root, child_start, child_end],
        |row| row.get::<_, usize>(0),
    )?;
    let symbol_count = connection.query_row(
        "select
           (select count(*) from symbols where path = ?1) +
           (select count(*) from symbols where path >= ?2 and path < ?3 and path != ?1)",
        params![root, child_start, child_end],
        |row| row.get::<_, usize>(0),
    )?;
    let doc_count = connection.query_row(
        "select
           (select count(*) from docs where path = ?1 and detail != 'module') +
           (select count(*) from docs
            where path >= ?2 and path < ?3 and path != ?1 and detail != 'module')",
        params![root, child_start, child_end],
        |row| row.get::<_, usize>(0),
    )?;
    Ok((file_count, symbol_count, doc_count))
}

fn child_path_range(root: &str) -> (String, String) {
    let separator = std::path::MAIN_SEPARATOR;
    let child_start = if root.ends_with(separator) {
        root.to_string()
    } else {
        format!("{root}{separator}")
    };
    let next_separator = char::from_u32(separator as u32 + 1)
        .expect("the platform path separator must have a following Unicode scalar");
    let child_end = format!(
        "{}{}",
        child_start
            .strip_suffix(separator)
            .expect("child path prefix must end with the platform separator"),
        next_separator
    );
    (child_start, child_end)
}

pub(super) fn load_file_fingerprints_from_db(
    connection: &Connection,
    roots: &[PathBuf],
) -> Result<BTreeMap<PathBuf, String>> {
    let mut statement = connection.prepare("select path, fingerprint from files order by path")?;
    let rows = statement.query_map([], |row| {
        Ok((
            PathBuf::from(row.get::<_, String>(0)?),
            row.get::<_, String>(1)?,
        ))
    })?;
    let mut fingerprints = BTreeMap::new();
    for row in rows {
        let (path, fingerprint) = row?;
        if path_is_under_roots(&path, roots) {
            fingerprints.insert(path, fingerprint);
        }
    }
    Ok(fingerprints)
}

pub(super) fn count_paths(
    connection: &Connection,
    table: &str,
    paths: &BTreeSet<String>,
) -> Result<usize> {
    let mut statement =
        connection.prepare(&format!("select count(*) from {table} where path = ?1"))?;
    let mut count = 0usize;
    for path in paths {
        count =
            count.saturating_add(statement.query_row(params![path], |row| row.get::<_, usize>(0))?);
    }
    Ok(count)
}

pub(super) fn count_docs_for_paths(
    connection: &Connection,
    paths: &BTreeSet<String>,
) -> Result<usize> {
    let mut statement =
        connection.prepare("select count(*) from docs where path = ?1 and detail != 'module'")?;
    let mut count = 0usize;
    for path in paths {
        count =
            count.saturating_add(statement.query_row(params![path], |row| row.get::<_, usize>(0))?);
    }
    Ok(count)
}
