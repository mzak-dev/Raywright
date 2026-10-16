//! How to start an app.

use std::path::PathBuf;

use raywright_protocol::BridgeAddr;

use crate::config::{ProfileSpec, ReadySpec};

/// Whether and where a launched app runs its automation bridge.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum BridgeMode {
    /// Tell the app where to listen (`RAYWRIGHT_BRIDGE`) and use the bridge if it answers.
    #[default]
    Auto,
    Off,
    /// The app listens here by itself.
    At(BridgeAddr),
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct LaunchSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    /// Pass a fresh temporary directory with this flag (`--data <dir>`); removed afterwards.
    pub temp_data_dir: Option<String>,
    pub bridge: BridgeMode,
    pub echo_output: bool,
    pub ready: ReadySpec,
}

impl LaunchSpec {
    pub fn exe(program: impl Into<PathBuf>) -> Self {
        LaunchSpec { program: program.into(), args: Vec::new(), env: Vec::new(), cwd: None, temp_data_dir: None, bridge: BridgeMode::Auto, echo_output: false, ready: ReadySpec::default() }
    }

    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.env.push((k.into(), v.into()));
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn temp_data_dir(mut self, flag: impl Into<String>) -> Self {
        self.temp_data_dir = Some(flag.into());
        self
    }

    pub fn bridge(mut self, mode: BridgeMode) -> Self {
        self.bridge = mode;
        self
    }

    pub fn echo_output(mut self, on: bool) -> Self {
        self.echo_output = on;
        self
    }

    /// Wait for at least `n` visible windows before `launch` returns.
    pub fn wait_for_windows(mut self, n: usize) -> Self {
        self.ready.windows = Some(n);
        self
    }

    /// Wait for a window matching `selector` before `launch` returns.
    pub fn wait_for_window(mut self, selector: impl Into<String>) -> Self {
        self.ready.window = Some(selector.into());
        self
    }

    /// Wait for the bridge to answer before `launch` returns.
    pub fn wait_for_bridge(mut self) -> Self {
        self.ready.bridge = true;
        self
    }

    /// A spec from a `[profiles.<name>]` table; relative paths start at `base`.
    pub fn from_profile(p: &ProfileSpec, base: &std::path::Path) -> Self {
        let abs = |path: &PathBuf| if path.is_absolute() { path.clone() } else { base.join(path) };
        LaunchSpec {
            program: abs(&p.exe),
            args: p.args.clone(),
            env: p.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            cwd: p.cwd.as_ref().map(abs),
            temp_data_dir: p.temp_data_dir.clone(),
            bridge: if p.bridge == Some(false) { BridgeMode::Off } else { BridgeMode::Auto },
            echo_output: false,
            ready: p.ready.clone(),
        }
    }
}
