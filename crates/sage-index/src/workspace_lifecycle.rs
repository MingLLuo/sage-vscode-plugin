use super::*;

impl WorkspaceIndex {
    pub fn new(mut options: IndexOptions) -> Self {
        options.roots = normalize_existing_paths(options.roots);
        options.editable_roots = normalize_existing_paths(options.editable_roots);
        let digest =
            cache_namespace_digest(&options.roots, &options.exclude_globs, options.enable_pyx);
        let db_path = options
            .cache_dir
            .join(format!("sage-index-{digest}.sqlite"));
        Self {
            options,
            db_path,
            ..Self::default()
        }
    }

    pub fn options(&self) -> &IndexOptions {
        &self.options
    }

    pub fn ensure_generation_after(&mut self, previous_generation: u64) {
        self.generation = self.generation.max(previous_generation.saturating_add(1));
    }

    pub fn clone_for_background_work(&self) -> Self {
        let mut clone = self.clone();
        clone.symbol_lookup_cache = Arc::new(Mutex::new(
            self.symbol_lookup_cache
                .lock()
                .map(|cache| cache.clone())
                .unwrap_or_default(),
        ));
        clone.file_lookup_cache = Arc::new(Mutex::new(
            self.file_lookup_cache
                .lock()
                .map(|cache| cache.clone())
                .unwrap_or_default(),
        ));
        clone.sage_method_lookup_cache = Arc::new(Mutex::new(
            self.sage_method_lookup_cache
                .lock()
                .map(|cache| cache.clone())
                .unwrap_or_default(),
        ));
        clone.reference_lookup_cache = Arc::new(Mutex::new(
            self.reference_lookup_cache
                .lock()
                .map(|cache| cache.clone())
                .unwrap_or_default(),
        ));
        clone
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn rebuild(&mut self) -> Result<IndexStatus> {
        let started = Instant::now();
        self.reset_operation_timings("rebuild");
        self.ensure_cache_dir()?;
        let paths = collect_indexable_paths(&self.options);
        let parsed: Vec<IndexedFile> = paths
            .par_iter()
            .filter_map(|path| parse_file_for_roots(path, &self.options.roots).ok())
            .collect();

        let mut files = BTreeMap::new();
        let mut symbols_by_name: HashMap<String, Vec<SymbolRecord>> = HashMap::new();
        for file in parsed {
            for symbol in &file.symbols {
                symbols_by_name
                    .entry(symbol.name.to_ascii_lowercase())
                    .or_default()
                    .push(symbol.clone());
            }
            files.insert(file.path.clone(), file);
        }

        self.files = files;
        self.symbols_by_name = symbols_by_name;
        self.generation = self.generation.saturating_add(1);
        self.last_index_ms = started.elapsed().as_millis();
        self.loaded_roots = self.options.roots.clone();
        self.cached_file_count = 0;
        self.cached_symbol_count = 0;
        self.cached_doc_count = 0;
        self.cached_root_fingerprint_mismatches.clear();
        self.clear_lookup_cache();
        self.last_error = None;
        let persist_started = Instant::now();
        if let Err(error) = self.persist_all() {
            let primary_error = error.to_string();
            if let Err(fallback_error) = self.persist_all_with_fallback() {
                self.last_error = Some(format!(
                    "{primary_error}; fallback cache failed: {fallback_error}"
                ));
            }
        }
        self.last_persist_ms = persist_started.elapsed().as_millis();
        Ok(self.status())
    }

    pub fn reconcile_with_cache(&mut self) -> Result<IndexStatus> {
        let started = Instant::now();
        self.reset_operation_timings("reconcile");
        self.ensure_cache_dir()?;
        self.seed_shared_roots_from_peer_caches();
        if self.cached_file_count > 0 && self.db_path.exists() {
            if let Ok(connection) = Connection::open(&self.db_path) {
                if let Ok(Some((file_count, symbol_count, doc_count))) =
                    load_cached_counts_from_metadata(&connection, &self.options.roots)
                {
                    let mismatches = load_root_fingerprint_mismatches_from_metadata(
                        &connection,
                        &self.options.roots,
                    )
                    .unwrap_or_default();
                    if mismatches.is_empty() && file_count > 0 {
                        self.files.clear();
                        self.symbols_by_name.clear();
                        self.clear_lookup_cache();
                        self.loaded_roots = self.options.roots.clone();
                        self.cached_file_count = file_count;
                        self.cached_symbol_count = symbol_count;
                        self.cached_doc_count = doc_count;
                        self.cached_root_fingerprint_mismatches.clear();
                        self.cache_hit_count = self.cache_hit_count.saturating_add(file_count);
                        self.last_persist_ms = 0;
                        let hot_started = Instant::now();
                        self.prewarm_hot_symbol_cache(false);
                        self.last_hot_cache_ms = hot_started.elapsed().as_millis();
                        self.last_reconcile_ms = started.elapsed().as_millis();
                        self.last_index_ms = self.last_reconcile_ms;
                        self.last_operation = Some("fast-reconcile".to_string());
                        self.generation = self.generation.saturating_add(1);
                        self.last_error = None;
                        return Ok(self.status());
                    }
                    self.cached_root_fingerprint_mismatches = mismatches;
                }
            }
        }
        let current_paths = collect_indexable_paths(&self.options);
        let current_path_set: BTreeSet<PathBuf> = current_paths.iter().cloned().collect();
        let cached_fingerprints = self.load_cached_fingerprints_for_current_roots()?;
        let mut unchanged_count = 0usize;
        let mut changed_paths = Vec::new();

        for path in &current_paths {
            let current_fingerprint = match file_fingerprint(path) {
                Ok(fingerprint) => fingerprint,
                Err(_) => {
                    changed_paths.push(path.clone());
                    continue;
                }
            };
            match cached_fingerprints.get(path) {
                Some(cached_fingerprint) if cached_fingerprint == &current_fingerprint => {
                    unchanged_count = unchanged_count.saturating_add(1);
                }
                _ => changed_paths.push(path.clone()),
            }
        }

        let deleted_paths: Vec<PathBuf> = cached_fingerprints
            .keys()
            .filter(|path| !current_path_set.contains(*path))
            .cloned()
            .collect();

        let changed_files: Vec<IndexedFile> = changed_paths
            .par_iter()
            .filter_map(|path| parse_file_for_roots(path, &self.options.roots).ok())
            .collect();

        self.files.clear();
        self.symbols_by_name.clear();
        self.clear_lookup_cache();
        self.loaded_roots = self.options.roots.clone();
        self.last_index_ms = started.elapsed().as_millis();
        self.generation = self.generation.saturating_add(1);
        self.last_error = None;

        let persist_started = Instant::now();
        let mut refresh_materialized = false;
        if changed_files.is_empty() && deleted_paths.is_empty() {
            self.last_persist_ms = 0;
        } else {
            let materialize_from_changed = deleted_paths.is_empty()
                && !changed_files.is_empty()
                && changed_files.len() == current_paths.len();
            refresh_materialized = materialize_from_changed
                || paths_need_materialized_cache_refresh(
                    &changed_files,
                    &deleted_paths,
                    &self.options.roots,
                );
            if let Err(error) = self.persist_paths(
                &changed_files,
                &deleted_paths,
                materialize_from_changed,
                refresh_materialized,
            ) {
                let primary_error = error.to_string();
                if let Err(fallback_error) = self.rebuild_into_fallback_cache() {
                    self.last_error = Some(format!(
                        "{primary_error}; fallback cache failed: {fallback_error}"
                    ));
                }
            }
            self.last_persist_ms = persist_started.elapsed().as_millis();
        }

        if let Ok((file_count, symbol_count, doc_count)) = self.cached_counts_for_current_roots() {
            self.cached_file_count = file_count;
            self.cached_symbol_count = symbol_count;
            self.cached_doc_count = doc_count;
        }
        self.cached_root_fingerprint_mismatches.clear();
        self.cache_hit_count = self.cache_hit_count.saturating_add(unchanged_count);
        self.cache_miss_count = self
            .cache_miss_count
            .saturating_add(changed_paths.len().saturating_add(deleted_paths.len()));
        let hot_started = Instant::now();
        self.prewarm_hot_symbol_cache(refresh_materialized);
        self.last_hot_cache_ms = hot_started.elapsed().as_millis();
        self.last_reconcile_ms = started.elapsed().as_millis();
        Ok(self.status())
    }

    pub fn hydrate_from_cache(&mut self) -> Result<IndexStatus> {
        let started = Instant::now();
        self.reset_operation_timings("hydrate");
        self.ensure_cache_dir()?;
        if !self.db_path.exists() {
            self.cache_miss_count = self.cache_miss_count.saturating_add(1);
            self.last_index_ms = started.elapsed().as_millis();
            self.last_hydrate_ms = self.last_index_ms;
            return Ok(self.status());
        }
        let connection = match Connection::open(&self.db_path) {
            Ok(connection) => connection,
            Err(error) => {
                if self.switch_to_fallback_cache().is_ok() && self.db_path.exists() {
                    if let Ok(connection) = Connection::open(&self.db_path) {
                        return self.hydrate_from_connection(started, connection);
                    }
                }
                self.cache_miss_count = self.cache_miss_count.saturating_add(1);
                self.last_error = Some(error.to_string());
                self.last_operation = Some("hydrate".to_string());
                self.last_hydrate_ms = started.elapsed().as_millis();
                self.last_index_ms = self.last_hydrate_ms;
                return Ok(self.status());
            }
        };
        self.hydrate_from_connection(started, connection)
    }

    fn hydrate_from_connection(
        &mut self,
        started: Instant,
        connection: Connection,
    ) -> Result<IndexStatus> {
        let (file_count, symbol_count, doc_count) =
            match cached_counts_for_roots(&connection, &self.options.roots) {
                Ok(counts) => counts,
                Err(error) => {
                    self.cache_miss_count = self.cache_miss_count.saturating_add(1);
                    self.last_error = Some(error.to_string());
                    self.last_hydrate_ms = started.elapsed().as_millis();
                    self.last_index_ms = self.last_hydrate_ms;
                    return Ok(self.status());
                }
            };
        if file_count == 0 {
            self.cache_miss_count = self.cache_miss_count.saturating_add(1);
        } else {
            self.cache_hit_count = self.cache_hit_count.saturating_add(file_count);
        }
        self.files.clear();
        self.symbols_by_name.clear();
        self.clear_lookup_cache();
        self.cached_file_count = file_count;
        self.cached_symbol_count = symbol_count;
        self.cached_doc_count = doc_count;
        let (source_root_fingerprints, stale_source_roots) =
            load_root_fingerprint_status_from_metadata(&connection, &self.options.roots)
                .unwrap_or_else(|_| {
                    (
                        source_root_fingerprints_for_roots(&self.options.roots),
                        Vec::new(),
                    )
                });
        self.source_root_fingerprints = source_root_fingerprints;
        self.cached_root_fingerprint_mismatches = stale_source_roots;
        self.loaded_roots = self.options.roots.clone();
        self.last_hydrate_ms = started.elapsed().as_millis();
        self.last_index_ms = self.last_hydrate_ms;
        Ok(self.status())
    }

    pub fn refresh_paths(
        &mut self,
        changed: &[PathBuf],
        deleted: &[PathBuf],
    ) -> Result<IndexStatus> {
        let started = Instant::now();
        self.reset_operation_timings("refresh");
        let mut changed_files = Vec::new();
        let mut dirty_lookup_names = BTreeSet::new();
        let cache_backed =
            self.cached_file_count > 0 || self.cached_symbol_count > 0 || self.cached_doc_count > 0;
        let mut lookup_cache_read_failed = false;
        if cache_backed
            && self.cached_counts_for_current_roots().ok()
                != Some((
                    self.cached_file_count,
                    self.cached_symbol_count,
                    self.cached_doc_count,
                ))
        {
            return self.rebuild();
        }
        let changed = normalize_existing_paths(changed.to_vec());
        let deleted = normalize_paths(deleted.to_vec());
        let deleted_set: BTreeSet<_> = deleted.iter().cloned().collect();
        if cache_backed {
            let affected_paths: BTreeSet<_> = changed.iter().chain(&deleted).cloned().collect();
            match load_lookup_names_for_paths_from_db(&self.db_path, &affected_paths) {
                Ok(names) => dirty_lookup_names.extend(names),
                Err(_) => lookup_cache_read_failed = true,
            }
        }
        if lookup_cache_read_failed {
            return self.rebuild();
        }
        for path in &deleted {
            if let Some(file) = self.files.remove(path) {
                insert_file_symbol_names(&mut dirty_lookup_names, &file);
            }
        }
        for path in &changed {
            if deleted_set.contains(path)
                || !path.exists()
                || !is_indexable(path, self.options.enable_pyx)
                || is_excluded(path, &self.options.exclude_globs)
            {
                if let Some(file) = self.files.remove(path) {
                    insert_file_symbol_names(&mut dirty_lookup_names, &file);
                }
                continue;
            }
            match parse_file_for_roots(path, &self.options.roots) {
                Ok(file) => {
                    if let Some(previous) = self.files.insert(file.path.clone(), file.clone()) {
                        insert_file_symbol_names(&mut dirty_lookup_names, &previous);
                    }
                    insert_file_symbol_names(&mut dirty_lookup_names, &file);
                    changed_files.push(file);
                }
                Err(error) => {
                    self.last_error = Some(error.to_string());
                }
            }
        }
        let refresh_materialized =
            paths_need_materialized_cache_refresh(&changed_files, &deleted, &self.options.roots);
        self.rebuild_symbol_map();
        if refresh_materialized {
            self.clear_lookup_cache();
        } else {
            self.clear_lookup_cache_entries(&dirty_lookup_names);
        }
        self.generation = self.generation.saturating_add(1);
        self.last_index_ms = started.elapsed().as_millis();
        self.ensure_cache_dir()?;
        let persist_started = Instant::now();
        let persisted =
            match self.persist_paths(&changed_files, &deleted, false, refresh_materialized) {
                Ok(()) => true,
                Err(error) => {
                    let primary_error = error.to_string();
                    match self.rebuild_into_fallback_cache() {
                        Ok(()) => true,
                        Err(fallback_error) => {
                            self.last_error = Some(format!(
                                "{primary_error}; fallback cache failed: {fallback_error}"
                            ));
                            false
                        }
                    }
                }
            };
        self.last_persist_ms = persist_started.elapsed().as_millis();
        if cache_backed && persisted {
            if let Ok((file_count, symbol_count, doc_count)) =
                self.cached_counts_for_current_roots()
            {
                self.cached_file_count = file_count;
                self.cached_symbol_count = symbol_count;
                self.cached_doc_count = doc_count;
            }
        }
        self.cached_root_fingerprint_mismatches.clear();
        let hot_started = Instant::now();
        if refresh_materialized {
            self.prewarm_hot_symbol_cache(true);
        }
        self.last_hot_cache_ms = hot_started.elapsed().as_millis();
        Ok(self.status())
    }

    pub fn preload_paths(&mut self, paths: &[PathBuf]) -> usize {
        let mut files = Vec::new();
        for path in normalize_existing_paths(paths.to_vec()) {
            if !path_is_under_roots(&path, &self.options.roots)
                || !is_indexable(&path, self.options.enable_pyx)
                || is_excluded(&path, &self.options.exclude_globs)
            {
                continue;
            }
            match parse_file_for_roots(&path, &self.options.roots) {
                Ok(file) => files.push(file),
                Err(error) => {
                    self.last_error = Some(error.to_string());
                }
            }
        }
        self.preload_indexed_files(files)
    }

    pub fn preload_indexed_files(&mut self, files: Vec<IndexedFile>) -> usize {
        let mut loaded = 0;
        let mut dirty_lookup_names = BTreeSet::new();
        for file in files {
            if !path_is_under_roots(&file.path, &self.options.roots)
                || is_excluded(&file.path, &self.options.exclude_globs)
            {
                continue;
            }
            if let Some(previous) = self.files.insert(file.path.clone(), file.clone()) {
                insert_file_symbol_names(&mut dirty_lookup_names, &previous);
            }
            insert_file_symbol_names(&mut dirty_lookup_names, &file);
            loaded += 1;
        }
        if loaded > 0 {
            self.rebuild_symbol_map();
            self.clear_lookup_cache_entries(&dirty_lookup_names);
        }
        loaded
    }

    pub fn status(&self) -> IndexStatus {
        let method_cache_stats = self.sage_method_cache_stats();
        IndexStatus {
            roots: self
                .options
                .roots
                .iter()
                .map(|root| root.display().to_string())
                .collect(),
            loaded_roots: self
                .loaded_roots
                .iter()
                .map(|root| root.display().to_string())
                .collect(),
            editable_roots: self
                .effective_editable_roots()
                .iter()
                .map(|root| root.display().to_string())
                .collect(),
            cache_namespace: cache_namespace_digest(
                &self.options.roots,
                &self.options.exclude_globs,
                self.options.enable_pyx,
            ),
            source_root_fingerprints: if self.source_root_fingerprints.len()
                == self.options.roots.len()
            {
                self.source_root_fingerprints.clone()
            } else {
                source_root_fingerprints_for_roots(&self.options.roots)
            },
            cache_stale: !self.cached_root_fingerprint_mismatches.is_empty(),
            stale_source_roots: self.cached_root_fingerprint_mismatches.clone(),
            indexed_file_count: self.cached_file_count.max(self.files.len()),
            deferred_file_count: 0,
            symbol_count: self
                .cached_symbol_count
                .max(self.symbols_by_name.values().map(Vec::len).sum()),
            doc_count: self.cached_doc_count.max(
                self.files
                    .values()
                    .flat_map(|file| &file.symbols)
                    .filter(|symbol| symbol.docstring.as_ref().is_some_and(|doc| !doc.is_empty()))
                    .count(),
            ),
            generation: self.generation,
            cache_path: self.db_path.display().to_string(),
            last_index_ms: self.last_index_ms,
            last_operation: self.last_operation.clone(),
            last_hydrate_ms: self.last_hydrate_ms,
            last_reconcile_ms: self.last_reconcile_ms,
            last_persist_ms: self.last_persist_ms,
            last_hot_cache_ms: self.last_hot_cache_ms,
            last_peer_seed_ms: self.last_peer_seed_ms,
            peer_seed_file_count: self.peer_seed_file_count,
            sage_method_cache_count: method_cache_stats.total,
            source_derived_method_cache_count: method_cache_stats.source_derived,
            static_method_cache_count: method_cache_stats.static_fallback,
            cache_hit_count: self.cache_hit_count,
            cache_miss_count: self.cache_miss_count,
            hot_symbol_cache_count: self.lookup_cache_len(),
            pending_jobs: 0,
            last_error: self.last_error.clone(),
        }
    }

    pub fn docs_status(&self) -> DocsStatus {
        DocsStatus {
            doc_db_path: self.db_path.display().to_string(),
            offline_doc_count: self.status().doc_count,
            preferred_source: "auto".to_string(),
            runtime_worker_state: "static-fallback".to_string(),
            runtime_degraded_reason: Some(
                "persistent Sage runtime docs worker is not enabled in Rust V2; static index and known Sage fallback are active".to_string(),
            ),
            runtime_queue_depth: 0,
            runtime_timeout_count: 0,
            runtime_cache_hits: 0,
            runtime_cache_misses: 0,
        }
    }

    fn rebuild_symbol_map(&mut self) {
        let files: Vec<_> = self.files.values().cloned().collect();
        self.symbols_by_name = symbol_map_from_files(&files);
    }

    fn persist_all(&self) -> Result<()> {
        let mut connection = Connection::open(&self.db_path)
            .with_context(|| format!("open index db {}", self.db_path.display()))?;
        tune_cache_connection(&connection)?;
        create_schema(&connection)?;
        let tx = connection.transaction()?;
        delete_roots_from_db(&tx, &self.options.roots)?;
        {
            let mut file_statement =
                tx.prepare("insert into files(path, module, fingerprint) values(?1, ?2, ?3)")?;
            let mut symbol_statement = tx.prepare(
                "insert into symbols(name, kind, module, path, start_line, start_character, end_line, end_character, detail, import_from, signature) values(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?;
            let mut doc_statement = tx.prepare(
                "insert into docs(name, module, path, detail, docstring) values(?1, ?2, ?3, ?4, ?5)",
            )?;
            let mut reference_statement = tx.prepare(
                "insert into reference_spans(name, path, start_line, start_character, end_line, end_character) values(?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for file in self.files.values() {
                let references = self
                    .should_persist_reference_spans(&file.path)
                    .then_some(&mut reference_statement);
                insert_file_rows(
                    file,
                    &mut file_statement,
                    &mut symbol_statement,
                    &mut doc_statement,
                    references,
                )?;
            }
        }
        clear_doc_fts(&tx)?;
        refresh_materialized_caches_from_symbols(&tx, &self.symbols_by_name)?;
        update_root_metadata(&tx, &self.options.roots)?;
        tx.commit()?;
        Ok(())
    }

    fn persist_paths(
        &self,
        changed: &[IndexedFile],
        deleted: &[PathBuf],
        materialize_from_changed: bool,
        refresh_materialized: bool,
    ) -> Result<()> {
        let mut connection = Connection::open(&self.db_path)
            .with_context(|| format!("open index db {}", self.db_path.display()))?;
        tune_cache_connection(&connection)?;
        create_schema(&connection)?;
        let tx = connection.transaction()?;
        let metadata_deltas = if materialize_from_changed || refresh_materialized {
            None
        } else {
            Some(metadata_deltas_for_path_refresh(
                &tx,
                changed,
                deleted,
                &self.options.roots,
            )?)
        };
        for path in deleted {
            delete_path_from_db(&tx, &path.display().to_string())?;
        }
        for file in changed {
            persist_file(&tx, file, self.should_persist_reference_spans(&file.path))?;
        }
        clear_doc_fts(&tx)?;
        if materialize_from_changed {
            let symbols_by_name = symbol_map_from_files(changed);
            refresh_materialized_caches_from_symbols(&tx, &symbols_by_name)?;
        } else if refresh_materialized {
            refresh_materialized_caches(&tx, &self.options.roots)?;
        }
        if let Some(deltas) = metadata_deltas {
            update_root_metadata_with_deltas(&tx, &self.options.roots, deltas)?;
        } else {
            update_root_metadata(&tx, &self.options.roots)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn persist_all_with_fallback(&mut self) -> Result<()> {
        self.switch_to_fallback_cache()?;
        self.persist_all()
    }

    fn rebuild_into_fallback_cache(&mut self) -> Result<()> {
        self.switch_to_fallback_cache()?;
        self.rebuild()?;
        if let Some(error) = self.last_error.as_deref() {
            bail!("fallback full rebuild could not be persisted: {error}");
        }
        Ok(())
    }

    fn seed_shared_roots_from_peer_caches(&mut self) {
        let started = Instant::now();
        if let Ok(imported) = self.try_seed_shared_roots_from_peer_caches() {
            if imported > 0 {
                self.peer_seed_file_count = self.peer_seed_file_count.saturating_add(imported);
                self.last_peer_seed_ms = started.elapsed().as_millis();
            }
        }
    }

    fn try_seed_shared_roots_from_peer_caches(&mut self) -> Result<usize> {
        self.ensure_cache_dir()?;
        let peer_paths = peer_cache_paths(&self.options.cache_dir, &self.db_path)?;
        if peer_paths.is_empty() {
            return Ok(0);
        }
        let seed_roots: Vec<PathBuf> = self
            .options
            .roots
            .iter()
            .filter(|root| {
                !self
                    .options
                    .editable_roots
                    .iter()
                    .any(|editable| root.starts_with(editable))
            })
            .cloned()
            .collect();
        if seed_roots.is_empty() {
            return Ok(0);
        }

        let mut connection = Connection::open(&self.db_path)
            .with_context(|| format!("open index db {}", self.db_path.display()))?;
        tune_cache_connection(&connection)?;
        create_schema(&connection)?;

        let mut imported = 0usize;
        for peer_path in peer_paths {
            if let Ok(count) =
                seed_shared_roots_from_peer_cache(&mut connection, &peer_path, &seed_roots)
            {
                imported = imported.saturating_add(count);
                if count > 0 && shared_roots_are_seeded(&connection, &seed_roots)? {
                    break;
                }
            }
        }
        if imported > 0 {
            refresh_materialized_caches(&connection, &self.options.roots)?;
        }
        Ok(imported)
    }

    fn ensure_cache_dir(&mut self) -> Result<()> {
        match fs::create_dir_all(&self.options.cache_dir) {
            Ok(()) => Ok(()),
            Err(primary_error) => self.switch_to_fallback_cache().with_context(|| {
                format!(
                    "create cache dir {}; fallback after: {primary_error}",
                    self.options.cache_dir.display()
                )
            }),
        }
    }

    fn switch_to_fallback_cache(&mut self) -> Result<()> {
        let fallback = fallback_cache_dir();
        fs::create_dir_all(&fallback)
            .with_context(|| format!("create fallback cache dir {}", fallback.display()))?;
        self.options.cache_dir = fallback;
        let digest = cache_namespace_digest(
            &self.options.roots,
            &self.options.exclude_globs,
            self.options.enable_pyx,
        );
        self.db_path = self
            .options
            .cache_dir
            .join(format!("sage-index-{digest}.sqlite"));
        Ok(())
    }

    fn load_cached_fingerprints_for_current_roots(&self) -> Result<BTreeMap<PathBuf, String>> {
        if !self.db_path.exists() {
            return Ok(BTreeMap::new());
        }
        let connection = Connection::open(&self.db_path)
            .with_context(|| format!("open index db {}", self.db_path.display()))?;
        create_schema(&connection)?;
        load_file_fingerprints_from_db(&connection, &self.options.roots)
    }

    fn cached_counts_for_current_roots(&self) -> Result<(usize, usize, usize)> {
        let connection = Connection::open(&self.db_path)
            .with_context(|| format!("open index db {}", self.db_path.display()))?;
        cached_counts_for_roots(&connection, &self.options.roots)
    }
}
