//! Versioned runtime sessions keep reconfiguration, cancellation and shutdown independent of slow Sage IO.
use anyhow::{anyhow, Context, Result};
use sage_index::{DocsStatus, DocumentationRecord, DocumentationSection};
use serde::Deserialize;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex, MutexGuard,
};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
mod process;
use process::{ProcessControl, RuntimeDocsProcess};

const WORKER_TIMEOUT: Duration = Duration::from_secs(3);
// Navigation never waits for a cold worker: it falls back to static results and the lookup
// keeps running in the background so the next attempt is served from the cache.
const FOREGROUND_WAIT: Duration = Duration::from_millis(300);
// A worker that misses this many replies in a row is treated as hung and replaced.
const MAX_CONSECUTIVE_TIMEOUTS: u32 = 3;
// Startup runs in the background: real source builds can take over 30 seconds to import Sage.
const WORKER_STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_FRAME_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PREFETCH: usize = 16;
const MAX_CACHE_ENTRIES: usize = 256;
const MAX_CACHE_BYTES: usize = 16 * 1024 * 1024;
const NEGATIVE_CACHE_TTL: Duration = Duration::from_secs(30);

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Clone, Debug, Default)]
pub struct RuntimeDocsConfig {
    pub enabled: bool,
    pub interpreter_path: String,
    pub interpreter_args: Vec<String>,
    pub source_roots: Vec<PathBuf>,
}
#[derive(Clone, Debug)]
pub(super) struct RuntimeSourceLocation {
    pub path: PathBuf,
    pub line: u32,
    pub source_text: String,
}
struct CacheEntry {
    record: Option<DocumentationRecord>,
    source: Option<RuntimeSourceLocation>,
    created: Instant,
    bytes: usize,
}
struct State {
    name: String,
    degraded_reason: Option<String>,
    cache: HashMap<String, CacheEntry>,
    cache_bytes: usize,
    inflight: HashSet<String>,
    cache_hits: usize,
    cache_misses: usize,
    timeout_count: usize,
    failures: u32,
    retry_at: Option<Instant>,
}
struct Session {
    config: RuntimeDocsConfig,
    state: Mutex<State>,
    // Only the bounded blocking request worker touches process. Shutdown uses control.
    process: Mutex<Option<RuntimeDocsProcess>>,
    control: Mutex<Option<Arc<ProcessControl>>>,
    stopped: AtomicBool,
    slots: Arc<Semaphore>,
    next_id: AtomicU64,
}
#[derive(Clone)]
pub struct RuntimeDocsWorker {
    session: Arc<Mutex<Arc<Session>>>,
}
impl Default for RuntimeDocsWorker {
    fn default() -> Self {
        Self {
            session: Arc::new(Mutex::new(Arc::new(Session::new(
                RuntimeDocsConfig::default(),
            )))),
        }
    }
}
impl Drop for RuntimeDocsWorker {
    fn drop(&mut self) {
        if Arc::strong_count(&self.session) == 1 {
            self.shutdown();
        }
    }
}
impl RuntimeDocsWorker {
    fn session(&self) -> Arc<Session> {
        lock(&self.session).clone()
    }
    pub async fn configure(&self, config: RuntimeDocsConfig) {
        let previous = std::mem::replace(&mut *lock(&self.session), Arc::new(Session::new(config)));
        previous.stop();
    }
    pub fn shutdown(&self) {
        self.session().stop();
    }
    pub async fn lookup(&self, symbol: &str) -> Option<DocumentationRecord> {
        self.session()
            .lookup(symbol)
            .await
            .and_then(|entry| entry.record)
    }
    pub fn cached(&self, symbol: &str) -> Option<DocumentationRecord> {
        self.session().cached(symbol).and_then(|entry| entry.record)
    }
    pub async fn source_location(&self, symbol: &str) -> Option<RuntimeSourceLocation> {
        let session = self.session();
        if let Some(entry) = session.cached(symbol) {
            return entry.source;
        }
        self.prefetch(symbol);
        let symbol = symbol.trim();
        tokio::time::timeout(FOREGROUND_WAIT, async {
            loop {
                tokio::time::sleep(Duration::from_millis(15)).await;
                if let Some(entry) = session.cached(symbol) {
                    return entry.source;
                }
                if !lock(&session.state).inflight.contains(symbol) {
                    return None;
                }
            }
        })
        .await
        .ok()
        .flatten()
    }
    pub fn prefetch(&self, symbol: &str) {
        let session = self.session();
        let symbol = symbol.trim().to_string();
        if !session.available() || symbol.is_empty() || session.cached(&symbol).is_some() {
            return;
        }
        {
            let mut state = lock(&session.state);
            if state.inflight.len() >= MAX_PREFETCH || !state.inflight.insert(symbol.clone()) {
                return;
            }
        }
        tokio::spawn(async move {
            let _ = session.lookup(&symbol).await;
            lock(&session.state).inflight.remove(&symbol);
        });
    }
    pub fn hover_status_message(&self) -> String {
        match lock(&self.session().state).name.as_str() {
            "disabled" => "Runtime documentation is disabled. Enable `sage.analysis.enableRuntimeIntrospection` to load full Sage documentation.",
            "unavailable" | "unconfigured-static-fallback" => "Select a Sage interpreter to load full documentation.",
            "degraded" => "Runtime documentation could not be loaded. Run `Sage: Show Docs Status` for details.",
            _ => "Loading Sage documentation. Hover again shortly, or run `Sage: Show Documentation`.",
        }.into()
    }
    pub async fn status(&self, mut base: DocsStatus) -> DocsStatus {
        let session = self.session();
        let state = lock(&session.state);
        base.runtime_worker_state = state.name.clone();
        base.runtime_degraded_reason = state.degraded_reason.clone();
        base.runtime_queue_depth = state.inflight.len();
        base.runtime_timeout_count = state.timeout_count;
        base.runtime_cache_hits = state.cache_hits;
        base.runtime_cache_misses = state.cache_misses;
        base
    }
}
#[derive(Clone)]
struct LookupResult {
    record: Option<DocumentationRecord>,
    source: Option<RuntimeSourceLocation>,
}
impl Session {
    fn new(config: RuntimeDocsConfig) -> Self {
        let (name, reason) = if !config.enabled {
            ("disabled", Some("runtime introspection is disabled".into()))
        } else if config.interpreter_path.trim().is_empty() {
            ("unavailable", Some("Sage interpreter path is empty".into()))
        } else {
            ("idle-static-fallback", None)
        };
        Self {
            config,
            state: Mutex::new(State {
                name: name.into(),
                degraded_reason: reason,
                cache: HashMap::new(),
                cache_bytes: 0,
                inflight: HashSet::new(),
                cache_hits: 0,
                cache_misses: 0,
                timeout_count: 0,
                failures: 0,
                retry_at: None,
            }),
            process: Mutex::new(None),
            control: Mutex::new(None),
            stopped: AtomicBool::new(false),
            slots: Arc::new(Semaphore::new(1)),
            next_id: AtomicU64::new(1),
        }
    }
    fn available(&self) -> bool {
        !self.stopped.load(Ordering::Acquire)
            && self.config.enabled
            && !self.config.interpreter_path.trim().is_empty()
            && lock(&self.state)
                .retry_at
                .is_none_or(|deadline| Instant::now() >= deadline)
    }
    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.slots.close();
        if let Some(control) = lock(&self.control).take() {
            control.terminate();
        }
        let mut state = lock(&self.state);
        state.name = "stopped".into();
        state.inflight.clear();
    }
    fn cached(&self, symbol: &str) -> Option<LookupResult> {
        let mut state = lock(&self.state);
        let entry = state.cache.get(symbol)?;
        if entry.record.is_none() && entry.created.elapsed() >= NEGATIVE_CACHE_TTL {
            if let Some(expired) = state.cache.remove(symbol) {
                state.cache_bytes -= expired.bytes;
            }
            return None;
        }
        let result = LookupResult {
            record: entry.record.clone(),
            source: entry.source.clone(),
        };
        state.cache_hits += 1;
        Some(result)
    }
    async fn lookup(self: &Arc<Self>, symbol: &str) -> Option<LookupResult> {
        if symbol.trim().is_empty() || symbol.len() > 4096 || !self.available() {
            return None;
        }
        if let Some(entry) = self.cached(symbol) {
            return Some(entry);
        }
        // Only one blocking worker exists per session; queued callers stay on async tasks.
        let permit = self.slots.clone().acquire_owned().await.ok()?;
        if !self.available() {
            return None;
        }
        if let Some(entry) = self.cached(symbol) {
            return Some(entry);
        }
        lock(&self.state).cache_misses += 1;
        // A started lookup always runs to completion and fills the cache. Editors cancel
        // requests constantly (Ctrl+hover); tying that to the worker would kill a Sage process
        // that took up to a minute to import.
        let cancelled = Arc::new(AtomicBool::new(false));
        let session = self.clone();
        let symbol = symbol.to_string();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let result = session.query(&symbol, &cancelled);
            let mut state = lock(&session.state);
            if session.stopped.load(Ordering::Acquire) {
                return None;
            }
            if cancelled.load(Ordering::Acquire) {
                state.name = "idle-static-fallback".into();
                state.degraded_reason = None;
                return None;
            }
            match result {
                Ok(entry) => {
                    state.name = "ready".into();
                    state.degraded_reason = None;
                    state.failures = 0;
                    state.retry_at = None;
                    let bytes = entry.record.as_ref().map_or(0, |r| {
                        r.name.len()
                            + r.module_name.len()
                            + r.kind.len()
                            + r.detail.len()
                            + r.summary.len()
                            + r.docstring.as_ref().map_or(0, String::len)
                            + r.uri.as_ref().map_or(0, String::len)
                    }) + symbol.len()
                        + entry.source.as_ref().map_or(0, |s| s.source_text.len());
                    while state.cache.len() >= MAX_CACHE_ENTRIES
                        || state.cache_bytes + bytes > MAX_CACHE_BYTES
                    {
                        let Some(key) = state
                            .cache
                            .iter()
                            .min_by_key(|(_, v)| v.created)
                            .map(|(k, _)| k.clone())
                        else {
                            break;
                        };
                        if let Some(expired) = state.cache.remove(&key) {
                            state.cache_bytes -= expired.bytes;
                        }
                    }
                    if bytes <= MAX_CACHE_BYTES {
                        state.cache_bytes += bytes;
                        state.cache.insert(
                            symbol,
                            CacheEntry {
                                record: entry.record.clone(),
                                source: entry.source.clone(),
                                created: Instant::now(),
                                bytes,
                            },
                        );
                    }
                    Some(entry)
                }
                Err(error) => {
                    if error.downcast_ref::<std::sync::mpsc::RecvTimeoutError>()
                        == Some(&std::sync::mpsc::RecvTimeoutError::Timeout)
                    {
                        state.timeout_count += 1;
                    }
                    state.failures = state.failures.saturating_add(1);
                    let delay =
                        Duration::from_secs(1u64 << state.failures.saturating_sub(1).min(6));
                    state.retry_at = Some(Instant::now() + delay);
                    state.name = "degraded".into();
                    state.degraded_reason = Some(error.to_string());
                    None
                }
            }
        })
        .await
        .ok()
        .flatten()
    }
    fn query(&self, symbol: &str, cancelled: &AtomicBool) -> Result<LookupResult> {
        let mut process = lock(&self.process);
        if self.stopped.load(Ordering::Acquire) || cancelled.load(Ordering::Acquire) {
            return Err(anyhow!("request cancelled"));
        }
        if process.is_none() {
            lock(&self.state).name = "starting".into();
            *process = Some(RuntimeDocsProcess::spawn(self, cancelled)?);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let response = process
            .as_mut()
            .expect("worker was started")
            .request(self, cancelled, id, symbol);
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                // A slow reply (lazy Sage imports) is not a dead worker: keep it and let the
                // next request discard the late frame by id. Only repeated timeouts or protocol
                // errors replace the process.
                let timed_out = error.downcast_ref::<std::sync::mpsc::RecvTimeoutError>()
                    == Some(&std::sync::mpsc::RecvTimeoutError::Timeout);
                let keep = timed_out
                    && process
                        .as_mut()
                        .is_some_and(|worker| worker.note_timeout() < MAX_CONSECUTIVE_TIMEOUTS);
                if !keep {
                    process.take();
                    lock(&self.control).take();
                }
                return Err(error);
            }
        };
        if !response.ok {
            if response.missing {
                return Ok(LookupResult {
                    record: None,
                    source: None,
                });
            }
            return Err(anyhow!(response
                .error
                .unwrap_or_else(|| "runtime lookup failed".into())));
        }
        let source = match (&response.uri, response.source_line, &response.source_text) {
            (Some(path), Some(line), Some(text))
                if Path::new(path).is_absolute() && line > 0 && !text.trim().is_empty() =>
            {
                Some(RuntimeSourceLocation {
                    path: PathBuf::from(path),
                    line: line - 1,
                    source_text: text.clone(),
                })
            }
            _ => None,
        };
        Ok(LookupResult {
            source,
            record: Some(DocumentationRecord {
                name: response.name,
                module_name: response
                    .module_name
                    .unwrap_or_else(|| "sage.runtime".into()),
                kind: response.kind.unwrap_or_else(|| "RuntimeObject".into()),
                detail: response.detail.unwrap_or_default(),
                summary: response.summary.unwrap_or_else(|| symbol.into()),
                docstring: response.docstring,
                uri: response.uri.filter(|value| !value.is_empty()),
                markers: vec!["runtime".into()],
                sections: Vec::<DocumentationSection>::new(),
            }),
        })
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}
#[derive(Deserialize)]
struct WorkerReady {
    ready: bool,
    error: Option<String>,
}
#[derive(Deserialize)]
struct WorkerResponse {
    id: u64,
    name: String,
    ok: bool,
    #[serde(default)]
    missing: bool,
    error: Option<String>,
    module_name: Option<String>,
    kind: Option<String>,
    detail: Option<String>,
    summary: Option<String>,
    docstring: Option<String>,
    uri: Option<String>,
    source_line: Option<u32>,
    source_text: Option<String>,
}
#[cfg(test)]
mod tests;
