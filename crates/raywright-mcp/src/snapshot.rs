//! A window as text, with a ref per element, like Playwright MCP's snapshots: an agent
//! reads the UI without a picture and acts on refs.

use std::collections::HashMap;

use raywright::codegen::suggest_selector;
use raywright::element::ElementNode;
use raywright::{Result, Window, WindowInfo};

/// What a ref stands for: a window and a selector that finds the element again.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub window: String,
    pub selector: String,
}

pub fn window_label(w: &WindowInfo) -> String {
    if w.title.is_empty() { format!("id={}", w.id) } else { format!("title=\"{}\"", w.title.replace('"', "\\\"")) }
}

/// Renders the snapshot text and fills `refs` (cleared first).
pub fn render(info: &WindowInfo, sources: &[(String, Vec<ElementNode>)], refs: &mut HashMap<String, Target>) -> String {
    refs.clear();
    let label = window_label(info);
    let all: Vec<ElementNode> = sources.iter().flat_map(|(_, n)| n.iter().cloned()).collect();
    let mut out = format!("window {label} {}x{}{}\n", info.rect.width, info.rect.height, info.kind.as_deref().map(|k| format!(" kind={k}")).unwrap_or_default());
    let mut offset = 0;
    let mut n = 0;
    for (source, nodes) in sources {
        for (i, node) in nodes.iter().enumerate() {
            let st = &node.state;
            let mut depth = 0;
            let mut p = node.parent;
            while let Some(pi) = p {
                depth += 1;
                p = nodes[pi].parent;
            }
            n += 1;
            let r = format!("e{n}");
            let selector = suggest_selector(&all, offset + i);
            out.push_str(&"  ".repeat(depth));
            out.push_str(&format!("- {}", st.role));
            if let Some(l) = st.label() {
                out.push_str(&format!(" {l:?}"));
            }
            out.push_str(&format!(" [ref={r}]"));
            if let Some(v) = &st.value
                && st.label() != Some(v.as_str())
            {
                out.push_str(&format!(" value={v:?}"));
            }
            if let Some(c) = st.checked {
                out.push_str(if c { " checked" } else { " unchecked" });
            }
            if !st.enabled {
                out.push_str(" disabled");
            }
            if !raywright::actionability::is_visible(st) {
                out.push_str(" hidden");
            }
            if let Some(a) = &st.action {
                out.push_str(&format!(" action={a}"));
            }
            out.push_str(&format!(" ({source})\n"));
            refs.insert(r, Target { window: label.clone(), selector });
        }
        offset += nodes.len();
    }
    out
}

pub async fn of(window: &Window, refs: &mut HashMap<String, Target>) -> Result<String> {
    let info = window.info().await?;
    let sources = window.elements().await?;
    Ok(render(&info, &sources, refs))
}
