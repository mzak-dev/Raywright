use std::path::PathBuf;
use std::time::Duration;

use crate::driver::WindowId;

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong. Messages are written to be read in a failed test's output:
/// they say what was being done, on what, and what was seen last.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Auto-waiting ran out of time. `last` is the last reason the action could not run yet.
    #[error("timed out after {} ms waiting to {action} {target}\n  last state: {last}", waited.as_millis())]
    Timeout { action: String, target: String, waited: Duration, last: String },

    #[error("{target} matched no element")]
    NotFound { target: String },

    /// Strict mode: an action needs exactly one element.
    #[error("strict mode violation: {target} matched {count} elements:\n{examples}\n  use .first(), .nth(i) or a narrower selector")]
    Ambiguous { target: String, count: usize, examples: String },

    #[error("invalid selector `{selector}`: {reason}")]
    InvalidSelector { selector: String, reason: String },

    #[error("expect({subject}).{matcher} failed\n  expected: {expected}\n  received: {received}")]
    Expect { subject: String, matcher: String, expected: String, received: String },

    #[error("snapshot `{name}` does not match: {detail}\n  actual: {}{}", actual.display(), diff.as_ref().map(|d| format!("\n  diff:   {}", d.display())).unwrap_or_default())]
    SnapshotMismatch { name: String, detail: String, actual: PathBuf, diff: Option<PathBuf> },

    #[error("window {0} is closed")]
    WindowClosed(WindowId),

    #[error("the app exited{}{}", code.map(|c| format!(" with code {c}")).unwrap_or_default(), if stderr_tail.is_empty() { String::new() } else { format!("\n  last output:\n{stderr_tail}") })]
    AppExited { code: Option<i32>, stderr_tail: String },

    #[error("could not launch {program}: {reason}")]
    Launch { program: String, reason: String },

    #[error("bridge: {0}")]
    Bridge(String),

    #[error("the app answered with an error: {0}")]
    Rpc(#[from] raywright_protocol::RpcError),

    #[error("{what} is not supported by the {driver} driver")]
    Unsupported { what: String, driver: String },

    #[error("Raywright has no driver for {0} yet (only Windows today); use a mock driver to run on this OS")]
    UnsupportedPlatform(String),

    #[error("config: {0}")]
    Config(String),

    #[error("image: {0}")]
    Image(String),

    #[error("{0}")]
    Platform(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }

    pub fn unsupported(what: impl Into<String>, driver: impl Into<String>) -> Self {
        Error::Unsupported { what: what.into(), driver: driver.into() }
    }

    pub fn invalid_selector(selector: impl Into<String>, reason: impl Into<String>) -> Self {
        Error::InvalidSelector { selector: selector.into(), reason: reason.into() }
    }

    /// Worth retrying while auto-waiting: the element or window may still appear.
    pub fn is_transient(&self) -> bool {
        matches!(self, Error::NotFound { .. } | Error::WindowClosed(_))
    }
}

impl From<image::ImageError> for Error {
    fn from(e: image::ImageError) -> Self {
        Error::Image(e.to_string())
    }
}

impl From<raywright_protocol::DecodeError> for Error {
    fn from(e: raywright_protocol::DecodeError) -> Self {
        Error::Bridge(e.to_string())
    }
}
