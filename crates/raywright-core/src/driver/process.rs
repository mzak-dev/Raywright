use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::{broadcast, mpsc, watch};

use crate::driver::ProcessId;
use crate::error::{Error, Result};
use crate::launch::LaunchSpec;

/// A process Raywright started (or a stand-in for one, in a mock driver).
#[async_trait]
pub trait ProcessHandle: Send + Sync {
    fn pid(&self) -> ProcessId;
    /// `Some(code)` once it exited (`code` is `None` when killed by a signal).
    fn exit_status(&self) -> Option<Option<i32>>;
    async fn wait(&self) -> Option<i32>;
    async fn kill(&self) -> Result<()>;
    /// What it printed on stdout and stderr.
    fn output(&self) -> OutputLog;
}

const KEEP_LINES: usize = 500;

/// The last lines a process printed, and a feed of new ones.
#[derive(Clone)]
pub struct OutputLog {
    inner: Arc<LogInner>,
}

struct LogInner {
    lines: Mutex<VecDeque<String>>,
    tx: broadcast::Sender<String>,
}

impl Default for OutputLog {
    fn default() -> Self {
        OutputLog { inner: Arc::new(LogInner { lines: Mutex::new(VecDeque::new()), tx: broadcast::channel(256).0 }) }
    }
}

impl std::fmt::Debug for OutputLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputLog").field("lines", &self.inner.lines.lock().unwrap().len()).finish()
    }
}

impl OutputLog {
    pub fn push(&self, line: impl Into<String>) {
        let line = line.into();
        {
            let mut lines = self.inner.lines.lock().unwrap();
            if lines.len() == KEEP_LINES {
                lines.pop_front();
            }
            lines.push_back(line.clone());
        }
        let _ = self.inner.tx.send(line);
    }

    pub fn lines(&self) -> Vec<String> {
        self.inner.lines.lock().unwrap().iter().cloned().collect()
    }

    /// The last `n` lines, indented, for error messages.
    pub fn tail(&self, n: usize) -> String {
        let lines = self.inner.lines.lock().unwrap();
        lines.iter().skip(lines.len().saturating_sub(n)).map(|l| format!("    {l}")).collect::<Vec<_>>().join("\n")
    }

    /// New lines from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.inner.tx.subscribe()
    }
}

/// A real child process, run through tokio. Killed when dropped.
pub struct OsProcess {
    pid: ProcessId,
    output: OutputLog,
    exit: watch::Receiver<Option<Option<i32>>>,
    kill: mpsc::Sender<()>,
}

impl OsProcess {
    pub async fn spawn(spec: &LaunchSpec, env: &[(String, String)]) -> Result<OsProcess> {
        let mut cmd = tokio::process::Command::new(&spec.program);
        cmd.args(&spec.args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        for (k, v) in spec.env.iter().chain(env) {
            cmd.env(k, v);
        }
        if let Some(dir) = &spec.cwd {
            cmd.current_dir(dir);
        }
        let mut child = cmd.spawn().map_err(|e| Error::Launch { program: spec.program.display().to_string(), reason: e.to_string() })?;
        let pid = ProcessId(child.id().ok_or_else(|| Error::Launch { program: spec.program.display().to_string(), reason: "exited at once".into() })?);
        let output = OutputLog::default();
        let echo = spec.echo_output;
        let name = spec.program.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(out) = child.stdout.take() {
            tokio::spawn(pump(out, output.clone(), echo.then(|| format!("[{name}] "))));
        }
        if let Some(err) = child.stderr.take() {
            tokio::spawn(pump(err, output.clone(), echo.then(|| format!("[{name}!] "))));
        }
        let (exit_tx, exit) = watch::channel(None);
        let (kill, mut kill_rx) = mpsc::channel::<()>(1);
        tokio::spawn(async move {
            let status = tokio::select! {
                s = child.wait() => s,
                _ = kill_rx.recv() => {
                    let _ = child.start_kill();
                    child.wait().await
                }
            };
            let _ = exit_tx.send(Some(status.ok().and_then(|s| s.code())));
        });
        Ok(OsProcess { pid, output, exit, kill })
    }
}

async fn pump(stream: impl AsyncRead + Unpin, log: OutputLog, echo: Option<String>) {
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(prefix) = &echo {
            eprintln!("{prefix}{line}");
        }
        log.push(line);
    }
}

#[async_trait]
impl ProcessHandle for OsProcess {
    fn pid(&self) -> ProcessId {
        self.pid
    }

    fn exit_status(&self) -> Option<Option<i32>> {
        *self.exit.borrow()
    }

    async fn wait(&self) -> Option<i32> {
        let mut rx = self.exit.clone();
        loop {
            if let Some(code) = *rx.borrow_and_update() {
                return code;
            }
            if rx.changed().await.is_err() {
                return None;
            }
        }
    }

    async fn kill(&self) -> Result<()> {
        let _ = self.kill.try_send(());
        self.wait().await;
        Ok(())
    }

    fn output(&self) -> OutputLog {
        self.output.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_keeps_the_last_lines() {
        let log = OutputLog::default();
        for i in 0..KEEP_LINES + 5 {
            log.push(i.to_string());
        }
        let lines = log.lines();
        assert_eq!(lines.len(), KEEP_LINES);
        assert_eq!(lines[0], "5");
        assert_eq!(log.tail(2), format!("    {}\n    {}", KEEP_LINES + 3, KEEP_LINES + 4));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runs_a_process_and_reads_its_output() {
        let spec = LaunchSpec::exe("sh").args(["-c", "echo hello; echo oops >&2; exit 3"]);
        let p = OsProcess::spawn(&spec, &[]).await.unwrap();
        assert_eq!(p.wait().await, Some(3));
        // the pumps may finish just after the exit
        for _ in 0..50 {
            if p.output().lines().len() == 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let mut lines = p.output().lines();
        lines.sort();
        assert_eq!(lines, vec!["hello", "oops"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kill_stops_it() {
        let p = OsProcess::spawn(&LaunchSpec::exe("sleep").args(["30"]), &[]).await.unwrap();
        assert_eq!(p.exit_status(), None);
        p.kill().await.unwrap();
        assert!(p.exit_status().is_some());
    }
}
