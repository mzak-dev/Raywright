//! Everything that can be added to Raywright without changing it.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::bridge::{TcpTransport, Transport};
use crate::element::ElementSource;
use crate::hooks::ActionHook;
use crate::record::{Exporter, builtin_exporters};
use crate::report::Reporter;
use crate::selector::{SelectorEngine, builtin_engines};
use crate::snapshot::{ImageComparator, builtin_comparators};

/// The extension points of one [`crate::Raywright`]:
///
/// | Kind | Trait | Built in |
/// |---|---|---|
/// | selector engines | [`SelectorEngine`] | `text`, `role`, `css`, `uia`, `bridge`, `key`, `action`, `id`, `name`, `nth`, `visible` |
/// | element sources | [`ElementSource`] | the driver's (UIA) and the bridge |
/// | action hooks | [`ActionHook`] | slow-mo, tracing |
/// | image comparators | [`ImageComparator`] | `pixelmatch`, `exact` |
/// | recording exporters | [`Exporter`] | `frames`, `manifest`, `contact-sheet`, `gif` |
/// | reporters | [`Reporter`] | console, GitHub annotations |
/// | bridge transports | [`Transport`] | `tcp`, plus the driver's |
pub struct Registry {
    engines: RwLock<BTreeMap<String, Arc<dyn SelectorEngine>>>,
    sources: RwLock<Vec<Arc<dyn ElementSource>>>,
    hooks: RwLock<Vec<Arc<dyn ActionHook>>>,
    comparators: RwLock<BTreeMap<String, Arc<dyn ImageComparator>>>,
    exporters: RwLock<Vec<Arc<dyn Exporter>>>,
    reporters: RwLock<Vec<Arc<dyn Reporter>>>,
    transports: RwLock<Vec<Arc<dyn Transport>>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::with_builtins()
    }
}

impl Registry {
    pub fn empty() -> Self {
        Registry {
            engines: RwLock::default(),
            sources: RwLock::default(),
            hooks: RwLock::default(),
            comparators: RwLock::default(),
            exporters: RwLock::default(),
            reporters: RwLock::default(),
            transports: RwLock::default(),
        }
    }

    pub fn with_builtins() -> Self {
        let r = Self::empty();
        for e in builtin_engines() {
            r.register_engine(e);
        }
        for (name, c) in builtin_comparators() {
            r.register_comparator(name, c);
        }
        for e in builtin_exporters() {
            r.register_exporter(e);
        }
        r.register_reporter(Arc::new(crate::report::ConsoleReporter));
        r.register_reporter(Arc::new(crate::report::GithubReporter));
        r.register_transport(Arc::new(TcpTransport));
        r
    }

    /// Adds an engine, replacing one of the same name.
    pub fn register_engine(&self, e: Arc<dyn SelectorEngine>) {
        self.engines.write().unwrap().insert(e.name().to_string(), e);
    }

    pub fn engine(&self, name: &str) -> Option<Arc<dyn SelectorEngine>> {
        self.engines.read().unwrap().get(name).cloned()
    }

    pub fn engine_names(&self) -> Vec<String> {
        self.engines.read().unwrap().keys().cloned().collect()
    }

    /// An element source every app gets, besides the driver's.
    pub fn register_source(&self, s: Arc<dyn ElementSource>) {
        self.sources.write().unwrap().push(s);
    }

    pub fn sources(&self) -> Vec<Arc<dyn ElementSource>> {
        self.sources.read().unwrap().clone()
    }

    pub fn register_hook(&self, h: Arc<dyn ActionHook>) {
        let mut hooks = self.hooks.write().unwrap();
        hooks.push(h);
        hooks.sort_by_key(|h| h.priority());
    }

    pub fn hooks(&self) -> Vec<Arc<dyn ActionHook>> {
        self.hooks.read().unwrap().clone()
    }

    pub fn register_comparator(&self, name: impl Into<String>, c: Arc<dyn ImageComparator>) {
        self.comparators.write().unwrap().insert(name.into(), c);
    }

    pub fn comparator(&self, name: &str) -> Option<Arc<dyn ImageComparator>> {
        self.comparators.read().unwrap().get(name).cloned()
    }

    pub fn register_exporter(&self, e: Arc<dyn Exporter>) {
        let mut all = self.exporters.write().unwrap();
        all.retain(|x| x.name() != e.name());
        all.push(e);
    }

    pub fn exporters(&self) -> Vec<Arc<dyn Exporter>> {
        self.exporters.read().unwrap().clone()
    }

    pub fn register_reporter(&self, r: Arc<dyn Reporter>) {
        self.reporters.write().unwrap().push(r);
    }

    pub fn reporters(&self) -> Vec<Arc<dyn Reporter>> {
        self.reporters.read().unwrap().clone()
    }

    pub fn register_transport(&self, t: Arc<dyn Transport>) {
        self.transports.write().unwrap().insert(0, t);
    }

    /// The transport for `scheme`; later registrations win.
    pub fn transport(&self, scheme: &str) -> Option<Arc<dyn Transport>> {
        self.transports.read().unwrap().iter().find(|t| t.schemes().contains(&scheme)).cloned()
    }
}
