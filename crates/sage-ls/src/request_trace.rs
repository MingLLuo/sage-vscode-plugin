//! Per-request timing written to stderr, which VS Code shows in the "Sage Language Server" output channel.
//!
//! Requests slower than [`SLOW_REQUEST`] are always logged. With `SAGE_LS_LOG=debug` (set by the extension from
//! `sage.logging.level`) every request is logged, so a slow or missing result can be matched to its request.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tower_lsp_server::ls_types::{Position, Uri};

const SLOW_REQUEST: Duration = Duration::from_millis(1000);

fn log_every_request() -> bool {
    static VERBOSE: OnceLock<bool> = OnceLock::new();
    *VERBOSE.get_or_init(|| {
        std::env::var("SAGE_LS_LOG").is_ok_and(|level| level.eq_ignore_ascii_case("debug"))
    })
}

/// Logs `method`, `detail`, and the elapsed time when dropped (at the end of the handler).
pub(crate) struct RequestTrace {
    method: &'static str,
    detail: String,
    started: Instant,
}

impl RequestTrace {
    pub(crate) fn start(method: &'static str, detail: String) -> Self {
        Self {
            method,
            detail,
            started: Instant::now(),
        }
    }

    pub(crate) fn at(method: &'static str, uri: &Uri, position: Position) -> Self {
        Self::start(
            method,
            format!(
                "{}:{}:{}",
                file_label(uri),
                position.line + 1,
                position.character + 1
            ),
        )
    }

    pub(crate) fn file(method: &'static str, uri: &Uri) -> Self {
        Self::start(method, file_label(uri))
    }
}

impl Drop for RequestTrace {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed();
        if let Some(line) = trace_line(self.method, &self.detail, elapsed, log_every_request()) {
            eprintln!("{line}");
        }
    }
}

fn trace_line(method: &str, detail: &str, elapsed: Duration, verbose: bool) -> Option<String> {
    let slow = elapsed >= SLOW_REQUEST;
    if !slow && !verbose {
        return None;
    }
    let marker = if slow { " (slow)" } else { "" };
    Some(format!(
        "[sage-ls] {method} {detail} {} ms{marker}",
        elapsed.as_millis()
    ))
}

/// The file name only: enough to identify the request without copying full local paths into logs.
fn file_label(uri: &Uri) -> String {
    let path = uri.path().as_str();
    path.rsplit('/').next().unwrap_or(path).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_requests_are_logged_only_in_debug_mode() {
        assert_eq!(
            trace_line("hover", "a.sage:1:1", Duration::from_millis(5), false),
            None
        );
        assert_eq!(
            trace_line("hover", "a.sage:1:1", Duration::from_millis(5), true).as_deref(),
            Some("[sage-ls] hover a.sage:1:1 5 ms")
        );
    }

    #[test]
    fn slow_requests_are_always_logged_and_marked() {
        assert_eq!(
            trace_line(
                "references",
                "a.sage:3:7",
                Duration::from_millis(1500),
                false
            )
            .as_deref(),
            Some("[sage-ls] references a.sage:3:7 1500 ms (slow)")
        );
    }

    #[test]
    fn file_label_keeps_only_the_file_name() {
        let uri: Uri = "file:///home/me/project/src/demo.sage".parse().unwrap();
        assert_eq!(file_label(&uri), "demo.sage");
        assert_eq!(
            RequestTrace::at("hover", &uri, Position::new(0, 4)).detail,
            "demo.sage:1:5"
        );
    }
}
