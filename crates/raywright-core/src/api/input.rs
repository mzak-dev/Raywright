use std::sync::OnceLock;
use std::time::Duration;

use crate::api::{App, Window};
use crate::config::InputMode;
use crate::driver::{InputAction, InputBatch, MouseButton, WindowInfo};
use crate::error::Result;
use crate::geometry::Point;
use crate::keys::{Chord, Key};

/// OS input moves the one real cursor: only one batch at a time in this process. (The
/// Windows driver also holds a named mutex, for other test processes.)
fn os_input_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub(crate) async fn send(app: &App, mode: InputMode, window: Option<WindowInfo>, actions: Vec<InputAction>) -> Result<()> {
    let driver = app.input(mode)?;
    let _guard = if mode == InputMode::Os { Some(os_input_lock().lock().await) } else { None };
    driver.send(InputBatch { window, actions }).await
}

/// The actions of a chord: modifiers down, key down and up, modifiers up.
pub(crate) fn chord_actions(chord: &Chord) -> Vec<InputAction> {
    let mut a: Vec<InputAction> = chord.modifiers.iter().cloned().map(InputAction::KeyDown).collect();
    a.push(InputAction::KeyDown(chord.key.clone()));
    a.push(InputAction::KeyUp(chord.key.clone()));
    a.extend(chord.modifiers.iter().rev().cloned().map(InputAction::KeyUp));
    a
}

/// The mouse, in screen coordinates (physical px).
#[derive(Clone)]
pub struct Mouse {
    app: App,
    window: Option<Window>,
    mode: InputMode,
}

impl Mouse {
    pub(crate) fn new(app: App, window: Option<Window>) -> Self {
        let mode = app.config().input_mode;
        Mouse { app, window, mode }
    }

    pub fn mode(mut self, mode: InputMode) -> Self {
        self.mode = mode;
        self
    }

    async fn run(&self, actions: Vec<InputAction>) -> Result<()> {
        let info = match &self.window {
            Some(w) => Some(w.info().await?),
            None => None,
        };
        send(&self.app, self.mode, info, actions).await
    }

    pub async fn move_to(&self, p: Point) -> Result<()> {
        self.run(vec![InputAction::MouseMove(p)]).await
    }

    pub async fn down(&self, button: MouseButton) -> Result<()> {
        self.run(vec![InputAction::MouseDown(button)]).await
    }

    pub async fn up(&self, button: MouseButton) -> Result<()> {
        self.run(vec![InputAction::MouseUp(button)]).await
    }

    pub async fn click(&self, p: Point) -> Result<()> {
        self.run(vec![InputAction::MouseMove(p), InputAction::MouseDown(MouseButton::Left), InputAction::MouseUp(MouseButton::Left)]).await
    }

    pub async fn dblclick(&self, p: Point) -> Result<()> {
        let mut a = vec![InputAction::MouseMove(p)];
        for _ in 0..2 {
            a.push(InputAction::MouseDown(MouseButton::Left));
            a.push(InputAction::MouseUp(MouseButton::Left));
        }
        self.run(a).await
    }

    /// Notches of the wheel; positive `dy` scrolls down.
    pub async fn wheel(&self, at: Point, dx: f64, dy: f64) -> Result<()> {
        self.run(vec![InputAction::MouseMove(at), InputAction::Wheel { dx, dy }]).await
    }

    /// Press at `from`, move in `steps`, release at `to`.
    pub async fn drag(&self, from: Point, to: Point, steps: u32) -> Result<()> {
        self.run(drag_actions(from, to, steps)).await
    }
}

pub(crate) fn drag_actions(from: Point, to: Point, steps: u32) -> Vec<InputAction> {
    let steps = steps.max(1);
    let mut a = vec![InputAction::MouseMove(from), InputAction::MouseDown(MouseButton::Left), InputAction::Pause(Duration::from_millis(30))];
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        a.push(InputAction::MouseMove(Point::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t)));
        a.push(InputAction::Pause(Duration::from_millis(16)));
    }
    a.push(InputAction::MouseUp(MouseButton::Left));
    a
}

/// The keyboard: to one window, or to whichever has the focus.
#[derive(Clone)]
pub struct Keyboard {
    app: App,
    window: Option<Window>,
    mode: InputMode,
}

impl Keyboard {
    pub(crate) fn new(app: App, window: Option<Window>) -> Self {
        let mode = app.config().input_mode;
        Keyboard { app, window, mode }
    }

    pub fn mode(mut self, mode: InputMode) -> Self {
        self.mode = mode;
        self
    }

    async fn run(&self, actions: Vec<InputAction>) -> Result<()> {
        let info = match &self.window {
            Some(w) => {
                let info = w.info().await?;
                if self.mode == InputMode::Os {
                    self.app.driver_app().focus(info.id).await?;
                }
                Some(info)
            }
            None => None,
        };
        send(&self.app, self.mode, info, actions).await
    }

    /// `"Enter"`, `"Control+Shift+E"`, `"ControlOrMeta+A"`.
    pub async fn press(&self, keys: &str) -> Result<()> {
        self.run(chord_actions(&Chord::parse(keys)?)).await
    }

    pub async fn down(&self, key: &str) -> Result<()> {
        self.run(vec![InputAction::KeyDown(Key::parse(key)?)]).await
    }

    pub async fn up(&self, key: &str) -> Result<()> {
        self.run(vec![InputAction::KeyUp(Key::parse(key)?)]).await
    }

    /// Types text as it is, whatever the keyboard layout.
    pub async fn type_text(&self, text: &str) -> Result<()> {
        self.run(vec![InputAction::Text(text.to_string())]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::NamedKey;

    #[test]
    fn chords_release_in_reverse() {
        let a = chord_actions(&Chord::parse("Control+Shift+E").unwrap());
        let ctrl = Key::Named(NamedKey::Control);
        let shift = Key::Named(NamedKey::Shift);
        assert_eq!(
            a,
            vec![
                InputAction::KeyDown(ctrl.clone()),
                InputAction::KeyDown(shift.clone()),
                InputAction::KeyDown(Key::Char('E')),
                InputAction::KeyUp(Key::Char('E')),
                InputAction::KeyUp(shift),
                InputAction::KeyUp(ctrl),
            ]
        );
    }

    #[test]
    fn drags_move_in_steps() {
        let a = drag_actions(Point::new(0.0, 0.0), Point::new(100.0, 0.0), 4);
        let moves: Vec<f64> = a.iter().filter_map(|x| if let InputAction::MouseMove(p) = x { Some(p.x) } else { None }).collect();
        assert_eq!(moves, vec![0.0, 25.0, 50.0, 75.0, 100.0]);
        assert_eq!(a.last(), Some(&InputAction::MouseUp(MouseButton::Left)));
    }
}
