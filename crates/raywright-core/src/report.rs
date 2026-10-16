//! Telling people how a test went: on the console, as GitHub annotations, as JUnit XML.

use std::path::PathBuf;
use std::time::Duration;

/// How one test ended.
#[derive(Clone, Debug)]
pub struct TestOutcome {
    pub name: String,
    pub file: String,
    pub passed: bool,
    pub duration: Duration,
    pub error: Option<String>,
    /// Traces, diffs, recordings it left behind.
    pub artifacts: Vec<PathBuf>,
}

pub trait Reporter: Send + Sync {
    fn on_test_end(&self, outcome: &TestOutcome);
}

/// Points at what a failed test left behind.
pub struct ConsoleReporter;

impl Reporter for ConsoleReporter {
    fn on_test_end(&self, o: &TestOutcome) {
        if !o.passed && !o.artifacts.is_empty() {
            eprintln!("raywright: {} failed; see:", o.name);
            for a in &o.artifacts {
                eprintln!("  {}", a.display());
            }
        }
    }
}

/// `::error` annotations when running in GitHub Actions.
pub struct GithubReporter;

impl Reporter for GithubReporter {
    fn on_test_end(&self, o: &TestOutcome) {
        if o.passed || std::env::var_os("GITHUB_ACTIONS").is_none() {
            return;
        }
        let msg = o.error.as_deref().unwrap_or("failed").replace('%', "%25").replace('\r', "%0D").replace('\n', "%0A");
        println!("::error file={},title={}::{}", o.file, o.name.replace(',', "%2C").replace(':', "%3A"), msg);
    }
}

/// One JUnit XML file per test in `dir`, for CI dashboards.
pub struct JunitReporter {
    pub dir: PathBuf,
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

impl Reporter for JunitReporter {
    fn on_test_end(&self, o: &TestOutcome) {
        let failure = match (&o.error, o.passed) {
            (_, true) => String::new(),
            (e, false) => format!("<failure message=\"{}\">{}</failure>", xml(e.as_deref().unwrap_or("failed").lines().next().unwrap_or("")), xml(e.as_deref().unwrap_or(""))),
        };
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuite name=\"raywright\" tests=\"1\" failures=\"{}\"><testcase name=\"{}\" classname=\"{}\" time=\"{:.3}\">{failure}</testcase></testsuite>\n",
            u8::from(!o.passed),
            xml(&o.name),
            xml(&o.file),
            o.duration.as_secs_f64()
        );
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = std::fs::write(self.dir.join(format!("{}.xml", crate::snapshot::sanitize(&o.name))), body);
    }
}
