//! Debounced background prewarming for linked and imported documents.
//!
//! Parsing is deliberately kept off Tokio's small LSP worker pool. Per-document
//! revisions coalesce rapid edits, while a shared gate ensures at most one detached
//! parser worker is active at a time.

use super::{
    document_links::sage_document_links,
    open_documents::{canonical_path_for_comparison, uri_to_path},
    NavigationQueryCache,
};
use sage_index::{parse_file_for_roots, parse_source, IndexedFile, WorkspaceIndex};
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::Duration,
};
use tokio::sync::{oneshot, Mutex, RwLock};
use tower_lsp::lsp_types::Url;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PrewarmRevision {
    document_version: i32,
    generation: u64,
}

#[derive(Clone, Default)]
pub(super) struct LinkedDocumentPrewarmer {
    revisions: Arc<StdMutex<HashMap<PathBuf, PrewarmRevision>>>,
    next_generation: Arc<AtomicU64>,
    parse_gate: Arc<Mutex<()>>,
}

impl LinkedDocumentPrewarmer {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn schedule(
        &self,
        index: Arc<RwLock<WorkspaceIndex>>,
        index_work_gate: Arc<Mutex<()>>,
        navigation_cache: Arc<RwLock<NavigationQueryCache>>,
        shutting_down: Arc<AtomicBool>,
        uri: Url,
        text: String,
        document_version: i32,
    ) {
        let Some(path) = uri_to_path(&uri) else {
            return;
        };
        let key = canonical_path_for_comparison(&path);
        let revision = self.register_revision(key.clone(), document_version);
        let prewarmer = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            if shutting_down.load(Ordering::Acquire) || !prewarmer.is_current(&key, revision) {
                return;
            }

            let _parse_guard = prewarmer.parse_gate.lock().await;
            if shutting_down.load(Ordering::Acquire) || !prewarmer.is_current(&key, revision) {
                return;
            }

            let Some((mut targets, import_modules)) =
                detached_worker("sage-linked-discovery", move || {
                    linked_document_targets_and_imports(&path, &text)
                })
                .await
            else {
                return;
            };
            if shutting_down.load(Ordering::Acquire) || !prewarmer.is_current(&key, revision) {
                return;
            }

            let roots = {
                let index = index.read().await;
                for module in import_modules.into_iter().take(16) {
                    if let Some(target) = index.source_path_for_module(&module) {
                        targets.push(target);
                    }
                }
                index.options().roots.clone()
            };
            let mut seen = BTreeSet::new();
            targets.retain(|target| {
                let target = canonical_path_for_comparison(target);
                target != key && seen.insert(target)
            });
            targets.truncate(16);
            if targets.is_empty() {
                return;
            }

            let Some(parsed_files) = detached_worker("sage-linked-parse", move || {
                parse_linked_files(&targets, &roots)
            })
            .await
            else {
                return;
            };
            if parsed_files.is_empty()
                || shutting_down.load(Ordering::Acquire)
                || !prewarmer.is_current(&key, revision)
            {
                return;
            }

            let _index_work_guard = index_work_gate.lock().await;
            if shutting_down.load(Ordering::Acquire) || !prewarmer.is_current(&key, revision) {
                return;
            }
            let mut index = index.write().await;
            let loaded = index.preload_indexed_files(parsed_files);
            if loaded > 0 {
                navigation_cache.write().await.clear();
            }
        });
    }

    pub(super) fn cancel(&self, uri: &Url) {
        let Some(path) = uri_to_path(uri) else {
            return;
        };
        self.revisions_lock()
            .remove(&canonical_path_for_comparison(&path));
    }

    pub(super) fn cancel_all(&self) {
        self.revisions_lock().clear();
    }

    fn register_revision(&self, key: PathBuf, document_version: i32) -> PrewarmRevision {
        let revision = PrewarmRevision {
            document_version,
            generation: self.next_generation.fetch_add(1, Ordering::AcqRel) + 1,
        };
        self.revisions_lock().insert(key, revision);
        revision
    }

    fn is_current(&self, key: &Path, revision: PrewarmRevision) -> bool {
        self.revisions_lock().get(key).copied() == Some(revision)
    }

    fn revisions_lock(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, PrewarmRevision>> {
        self.revisions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

async fn detached_worker<T, F>(name: &'static str, work: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (sender, receiver) = oneshot::channel();
    let worker = std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            let _ = sender.send(work());
        })
        .ok()?;
    drop(worker);
    receiver.await.ok()
}

fn linked_document_targets_and_imports(path: &Path, text: &str) -> (Vec<PathBuf>, Vec<String>) {
    let targets = sage_document_links(text, path)
        .into_iter()
        .filter_map(|link| link.target)
        .filter_map(|target| uri_to_path(&target))
        .collect();
    (targets, import_modules_for_prewarm(path, text))
}

fn parse_linked_files(targets: &[PathBuf], roots: &[PathBuf]) -> Vec<IndexedFile> {
    targets
        .iter()
        .filter_map(|target| parse_file_for_roots(target, roots).ok())
        .collect()
}

pub(super) fn import_modules_for_prewarm(path: &Path, text: &str) -> Vec<String> {
    let mut modules = BTreeSet::new();
    let parsed = parse_source(module_name_for_path(path), path, text);
    for symbol in parsed.symbols {
        let Some(import_from) = symbol.import_from.as_deref() else {
            continue;
        };
        let module = import_from
            .split_once("::")
            .map_or(import_from, |(module, _)| module);
        if !module.is_empty() {
            modules.insert(module.to_string());
        }
    }
    modules.into_iter().collect()
}

fn module_name_for_path(path: &Path) -> &str {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("document")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sage_index::IndexOptions;
    use std::{fs, time::SystemTime};

    #[test]
    fn newer_document_revision_supersedes_queued_work() {
        let prewarmer = LinkedDocumentPrewarmer::default();
        let path = PathBuf::from("/workspace/demo.sage");
        let first = prewarmer.register_revision(path.clone(), 1);
        let second = prewarmer.register_revision(path.clone(), 2);

        assert!(!prewarmer.is_current(&path, first));
        assert!(prewarmer.is_current(&path, second));
    }

    #[test]
    fn shutdown_cancels_all_queued_revisions() {
        let prewarmer = LinkedDocumentPrewarmer::default();
        let first_path = PathBuf::from("/workspace/first.sage");
        let second_path = PathBuf::from("/workspace/second.sage");
        let first = prewarmer.register_revision(first_path.clone(), 1);
        let second = prewarmer.register_revision(second_path.clone(), 1);

        prewarmer.cancel_all();

        assert!(!prewarmer.is_current(&first_path, first));
        assert!(!prewarmer.is_current(&second_path, second));
    }

    #[tokio::test]
    async fn rapid_edits_only_prewarm_latest_document_version() {
        let root = unique_test_dir("prewarm-coalesce");
        fs::create_dir_all(&root).unwrap();
        let document_path = root.join("main.sage");
        let first_target = root.join("first.py");
        let second_target = root.join("second.py");
        fs::write(&document_path, "# open document\n").unwrap();
        fs::write(&first_target, "def first_target():\n    return 1\n").unwrap();
        fs::write(&second_target, "def second_target():\n    return 2\n").unwrap();
        let uri = Url::from_file_path(&document_path).unwrap();
        let index = Arc::new(RwLock::new(WorkspaceIndex::new(IndexOptions {
            roots: vec![root.clone()],
            editable_roots: vec![root.clone()],
            exclude_globs: Vec::new(),
            cache_dir: root.join("cache"),
            enable_pyx: true,
        })));
        let index_work_gate = Arc::new(Mutex::new(()));
        let navigation_cache = Arc::new(RwLock::new(NavigationQueryCache::default()));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let prewarmer = LinkedDocumentPrewarmer::default();

        prewarmer.schedule(
            index.clone(),
            index_work_gate.clone(),
            navigation_cache.clone(),
            shutting_down.clone(),
            uri.clone(),
            "load('first.py')\n".to_string(),
            1,
        );
        prewarmer.schedule(
            index.clone(),
            index_work_gate,
            navigation_cache,
            shutting_down,
            uri,
            "load('second.py')\n".to_string(),
            2,
        );

        tokio::time::sleep(Duration::from_millis(300)).await;
        let index = index.read().await;
        assert!(index.resolve_symbol("first_target", None).is_none());
        assert!(index.resolve_symbol("second_target", None).is_some());
        drop(index);

        fs::remove_dir_all(root).unwrap();
    }

    fn unique_test_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("sage-ls-{label}-{}-{nonce}", std::process::id()))
    }
}
