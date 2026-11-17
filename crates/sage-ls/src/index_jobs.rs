//! One writer publishes immutable index snapshots. Notifications only enqueue work.
use super::Backend;
use arc_swap::ArcSwap;
use sage_index::{parse_file_for_roots, WorkspaceIndex};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::{oneshot, Notify};
use tower_lsp_server::{ls_types::MessageType, Client};

pub(super) type IndexHandle = Arc<ArcSwap<WorkspaceIndex>>;

#[derive(Default)]
struct Queue {
    reconcile: bool,
    rebuild: bool,
    paths: BTreeMap<PathBuf, bool>, // true = deleted; the last event wins
    preloads: BTreeSet<PathBuf>,
    active: Option<&'static str>,
}

impl Queue {
    fn pending(&self) -> usize {
        usize::from(self.reconcile)
            + usize::from(self.rebuild)
            + usize::from(!self.paths.is_empty())
            + usize::from(!self.preloads.is_empty())
            + usize::from(self.active.is_some())
    }

    fn take(&mut self) -> Option<Job> {
        let job = if std::mem::take(&mut self.reconcile) {
            Job::Reconcile
        } else if std::mem::take(&mut self.rebuild) {
            Job::Rebuild
        } else if !self.paths.is_empty() {
            Job::Refresh(std::mem::take(&mut self.paths))
        } else if !self.preloads.is_empty() {
            Job::Preload(std::mem::take(&mut self.preloads).into_iter().collect())
        } else {
            return None;
        };
        self.active = Some(job.label());
        Some(job)
    }
}

enum Job {
    Reconcile,
    Rebuild,
    Refresh(BTreeMap<PathBuf, bool>),
    Preload(Vec<PathBuf>),
}

impl Job {
    fn label(&self) -> &'static str {
        match self {
            Self::Reconcile => "cache-check",
            Self::Rebuild => "rebuild",
            Self::Refresh(_) => "refresh",
            Self::Preload(_) => "prewarm",
        }
    }
}

#[derive(Clone)]
pub(super) struct IndexActor {
    index: IndexHandle,
    queue: Arc<Mutex<Queue>>,
    wake: Arc<Notify>,
    stopped: Arc<AtomicBool>,
}

impl IndexActor {
    pub(super) fn new(
        index: IndexHandle,
        client: Option<Client>,
        stopped: Arc<AtomicBool>,
    ) -> Self {
        let actor = Self {
            index,
            queue: Arc::default(),
            wake: Arc::default(),
            stopped,
        };
        tokio::spawn(actor.clone().run(client));
        actor
    }

    fn queue(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn reconcile(&self) {
        self.queue().reconcile = true;
        self.wake.notify_one();
    }

    pub(super) fn rebuild(&self) {
        self.queue().rebuild = true;
        self.wake.notify_one();
    }

    pub(super) fn refresh(&self, changed: Vec<PathBuf>, deleted: Vec<PathBuf>) {
        // Serialize marking and installation so an event cannot land in an old snapshot.
        let mut queue = self.queue();
        self.index
            .load()
            .mark_paths_pending_refresh(&changed, &deleted);
        for path in changed {
            queue.paths.insert(path, false);
        }
        for path in deleted {
            queue.paths.insert(path, true);
        }
        drop(queue);
        self.wake.notify_one();
    }

    pub(super) fn preload(&self, paths: Vec<PathBuf>) {
        let mut queue = self.queue();
        queue.preloads.extend(paths);
        drop(queue);
        self.wake.notify_one();
    }

    pub(super) fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        *self.queue() = Queue::default();
        self.wake.notify_one();
    }

    async fn run(self, client: Option<Client>) {
        loop {
            self.wake.notified().await;
            loop {
                if self.stopped.load(Ordering::Acquire) {
                    return;
                }
                let Some(job) = self.queue().take() else {
                    break;
                };
                let label = job.label();
                let current = self.index.load_full();
                let index_handle = self.index.clone();
                let queue = self.queue.clone();
                let stopped = self.stopped.clone();
                let (sender, receiver) = oneshot::channel();
                // A detached worker avoids Tokio waiting for a long filesystem scan on exit.
                let started = std::thread::Builder::new()
                    .name(format!("sage-index-{label}"))
                    .spawn(move || {
                        let result = (|| -> anyhow::Result<()> {
                            let previous_generation = current.generation();
                            let mut next = match job {
                                Job::Rebuild => WorkspaceIndex::new(current.options().clone()),
                                _ => current.clone_for_background_work(),
                            };
                            if matches!(job, Job::Reconcile) {
                                let _ = next.hydrate_from_cache();
                                let _guard = queue
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                if !stopped.load(Ordering::Acquire) {
                                    let mut hydrated = next.clone();
                                    hydrated.finalize_pending_refresh_install(
                                        &index_handle.load_full(),
                                    );
                                    index_handle.store(Arc::new(hydrated));
                                }
                            }
                            if matches!(job, Job::Reconcile | Job::Rebuild) {
                                maybe_delay_background_index_for_test();
                            }
                            if stopped.load(Ordering::Acquire) {
                                return Ok(());
                            }
                            match job {
                                Job::Reconcile => {
                                    next.reconcile_with_cache()?;
                                }
                                Job::Rebuild => {
                                    next.rebuild()?;
                                }
                                Job::Refresh(paths) => {
                                    let (deleted, changed): (Vec<_>, Vec<_>) =
                                        paths.into_iter().partition(|(_, deleted)| *deleted);
                                    next.refresh_paths(
                                        &changed
                                            .into_iter()
                                            .map(|(path, _)| path)
                                            .collect::<Vec<_>>(),
                                        &deleted
                                            .into_iter()
                                            .map(|(path, _)| path)
                                            .collect::<Vec<_>>(),
                                    )?;
                                }
                                Job::Preload(paths) => {
                                    // Read at execution time: a discovery result may have
                                    // waited behind a newer save or deletion.
                                    let files = paths
                                        .iter()
                                        .filter_map(|path| {
                                            parse_file_for_roots(path, &next.options().roots).ok()
                                        })
                                        .collect();
                                    next.preload_indexed_files(files);
                                }
                            }
                            next.ensure_generation_after(previous_generation);
                            let _guard = queue
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if !stopped.load(Ordering::Acquire) {
                                next.finalize_pending_refresh_install(&index_handle.load_full());
                                index_handle.store(Arc::new(next));
                            }
                            Ok(())
                        })();
                        let _ = sender.send(result.map_err(|error| format!("{error:#}")));
                    });
                let result = match started {
                    Ok(_) => receiver
                        .await
                        .unwrap_or_else(|error| Err(error.to_string())),
                    Err(error) => Err(error.to_string()),
                };
                self.queue().active = None;
                if self.stopped.load(Ordering::Acquire) {
                    return;
                }
                if let Some(client) = &client {
                    if let Err(error) = result {
                        client
                            .log_message(
                                MessageType::WARNING,
                                format!("sage-ls {label} failed: {error}"),
                            )
                            .await;
                    } else {
                        // Refresh requests may wait for a client response; never hold up the writer.
                        let client = client.clone();
                        tokio::spawn(async move {
                            super::refresh_editor_feature_caches(&client).await;
                        });
                    }
                }
            }
        }
    }
}

#[cfg(debug_assertions)]
fn maybe_delay_background_index_for_test() {
    let delay = std::env::var("SAGE_LS_TEST_BACKGROUND_DELAY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
        .min(30_000);
    std::thread::sleep(std::time::Duration::from_millis(delay));
}
#[cfg(not(debug_assertions))]
fn maybe_delay_background_index_for_test() {}

impl Backend {
    pub(super) async fn index_status_payload(&self) -> Value {
        let analysis_mode = *self.analysis_mode.read().await;
        let mut payload =
            serde_json::to_value(self.index.load_full().status()).unwrap_or_else(|_| json!({}));
        if let Some(object) = payload.as_object_mut() {
            let queue = self.index_actor.queue();
            object.insert("analysis_mode".into(), json!(analysis_mode.as_str()));
            object.insert(
                "workspace_symbol_limit".into(),
                json!(analysis_mode.workspace_symbol_limit()),
            );
            object.insert("pending_jobs".into(), json!(queue.pending()));
            object.insert("pending_task".into(), json!(queue.active));
        }
        payload
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refreshes_coalesce_and_latest_event_wins() {
        let mut queue = Queue::default();
        let path = PathBuf::from("demo.sage");
        queue.paths.insert(path.clone(), true);
        queue.paths.insert(path.clone(), false);
        assert_eq!(queue.pending(), 1);
        let Some(Job::Refresh(paths)) = queue.take() else {
            panic!("expected refresh");
        };
        assert_eq!(paths.get(&path), Some(&false));
        assert_eq!(queue.pending(), 1);
        assert!(queue.take().is_none());
    }
}
