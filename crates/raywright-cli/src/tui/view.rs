//! Drawing the state. Panels: windows, elements, details, preview, and a tabbed bottom.

use image::RgbaImage;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Tabs, Widget, Wrap};

use super::keymap::BINDINGS;
use super::state::{Focus, Mode, State, Tab};

const ACCENT: Color = Color::Cyan;

fn block(title: &str, focused: bool) -> Block<'_> {
    let style = if focused { Style::default().fg(ACCENT) } else { Style::default().fg(Color::DarkGray) };
    Block::default().borders(Borders::ALL).border_style(style).title(Span::styled(format!(" {title} "), Style::default().add_modifier(Modifier::BOLD)))
}

fn selected() -> Style {
    Style::default().bg(Color::Rgb(40, 60, 80)).add_modifier(Modifier::BOLD)
}

/// A picture in half blocks: each cell shows two pixels, top as color, bottom as background.
pub struct HalfBlocks<'a>(pub &'a RgbaImage);

impl Widget for HalfBlocks<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (iw, ih) = self.0.dimensions();
        if iw == 0 || ih == 0 || area.width == 0 || area.height == 0 {
            return;
        }
        let s = (area.width as f64 / iw as f64).min((area.height as f64 * 2.0) / ih as f64);
        let (w, h) = (((iw as f64 * s) as u16).max(1), (((ih as f64 * s) / 2.0) as u16).max(1));
        let px = |x: u16, y: u16| {
            let p = self.0.get_pixel(((x as f64 / s) as u32).min(iw - 1), ((y as f64 / s) as u32).min(ih - 1));
            Color::Rgb(p[0], p[1], p[2])
        };
        for cy in 0..h.min(area.height) {
            for cx in 0..w.min(area.width) {
                if let Some(cell) = buf.cell_mut((area.x + cx, area.y + cy)) {
                    cell.set_symbol("▀").set_fg(px(cx, cy * 2)).set_bg(px(cx, cy * 2 + 1));
                }
            }
        }
    }
}

pub fn draw(f: &mut Frame, s: &State) {
    let [main, bottom, status] = Layout::vertical([Constraint::Min(10), Constraint::Length(10), Constraint::Length(1)]).areas(f.area());
    let [left, middle, right] = Layout::horizontal([Constraint::Percentage(24), Constraint::Percentage(46), Constraint::Percentage(30)]).areas(main);
    let [windows, preview] = Layout::vertical([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(left);

    draw_windows(f, s, windows);
    draw_preview(f, s, preview);
    draw_tree(f, s, middle);
    draw_details(f, s, right);
    draw_bottom(f, s, bottom);
    draw_status(f, s, status);
    if s.mode == Mode::Help {
        draw_help(f);
    }
}

fn draw_windows(f: &mut Frame, s: &State, area: Rect) {
    let items: Vec<ListItem> = s
        .windows
        .iter()
        .map(|w| {
            let title = if w.title.is_empty() { format!("({})", w.class) } else { w.title.clone() };
            let dim = if w.is_shown() { Style::default() } else { Style::default().fg(Color::DarkGray) };
            ListItem::new(Line::from(vec![Span::styled(title, dim), Span::styled(format!("  {}", w.kind.as_deref().unwrap_or("")), Style::default().fg(Color::DarkGray))]))
        })
        .collect();
    let mut st = ListState::default().with_selected((!s.windows.is_empty()).then_some(s.window_sel));
    f.render_stateful_widget(List::new(items).block(block("Windows", s.focus == Focus::Windows)).highlight_style(selected()), area, &mut st);
}

fn draw_preview(f: &mut Frame, s: &State, area: Rect) {
    let b = block("Preview", false);
    let inner = b.inner(area);
    f.render_widget(b, area);
    match &s.preview {
        Some(img) => f.render_widget(HalfBlocks(img), inner),
        None => f.render_widget(Paragraph::new("no picture yet").style(Style::default().fg(Color::DarkGray)), inner),
    }
}

fn draw_tree(f: &mut Frame, s: &State, area: Rect) {
    let items: Vec<ListItem> = s
        .rows
        .iter()
        .map(|r| {
            let st = &r.node.state;
            let mut spans = vec![Span::raw("  ".repeat(r.depth)), Span::styled(st.role.to_string(), Style::default().fg(Color::Yellow))];
            if let Some(l) = st.label() {
                spans.push(Span::raw(format!(" {l:?}")));
            }
            spans.push(Span::styled(format!("  [{}]", r.source), Style::default().fg(Color::DarkGray)));
            if !crate::tui::visible(st) {
                spans.push(Span::styled(" hidden", Style::default().fg(Color::DarkGray)));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let mut st = ListState::default().with_selected((!s.rows.is_empty()).then_some(s.row_sel));
    let title = format!("Elements ({})", s.rows.len());
    f.render_stateful_widget(List::new(items).block(block(&title, s.focus == Focus::Tree)).highlight_style(selected()), area, &mut st);
}

fn draw_details(f: &mut Frame, s: &State, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    let kv = |k: &str, v: String| Line::from(vec![Span::styled(format!("{k:<9}"), Style::default().fg(Color::DarkGray)), Span::raw(v)]);
    if let Some(r) = s.row() {
        let st = &r.node.state;
        lines.push(Line::from(Span::styled(r.selector.clone(), Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))));
        lines.push(Line::raw(""));
        lines.push(kv("role", st.role.to_string()));
        for (k, v) in [("name", &st.name), ("text", &st.text), ("value", &st.value), ("key", &st.key), ("action", &st.action)] {
            if let Some(v) = v {
                lines.push(kv(k, v.clone()));
            }
        }
        if let Some(rect) = st.rect {
            lines.push(kv("rect", format!("{:.0},{:.0} {:.0}x{:.0}", rect.x, rect.y, rect.width, rect.height)));
        }
        lines.push(kv(
            "state",
            format!(
                "{}{}{}{}",
                if st.enabled { "enabled" } else { "disabled" },
                if st.editable { " editable" } else { "" },
                if st.focused { " focused" } else { "" },
                st.checked.map(|c| if c { " checked" } else { " unchecked" }).unwrap_or("")
            ),
        ));
        lines.push(kv("source", r.source.clone()));
        for (k, v) in &st.attrs {
            lines.push(kv(k, v.clone()));
        }
    } else {
        lines.push(Line::styled("select an element", Style::default().fg(Color::DarkGray)));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block("Details", false)), area);
}

fn draw_bottom(f: &mut Frame, s: &State, area: Rect) {
    let b = block("", s.focus == Focus::Bottom);
    let inner = b.inner(area);
    f.render_widget(b, area);
    let [tabs, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(inner);
    let idx = Tab::ALL.iter().position(|t| *t == s.tab).unwrap_or(0);
    f.render_widget(Tabs::new(Tab::ALL.iter().map(|t| t.title())).select(idx).highlight_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)), tabs);
    let tail = |v: Vec<String>| -> Vec<Line> {
        let n = body.height as usize;
        v.into_iter().rev().take(n).collect::<Vec<_>>().into_iter().rev().map(Line::raw).collect()
    };
    let text: Vec<Line> = match s.tab {
        Tab::Code => tail(s.code().lines().map(str::to_string).collect()),
        Tab::Log => tail(s.log.clone()),
        Tab::Recordings => {
            let mut v = s.recordings.clone();
            if s.recording {
                v.push("● recording… (r to stop)".into());
            }
            if v.is_empty() {
                v.push("r records the window".into());
            }
            tail(v)
        }
        Tab::Trace => match &s.trace {
            None => vec![Line::raw("open one with: raywright ui --trace trace.zip")],
            Some((t, _)) => t
                .events
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let line = format!("{} {:>6.0} ms  {} {}", if e.error.is_some() { "✗" } else { "✓" }, e.duration_ms, e.action, e.selector.as_deref().unwrap_or(""));
                    if i == s.trace_sel { Line::styled(line, selected()) } else { Line::raw(line) }
                })
                .collect(),
        },
    };
    f.render_widget(Paragraph::new(text), body);
}

fn draw_status(f: &mut Frame, s: &State, area: Rect) {
    let line = match s.mode {
        Mode::Selector => Line::from(vec![Span::styled(":", Style::default().fg(ACCENT)), Span::raw(s.input.clone()), Span::styled("▏", Style::default().fg(ACCENT))]),
        Mode::Type => Line::from(vec![Span::styled("type: ", Style::default().fg(ACCENT)), Span::raw(s.input.clone()), Span::styled("▏", Style::default().fg(ACCENT))]),
        Mode::Search => Line::from(vec![Span::styled("/", Style::default().fg(ACCENT)), Span::raw(s.input.clone()), Span::styled("▏", Style::default().fg(ACCENT))]),
        _ => {
            let mut spans = Vec::new();
            if s.picking {
                spans.push(Span::styled(" PICK ", Style::default().bg(Color::Magenta).fg(Color::Black)));
                spans.push(Span::raw(" "));
            }
            if s.recording {
                spans.push(Span::styled(" REC ", Style::default().bg(Color::Red).fg(Color::White)));
                spans.push(Span::raw(" "));
            }
            spans.push(Span::raw(s.status.clone()));
            Line::from(spans)
        }
    };
    f.render_widget(Paragraph::new(line), area);
}

fn draw_help(f: &mut Frame) {
    let area = f.area();
    let w = 64.min(area.width);
    let h = (BINDINGS.len() as u16 + 4).min(area.height);
    let r = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
    let mut lines: Vec<Line> = BINDINGS.iter().map(|(_, k, what)| Line::from(vec![Span::styled(format!("{k:>8}  "), Style::default().fg(ACCENT)), Span::raw(*what)])).collect();
    lines.push(Line::raw(""));
    lines.push(Line::styled("   ↑ ↓ move · Enter select · Esc cancel · keys: [tui.keys] in raywright.toml", Style::default().fg(Color::DarkGray)));
    f.render_widget(Clear, r);
    f.render_widget(Paragraph::new(lines).block(block("Keys", true)), r);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::state::tests::loaded;
    use crate::tui::state::{Action, Key};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn screen(s: &State) -> String {
        let mut t = Terminal::new(TestBackend::new(140, 36)).unwrap();
        t.draw(|f| draw(f, s)).unwrap();
        let buf = t.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn draws_every_panel() {
        let mut s = loaded();
        s.update(Action::Key(Key::Down));
        s.preview = Some(RgbaImage::from_pixel(8, 4, image::Rgba([200, 10, 10, 255])));
        let out = screen(&s);
        for want in ["Windows", "Demo", "Elements (3)", "button \"OK\"", "key=\"ok\"", "Details", "Preview", "Code", "#[raywright::test]", "? for keys"] {
            assert!(out.contains(want), "missing {want:?} in\n{out}");
        }
        assert!(out.contains('▀'), "the preview draws half blocks");
    }

    #[test]
    fn help_and_input_lines() {
        let mut s = loaded();
        s.update(Action::Key(Key::Help));
        assert!(screen(&s).contains("try a selector"));
        s.update(Action::Key(Key::Cancel));
        s.update(Action::Key(Key::Selector));
        s.update(Action::Key(Key::Char('x')));
        assert!(screen(&s).contains(":x▏"));
    }
}
