use super::*;

fn base_status() -> DocsStatus {
    DocsStatus {
        doc_db_path: "test.sqlite".into(),
        offline_doc_count: 12,
        preferred_source: "auto".into(),
        runtime_worker_state: "static-fallback".into(),
        runtime_degraded_reason: None,
        runtime_queue_depth: 0,
        runtime_timeout_count: 0,
        runtime_cache_hits: 0,
        runtime_cache_misses: 0,
    }
}
async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(4), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("condition should settle without blocking Tokio");
}
#[cfg(unix)]
fn fake_python(script: &str) -> RuntimeDocsConfig {
    RuntimeDocsConfig {
        enabled: true,
        interpreter_path: "/usr/bin/python3".into(),
        interpreter_args: vec!["-I".into(), "-u".into(), "-c".into(), script.into()],
        source_roots: vec![],
    }
}
#[cfg(unix)]
fn responder(body: &str) -> RuntimeDocsConfig {
    fake_python(&format!("import json, sys, time, os\nprint(json.dumps({{'ready':True}}), flush=True)\nfor line in sys.stdin:\n r=json.loads(line)\n {body}\n"))
}
#[tokio::test]
async fn configure_reports_explicit_static_fallback_states() {
    let worker = RuntimeDocsWorker::default();
    worker
        .configure(RuntimeDocsConfig {
            enabled: true,
            interpreter_path: "python3".into(),
            ..Default::default()
        })
        .await;
    assert_eq!(
        worker.status(base_status()).await.runtime_worker_state,
        "idle-static-fallback"
    );
    worker.configure(RuntimeDocsConfig::default()).await;
    assert_eq!(
        worker.status(base_status()).await.runtime_worker_state,
        "disabled"
    );
    worker
        .configure(RuntimeDocsConfig {
            enabled: true,
            ..Default::default()
        })
        .await;
    assert_eq!(
        worker.status(base_status()).await.runtime_worker_state,
        "unavailable"
    );
}
#[tokio::test]
async fn failed_start_backs_off_without_blocking_or_relaunching_on_every_hover() {
    let worker = RuntimeDocsWorker::default();
    worker
        .configure(RuntimeDocsConfig {
            enabled: true,
            interpreter_path: "sage-docs-no-such-interpreter".into(),
            ..Default::default()
        })
        .await;
    worker.prefetch("sin");
    assert_eq!(worker.status(base_status()).await.runtime_queue_depth, 1);
    until(|| lock(&worker.session().state).inflight.is_empty()).await;
    for _ in 0..20 {
        assert!(worker.lookup("sin").await.is_none());
        worker.prefetch("cos");
    }
    let status = worker.status(base_status()).await;
    assert_eq!(status.runtime_worker_state, "degraded");
    assert_eq!(status.runtime_cache_misses, 1);
}
#[cfg(unix)]
#[tokio::test]
async fn cold_prefetch_coalesces_and_caches_documentation() {
    let worker = RuntimeDocsWorker::default();
    let config = responder("time.sleep(0.1); print(json.dumps({'id':r['id'],'name':r['symbol'],'ok':True,'docstring':'Sine docs'}), flush=True)");
    worker.configure(config).await;
    worker.prefetch("sin");
    worker.prefetch("sin");
    until(|| worker.cached("sin").is_some()).await;
    assert_eq!(
        worker.cached("sin").unwrap().docstring.as_deref(),
        Some("Sine docs")
    );
    assert_eq!(worker.status(base_status()).await.runtime_cache_misses, 1);
}
#[cfg(unix)]
#[tokio::test]
async fn missing_symbols_are_negative_cached_without_degrading_the_worker() {
    let worker = RuntimeDocsWorker::default();
    worker.configure(responder("print(json.dumps({'id':r['id'],'name':r['symbol'],'ok':r['symbol']=='sin','missing':r['symbol']!='sin'}), flush=True)")).await;
    assert!(worker.lookup("absent").await.is_none());
    assert!(worker.lookup("absent").await.is_none());
    assert!(worker.lookup("sin").await.is_some());
    let status = worker.status(base_status()).await;
    assert_eq!(status.runtime_worker_state, "ready");
    assert_eq!(status.runtime_degraded_reason, None);
    assert_eq!(status.runtime_cache_misses, 2);
    assert_eq!(status.runtime_cache_hits, 1);
}
#[cfg(unix)]
#[tokio::test]
async fn mismatched_response_is_discarded_and_worker_is_stopped() {
    for body in [
        "print(json.dumps({'id':r['id']+1,'name':r['symbol'],'ok':True}), flush=True)",
        "print(json.dumps({'id':r['id'],'name':'wrong','ok':True}), flush=True)",
    ] {
        let worker = RuntimeDocsWorker::default();
        worker.configure(responder(body)).await;
        assert!(worker.lookup("sin").await.is_none());
        assert!(lock(&worker.session().process).is_none());
        assert!(worker
            .status(base_status())
            .await
            .runtime_degraded_reason
            .unwrap()
            .contains("does not match"));
    }
}
#[cfg(unix)]
#[tokio::test]
async fn stalled_response_times_out_and_discards_worker() {
    let worker = RuntimeDocsWorker::default();
    worker.configure(responder("time.sleep(30)")).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(5), worker.lookup("sin"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(lock(&worker.session().process).is_none());
    assert_eq!(worker.status(base_status()).await.runtime_timeout_count, 1);
}
#[cfg(unix)]
#[tokio::test]
async fn cancellation_releases_active_request_and_does_not_enter_backoff() {
    let worker = RuntimeDocsWorker::default();
    worker.configure(responder("time.sleep(30)")).await;
    let session = worker.session();
    let lookup = tokio::spawn({
        let session = session.clone();
        async move { session.lookup("sin").await }
    });
    until(|| lock(&session.control).is_some()).await;
    lookup.abort();
    let _ = lookup.await;
    until(|| session.slots.available_permits() == 1).await;
    assert!(lock(&session.process).is_none());
    assert_eq!(lock(&session.state).retry_at, None);
    assert_eq!(lock(&session.state).name, "idle-static-fallback");
}
#[cfg(unix)]
#[tokio::test]
async fn reconfigure_interrupts_startup_and_old_results_cannot_enter_new_session() {
    let worker = RuntimeDocsWorker::default();
    worker
        .configure(fake_python("import time; time.sleep(30)"))
        .await;
    worker.prefetch("sin");
    let old = worker.session();
    until(|| lock(&old.control).is_some()).await;
    tokio::time::timeout(Duration::from_millis(200), worker.configure(responder(
        "print(json.dumps({'id':r['id'],'name':r['symbol'],'ok':True,'summary':'new session'}), flush=True)"
    ))).await.expect("reconfigure must not wait for the old process lock");
    assert_eq!(worker.lookup("sin").await.unwrap().summary, "new session");
    until(|| old.slots.available_permits() == 1).await;
    assert_eq!(worker.status(base_status()).await.runtime_cache_misses, 1);
}
#[cfg(unix)]
#[tokio::test]
async fn shutdown_kills_worker_descendants_and_bounds_prefetch_queue() {
    let home = process::RuntimeHome::new().unwrap();
    let marker = home.0.join("descendant-survived");
    let code = format!(
        "import time; time.sleep(1); open({}, 'w').write('alive')",
        serde_json::to_string(&marker).unwrap()
    );
    let script = format!("import json,sys,subprocess,time\nsubprocess.Popen([sys.executable,'-c',{}])\nprint(json.dumps({{'ready':True}}),flush=True)\nfor line in sys.stdin: time.sleep(30)", serde_json::to_string(&code).unwrap());
    let worker = RuntimeDocsWorker::default();
    worker.configure(fake_python(&script)).await;
    for i in 0..100 {
        worker.prefetch(&format!("symbol_{i}"));
    }
    assert!(worker.status(base_status()).await.runtime_queue_depth <= MAX_PREFETCH);
    let session = worker.session();
    until(|| lock(&session.control).is_some()).await;
    // Allow the stub to spawn its descendant before terminating the whole group.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let started = Instant::now();
    worker.shutdown();
    assert!(started.elapsed() < Duration::from_millis(100));
    until(|| session.slots.available_permits() == 1).await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(
        !marker.exists(),
        "the descendant must not outlive server shutdown"
    );
    assert_eq!(
        worker.status(base_status()).await.runtime_worker_state,
        "stopped"
    );
}
#[cfg(unix)]
#[tokio::test]
async fn worker_protocol_survives_python_and_native_stdout_noise() {
    let script = format!(
        r#"
import sys, types, os
allmod = types.ModuleType('sage.all')
allmod.sin = lambda x: x
allmod.__all__ = ['sin']
inspectmod = types.ModuleType('sage.misc.sageinspect')
def docs(*args, **kwargs):
    print('ordinary stdout noise')
    os.write(1, b'native stdout noise\n')
    return 'isolated docs'
inspectmod.sage_getdoc = docs
inspectmod.sage_getdef = lambda *a: 'sin(x)'
inspectmod.sage_getfile = lambda *a: '/tmp/sin.py'
inspectmod.sage_getsourcelines = lambda *a: (['def sin(x):'], 7)
sys.modules['sage.all'] = allmod
sys.modules['sage.misc.sageinspect'] = inspectmod
exec({})
"#,
        serde_json::to_string(include_str!("worker.py")).unwrap()
    );
    let worker = RuntimeDocsWorker::default();
    worker.configure(fake_python(&script)).await;
    assert_eq!(
        worker.lookup("sin").await.unwrap().docstring.as_deref(),
        Some("isolated docs")
    );
    assert_eq!(worker.source_location("sin").await.unwrap().line, 6);
    assert!(worker.lookup("missing").await.is_none());
    assert_eq!(
        worker.status(base_status()).await.runtime_worker_state,
        "ready"
    );
}
#[test]
fn runtime_cache_directories_are_private_unique_and_removed() {
    let first = process::RuntimeHome::new().unwrap();
    let second = process::RuntimeHome::new().unwrap();
    assert_ne!(first.0, second.0);
    assert!(first.0.join(".sage").is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&first.0).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    let path = first.0.clone();
    drop(first);
    assert!(!path.exists());
}
