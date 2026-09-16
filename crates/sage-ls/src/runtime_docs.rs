use anyhow::{anyhow, Context, Result};
use sage_index::{DocsStatus, DocumentationRecord, DocumentationSection};
use serde::Deserialize;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

const WORKER_TIMEOUT: Duration = Duration::from_secs(3);
const WORKER_STARTUP_TIMEOUT: Duration = Duration::from_secs(8);
const WORKER_SCRIPT: &str = r#"
import inspect
import json
import sys
import traceback

try:
    from sage.all import *  # noqa: F401,F403
    from sage.misc.sageinspect import sage_getdef, sage_getdoc, sage_getfile
    print(json.dumps({"ready": True}), flush=True)
except Exception as exc:
    print(json.dumps({"ready": False, "error": traceback.format_exc()}), flush=True)
    sys.exit(0)

def resolve_symbol(name):
    obj = None
    namespace = globals()
    for part in name.split("."):
        if not part:
            raise KeyError(name)
        if obj is None:
            obj = namespace[part]
        else:
            obj = getattr(obj, part)
    return obj

def one_line(value):
    value = (value or "").strip()
    for line in value.splitlines():
        line = line.strip()
        if line:
            return line
    return ""

for line in sys.stdin:
    try:
        request = json.loads(line)
        name = request.get("symbol", "")
        obj = resolve_symbol(name)
        try:
            # Embedded formatting preserves docs without probing optional packages
            # (e.g. starting Maxima merely to format sin's doctest tags).
            doc = sage_getdoc(obj, obj_name=name, embedded=True) or ""
        except TypeError:
            doc = sage_getdoc(obj) or ""
        if not doc:
            doc = inspect.getdoc(obj) or ""
        try:
            detail = sage_getdef(obj, name) or ""
        except Exception:
            detail = ""
        try:
            path = sage_getfile(obj) or ""
        except Exception:
            path = ""
        module = getattr(obj, "__module__", "") or "sage.runtime"
        kind = type(obj).__name__
        print(json.dumps({
            "ok": True,
            "name": name,
            "module_name": module,
            "kind": kind,
            "detail": detail,
            "summary": one_line(doc) or detail or name,
            "docstring": doc,
            "uri": path,
        }), flush=True)
    except Exception as exc:
        print(json.dumps({
            "ok": False,
            "error": str(exc),
            "traceback": traceback.format_exc(limit=2),
        }), flush=True)
"#;

#[derive(Clone, Debug, Default)]
pub struct RuntimeDocsConfig {
    pub enabled: bool,
    pub interpreter_path: String,
    pub interpreter_args: Vec<String>,
    pub source_roots: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
struct RuntimeDocsCounters {
    state: String,
    degraded_reason: Option<String>,
    cache_hits: usize,
    cache_misses: usize,
    timeout_count: usize,
    queue_depth: usize,
}

impl Default for RuntimeDocsCounters {
    fn default() -> Self {
        Self {
            state: "unconfigured-static-fallback".to_string(),
            degraded_reason: Some(
                "runtime docs worker is not configured yet; static indexed docs are active"
                    .to_string(),
            ),
            cache_hits: 0,
            cache_misses: 0,
            timeout_count: 0,
            queue_depth: 0,
        }
    }
}

#[derive(Clone, Default)]
pub struct RuntimeDocsWorker {
    config: Arc<Mutex<RuntimeDocsConfig>>,
    counters: Arc<Mutex<RuntimeDocsCounters>>,
    cache: Arc<Mutex<HashMap<String, DocumentationRecord>>>,
    inflight: Arc<Mutex<HashSet<String>>>,
    process: Arc<Mutex<Option<RuntimeDocsProcess>>>,
}

struct RuntimeDocsProcess {
    child: Child,
    stdin: ChildStdin,
    output: mpsc::Receiver<std::io::Result<String>>,
}

#[derive(Clone, Debug, Deserialize)]
struct WorkerReady {
    ready: bool,
    error: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct WorkerResponse {
    ok: bool,
    error: Option<String>,
    name: Option<String>,
    module_name: Option<String>,
    kind: Option<String>,
    detail: Option<String>,
    summary: Option<String>,
    docstring: Option<String>,
    uri: Option<String>,
}

impl RuntimeDocsWorker {
    pub async fn configure(&self, config: RuntimeDocsConfig) {
        *self
            .config
            .lock()
            .expect("runtime docs config lock poisoned") = config.clone();
        self.cache
            .lock()
            .expect("runtime docs cache lock poisoned")
            .clear();
        self.inflight
            .lock()
            .expect("runtime docs inflight lock poisoned")
            .clear();
        self.stop_process();
        let mut counters = self
            .counters
            .lock()
            .expect("runtime docs counters lock poisoned");
        counters.cache_hits = 0;
        counters.cache_misses = 0;
        counters.timeout_count = 0;
        counters.queue_depth = 0;
        if !config.enabled {
            counters.state = "disabled".to_string();
            counters.degraded_reason = Some("runtime introspection is disabled".to_string());
        } else if config.interpreter_path.trim().is_empty() {
            counters.state = "unavailable".to_string();
            counters.degraded_reason = Some("Sage interpreter path is empty".to_string());
        } else {
            counters.state = "idle-static-fallback".to_string();
            counters.degraded_reason = None;
        }
    }

    pub async fn lookup(&self, symbol: &str) -> Option<DocumentationRecord> {
        if symbol.trim().is_empty() {
            return None;
        }
        if let Some(record) = self
            .cache
            .lock()
            .expect("runtime docs cache lock poisoned")
            .get(symbol)
            .cloned()
        {
            self.counters
                .lock()
                .expect("runtime docs counters lock poisoned")
                .cache_hits += 1;
            return Some(record);
        }
        self.counters
            .lock()
            .expect("runtime docs counters lock poisoned")
            .cache_misses += 1;
        let worker = self.clone();
        let symbol = symbol.to_string();
        let result =
            tokio::task::spawn_blocking(move || worker.lookup_uncached_blocking(&symbol)).await;
        match result {
            Ok(Ok(Some(record))) => {
                self.cache
                    .lock()
                    .expect("runtime docs cache lock poisoned")
                    .insert(record.name.clone(), record.clone());
                Some(record)
            }
            Ok(Ok(None)) => None,
            Ok(Err(error)) => {
                let mut counters = self
                    .counters
                    .lock()
                    .expect("runtime docs counters lock poisoned");
                if error.downcast_ref::<mpsc::RecvTimeoutError>()
                    == Some(&mpsc::RecvTimeoutError::Timeout)
                {
                    counters.timeout_count += 1;
                }
                counters.state = "degraded".to_string();
                counters.degraded_reason = Some(error.to_string());
                None
            }
            Err(error) => {
                let mut counters = self
                    .counters
                    .lock()
                    .expect("runtime docs counters lock poisoned");
                counters.state = "degraded".to_string();
                counters.degraded_reason = Some(error.to_string());
                None
            }
        }
    }

    pub fn cached(&self, symbol: &str) -> Option<DocumentationRecord> {
        if symbol.trim().is_empty() {
            return None;
        }
        let record = self
            .cache
            .lock()
            .expect("runtime docs cache lock poisoned")
            .get(symbol)
            .cloned();
        if record.is_some() {
            self.counters
                .lock()
                .expect("runtime docs counters lock poisoned")
                .cache_hits += 1;
        }
        record
    }

    pub fn hover_status_message(&self) -> String {
        let counters = self
            .counters
            .lock()
            .expect("runtime docs counters lock poisoned");
        match counters.state.as_str() {
            "disabled" => "Runtime documentation is disabled. Enable `sage.analysis.enableRuntimeIntrospection` to load full Sage documentation.",
            "unavailable" | "unconfigured-static-fallback" => "Select a Sage interpreter to load full documentation.",
            "degraded" => "Runtime documentation could not be loaded. Run `Sage: Show Docs Status` for details.",
            _ => "Loading Sage documentation. Hover again shortly, or run `Sage: Show Documentation`.",
        }.to_string()
    }

    pub fn prefetch(&self, symbol: &str) {
        let symbol = symbol.trim().to_string();
        if symbol.is_empty() {
            return;
        }
        let config = self
            .config
            .lock()
            .expect("runtime docs config lock poisoned")
            .clone();
        if !config.enabled || config.interpreter_path.trim().is_empty() {
            return;
        }
        if self
            .cache
            .lock()
            .expect("runtime docs cache lock poisoned")
            .contains_key(&symbol)
        {
            return;
        }
        {
            let mut inflight = self
                .inflight
                .lock()
                .expect("runtime docs inflight lock poisoned");
            if !inflight.insert(symbol.clone()) {
                return;
            }
            self.counters
                .lock()
                .expect("runtime docs counters lock poisoned")
                .queue_depth = inflight.len();
        }
        let worker = self.clone();
        tokio::spawn(async move {
            let _ = worker.lookup(&symbol).await;
            worker.finish_prefetch(&symbol);
        });
    }

    pub async fn status(&self, mut base: DocsStatus) -> DocsStatus {
        let counters = self
            .counters
            .lock()
            .expect("runtime docs counters lock poisoned");
        base.runtime_worker_state = counters.state.clone();
        base.runtime_degraded_reason = counters.degraded_reason.clone();
        base.runtime_queue_depth = counters.queue_depth;
        base.runtime_timeout_count = counters.timeout_count;
        base.runtime_cache_hits = counters.cache_hits;
        base.runtime_cache_misses = counters.cache_misses;
        base
    }

    fn lookup_uncached_blocking(&self, symbol: &str) -> Result<Option<DocumentationRecord>> {
        let config = self
            .config
            .lock()
            .expect("runtime docs config lock poisoned")
            .clone();
        if !config.enabled || config.interpreter_path.trim().is_empty() {
            return Ok(None);
        }
        let mut guard = self
            .process
            .lock()
            .expect("runtime docs process lock poisoned");
        if guard.is_none() {
            *guard = Some(self.spawn_process(&config)?);
        }
        let process = guard
            .as_mut()
            .ok_or_else(|| anyhow!("runtime docs process is unavailable"))?;
        let response = (|| -> Result<WorkerResponse> {
            let request = json!({ "symbol": symbol }).to_string();
            process.stdin.write_all(request.as_bytes())?;
            process.stdin.write_all(b"\n")?;
            process.stdin.flush()?;
            let line = process.read_line(WORKER_TIMEOUT)?;
            serde_json::from_str(&line)
                .with_context(|| format!("parse runtime docs response: {line}"))
        })();
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                *guard = None;
                let message = format!("runtime documentation lookup for {symbol}: {error}");
                return Err(error.context(message));
            }
        };
        if !response.ok {
            return Err(anyhow!(response
                .error
                .unwrap_or_else(|| "runtime lookup failed".to_string())));
        }
        let record = DocumentationRecord {
            name: response.name.unwrap_or_else(|| symbol.to_string()),
            module_name: response
                .module_name
                .unwrap_or_else(|| "sage.runtime".to_string()),
            kind: response.kind.unwrap_or_else(|| "RuntimeObject".to_string()),
            detail: response.detail.unwrap_or_default(),
            summary: response.summary.unwrap_or_else(|| symbol.to_string()),
            docstring: response.docstring,
            uri: response.uri.filter(|value| !value.is_empty()),
            markers: vec!["runtime".to_string()],
            sections: Vec::<DocumentationSection>::new(),
        };
        let mut counters = self
            .counters
            .lock()
            .expect("runtime docs counters lock poisoned");
        counters.state = "ready".to_string();
        counters.degraded_reason = None;
        Ok(Some(record))
    }

    fn spawn_process(&self, config: &RuntimeDocsConfig) -> Result<RuntimeDocsProcess> {
        self.counters
            .lock()
            .expect("runtime docs counters lock poisoned")
            .state = "starting".to_string();
        let mut command = Command::new(&config.interpreter_path);
        command.args(&config.interpreter_args);
        if looks_like_sage_command(&config.interpreter_path) {
            command.arg("-python");
        }
        command.arg("-u").arg("-c").arg(WORKER_SCRIPT);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command.env("PYTHONUNBUFFERED", "1");
        let runtime_home = std::env::temp_dir().join("sage-vscode-runtime-home");
        prepare_runtime_home(&runtime_home)?;
        // Keep the interpreter's HOME: replacing it also changes GAP's startup
        // configuration and can break sage.all imports. Isolate only the caches.
        command.env("DOT_SAGE", runtime_home.join(".sage"));
        command.env("XDG_CACHE_HOME", runtime_home.join(".cache"));
        if !config.source_roots.is_empty() {
            let joined = std::env::join_paths(&config.source_roots)
                .context("join runtime source roots for PYTHONPATH")?;
            command.env("PYTHONPATH", joined);
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("spawn runtime docs worker {}", config.interpreter_path))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("runtime docs worker stdin is unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("runtime docs worker stdout is unavailable"))?;
        let (sender, output) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let failed = line.is_err();
                if sender.send(line).is_err() || failed {
                    break;
                }
            }
        });
        let process = RuntimeDocsProcess {
            child,
            stdin,
            output,
        };
        let line = process.read_line(WORKER_STARTUP_TIMEOUT)?;
        let ready: WorkerReady = serde_json::from_str(&line)
            .with_context(|| format!("parse runtime docs worker ready response: {line}"))?;
        if !ready.ready {
            return Err(anyhow!(
                "{}",
                ready
                    .error
                    .unwrap_or_else(|| "Sage imports failed".to_string())
            ));
        }
        self.counters
            .lock()
            .expect("runtime docs counters lock poisoned")
            .state = "ready".to_string();
        Ok(process)
    }

    fn stop_process(&self) {
        self.process
            .lock()
            .expect("runtime docs process lock poisoned")
            .take();
    }

    fn finish_prefetch(&self, symbol: &str) {
        let queue_depth = {
            let mut inflight = self
                .inflight
                .lock()
                .expect("runtime docs inflight lock poisoned");
            inflight.remove(symbol);
            inflight.len()
        };
        self.counters
            .lock()
            .expect("runtime docs counters lock poisoned")
            .queue_depth = queue_depth;
    }
}

impl RuntimeDocsProcess {
    fn read_line(&self, timeout: Duration) -> Result<String> {
        self.output
            .recv_timeout(timeout)
            .with_context(|| {
                format!(
                    "runtime docs worker response exceeded {}ms or worker exited",
                    timeout.as_millis()
                )
            })?
            .context("read runtime docs worker response")
    }
}

impl Drop for RuntimeDocsProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn prepare_runtime_home(runtime_home: &Path) -> Result<()> {
    for directory in [runtime_home.join(".sage"), runtime_home.join(".cache")] {
        fs::create_dir_all(&directory).with_context(|| {
            format!(
                "create Sage runtime worker directory {}",
                directory.display()
            )
        })?;
    }
    Ok(())
}

fn looks_like_sage_command(path: &str) -> bool {
    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(path)
        .to_ascii_lowercase();
    name == "sage" || name.starts_with("sage-") || name.starts_with("sage.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sage_index::DocsStatus;

    fn base_status() -> DocsStatus {
        DocsStatus {
            doc_db_path: "/tmp/sage-index.sqlite".to_string(),
            offline_doc_count: 12,
            preferred_source: "auto".to_string(),
            runtime_worker_state: "static-fallback".to_string(),
            runtime_degraded_reason: Some("static fallback active".to_string()),
            runtime_queue_depth: 0,
            runtime_timeout_count: 0,
            runtime_cache_hits: 0,
            runtime_cache_misses: 0,
        }
    }

    #[tokio::test]
    async fn configure_reports_explicit_static_fallback_states() {
        let worker = RuntimeDocsWorker::default();

        worker
            .configure(RuntimeDocsConfig {
                enabled: true,
                interpreter_path: "/usr/bin/python3".to_string(),
                interpreter_args: Vec::new(),
                source_roots: Vec::new(),
            })
            .await;
        let status = worker.status(base_status()).await;
        assert_eq!(status.runtime_worker_state, "idle-static-fallback");
        assert_eq!(status.runtime_degraded_reason, None);

        worker
            .configure(RuntimeDocsConfig {
                enabled: false,
                interpreter_path: "/usr/bin/python3".to_string(),
                interpreter_args: Vec::new(),
                source_roots: Vec::new(),
            })
            .await;
        let status = worker.status(base_status()).await;
        assert_eq!(status.runtime_worker_state, "disabled");
        assert!(status
            .runtime_degraded_reason
            .as_deref()
            .unwrap_or_default()
            .contains("disabled"));

        worker
            .configure(RuntimeDocsConfig {
                enabled: true,
                interpreter_path: " ".to_string(),
                interpreter_args: Vec::new(),
                source_roots: Vec::new(),
            })
            .await;
        let status = worker.status(base_status()).await;
        assert_eq!(status.runtime_worker_state, "unavailable");
        assert!(status
            .runtime_degraded_reason
            .as_deref()
            .unwrap_or_default()
            .contains("empty"));
    }

    #[tokio::test]
    async fn hover_prefetch_starts_runtime_process_without_blocking() {
        let worker = RuntimeDocsWorker::default();
        worker
            .configure(RuntimeDocsConfig {
                enabled: true,
                interpreter_path: "/bin/false".to_string(),
                interpreter_args: Vec::new(),
                source_roots: Vec::new(),
            })
            .await;

        worker.prefetch("PolynomialRing");

        let status = worker.status(base_status()).await;
        assert_eq!(status.runtime_queue_depth, 1);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = worker.status(base_status()).await;
                if status.runtime_queue_depth == 0 {
                    assert_eq!(status.runtime_cache_misses, 1);
                    assert_eq!(status.runtime_worker_state, "degraded");
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("prefetch must finish without blocking the async executor");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cold_prefetch_allows_slow_startup_and_caches_documentation() {
        let worker = RuntimeDocsWorker::default();
        worker.configure(RuntimeDocsConfig {
            enabled: true,
            interpreter_path: "/bin/sh".to_string(),
            interpreter_args: vec!["-c".to_string(), r#"
sleep 1
printf '%s\n' '{"ready":true}'
while IFS= read -r request; do
    printf '%s\n' '{"ok":true,"name":"sin","docstring":"The sine function.","summary":"The sine function."}'
done
"#.to_string()],
            source_roots: Vec::new(),
        }).await;
        worker.prefetch("sin");
        worker.prefetch("sin");
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if let Some(record) = worker.cached("sin") {
                    assert_eq!(record.docstring.as_deref(), Some("The sine function."));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cold hover should eventually get real docs");
        let status = worker.status(base_status()).await;
        assert_eq!(status.runtime_cache_misses, 1);
        assert_eq!(status.runtime_timeout_count, 0);
        assert_eq!(status.runtime_degraded_reason, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stalled_response_times_out_and_discards_worker() {
        let worker = RuntimeDocsWorker::default();
        worker
            .configure(RuntimeDocsConfig {
                enabled: true,
                interpreter_path: "/bin/sh".to_string(),
                interpreter_args: vec![
                    "-c".to_string(),
                    "printf '%s\\n' '{\"ready\":true}'; read -r request; exec sleep 5".to_string(),
                ],
                source_roots: Vec::new(),
            })
            .await;
        let record = tokio::time::timeout(Duration::from_secs(5), worker.lookup("sin"))
            .await
            .expect("timeout must not block while trying to kill the worker");
        assert!(record.is_none());
        assert!(worker.process.lock().unwrap().is_none());
        let status = worker.status(base_status()).await;
        assert_eq!(status.runtime_timeout_count, 1);
        assert_eq!(status.runtime_worker_state, "degraded");
    }

    #[test]
    fn runtime_home_is_created_before_starting_sage() {
        let runtime_home =
            std::env::temp_dir().join(format!("sage-ls-runtime-home-test-{}", std::process::id()));
        fs::remove_dir_all(&runtime_home).ok();

        prepare_runtime_home(&runtime_home).expect("runtime home should be prepared");

        assert!(runtime_home.is_dir());
        assert!(runtime_home.join(".sage").is_dir());
        assert!(runtime_home.join(".cache").is_dir());
        fs::remove_dir_all(runtime_home).ok();
    }
}
