//! Settings, in layers from weakest to strongest:
//!
//! 1. the defaults here;
//! 2. `raywright.toml` (in `$RAYWRIGHT_CONFIG`, else the crate under test, else the working
//!    directory);
//! 3. `RAYWRIGHT_*` environment variables;
//! 4. [`crate::RaywrightBuilder`] calls;
//! 5. options on a single call (`.timeout()` on a click).
//!
//! A [`Config`] never changes once built; a stronger layer makes a new one.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{Error, Result};

/// A duration written as `"500ms"`, `"5s"`, `"2m"` or a number of milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dur(pub Duration);

impl Dur {
    pub const fn ms(ms: u64) -> Dur {
        Dur(Duration::from_millis(ms))
    }
    pub const fn secs(s: u64) -> Dur {
        Dur(Duration::from_secs(s))
    }
}

impl From<Duration> for Dur {
    fn from(d: Duration) -> Self {
        Dur(d)
    }
}

impl From<Dur> for Duration {
    fn from(d: Dur) -> Self {
        d.0
    }
}

impl fmt::Display for Dur {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ms = self.0.as_millis();
        if ms % 1000 == 0 && ms > 0 { write!(f, "{}s", ms / 1000) } else { write!(f, "{ms}ms") }
    }
}

impl FromStr for Dur {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let s = s.trim();
        let (num, unit) = match s.find(|c: char| !(c.is_ascii_digit() || c == '.')) {
            Some(i) => (&s[..i], s[i..].trim()),
            None => (s, "ms"),
        };
        let n: f64 = num.parse().map_err(|_| format!("`{s}` is not a duration (try 500ms, 5s, 2m)"))?;
        let ms = match unit {
            "ms" => n,
            "s" => n * 1000.0,
            "m" | "min" => n * 60_000.0,
            other => return Err(format!("unknown duration unit `{other}` in `{s}`")),
        };
        if !ms.is_finite() || ms < 0.0 {
            return Err(format!("`{s}` is not a duration"));
        }
        Ok(Dur(Duration::from_secs_f64(ms / 1000.0)))
    }
}

impl Serialize for Dur {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Dur {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Ms(u64),
            Text(String),
        }
        match Raw::deserialize(d)? {
            Raw::Ms(ms) => Ok(Dur::ms(ms)),
            Raw::Text(t) => t.parse().map_err(serde::de::Error::custom),
        }
    }
}

/// How actions reach the app.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum InputMode {
    /// Real OS input (moves the real cursor). Closest to a user; one test at a time.
    #[default]
    Os,
    /// Window messages posted to the target window: leaves the user's cursor alone.
    Message,
    /// The element's own API where its source has one (UI Automation Invoke and Value,
    /// bridge `input`), else `Message`.
    Native,
}

impl FromStr for InputMode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "os" => Ok(InputMode::Os),
            "message" => Ok(InputMode::Message),
            "native" => Ok(InputMode::Native),
            _ => Err(format!("input mode `{s}`: use os, message or native")),
        }
    }
}

/// Where pictures come from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum CaptureSource {
    /// The composed screen, as the user sees it (overlapping windows included).
    #[default]
    Screen,
    /// The window's own content, even when covered.
    Window,
    /// The app renders its own frame through the bridge: the same pixels on any desktop.
    App,
}

impl FromStr for CaptureSource {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "screen" => Ok(CaptureSource::Screen),
            "window" => Ok(CaptureSource::Window),
            "app" => Ok(CaptureSource::App),
            _ => Err(format!("capture source `{s}`: use screen, window or app")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum TraceMode {
    #[default]
    Off,
    On,
    /// Record always, keep only for failed tests.
    RetainOnFailure,
}

impl FromStr for TraceMode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "off" | "0" | "false" => Ok(TraceMode::Off),
            "on" | "1" | "true" => Ok(TraceMode::On),
            "retain-on-failure" => Ok(TraceMode::RetainOnFailure),
            _ => Err(format!("trace mode `{s}`: use off, on or retain-on-failure")),
        }
    }
}

/// When `to_have_screenshot` writes the expected picture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum UpdateSnapshots {
    /// Write missing ones, except in CI (`CI` is set) where a missing one fails.
    #[default]
    Missing,
    /// Overwrite every one that differs.
    All,
    /// Never write; missing ones fail.
    None,
}

impl FromStr for UpdateSnapshots {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "missing" => Ok(UpdateSnapshots::Missing),
            "all" | "1" | "true" => Ok(UpdateSnapshots::All),
            "none" | "0" | "false" => Ok(UpdateSnapshots::None),
            _ => Err(format!("update mode `{s}`: use missing, all or none")),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct SnapshotConfig {
    /// Expected pictures, relative to the config's directory.
    pub dir: PathBuf,
    /// Per-pixel color distance (0..1) still counted as equal.
    pub threshold: f64,
    pub max_diff_pixels: u64,
    /// Share of pixels (0..1) allowed to differ.
    pub max_diff_ratio: f64,
    /// Count anti-aliased pixels as differences.
    pub include_aa: bool,
    pub update: UpdateSnapshots,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        SnapshotConfig { dir: "tests/__screenshots__".into(), threshold: 0.1, max_diff_pixels: 0, max_diff_ratio: 0.0, include_aa: false, update: UpdateSnapshots::Missing }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct RecordConfig {
    pub fps: f64,
    pub max_duration: Dur,
    pub max_frames: usize,
    /// No change for this long ends a recording that waits to settle.
    pub settle: Dur,
    /// A pixel whose color moves less than this (0..1) is "unchanged".
    pub change_threshold: f64,
}

impl Default for RecordConfig {
    fn default() -> Self {
        RecordConfig { fps: 30.0, max_duration: Dur::secs(10), max_frames: 1200, settle: Dur::ms(300), change_threshold: 0.02 }
    }
}

/// A named way to start an app, for `launch_profile("name")`, the CLI and the MCP server.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct ProfileSpec {
    pub exe: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: Option<PathBuf>,
    /// Give the app a fresh temporary directory through this flag (`--data`), removed after.
    pub temp_data_dir: Option<String>,
    /// Ask the app to start its automation bridge.
    pub bridge: Option<bool>,
    pub ready: ReadySpec,
}

/// What "started" means for a profile.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct ReadySpec {
    /// At least this many visible windows.
    pub windows: Option<usize>,
    /// A window matching this selector.
    pub window: Option<String>,
    /// The bridge answered `hello`.
    pub bridge: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct Config {
    /// Auto-wait limit of one action.
    pub timeout: Dur,
    pub expect_timeout: Dur,
    pub launch_timeout: Dur,
    pub bridge_timeout: Dur,
    pub input_mode: InputMode,
    pub capture: CaptureSource,
    pub trace: TraceMode,
    /// Pause after every action (debugging).
    pub slow_mo: Dur,
    /// Traces, diffs and recordings, relative to the config's directory.
    pub output_dir: PathBuf,
    /// Copy the app's stdout and stderr to the test's stderr.
    pub echo_output: bool,
    pub snapshot: SnapshotConfig,
    pub record: RecordConfig,
    pub profiles: BTreeMap<String, ProfileSpec>,
    /// Sections other tools read (`[tui]`, `[mcp]`).
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
    /// Where relative paths start: the config file's directory.
    #[serde(skip)]
    pub base_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            timeout: Dur::secs(5),
            expect_timeout: Dur::secs(5),
            launch_timeout: Dur::secs(30),
            bridge_timeout: Dur::secs(10),
            input_mode: InputMode::Os,
            capture: CaptureSource::Screen,
            trace: TraceMode::Off,
            slow_mo: Dur::ms(0),
            output_dir: "target/raywright".into(),
            echo_output: false,
            snapshot: SnapshotConfig::default(),
            record: RecordConfig::default(),
            profiles: BTreeMap::new(),
            extra: BTreeMap::new(),
            base_dir: std::env::current_dir().unwrap_or_default(),
        }
    }
}

pub const FILE_NAME: &str = "raywright.toml";

impl Config {
    /// Defaults, then the config file, then the environment.
    pub fn load() -> Result<Config> {
        let mut c = match Self::find_file() {
            Some(path) => Self::from_file(&path)?,
            None => {
                let mut c = Config::default();
                if let Some(dir) = std::env::var_os("CARGO_MANIFEST_DIR") {
                    c.base_dir = dir.into();
                }
                c
            }
        };
        c.apply_env(|k| std::env::var(k).ok())?;
        Ok(c)
    }

    pub fn find_file() -> Option<PathBuf> {
        if let Some(p) = std::env::var_os("RAYWRIGHT_CONFIG") {
            return Some(p.into());
        }
        let dirs = [std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from), std::env::current_dir().ok()];
        dirs.into_iter().flatten().map(|d| d.join(FILE_NAME)).find(|p| p.is_file())
    }

    pub fn from_file(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
        let mut c = Self::from_toml(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
        c.base_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Ok(c)
    }

    pub fn from_toml(text: &str) -> std::result::Result<Config, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// Applies `RAYWRIGHT_*` variables read through `var`.
    pub fn apply_env(&mut self, var: impl Fn(&str) -> Option<String>) -> Result<()> {
        fn set<T: FromStr>(slot: &mut T, name: &str, var: &impl Fn(&str) -> Option<String>) -> Result<()>
        where
            T::Err: fmt::Display,
        {
            if let Some(v) = var(name).filter(|v| !v.is_empty()) {
                *slot = v.parse().map_err(|e| Error::Config(format!("{name}: {e}")))?;
            }
            Ok(())
        }
        set(&mut self.timeout, "RAYWRIGHT_TIMEOUT", &var)?;
        set(&mut self.expect_timeout, "RAYWRIGHT_EXPECT_TIMEOUT", &var)?;
        set(&mut self.launch_timeout, "RAYWRIGHT_LAUNCH_TIMEOUT", &var)?;
        set(&mut self.input_mode, "RAYWRIGHT_INPUT_MODE", &var)?;
        set(&mut self.capture, "RAYWRIGHT_CAPTURE", &var)?;
        set(&mut self.trace, "RAYWRIGHT_TRACE", &var)?;
        set(&mut self.slow_mo, "RAYWRIGHT_SLOW_MO", &var)?;
        set(&mut self.snapshot.update, "RAYWRIGHT_UPDATE_SNAPSHOTS", &var)?;
        set(&mut self.echo_output, "RAYWRIGHT_ECHO_OUTPUT", &var)?;
        if let Some(dir) = var("RAYWRIGHT_OUTPUT_DIR").filter(|v| !v.is_empty()) {
            self.output_dir = dir.into();
        }
        Ok(())
    }

    /// A path from the config, made absolute against [`Config::base_dir`].
    pub fn resolve(&self, p: &Path) -> PathBuf {
        if p.is_absolute() { p.to_path_buf() } else { self.base_dir.join(p) }
    }

    pub fn output_dir(&self) -> PathBuf {
        self.resolve(&self.output_dir)
    }

    pub fn snapshot_dir(&self) -> PathBuf {
        self.resolve(&self.snapshot.dir)
    }

    /// Running in CI: snapshots are never written implicitly there.
    pub fn in_ci() -> bool {
        std::env::var_os("CI").is_some_and(|v| !v.is_empty() && v != "false" && v != "0")
    }

    /// A section another tool owns, parsed as `T`.
    pub fn section<T: for<'de> Deserialize<'de> + Default>(&self, name: &str) -> Result<T> {
        match self.extra.get(name) {
            Some(v) => v.clone().try_into().map_err(|e| Error::Config(format!("[{name}]: {e}"))),
            None => Ok(T::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_parse() {
        assert_eq!("500ms".parse::<Dur>().unwrap(), Dur::ms(500));
        assert_eq!("5s".parse::<Dur>().unwrap(), Dur::secs(5));
        assert_eq!("1.5s".parse::<Dur>().unwrap(), Dur::ms(1500));
        assert_eq!("2m".parse::<Dur>().unwrap(), Dur::secs(120));
        assert_eq!("250".parse::<Dur>().unwrap(), Dur::ms(250));
        assert!("5 years".parse::<Dur>().is_err());
        assert_eq!(Dur::ms(1500).to_string(), "1500ms");
        assert_eq!(Dur::secs(3).to_string(), "3s");
    }

    #[test]
    fn file_layer_overrides_defaults_and_keeps_the_rest() {
        let c = Config::from_toml(
            r#"
            timeout = "2s"
            input_mode = "message"
            [snapshot]
            threshold = 0.2
            [profiles.wayfinder]
            exe = "target/debug/wayfinder.exe"
            args = ["--automation"]
            temp_data_dir = "--data"
            ready = { windows = 4, bridge = true }
            [tui]
            theme = "dark"
            "#,
        )
        .unwrap();
        assert_eq!(c.timeout, Dur::secs(2));
        assert_eq!(c.expect_timeout, Dur::secs(5));
        assert_eq!(c.input_mode, InputMode::Message);
        assert_eq!(c.snapshot.threshold, 0.2);
        assert_eq!(c.snapshot.dir, PathBuf::from("tests/__screenshots__"));
        let p = &c.profiles["wayfinder"];
        assert_eq!(p.ready.windows, Some(4));
        assert!(p.ready.bridge);
        assert!(c.extra.contains_key("tui"));
    }

    #[test]
    fn env_layer_overrides_the_file() {
        let mut c = Config::from_toml("timeout = 100").unwrap();
        c.apply_env(|k| match k {
            "RAYWRIGHT_TIMEOUT" => Some("3s".into()),
            "RAYWRIGHT_TRACE" => Some("retain-on-failure".into()),
            "RAYWRIGHT_UPDATE_SNAPSHOTS" => Some("1".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(c.timeout, Dur::secs(3));
        assert_eq!(c.trace, TraceMode::RetainOnFailure);
        assert_eq!(c.snapshot.update, UpdateSnapshots::All);
        let err = c.apply_env(|k| (k == "RAYWRIGHT_INPUT_MODE").then(|| "telepathy".into())).unwrap_err();
        assert!(err.to_string().contains("RAYWRIGHT_INPUT_MODE"));
    }

    #[test]
    fn sections_for_other_tools() {
        #[derive(Deserialize, Default)]
        struct Tui {
            theme: String,
        }
        let c = Config::from_toml("[tui]\ntheme = \"dark\"").unwrap();
        assert_eq!(c.section::<Tui>("tui").unwrap().theme, "dark");
        assert_eq!(c.section::<Tui>("mcp").unwrap().theme, "");
    }
}
