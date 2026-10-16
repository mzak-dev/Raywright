use std::sync::Arc;
use std::time::Duration;

use crate::api::App;
use crate::bridge::Transport;
use crate::config::{CaptureSource, Config, InputMode, TraceMode};
use crate::driver::{Driver, ProcessId};
use crate::element::ElementSource;
use crate::error::{Error, Result};
use crate::hooks::{ActionHook, SlowMo};
use crate::launch::LaunchSpec;
use crate::profile::AppProfile;
use crate::record::Exporter;
use crate::registry::Registry;
use crate::report::Reporter;
use crate::selector::{SelectorEngine, WindowMatcher};
use crate::snapshot::ImageComparator;

/// The entry point: a driver, settings and extensions. Cheap to clone.
#[derive(Clone)]
pub struct Raywright {
    pub(crate) inner: Arc<RwInner>,
}

pub(crate) struct RwInner {
    pub driver: Arc<dyn Driver>,
    pub config: Arc<Config>,
    pub registry: Arc<Registry>,
}

/// How to find a running app to attach to.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum AttachTarget {
    Pid(u32),
    /// The process owning a window that matches this selector (`title=Notepad`).
    Window(String),
}

impl Raywright {
    pub fn builder() -> RaywrightBuilder {
        RaywrightBuilder::default()
    }

    /// `driver`, with the config from `raywright.toml` and the environment.
    pub fn with_driver(driver: Arc<dyn Driver>) -> Result<Raywright> {
        Self::builder().driver(driver).build()
    }

    pub fn driver(&self) -> &Arc<dyn Driver> {
        &self.inner.driver
    }

    pub fn config(&self) -> &Arc<Config> {
        &self.inner.config
    }

    pub fn registry(&self) -> &Arc<Registry> {
        &self.inner.registry
    }

    /// Adds a selector engine at run time (`selectors.register` in Playwright).
    pub fn register_engine(&self, e: Arc<dyn SelectorEngine>) {
        self.inner.registry.register_engine(e);
    }

    /// Starts an app and waits until it is ready (see [`LaunchSpec::wait_for_windows`]).
    pub async fn launch(&self, spec: LaunchSpec) -> Result<App> {
        App::launch(self.clone(), spec, Vec::new()).await
    }

    /// Starts an app the way `profile` says, and waits for what it calls ready.
    pub async fn launch_profile(&self, profile: &dyn AppProfile) -> Result<App> {
        let spec = profile.launch_spec(&self.inner.config)?;
        let app = App::launch(self.clone(), spec, profile.hooks()).await?;
        profile.ready(&app).await?;
        Ok(app)
    }

    /// Starts the `[profiles.<name>]` of the config.
    pub async fn launch_named(&self, name: &str) -> Result<App> {
        let p = self.inner.config.profiles.get(name).ok_or_else(|| {
            let known: Vec<&String> = self.inner.config.profiles.keys().collect();
            Error::Config(format!("no profile `{name}` in raywright.toml (have: {known:?})"))
        })?;
        self.launch(LaunchSpec::from_profile(p, &self.inner.config.base_dir)).await
    }

    /// Takes over an app that is already running. It is not closed when the [`App`] drops.
    pub async fn attach(&self, target: AttachTarget) -> Result<App> {
        let pid = match target {
            AttachTarget::Pid(p) => ProcessId(p),
            AttachTarget::Window(sel) => {
                let m = WindowMatcher::parse(&sel)?;
                let all = self.inner.driver.all_windows().await?;
                let hits: Vec<_> = all.iter().filter(|w| m.matches(w)).collect();
                match hits.as_slice() {
                    [] => return Err(Error::NotFound { target: format!("window {sel}") }),
                    [one] => one.pid,
                    many => {
                        let pids: std::collections::BTreeSet<_> = many.iter().map(|w| w.pid).collect();
                        if pids.len() > 1 {
                            let examples = many.iter().map(|w| format!("    {} \"{}\" (pid {})", w.id, w.title, w.pid)).collect::<Vec<_>>().join("\n");
                            return Err(Error::Ambiguous { target: format!("window {sel}"), count: many.len(), examples });
                        }
                        many[0].pid
                    }
                }
            }
        };
        App::attach(self.clone(), pid).await
    }
}

/// Builds a [`Raywright`]: the config file and environment first, then these calls.
#[derive(Default)]
pub struct RaywrightBuilder {
    driver: Option<Arc<dyn Driver>>,
    config: Option<Config>,
    edits: Vec<Box<dyn FnOnce(&mut Config) + Send>>,
    registry: Option<Registry>,
    engines: Vec<Arc<dyn SelectorEngine>>,
    sources: Vec<Arc<dyn ElementSource>>,
    hooks: Vec<Arc<dyn ActionHook>>,
    comparators: Vec<(String, Arc<dyn ImageComparator>)>,
    exporters: Vec<Arc<dyn Exporter>>,
    reporters: Vec<Arc<dyn Reporter>>,
    transports: Vec<Arc<dyn Transport>>,
}

impl RaywrightBuilder {
    pub fn driver(mut self, d: Arc<dyn Driver>) -> Self {
        self.driver = Some(d);
        self
    }

    pub fn has_driver(&self) -> bool {
        self.driver.is_some()
    }

    /// Use this config instead of loading `raywright.toml` and the environment.
    pub fn config(mut self, c: Config) -> Self {
        self.config = Some(c);
        self
    }

    /// Changes the config after it is loaded.
    pub fn configure(mut self, f: impl FnOnce(&mut Config) + Send + 'static) -> Self {
        self.edits.push(Box::new(f));
        self
    }

    pub fn timeout(self, d: Duration) -> Self {
        self.configure(move |c| c.timeout = d.into())
    }

    pub fn expect_timeout(self, d: Duration) -> Self {
        self.configure(move |c| c.expect_timeout = d.into())
    }

    pub fn input_mode(self, m: InputMode) -> Self {
        self.configure(move |c| c.input_mode = m)
    }

    pub fn capture(self, s: CaptureSource) -> Self {
        self.configure(move |c| c.capture = s)
    }

    pub fn trace(self, t: TraceMode) -> Self {
        self.configure(move |c| c.trace = t)
    }

    pub fn slow_mo(self, d: Duration) -> Self {
        self.configure(move |c| c.slow_mo = d.into())
    }

    /// Start from this registry instead of the built-ins.
    pub fn registry(mut self, r: Registry) -> Self {
        self.registry = Some(r);
        self
    }

    pub fn engine(mut self, e: Arc<dyn SelectorEngine>) -> Self {
        self.engines.push(e);
        self
    }

    pub fn source(mut self, s: Arc<dyn ElementSource>) -> Self {
        self.sources.push(s);
        self
    }

    pub fn hook(mut self, h: Arc<dyn ActionHook>) -> Self {
        self.hooks.push(h);
        self
    }

    pub fn comparator(mut self, name: impl Into<String>, c: Arc<dyn ImageComparator>) -> Self {
        self.comparators.push((name.into(), c));
        self
    }

    pub fn exporter(mut self, e: Arc<dyn Exporter>) -> Self {
        self.exporters.push(e);
        self
    }

    pub fn reporter(mut self, r: Arc<dyn Reporter>) -> Self {
        self.reporters.push(r);
        self
    }

    pub fn transport(mut self, t: Arc<dyn Transport>) -> Self {
        self.transports.push(t);
        self
    }

    pub fn build(self) -> Result<Raywright> {
        let driver = self.driver.ok_or_else(|| Error::Config("no driver: use `raywright::Raywright::new()` (the facade picks the platform's) or `.driver(...)`".into()))?;
        let mut config = match self.config {
            Some(c) => c,
            None => Config::load()?,
        };
        for edit in self.edits {
            edit(&mut config);
        }
        let registry = self.registry.unwrap_or_default();
        for t in driver.transports() {
            registry.register_transport(t);
        }
        for e in driver.engines() {
            registry.register_engine(e);
        }
        for e in self.engines {
            registry.register_engine(e);
        }
        for s in self.sources {
            registry.register_source(s);
        }
        if !config.slow_mo.0.is_zero() {
            registry.register_hook(Arc::new(SlowMo(config.slow_mo.0)));
        }
        for h in self.hooks {
            registry.register_hook(h);
        }
        for (n, c) in self.comparators {
            registry.register_comparator(n, c);
        }
        for e in self.exporters {
            registry.register_exporter(e);
        }
        for r in self.reporters {
            registry.register_reporter(r);
        }
        for t in self.transports {
            registry.register_transport(t);
        }
        Ok(Raywright { inner: Arc::new(RwInner { driver, config: Arc::new(config), registry: Arc::new(registry) }) })
    }
}
