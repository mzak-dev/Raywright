//! What `#[raywright::test]` runs: a tokio runtime, a test context (name, traces, soft
//! failures), and reports at the end.

use std::fmt;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures::FutureExt;

use crate::registry::Registry;
use crate::report::{Reporter, TestOutcome};
use crate::trace::TraceRecorder;

tokio::task_local! {
    static CURRENT: Arc<TestInfo>;
}

/// The test running now.
pub struct TestInfo {
    pub name: String,
    pub file: String,
    started: Instant,
    tracers: Mutex<Vec<Arc<TraceRecorder>>>,
    soft: Mutex<Vec<String>>,
    artifacts: Mutex<Vec<PathBuf>>,
}

impl TestInfo {
    pub fn new(name: &str, file: &str) -> Self {
        TestInfo { name: name.to_string(), file: file.to_string(), started: Instant::now(), tracers: Mutex::default(), soft: Mutex::default(), artifacts: Mutex::default() }
    }
}

pub fn current() -> Option<Arc<TestInfo>> {
    CURRENT.try_with(Arc::clone).ok()
}

/// The running test's name, for picture and trace paths: from `#[raywright::test]`, else
/// the test thread's name (libtest names it after the test), else `"test"`.
pub fn test_id() -> String {
    if let Some(t) = current() {
        return t.name.clone();
    }
    match std::thread::current().name() {
        Some(n) if n != "main" && !n.starts_with("tokio-runtime") => n.to_string(),
        _ => "test".into(),
    }
}

pub(crate) fn register_tracer(t: Arc<TraceRecorder>) {
    if let Some(info) = current() {
        t.managed.store(true, Ordering::SeqCst);
        info.tracers.lock().unwrap().push(t);
    }
}

/// Records a soft failure; `false` when not inside a test (then it isn't soft).
pub(crate) fn add_soft_failure(msg: String) -> bool {
    match current() {
        Some(info) => {
            eprintln!("raywright: soft assertion failed: {msg}");
            info.soft.lock().unwrap().push(msg);
            true
        }
        None => false,
    }
}

pub(crate) fn add_artifact(p: PathBuf) {
    if let Some(info) = current() {
        info.artifacts.lock().unwrap().push(p);
    }
}

/// What a test body may return.
pub trait TestResult {
    fn into_outcome(self) -> Result<(), String>;
}

impl TestResult for () {
    fn into_outcome(self) -> Result<(), String> {
        Ok(())
    }
}

impl<E: fmt::Display> TestResult for Result<(), E> {
    fn into_outcome(self) -> Result<(), String> {
        self.map_err(|e| e.to_string())
    }
}

/// Runs one test body. Used by `#[raywright::test]`; not meant to be called directly.
#[doc(hidden)]
pub fn run_test<F>(name: &str, file: &str, body: F)
where
    F: Future,
    F::Output: TestResult,
{
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("tokio runtime");
    let info = Arc::new(TestInfo::new(name, file));
    let result = rt.block_on(CURRENT.scope(info.clone(), AssertUnwindSafe(body).catch_unwind()));
    let mut failed = result.is_err();
    let mut outcome_error: Option<String> = None;
    let mut panic = None;
    match result {
        Ok(r) => {
            if let Err(e) = r.into_outcome() {
                failed = true;
                outcome_error = Some(e);
            }
        }
        Err(p) => {
            outcome_error = Some(panic_message(&p));
            panic = Some(p);
        }
    }
    let soft = std::mem::take(&mut *info.soft.lock().unwrap());
    if !soft.is_empty() && !failed {
        failed = true;
        outcome_error = Some(format!("{} soft assertion(s) failed:\n{}", soft.len(), soft.join("\n")));
    }
    for t in info.tracers.lock().unwrap().drain(..) {
        match t.finish(name, failed) {
            Ok(Some(dir)) => info.artifacts.lock().unwrap().push(dir.join("report.html")),
            Ok(None) => {}
            Err(e) => eprintln!("raywright: could not save a trace: {e}"),
        }
    }
    let mut artifacts = info.artifacts.lock().unwrap().clone();
    artifacts.dedup();
    let outcome = TestOutcome { name: name.to_string(), file: file.to_string(), passed: !failed, duration: info.started.elapsed(), error: outcome_error.clone(), artifacts };
    for r in reporters() {
        r.on_test_end(&outcome);
    }
    rt.shutdown_timeout(std::time::Duration::from_secs(5));
    if let Some(p) = panic {
        std::panic::resume_unwind(p);
    }
    if let Some(e) = outcome_error {
        panic!("{e}");
    }
}

fn reporters() -> Vec<Arc<dyn Reporter>> {
    Registry::with_builtins().reporters()
}

fn panic_message(p: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "panicked".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_passing_body_passes() {
        run_test("t::ok", file!(), async {
            assert_eq!(test_id(), "t::ok");
            Ok::<(), crate::Error>(())
        });
    }

    #[test]
    #[should_panic(expected = "boom")]
    fn an_err_fails_the_test() {
        run_test("t::err", file!(), async { Err::<(), _>(crate::Error::other("boom")) });
    }

    #[test]
    #[should_panic(expected = "soft assertion")]
    fn soft_failures_fail_at_the_end() {
        run_test("t::soft", file!(), async {
            assert!(add_soft_failure("first".into()));
        });
    }

    #[test]
    fn outside_a_test_the_thread_name_is_the_id() {
        assert!(test_id().contains("outside_a_test_the_thread_name_is_the_id"));
    }
}
