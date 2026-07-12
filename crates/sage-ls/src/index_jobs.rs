//! Background index lifecycle and coordination.
//!
//! Rebuilds, cache reconciliation, and file refreshes share one work gate. Keeping their
//! generation checks together makes index installation ordering explicit.

use super::{refresh_editor_feature_caches, Backend};
use sage_index::WorkspaceIndex;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::atomic::Ordering};
use tokio::sync::{oneshot, RwLock};
use tower_lsp::lsp_types::MessageType;

async fn finish_pending_index_job(
    pending_jobs: &RwLock<usize>,
    pending_index_task: &RwLock<Option<String>>,
) {
    let mut pending = pending_jobs.write().await;
    *pending = pending.saturating_sub(1);
    if *pending == 0 {
        *pending_index_task.write().await = None;
    }
}

#[cfg(debug_assertions)]
fn maybe_delay_background_index_for_test() {
    let delay_ms = std::env::var("SAGE_LS_TEST_BACKGROUND_DELAY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value <= 30_000)
        .unwrap_or_default();
    if delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
    }
}

#[cfg(not(debug_assertions))]
fn maybe_delay_background_index_for_test() {}

pub(super) fn index_job_result_is_current(
    latest_job_generation: u64,
    job_generation: u64,
    current_index_generation: u64,
    initial_index_generation: u64,
) -> bool {
    latest_job_generation == job_generation && current_index_generation == initial_index_generation
}

impl Backend {
    pub(super) fn spawn_rebuild(&self) {
        let index = self.index.clone();
        let pending_jobs = self.pending_jobs.clone();
        let pending_index_task = self.pending_index_task.clone();
        let navigation_cache = self.navigation_cache.clone();
        let client = self.client.clone();
        let index_job_generation = self.index_job_generation.clone();
        let job_generation = index_job_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let index_work_gate = self.index_work_gate.clone();
        let shutting_down = self.shutting_down.clone();
        tokio::spawn(async move {
            if shutting_down.load(Ordering::Acquire) {
                return;
            }
            {
                let mut pending = pending_jobs.write().await;
                *pending = pending.saturating_add(1);
                *pending_index_task.write().await = Some("rebuild".to_string());
            }
            let work_guard = index_work_gate.lock_owned().await;
            if shutting_down.load(Ordering::Acquire)
                || index_job_generation.load(Ordering::Acquire) != job_generation
            {
                finish_pending_index_job(&pending_jobs, &pending_index_task).await;
                return;
            }
            let (initial_generation, options) = {
                let current = index.read().await;
                (current.status().generation, current.options().clone())
            };
            let (sender, receiver) = oneshot::channel();
            let worker = std::thread::Builder::new()
                .name("sage-index-rebuild".to_string())
                .spawn(move || {
                    maybe_delay_background_index_for_test();
                    let mut rebuilt = WorkspaceIndex::new(options);
                    let result = rebuilt
                        .rebuild()
                        .map(|_| {
                            rebuilt.ensure_generation_after(initial_generation);
                            let status = rebuilt.status();
                            (rebuilt, status)
                        })
                        .map_err(|error| format!("{error:#}"));
                    let _ = sender.send((result, work_guard));
                });
            let (result, work_guard) = match worker {
                Ok(handle) => {
                    drop(handle);
                    match receiver.await {
                        Ok((result, work_guard)) => (result, Some(work_guard)),
                        Err(error) => (
                            Err(format!(
                                "index rebuild worker stopped before returning: {error}"
                            )),
                            None,
                        ),
                    }
                }
                Err(error) => (Err(format!("start index rebuild worker: {error}")), None),
            };
            if shutting_down.load(Ordering::Acquire) {
                drop(work_guard);
                finish_pending_index_job(&pending_jobs, &pending_index_task).await;
                return;
            }
            let result = match result {
                Ok((rebuilt, status)) => {
                    let mut current = index.write().await;
                    if index_job_result_is_current(
                        index_job_generation.load(Ordering::Acquire),
                        job_generation,
                        current.status().generation,
                        initial_generation,
                    ) {
                        *current = rebuilt;
                        Ok((status, true))
                    } else {
                        Ok((current.status(), false))
                    }
                }
                Err(error) => Err(error),
            };
            drop(work_guard);
            finish_pending_index_job(&pending_jobs, &pending_index_task).await;
            if shutting_down.load(Ordering::Acquire) {
                return;
            }
            match result {
                Ok((status, installed)) => {
                    if installed {
                        navigation_cache.write().await.clear();
                    }
                    client
                        .log_message(
                            MessageType::INFO,
                            format!(
                                "sage-ls indexed {} files and {} symbols in {}ms (installed={})",
                                status.indexed_file_count,
                                status.symbol_count,
                                status.last_index_ms,
                                installed,
                            ),
                        )
                        .await;
                    if installed {
                        refresh_editor_feature_caches(&client).await;
                    }
                }
                Err(error) => {
                    client
                        .log_message(
                            MessageType::WARNING,
                            format!("sage-ls index rebuild failed: {error}"),
                        )
                        .await;
                }
            }
        });
    }

    pub(super) fn spawn_cache_reconcile(&self) {
        let index = self.index.clone();
        let pending_jobs = self.pending_jobs.clone();
        let pending_index_task = self.pending_index_task.clone();
        let navigation_cache = self.navigation_cache.clone();
        let client = self.client.clone();
        let index_job_generation = self.index_job_generation.clone();
        let job_generation = index_job_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let index_work_gate = self.index_work_gate.clone();
        let shutting_down = self.shutting_down.clone();
        tokio::spawn(async move {
            if shutting_down.load(Ordering::Acquire) {
                return;
            }
            {
                let mut pending = pending_jobs.write().await;
                *pending = pending.saturating_add(1);
                *pending_index_task.write().await = Some("cache-check".to_string());
            }
            let work_guard = index_work_gate.lock_owned().await;
            if shutting_down.load(Ordering::Acquire)
                || index_job_generation.load(Ordering::Acquire) != job_generation
            {
                finish_pending_index_job(&pending_jobs, &pending_index_task).await;
                return;
            }
            let (initial_generation, reconciled) = {
                let index = index.read().await;
                (index.status().generation, index.clone_for_background_work())
            };
            let (sender, receiver) = oneshot::channel();
            let worker = std::thread::Builder::new()
                .name("sage-index-reconcile".to_string())
                .spawn(move || {
                    maybe_delay_background_index_for_test();
                    let mut reconciled = reconciled;
                    let result = reconciled
                        .reconcile_with_cache()
                        .map(|_| {
                            let status = reconciled.status();
                            (reconciled, status)
                        })
                        .map_err(|error| format!("{error:#}"));
                    let _ = sender.send((result, work_guard));
                });
            let (result, work_guard) = match worker {
                Ok(handle) => {
                    drop(handle);
                    match receiver.await {
                        Ok((result, work_guard)) => (result, Some(work_guard)),
                        Err(error) => (
                            Err(format!(
                                "cache reconcile worker stopped before returning: {error}"
                            )),
                            None,
                        ),
                    }
                }
                Err(error) => (Err(format!("start cache reconcile worker: {error}")), None),
            };
            if shutting_down.load(Ordering::Acquire) {
                drop(work_guard);
                finish_pending_index_job(&pending_jobs, &pending_index_task).await;
                return;
            }
            let result = match result {
                Ok((reconciled, status)) => {
                    let mut current = index.write().await;
                    if index_job_result_is_current(
                        index_job_generation.load(Ordering::Acquire),
                        job_generation,
                        current.status().generation,
                        initial_generation,
                    ) {
                        *current = reconciled;
                        Ok((status, true))
                    } else {
                        Ok((current.status(), false))
                    }
                }
                Err(error) => Err(error),
            };
            drop(work_guard);
            finish_pending_index_job(&pending_jobs, &pending_index_task).await;
            if shutting_down.load(Ordering::Acquire) {
                return;
            }
            match result {
                Ok((status, installed)) => {
                    if installed {
                        navigation_cache.write().await.clear();
                    }
                    client
                        .log_message(
                            MessageType::INFO,
                            format!(
                                "sage-ls reconciled {} files and {} symbols from persistent cache in {}ms ({} hit/{} miss, installed={})",
                                status.indexed_file_count,
                                status.symbol_count,
                                status.last_index_ms,
                                status.cache_hit_count,
                                status.cache_miss_count,
                                installed,
                            ),
                        )
                        .await;
                    if installed {
                        refresh_editor_feature_caches(&client).await;
                    }
                }
                Err(error) => {
                    client
                        .log_message(
                            MessageType::WARNING,
                            format!("sage-ls cache reconcile failed: {error}"),
                        )
                        .await;
                }
            }
        });
    }

    pub(super) async fn refresh_paths(&self, changed: Vec<PathBuf>, deleted: Vec<PathBuf>) {
        let work_guard = self.index_work_gate.clone().lock_owned().await;
        if self.shutting_down.load(Ordering::Acquire) {
            return;
        }
        let changed_count = changed.len();
        let deleted_count = deleted.len();
        let (initial_generation, refreshed) = {
            let index = self.index.read().await;
            (index.status().generation, index.clone_for_background_work())
        };
        let (sender, receiver) = oneshot::channel();
        let worker = std::thread::Builder::new()
            .name("sage-index-refresh".to_string())
            .spawn(move || {
                let mut refreshed = refreshed;
                let result = refreshed
                    .refresh_paths(&changed, &deleted)
                    .map(|status| (refreshed, status))
                    .map_err(|error| format!("{error:#}"));
                let _ = sender.send((result, work_guard));
            });
        let (result, work_guard) = match worker {
            Ok(handle) => {
                drop(handle);
                match receiver.await {
                    Ok((result, work_guard)) => (result, Some(work_guard)),
                    Err(error) => (
                        Err(format!(
                            "index refresh worker stopped before returning: {error}"
                        )),
                        None,
                    ),
                }
            }
            Err(error) => (Err(format!("start index refresh worker: {error}")), None),
        };
        if self.shutting_down.load(Ordering::Acquire) {
            drop(work_guard);
            return;
        }
        let result = match result {
            Ok((refreshed, status)) => {
                let mut current = self.index.write().await;
                if current.status().generation == initial_generation {
                    *current = refreshed;
                    Ok((status, true))
                } else {
                    Ok((current.status(), false))
                }
            }
            Err(error) => Err(error),
        };
        drop(work_guard);
        if self.shutting_down.load(Ordering::Acquire) {
            return;
        }
        match result {
            Ok((status, installed)) => {
                if installed {
                    self.navigation_cache.write().await.clear();
                }
                self.client
                    .log_message(
                        MessageType::INFO,
                        format!(
                            "sage-ls refreshed {} changed and {} deleted files in {}ms (installed={})",
                            changed_count, deleted_count, status.last_index_ms, installed,
                        ),
                    )
                    .await;
                if installed {
                    refresh_editor_feature_caches(&self.client).await;
                }
            }
            Err(error) => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("sage-ls incremental refresh failed: {error}"),
                    )
                    .await;
            }
        }
    }

    pub(super) async fn index_status_payload(&self) -> Value {
        let mut payload =
            serde_json::to_value(self.index.read().await.status()).unwrap_or_else(|_| json!({}));
        if let Some(object) = payload.as_object_mut() {
            object.insert(
                "pending_jobs".to_string(),
                json!(*self.pending_jobs.read().await),
            );
            object.insert(
                "pending_task".to_string(),
                json!(self.pending_index_task.read().await.clone()),
            );
        }
        payload
    }
}
