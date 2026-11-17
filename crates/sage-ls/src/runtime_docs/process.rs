use super::*;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;

pub(super) struct ProcessControl {
    child: Mutex<Option<Child>>,
    #[cfg(windows)]
    job: WindowsJob,
}

impl ProcessControl {
    fn new(child: Child) -> Result<Self> {
        #[cfg(windows)]
        let job = match WindowsJob::new(&child) {
            Ok(job) => job,
            Err(error) => {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        // Keep the child owned until reaping, so a PID cannot be reused before termination.
        Ok(Self {
            child: Mutex::new(Some(child)),
            #[cfg(windows)]
            job,
        })
    }

    pub(super) fn terminate(&self) {
        let child = lock(&self.child);
        #[cfg(unix)]
        if let Some(child) = child.as_ref() {
            // SAFETY: this PID is our unreaped child and was created in its own process group.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        if child.is_some() {
            self.job.terminate();
        }
    }

    fn reap(&self, home: Option<RuntimeHome>) {
        self.terminate();
        if let Some(mut child) = lock(&self.child).take() {
            // Never make the LSP executor wait for an OS process to be reaped.
            let _ = std::thread::Builder::new()
                .name("sage-docs-reap".into())
                .spawn(move || {
                    let _ = child.wait();
                    drop(home);
                });
        }
    }
}

impl Drop for ProcessControl {
    fn drop(&mut self) {
        self.reap(None);
    }
}

#[cfg(windows)]
struct WindowsJob(usize);
#[cfg(windows)]
impl WindowsJob {
    fn new(child: &Child) -> Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        // SAFETY: the job is private; the supplied child handle is live for this call.
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            let job = Self(handle as usize);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, child.as_raw_handle()) == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(job)
        }
    }
    fn terminate(&self) {
        // SAFETY: this job handle remains owned and open until Drop.
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0 as _, 1);
        }
    }
}
#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0 as _);
        }
    }
}

pub(super) struct RuntimeDocsProcess {
    pub(super) control: Arc<ProcessControl>,
    stdin: ChildStdin,
    output: mpsc::Receiver<std::io::Result<String>>,
    home: Option<RuntimeHome>,
    consecutive_timeouts: u32,
}

impl RuntimeDocsProcess {
    pub(super) fn spawn(session: &Session, cancelled: &AtomicBool) -> Result<Self> {
        let config = &session.config;
        let home = RuntimeHome::new()?;
        let mut command = Command::new(&config.interpreter_path);
        if looks_like_sage_command(&config.interpreter_path) {
            // The `sage` launcher only recognises `-python` as its first argument, and the
            // configured interpreter arguments are meant for running files, not for Python.
            command.arg("-python");
        } else {
            command.args(&config.interpreter_args);
        }
        // Isolated Python removes CWD, user-site and PYTHONPATH from import resolution.
        command.args(["-I", "-u", "-c", include_str!("worker.py")]);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // Preserve HOME: changing it breaks GAP configuration on supported Sage installations.
        command.env("DOT_SAGE", home.0.join(".sage"));
        command.env("XDG_CACHE_HOME", home.0.join(".cache"));
        command.env(
            "SAGE_LS_SOURCE_ROOTS",
            serde_json::to_string(&config.source_roots)?,
        );
        command.env_remove("PYTHONPATH");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("start runtime docs worker {}", config.interpreter_path))?;
        let stdin = child.stdin.take().expect("piped child stdin");
        let stdout = child.stdout.take().expect("piped child stdout");
        let control = Arc::new(ProcessControl::new(child)?);
        {
            let mut active = lock(&session.control);
            *active = Some(control.clone());
            if session.stopped.load(Ordering::Acquire) {
                control.terminate();
            }
        }
        let (sender, output) = mpsc::sync_channel(2);
        let process = Self {
            control,
            stdin,
            output,
            home: Some(home),
            consecutive_timeouts: 0,
        };
        std::thread::Builder::new()
            .name("sage-docs-output".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    let mut line = Vec::new();
                    // Bound protocol frames even if a broken interpreter never emits a newline.
                    use std::io::Read;
                    let result = reader
                        .by_ref()
                        .take(MAX_FRAME_BYTES + 1)
                        .read_until(b'\n', &mut line);
                    let message = match result {
                        Ok(0) => break,
                        Ok(_) if line.len() as u64 > MAX_FRAME_BYTES => {
                            Err(std::io::Error::other("runtime docs frame exceeds 8 MiB"))
                        }
                        Ok(_) => String::from_utf8(line)
                            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
                        Err(e) => Err(e),
                    };
                    let failed = message.is_err();
                    if sender.send(message).is_err() || failed {
                        break;
                    }
                }
            })?;
        let ready: WorkerReady =
            serde_json::from_str(&process.read_line(session, cancelled, WORKER_STARTUP_TIMEOUT)?)
                .context("invalid runtime docs startup response")?;
        if !ready.ready {
            return Err(anyhow!(ready
                .error
                .unwrap_or_else(|| "Sage imports failed".into())));
        }
        Ok(process)
    }

    /// Records a missed reply and returns how many were missed in a row.
    pub(super) fn note_timeout(&mut self) -> u32 {
        self.consecutive_timeouts += 1;
        self.consecutive_timeouts
    }

    pub(super) fn request(
        &mut self,
        session: &Session,
        cancelled: &AtomicBool,
        id: u64,
        symbol: &str,
    ) -> Result<WorkerResponse> {
        serde_json::to_writer(&mut self.stdin, &json!({"id":id,"symbol":symbol}))?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;
        let deadline = Instant::now() + WORKER_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let response: WorkerResponse =
                serde_json::from_str(&self.read_line(session, cancelled, remaining)?)
                    .context("invalid runtime docs response")?;
            // Replies to requests that timed out earlier arrive late and are skipped.
            if response.id < id {
                continue;
            }
            if response.id != id || response.name != symbol {
                return Err(anyhow!("runtime docs response does not match its request"));
            }
            self.consecutive_timeouts = 0;
            return Ok(response);
        }
    }

    fn read_line(
        &self,
        session: &Session,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<String> {
        let deadline = Instant::now() + timeout;
        loop {
            if session.stopped.load(Ordering::Acquire) || cancelled.load(Ordering::Acquire) {
                return Err(anyhow!("runtime documentation request cancelled"));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(mpsc::RecvTimeoutError::Timeout.into());
            }
            match self
                .output
                .recv_timeout(remaining.min(Duration::from_millis(25)))
            {
                Ok(line) => return line.context("read runtime docs response"),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
}
impl Drop for RuntimeDocsProcess {
    fn drop(&mut self) {
        self.control.reap(self.home.take());
    }
}

pub(super) struct RuntimeHome(pub(super) PathBuf);
impl RuntimeHome {
    pub(super) fn new() -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let root = std::env::temp_dir().join(format!(
                "sage-docs-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&root) {
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
                Ok(()) => {
                    let home = Self(root);
                    std::fs::create_dir(home.0.join(".sage"))?;
                    std::fs::create_dir(home.0.join(".cache"))?;
                    return Ok(home);
                }
            }
        }
    }
}
impl Drop for RuntimeHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn looks_like_sage_command(path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_ascii_lowercase();
    // `sage-python`, `sage-ipython` and similar helpers are Python entry points already.
    matches!(name.as_str(), "sage" | "sage.exe" | "sage.bat" | "sage.cmd" | "sage.sh")
}
