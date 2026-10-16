//! The non-interactive commands.

use std::path::Path;
use std::str::FromStr;

use raywright::codegen::suggest_selector;
use raywright::config::Dur;
use raywright::prelude::*;

use crate::session::Session;
use crate::{Res, Target};

fn dur(s: &str) -> Res<std::time::Duration> {
    Ok(Dur::from_str(s)?.0)
}

/// Every element with its depth, source and a selector, in tree order.
pub fn rows(sources: &[(String, Vec<raywright::element::ElementNode>)]) -> Vec<(String, usize, raywright::element::ElementNode, String)> {
    let all: Vec<raywright::element::ElementNode> = sources.iter().flat_map(|(_, n)| n.iter().cloned()).collect();
    let mut out = Vec::new();
    let mut offset = 0;
    for (source, nodes) in sources {
        for (i, n) in nodes.iter().enumerate() {
            let mut depth = 0;
            let mut p = n.parent;
            while let Some(pi) = p {
                depth += 1;
                p = nodes[pi].parent;
            }
            out.push((source.clone(), depth, n.clone(), suggest_selector(&all, offset + i)));
        }
        offset += nodes.len();
    }
    out
}

pub async fn inspect(driver: &str, t: &Target, json: bool) -> Res<()> {
    let s = Session::open(driver, t).await?;
    let info = s.window.info().await?;
    let sources = s.window.elements().await?;
    if json {
        let v = serde_json::json!({ "window": info, "sources": sources });
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        println!("window {} {:?} pid {} ({}x{} at {},{})", info.id, info.title, info.pid, info.rect.width, info.rect.height, info.rect.x, info.rect.y);
        for (source, depth, n, sel) in rows(&sources) {
            let st = &n.state;
            let label = st.label().map(|l| format!(" {l:?}")).unwrap_or_default();
            println!("{}[{source}] {}{label}    {sel}", "  ".repeat(depth + 1), st.role);
        }
    }
    s.finish().await;
    Ok(())
}

pub async fn screenshot(driver: &str, t: &Target, selector: Option<&str>, source: Option<&str>, out: &Path) -> Res<()> {
    let s = Session::open(driver, t).await?;
    let mut shot = match selector {
        Some(sel) => s.window.locator(sel).screenshot(),
        None => s.window.screenshot(),
    };
    if let Some(src) = source {
        shot = shot.source(CaptureSource::from_str(src)?);
    }
    let img = shot.path(out).await?;
    println!("{} ({}x{})", out.display(), img.width(), img.height());
    s.finish().await;
    Ok(())
}

pub async fn record(driver: &str, t: &Target, fps: f64, duration: Option<&str>, settled: Option<&str>, manual: Option<&str>, out: &Path) -> Res<()> {
    let s = Session::open(driver, t).await?;
    let mut opts = RecordOptions::from_config(&s.rw.config().record).fps(fps);
    if let Some(d) = settled {
        opts = opts.until_settled(dur(d)?);
    }
    if let Some(d) = duration {
        opts = opts.max_duration(dur(d)?);
    }
    if let Some(step) = manual {
        opts = opts.manual_clock(dur(step)?);
    }
    let rec = s.window.record(opts).await?.finished().await?;
    let files = rec.save_dir(out)?;
    let a = rec.analysis();
    println!("{} frames over {:.0} ms ({:.1} fps), last change at {:?} ms, settled at {:?} ms", a.frames, a.duration_ms, a.effective_fps, a.last_change_ms, a.settled_ms);
    println!("{} files in {}", files.len(), out.display());
    s.finish().await;
    Ok(())
}

pub fn show_trace(path: &Path, html: Option<&Path>) -> Res<()> {
    let (trace, files) = raywright::trace::read_zip(path)?;
    println!("{} · {} · {} actions", trace.test, trace.app, trace.events.len());
    for (i, e) in trace.events.iter().enumerate() {
        let mark = if e.error.is_some() { "✗" } else { "✓" };
        println!("{mark} {:>3}. {:>7.0} ms {:>6.0} ms  {} {}", i + 1, e.start_ms, e.duration_ms, e.action, e.selector.as_deref().unwrap_or(""));
        if let Some(err) = &e.error {
            for l in err.lines() {
                println!("              {l}");
            }
        }
    }
    if let Some(h) = html {
        std::fs::write(h, raywright::trace::render_html(&trace, &files))?;
        println!("report: {}", h.display());
    }
    Ok(())
}

/// Runs `raywright-mcp` (next to this program, else on PATH).
pub fn mcp(driver: &str, args: &[String]) -> Res<()> {
    let name = if cfg!(windows) { "raywright-mcp.exe" } else { "raywright-mcp" };
    let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(name))).filter(|p| p.exists());
    let program = beside.unwrap_or_else(|| name.into());
    let mut cmd = std::process::Command::new(&program);
    if driver != "platform" {
        cmd.arg("--driver").arg(driver);
    }
    let status = cmd.args(args).status().map_err(|e| format!("{}: {e} (cargo install raywright-mcp)", program.display()))?;
    if status.success() { Ok(()) } else { Err(format!("raywright-mcp exited with {status}").into()) }
}
