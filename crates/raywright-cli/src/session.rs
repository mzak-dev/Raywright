//! Opening the app and window a command works on.

use raywright::WindowInfo;
use raywright::codegen::Launch;
use raywright::prelude::*;

use crate::{Res, Target};

pub struct Session {
    pub rw: Raywright,
    pub app: App,
    /// The window asked for, else the first.
    pub window: Window,
    /// How the app was opened, for generated code.
    pub launch: Launch,
    /// Raywright started it (and closes it at the end).
    pub owned: bool,
}

pub fn raywright(driver: &str) -> Res<Raywright> {
    let b = match driver {
        "mock" => Raywright::builder().driver(raywright::mock::MockDriver::new().into_arc()),
        "platform" => Raywright::platform(),
        other => return Err(format!("unknown driver `{other}` (platform or mock)").into()),
    };
    Ok(b.build()?)
}

impl Session {
    pub async fn open(driver: &str, t: &Target) -> Res<Session> {
        let rw = raywright(driver)?;
        let (app, launch, owned) = if let Some(p) = &t.profile {
            (rw.launch_named(p).await?, Launch::Profile(p.clone()), true)
        } else if let Some(exe) = &t.exe {
            let app = rw.launch(LaunchSpec::exe(exe).args(t.args.clone())).await?;
            (app, Launch::Exe { program: exe.display().to_string(), args: t.args.clone() }, true)
        } else if let Some(pid) = t.pid {
            (rw.attach(AttachTarget::Pid(pid)).await?, Launch::Attach { window: format!("pid={pid}") }, false)
        } else if let Some(w) = &t.window {
            (rw.attach(AttachTarget::Window(w.clone())).await?, Launch::Attach { window: w.clone() }, false)
        } else if driver == "mock" {
            let app = rw.launch(LaunchSpec::exe("demo-app").wait_for_bridge()).await?;
            (app, Launch::Exe { program: "demo-app".into(), args: Vec::new() }, true)
        } else {
            return Err("say which app: --window <selector>, --pid, --profile or --exe (or --driver mock for a demo)".into());
        };
        let window = match &t.window {
            Some(sel) => app.window(sel).await?,
            None => app.first_window().await?,
        };
        Ok(Session { rw, app, window, launch, owned })
    }

    pub async fn finish(&self) {
        if self.owned {
            let _ = self.app.close().await;
        }
    }
}

/// `title="..."`: how generated code finds a window again.
pub fn window_label(w: &WindowInfo) -> String {
    if w.title.is_empty() { format!("id={}", w.id) } else { format!("title=\"{}\"", w.title.replace('"', "\\\"")) }
}
