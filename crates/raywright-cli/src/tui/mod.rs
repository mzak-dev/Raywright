//! `raywright ui`: an inspector in the terminal, like Playwright's.
//!
//! The loop draws [`State`], turns terminal keys and finished work into [`Action`]s, and
//! runs the [`Effect`]s `update` asks for on tokio, sending their results back as actions.

mod keymap;
mod state;
mod view;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use raywright::codegen::Step;
use raywright::element::ElementState;
use raywright::prelude::*;
use tokio::sync::{Mutex, mpsc};

use crate::commands::rows;
use crate::session::Session;
use crate::{Res, Target};

pub use state::{Action, Effect, Row, State};

pub fn visible(st: &ElementState) -> bool {
    raywright::actionability::is_visible(st)
}

struct Worker {
    session: Session,
    recording: Mutex<Option<raywright::RecordingHandle>>,
    out: PathBuf,
}

impl Worker {
    async fn window(&self, label: &str) -> raywright::Result<Window> {
        self.session.app.wait_for_window(label, Duration::from_secs(2)).await
    }

    async fn run(&self, e: Effect) -> Vec<Action> {
        match self.effect(e).await {
            Ok(a) => a,
            Err(e) => vec![Action::Failed(e.to_string())],
        }
    }

    async fn effect(&self, e: Effect) -> Res<Vec<Action>> {
        let app = &self.session.app;
        Ok(match e {
            Effect::LoadWindows => vec![Action::Windows(app.windows().await?)],
            Effect::LoadTree(id) => {
                let w = app.wait_for_window(&format!("id={id}"), Duration::from_secs(1)).await?;
                let r = rows(&w.elements().await?).into_iter().map(|(source, depth, node, selector)| Row { source, depth, node, selector }).collect();
                vec![Action::Tree(id, r)]
            }
            Effect::LoadPreview(id) => {
                let w = app.wait_for_window(&format!("id={id}"), Duration::from_secs(1)).await?;
                vec![Action::Preview(w.screenshot().timeout(Duration::from_secs(1)).await?)]
            }
            Effect::Count { window, selector } => {
                let w = self.window(&window).await?;
                vec![Action::Matches(selector.clone(), w.locator(&selector).count().await.map_err(|e| e.to_string()))]
            }
            Effect::Click { window, selector } => {
                let w = self.window(&window).await?;
                w.locator(&selector).click().timeout(Duration::from_secs(3)).await?;
                vec![Action::Step(step("click", &window, &selector, serde_json::json!({}))), Action::Done(format!("clicked {selector}"))]
            }
            Effect::Type { window, selector, text } => {
                let w = self.window(&window).await?;
                w.locator(&selector).type_text(&text).timeout(Duration::from_secs(3)).await?;
                vec![Action::Step(step("type", &window, &selector, serde_json::json!({ "text": text }))), Action::Done(format!("typed into {selector}"))]
            }
            Effect::Screenshot { window, selector } => {
                let w = self.window(&window).await?;
                let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
                let path = self.out.join(format!("screenshot-{n}.png"));
                match &selector {
                    Some(sel) => w.locator(sel).screenshot().path(&path).await?,
                    None => w.screenshot().path(&path).await?,
                };
                vec![Action::Done(format!("saved {}", path.display()))]
            }
            Effect::StartRecording { window } => {
                let w = self.window(&window).await?;
                let h = w.record(RecordOptions::from_config(&self.session.rw.config().record)).await?;
                *self.recording.lock().await = Some(h);
                vec![Action::Done(format!("recording {window}"))]
            }
            Effect::StopRecording => {
                let Some(h) = self.recording.lock().await.take() else { return Ok(Vec::new()) };
                let rec = h.stop().await?;
                let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
                let dir = self.out.join(format!("recording-{n}"));
                rec.save_dir(&dir)?;
                let a = rec.analysis();
                vec![Action::Recorded(format!("{} frames, {:.0} ms, last change {:?} ms → {}", a.frames, a.duration_ms, a.last_change_ms, dir.display()))]
            }
            Effect::ReadCursor => match self.session.rw.driver().cursor_position().await? {
                Some(p) => vec![Action::Cursor(p)],
                None => vec![Action::Failed("this driver can't tell where the cursor is".into())],
            },
            Effect::WriteCode(code) => {
                let path = self.out.join("recorded_test.rs");
                std::fs::create_dir_all(&self.out)?;
                std::fs::write(&path, code)?;
                vec![Action::Done(format!("wrote {}", path.display()))]
            }
        })
    }
}

fn step(action: &str, window: &str, selector: &str, args: serde_json::Value) -> Step {
    Step { action: action.into(), window: Some(window.into()), selector: Some(selector.into()), args }
}

pub async fn run(driver: &str, target: Target, trace: Option<PathBuf>, code: bool) -> Res<()> {
    let trace = trace.map(|p| raywright::trace::read_zip(&p)).transpose()?;
    let session = Session::open(driver, &target).await?;
    let config: keymap::TuiConfig = session.rw.config().section("tui")?;
    let keys = keymap::Keymap::new(&config.keys)?;
    let mut st = State::new(session.launch.clone());
    if code {
        st.tab = state::Tab::Code;
    }
    if trace.is_some() {
        st.tab = state::Tab::Trace;
    }
    st.trace = trace;
    let out = session.rw.config().output_dir().join("inspector");
    std::fs::create_dir_all(&out)?;
    let worker = Arc::new(Worker { session, recording: Mutex::new(None), out });

    let (tx, mut rx) = mpsc::unbounded_channel::<Action>();
    let spawn = |e: Effect, tx: mpsc::UnboundedSender<Action>, w: Arc<Worker>| {
        tokio::spawn(async move {
            for a in w.run(e).await {
                let _ = tx.send(a);
            }
        });
    };
    spawn(Effect::LoadWindows, tx.clone(), worker.clone());

    let mut terminal = ratatui::init();
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    let result: Res<()> = async {
        loop {
            terminal.draw(|f| view::draw(f, &st))?;
            let action = tokio::select! {
                ev = events.next() => match ev {
                    Some(Ok(Event::Key(k))) if k.kind == KeyEventKind::Press => {
                        let typing = matches!(st.mode, state::Mode::Selector | state::Mode::Type | state::Mode::Search);
                        match keys.action(k, typing) { Some(k) => Action::Key(k), None => continue }
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return Err(e.into()),
                    None => break,
                },
                Some(a) = rx.recv() => a,
                _ = tick.tick() => Action::Tick,
            };
            for e in st.update(action) {
                spawn(e, tx.clone(), worker.clone());
            }
            if st.quit {
                break;
            }
        }
        Ok(())
    }
    .await;
    ratatui::restore();
    if let Some(h) = worker.recording.lock().await.take() {
        let _ = h.stop().await;
    }
    worker.session.finish().await;
    result
}
