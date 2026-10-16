use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use raywright_protocol::{Button, ElementDesc, InputEvent, WindowDesc, capability};
use tokio::sync::watch;

use crate::bridge::BridgeState;
use crate::driver::WindowInfo;
use crate::element::{ElementNode, ElementRef, ElementSource, ElementState, Handled, NativeAction, Role};
use crate::error::Result;
use crate::geometry::Rect;

/// The elements an app reports through its bridge, as source `"bridge"`.
pub struct BridgeSource {
    state: watch::Receiver<BridgeState>,
    name: Arc<str>,
}

impl BridgeSource {
    pub fn new(state: watch::Receiver<BridgeState>) -> Self {
        BridgeSource { state, name: Arc::from("bridge") }
    }

    /// Where the window's client area is and its scale: the app's word first, the OS's
    /// when the app left them out.
    pub fn placement(desc: Option<&WindowDesc>, window: &WindowInfo) -> (Rect, f64) {
        match desc {
            Some(d) if d.client.w > 0.0 && d.client.h > 0.0 => (Rect::from(d.client), if d.scale > 0.0 { d.scale } else { window.scale }),
            Some(d) if d.scale > 0.0 => (window.client, d.scale),
            _ => (window.client, window.scale),
        }
    }

    /// The bridge's elements as nodes on screen.
    pub fn to_nodes(name: &Arc<str>, window: &WindowInfo, client: Rect, scale: f64, elements: Vec<ElementDesc>) -> Vec<ElementNode> {
        let index: HashMap<String, usize> = elements.iter().enumerate().map(|(i, e)| (e.key.clone(), i)).collect();
        let on_screen = |r: raywright_protocol::Rect| Rect::from(r).scale(scale).translate(client.x, client.y);
        elements
            .iter()
            .map(|e| {
                let full = on_screen(e.rect);
                let rect = match e.clip {
                    Some(c) => full.intersect(&on_screen(c)),
                    None => Some(full),
                };
                let role = match &e.role {
                    Some(r) => Role::parse(r),
                    None if e.action.is_some() => Role::Button,
                    None if e.text.is_some() => Role::Text,
                    None => Role::Group,
                };
                let mut state = ElementState::new(role);
                state.name = e.text.clone();
                state.text = e.text.clone();
                state.rect = rect;
                state.visible = e.visible && rect.is_some_and(|r| !r.is_empty());
                state.enabled = e.enabled;
                state.key = Some(e.key.clone());
                state.action = e.action.clone();
                state.attrs = e.attrs.clone().into_iter().collect();
                ElementNode { id: ElementRef { source: name.clone(), window: window.id, token: e.key.clone() }, parent: e.parent.as_ref().and_then(|p| index.get(p).copied()), state }
            })
            .collect()
    }
}

#[async_trait]
impl ElementSource for BridgeSource {
    fn name(&self) -> &str {
        &self.name
    }

    async fn snapshot(&self, window: &WindowInfo) -> Result<Vec<ElementNode>> {
        let Some(client) = self.state.borrow().client().cloned() else { return Ok(Vec::new()) };
        if !client.has(capability::ELEMENTS) {
            return Ok(Vec::new());
        }
        let windows = client.windows().await?;
        let Some(desc) = windows.iter().find(|d| d.id == window.id.0) else { return Ok(Vec::new()) };
        let (origin, scale) = Self::placement(Some(desc), window);
        let elements = client.elements(window.id.0).await?;
        Ok(Self::to_nodes(&self.name, window, origin, scale, elements))
    }

    async fn perform(&self, node: &ElementNode, action: &NativeAction) -> Result<Handled> {
        let Some(client) = self.state.borrow().client().cloned() else { return Ok(Handled::No) };
        if !client.has(capability::INPUT) || !matches!(action, NativeAction::Click) {
            return Ok(Handled::No);
        }
        let windows = client.windows().await?;
        let Some(desc) = windows.iter().find(|d| d.id == node.id.window.0) else { return Ok(Handled::No) };
        let Some(rect) = node.state.rect else { return Ok(Handled::No) };
        let scale = if desc.scale > 0.0 { desc.scale } else { 1.0 };
        let c = rect.center();
        let (x, y) = ((c.x - desc.client.x) / scale, (c.y - desc.client.y) / scale);
        let w = desc.id;
        client.input(w, InputEvent::MouseMove { x, y }).await?;
        client.input(w, InputEvent::MouseDown { x, y, button: Button::Left }).await?;
        client.input(w, InputEvent::MouseUp { x, y, button: Button::Left }).await?;
        Ok(Handled::Yes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::{ProcessId, WindowId};
    use raywright_protocol::Rect as PRect;

    #[test]
    fn elements_land_on_screen_with_scale_clip_and_parents() {
        let w = WindowInfo::new(WindowId(1), ProcessId(1));
        let els = vec![
            ElementDesc::new("card", PRect::new(0.0, 0.0, 100.0, 50.0)),
            ElementDesc::new("play", PRect::new(10.0, 10.0, 20.0, 20.0)).action("media.play").parent("card"),
            ElementDesc::new("hidden", PRect::new(90.0, 0.0, 20.0, 10.0)).text("x").clip(PRect::new(0.0, 0.0, 100.0, 50.0)),
        ];
        let name: Arc<str> = Arc::from("bridge");
        let nodes = BridgeSource::to_nodes(&name, &w, Rect::new(500.0, 300.0, 200.0, 100.0), 2.0, els);
        assert_eq!(nodes[1].state.rect, Some(Rect::new(520.0, 320.0, 40.0, 40.0)));
        assert_eq!(nodes[1].state.role, Role::Button);
        assert_eq!(nodes[1].parent, Some(0));
        assert_eq!(nodes[2].state.rect, Some(Rect::new(680.0, 300.0, 20.0, 20.0)));
        assert_eq!(nodes[2].state.role, Role::Text);
        assert_eq!(nodes[0].state.role, Role::Group);
    }
}
