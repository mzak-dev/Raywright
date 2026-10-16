//! How to start a particular app and know it is ready: the extension point for app-specific
//! test support (page objects build on it).

use std::sync::Arc;

use async_trait::async_trait;

use crate::api::App;
use crate::config::{Config, ProfileSpec};
use crate::error::{Error, Result};
use crate::hooks::ActionHook;
use crate::launch::LaunchSpec;

#[async_trait]
pub trait AppProfile: Send + Sync {
    fn name(&self) -> &str;

    fn launch_spec(&self, config: &Config) -> Result<LaunchSpec>;

    /// Waits for what "started" means beyond the spec's own readiness.
    async fn ready(&self, _app: &App) -> Result<()> {
        Ok(())
    }

    /// Hooks only for this app's actions.
    fn hooks(&self) -> Vec<Arc<dyn ActionHook>> {
        Vec::new()
    }
}

/// A `[profiles.<name>]` table as a profile.
pub struct ConfigProfile {
    pub name: String,
    pub spec: ProfileSpec,
}

impl ConfigProfile {
    pub fn from_config(config: &Config, name: &str) -> Result<Self> {
        let spec = config.profiles.get(name).cloned().ok_or_else(|| Error::Config(format!("no profile `{name}`")))?;
        Ok(ConfigProfile { name: name.to_string(), spec })
    }
}

#[async_trait]
impl AppProfile for ConfigProfile {
    fn name(&self) -> &str {
        &self.name
    }

    fn launch_spec(&self, config: &Config) -> Result<LaunchSpec> {
        Ok(LaunchSpec::from_profile(&self.spec, &config.base_dir))
    }
}
