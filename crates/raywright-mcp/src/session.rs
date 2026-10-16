//! What one agent session holds: apps, the current window, refs from the last snapshot,
//! the steps done (for `generate_test`) and recordings.

use std::collections::HashMap;
use std::io::Write;

use raywright::codegen::{Launch, Step};
use raywright::prelude::*;
use raywright::{Recording, RecordingHandle};

use crate::policy::Policy;
use crate::snapshot::{self, Target};

pub struct Session {
    pub rw: Raywright,
    pub policy: Policy,
    pub apps: Vec<App>,
    pub current: Option<(usize, Window)>,
    pub refs: HashMap<String, Target>,
    pub steps: Vec<Step>,
    pub launch: Option<Launch>,
    pub recording: Option<(String, RecordingHandle)>,
    pub recordings: HashMap<String, Recording>,
    next_recording: usize,
}

impl Session {
    pub fn new(rw: Raywright, policy: Policy) -> Self {
        Session { rw, policy, apps: Vec::new(), current: None, refs: HashMap::new(), steps: Vec::new(), launch: None, recording: None, recordings: HashMap::new(), next_recording: 1 }
    }

    pub fn log(&self, line: &str) {
        let _ = std::fs::create_dir_all(&self.policy.output_dir);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.policy.output_dir.join("session.log")) {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let _ = writeln!(f, "{t} {line}");
        }
    }

    pub fn app(&self) -> Result<&App, String> {
        self.current.as_ref().map(|(i, _)| &self.apps[*i]).ok_or_else(|| "no app yet: call app_launch (or app_attach) first".to_string())
    }

    pub fn window(&self) -> Result<&Window, String> {
        self.current.as_ref().map(|(_, w)| w).ok_or_else(|| "no window yet: call app_launch first".to_string())
    }

    /// The window and selector an action names: `ref` from the last snapshot, or `selector`
    /// in the current window.
    pub async fn target(&self, args: &serde_json::Value) -> Result<(Window, String, String), String> {
        if let Some(r) = args.get("ref").and_then(|v| v.as_str()) {
            let t = self.refs.get(r).ok_or_else(|| format!("no ref {r} in the last snapshot; take a new snapshot"))?;
            let w = self.app()?.wait_for_window(&t.window, secs(5)).await.map_err(|e| e.to_string())?;
            return Ok((w, t.window.clone(), t.selector.clone()));
        }
        if let Some(sel) = args.get("selector").and_then(|v| v.as_str()) {
            let w = self.window()?.clone();
            let label = snapshot::window_label(&w.info().await.map_err(|e| e.to_string())?);
            return Ok((w, label, sel.to_string()));
        }
        Err("give `ref` (from snapshot) or `selector`".into())
    }

    pub async fn snapshot(&mut self) -> Result<String, String> {
        let w = self.window()?.clone();
        snapshot::of(&w, &mut self.refs).await.map_err(|e| e.to_string())
    }

    pub fn record(&mut self, action: &str, window: Option<String>, selector: Option<String>, args: serde_json::Value) {
        self.log(&format!("{action} {} {}", window.as_deref().unwrap_or(""), selector.as_deref().unwrap_or("")));
        self.steps.push(Step { action: action.into(), window, selector, args });
    }

    pub fn recording_id(&mut self) -> String {
        let id = format!("r{}", self.next_recording);
        self.next_recording += 1;
        id
    }
}
