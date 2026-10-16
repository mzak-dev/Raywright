//! The tools. Each is an [`McpTool`]; an app can add its own next to the built-ins.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use image::RgbaImage;
use raywright::codegen::Launch;
use raywright::prelude::*;
use rmcp::model::ContentBlock;
use serde_json::{Value, json};

use crate::session::Session;

pub type ToolResult = Result<Vec<ContentBlock>, String>;

/// One tool. `schema` is a JSON Schema object for the arguments.
#[async_trait]
pub trait McpTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> Value;
    async fn call(&self, s: &mut Session, args: Value) -> ToolResult;
}

fn text(t: impl Into<String>) -> ContentBlock {
    ContentBlock::text(t)
}

fn png(img: &RgbaImage, max: u32) -> Result<ContentBlock, String> {
    let fit = raywright::imaging::fit(img, max);
    let bytes = raywright::imaging::encode_png(&fit).map_err(|e| e.to_string())?;
    Ok(ContentBlock::image(base64::engine::general_purpose::STANDARD.encode(bytes), "image/png"))
}

fn str_arg<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str)
}

fn ms_arg(a: &Value, k: &str) -> Option<Duration> {
    a.get(k).and_then(Value::as_u64).map(Duration::from_millis)
}

fn e(err: impl std::fmt::Display) -> String {
    err.to_string()
}

/// `{ "ref": ..., "selector": ... }` and more properties.
fn target_schema(extra: Value, required: &[&str]) -> Value {
    let mut props = json!({
        "ref": { "type": "string", "description": "An element ref from the last snapshot, like e3" },
        "selector": { "type": "string", "description": "A Raywright selector instead of a ref: role=button[name=\"OK\"], text=Save, key=..., uia=Edit, bridge=action=..." }
    });
    if let (Some(p), Some(x)) = (props.as_object_mut(), extra.as_object()) {
        p.extend(x.clone());
    }
    json!({ "type": "object", "properties": props, "required": required })
}

async fn after_action(s: &mut Session, done: String) -> ToolResult {
    tokio::time::sleep(Duration::from_millis(150)).await;
    let snap = s.snapshot().await.unwrap_or_else(|e| format!("(no snapshot: {e})"));
    Ok(vec![text(format!("{done}\n\n{snap}"))])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    AppLaunch,
    AppAttach,
    AppClose,
    AppList,
    WindowList,
    WindowSelect,
    Snapshot,
    Click,
    Hover,
    Drag,
    Scroll,
    Type,
    Fill,
    PressKey,
    Screenshot,
    Expect,
    WaitFor,
    BridgeCall,
    RecordStart,
    RecordStop,
    RecordAction,
    RecordingFrames,
    GenerateTest,
}

pub const BUILTINS: &[Builtin] = &[
    Builtin::AppLaunch,
    Builtin::AppAttach,
    Builtin::AppClose,
    Builtin::AppList,
    Builtin::WindowList,
    Builtin::WindowSelect,
    Builtin::Snapshot,
    Builtin::Click,
    Builtin::Hover,
    Builtin::Drag,
    Builtin::Scroll,
    Builtin::Type,
    Builtin::Fill,
    Builtin::PressKey,
    Builtin::Screenshot,
    Builtin::Expect,
    Builtin::WaitFor,
    Builtin::BridgeCall,
    Builtin::RecordStart,
    Builtin::RecordStop,
    Builtin::RecordAction,
    Builtin::RecordingFrames,
    Builtin::GenerateTest,
];

pub fn builtins() -> Vec<Arc<dyn McpTool>> {
    BUILTINS.iter().map(|b| Arc::new(*b) as Arc<dyn McpTool>).collect()
}

#[async_trait]
impl McpTool for Builtin {
    fn name(&self) -> &str {
        match self {
            Builtin::AppLaunch => "app_launch",
            Builtin::AppAttach => "app_attach",
            Builtin::AppClose => "app_close",
            Builtin::AppList => "app_list",
            Builtin::WindowList => "window_list",
            Builtin::WindowSelect => "window_select",
            Builtin::Snapshot => "snapshot",
            Builtin::Click => "click",
            Builtin::Hover => "hover",
            Builtin::Drag => "drag",
            Builtin::Scroll => "scroll",
            Builtin::Type => "type",
            Builtin::Fill => "fill",
            Builtin::PressKey => "press_key",
            Builtin::Screenshot => "screenshot",
            Builtin::Expect => "expect",
            Builtin::WaitFor => "wait_for",
            Builtin::BridgeCall => "bridge_call",
            Builtin::RecordStart => "record_start",
            Builtin::RecordStop => "record_stop",
            Builtin::RecordAction => "record_action",
            Builtin::RecordingFrames => "recording_frames",
            Builtin::GenerateTest => "generate_test",
        }
    }

    fn description(&self) -> &str {
        match self {
            Builtin::AppLaunch => "Start a desktop app (a profile from raywright.toml, or an allowed program) and return a snapshot of its first window.",
            Builtin::AppAttach => "Take over an app that is already running, by process id or window selector (needs --allow-attach).",
            Builtin::AppClose => "Close the current app.",
            Builtin::AppList => "List the apps of this session.",
            Builtin::WindowList => "List the current app's windows.",
            Builtin::WindowSelect => "Make the window matching a selector (title=..., kind=...) the current one, and snapshot it.",
            Builtin::Snapshot => "The current window as text: every element with its role, name and a ref (e1, e2...) to act on. Refs last until the next snapshot.",
            Builtin::Click => "Click an element (waits until it is visible, stable and enabled). Returns a fresh snapshot.",
            Builtin::Hover => "Move the mouse over an element.",
            Builtin::Drag => "Drag one element onto another.",
            Builtin::Scroll => "Turn the mouse wheel over an element.",
            Builtin::Type => "Type text into an element, key by key; submit presses Enter after.",
            Builtin::Fill => "Replace the content of an input.",
            Builtin::PressKey => "Press a key or chord (Enter, Control+S, ControlOrMeta+A), on an element or the window.",
            Builtin::Screenshot => "A picture of the window or an element.",
            Builtin::Expect => "Check something about an element, waiting up to the timeout: visible, hidden, enabled, disabled, checked, unchecked, text (exact), contains, value, count.",
            Builtin::WaitFor => "Wait until an element is attached, detached, visible or hidden.",
            Builtin::BridgeCall => "Run one of the app's own commands through its automation bridge.",
            Builtin::RecordStart => "Start recording the window (or an element's area) as fast pictures in a row.",
            Builtin::RecordStop => "Stop recording; returns a contact sheet of the frames, a summary of what moved and when it settled, and the frames' folder.",
            Builtin::RecordAction => {
                "Do one action (click, press_key, type) and record until the window settles: the way to look at an animation. Returns the contact sheet and the frames that changed most."
            }
            Builtin::RecordingFrames => "Single frames of a recording, full size.",
            Builtin::GenerateTest => "The steps of this session as a Rust test (#[raywright::test]).",
        }
    }

    fn schema(&self) -> Value {
        let empty = json!({ "type": "object", "properties": {} });
        match self {
            Builtin::AppLaunch => json!({ "type": "object", "properties": {
                "profile": { "type": "string", "description": "A [profiles.<name>] of raywright.toml" },
                "exe": { "type": "string", "description": "A program allowed with --allow-exe" },
                "args": { "type": "array", "items": { "type": "string" } },
                "temp_data_dir": { "type": "string", "description": "Pass a fresh temporary directory with this flag, e.g. --data" }
            }}),
            Builtin::AppAttach => json!({ "type": "object", "properties": { "pid": { "type": "integer" }, "window": { "type": "string" } } }),
            Builtin::AppClose | Builtin::AppList | Builtin::WindowList | Builtin::Snapshot | Builtin::RecordStop => empty,
            Builtin::WindowSelect => json!({ "type": "object", "properties": { "selector": { "type": "string" }, "timeout_ms": { "type": "integer" } }, "required": ["selector"] }),
            Builtin::Click => target_schema(json!({ "button": { "type": "string", "enum": ["left", "right", "middle"] }, "double": { "type": "boolean" } }), &[]),
            Builtin::Hover => target_schema(json!({}), &[]),
            Builtin::Drag => {
                json!({ "type": "object", "properties": { "from": { "type": "string", "description": "ref or selector" }, "to": { "type": "string", "description": "ref or selector" } }, "required": ["from", "to"] })
            }
            Builtin::Scroll => target_schema(json!({ "dy": { "type": "number", "description": "wheel notches, positive is down" }, "dx": { "type": "number" } }), &["dy"]),
            Builtin::Type => target_schema(json!({ "text": { "type": "string" }, "submit": { "type": "boolean" } }), &["text"]),
            Builtin::Fill => target_schema(json!({ "text": { "type": "string" } }), &["text"]),
            Builtin::PressKey => target_schema(json!({ "keys": { "type": "string" } }), &["keys"]),
            Builtin::Screenshot => target_schema(json!({ "source": { "type": "string", "enum": ["screen", "window", "app"] }, "save_as": { "type": "string" } }), &[]),
            Builtin::Expect => target_schema(
                json!({ "assertion": { "type": "string", "enum": ["visible", "hidden", "enabled", "disabled", "checked", "unchecked", "text", "contains", "value", "count"] }, "value": { "type": ["string", "integer"] }, "timeout_ms": { "type": "integer" } }),
                &["assertion"],
            ),
            Builtin::WaitFor => target_schema(json!({ "state": { "type": "string", "enum": ["attached", "detached", "visible", "hidden"] }, "timeout_ms": { "type": "integer" } }), &["state"]),
            Builtin::BridgeCall => json!({ "type": "object", "properties": { "name": { "type": "string" }, "args": { "type": "object" } }, "required": ["name"] }),
            Builtin::RecordStart => target_schema(
                json!({ "fps": { "type": "number" }, "max_duration_ms": { "type": "integer" }, "manual_clock_ms": { "type": "integer", "description": "step the app's animation clock (bridge apps): the same frames every time" } }),
                &[],
            ),
            Builtin::RecordAction => target_schema(
                json!({
                    "action": { "type": "string", "enum": ["click", "press_key", "type"] },
                    "keys": { "type": "string" }, "text": { "type": "string" },
                    "settle_ms": { "type": "integer", "description": "stop once nothing moved for this long (default 300)" },
                    "keyframes": { "type": "integer", "description": "how many frames to return (default 6)" },
                    "fps": { "type": "number" }, "manual_clock_ms": { "type": "integer" }
                }),
                &["action"],
            ),
            Builtin::RecordingFrames => {
                json!({ "type": "object", "properties": { "id": { "type": "string" }, "frames": { "type": "array", "items": { "type": "integer" } } }, "required": ["id", "frames"] })
            }
            Builtin::GenerateTest => json!({ "type": "object", "properties": { "name": { "type": "string" } } }),
        }
    }

    async fn call(&self, s: &mut Session, a: Value) -> ToolResult {
        match self {
            Builtin::AppLaunch => {
                let (spec, launch) = if let Some(p) = str_arg(&a, "profile") {
                    let prof = s.rw.config().profiles.get(p).ok_or_else(|| format!("no profile `{p}` in raywright.toml"))?;
                    (LaunchSpec::from_profile(prof, &s.rw.config().base_dir), Launch::Profile(p.into()))
                } else if let Some(exe) = str_arg(&a, "exe") {
                    s.policy.may_launch(std::path::Path::new(exe))?;
                    let args: Vec<String> = a.get("args").and_then(Value::as_array).map(|v| v.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                    let mut spec = LaunchSpec::exe(exe).args(args.clone());
                    if let Some(flag) = str_arg(&a, "temp_data_dir") {
                        spec = spec.temp_data_dir(flag);
                    }
                    (spec, Launch::Exe { program: exe.into(), args })
                } else {
                    return Err("give `profile` or `exe`".into());
                };
                let app = s.rw.launch(spec).await.map_err(e)?;
                let w = app.first_window().await.map_err(e)?;
                s.apps.push(app);
                s.current = Some((s.apps.len() - 1, w));
                s.launch.get_or_insert(launch);
                s.log("app_launch");
                after_action(s, "launched".into()).await
            }
            Builtin::AppAttach => {
                s.policy.may_attach()?;
                let target = match (a.get("pid").and_then(Value::as_u64), str_arg(&a, "window")) {
                    (Some(p), _) => AttachTarget::Pid(p as u32),
                    (None, Some(w)) => AttachTarget::Window(w.into()),
                    _ => return Err("give `pid` or `window`".into()),
                };
                let app = s.rw.attach(target.clone()).await.map_err(e)?;
                let w = match &target {
                    AttachTarget::Window(sel) => app.window(sel).await.map_err(e)?,
                    _ => app.first_window().await.map_err(e)?,
                };
                s.apps.push(app);
                s.current = Some((s.apps.len() - 1, w));
                if let AttachTarget::Window(sel) = target {
                    s.launch.get_or_insert(Launch::Attach { window: sel });
                }
                after_action(s, "attached".into()).await
            }
            Builtin::AppClose => {
                let app = s.app()?.clone();
                let code = app.close().await.map_err(e)?;
                s.current = None;
                s.refs.clear();
                Ok(vec![text(format!("closed (exit code {code:?})"))])
            }
            Builtin::AppList => {
                let lines: Vec<String> = s.apps.iter().enumerate().map(|(i, a)| format!("{i}: {} pid {}{}", a.name(), a.pid(), if a.exit_status().is_some() { " (exited)" } else { "" })).collect();
                Ok(vec![text(if lines.is_empty() { "no apps".into() } else { lines.join("\n") })])
            }
            Builtin::WindowList => {
                let ws = s.app()?.windows().await.map_err(e)?;
                let lines: Vec<String> = ws
                    .iter()
                    .map(|w| {
                        format!(
                            "{} {} {}x{}{}{}",
                            crate::snapshot::window_label(w),
                            w.id,
                            w.rect.width,
                            w.rect.height,
                            w.kind.as_deref().map(|k| format!(" kind={k}")).unwrap_or_default(),
                            if w.is_shown() { "" } else { " hidden" }
                        )
                    })
                    .collect();
                Ok(vec![text(lines.join("\n"))])
            }
            Builtin::WindowSelect => {
                let sel = str_arg(&a, "selector").ok_or("give `selector`")?;
                let timeout = ms_arg(&a, "timeout_ms").unwrap_or(secs(5));
                let (i, _) = s.current.clone().ok_or("no app yet")?;
                let w = s.apps[i].wait_for_window(sel, timeout).await.map_err(e)?;
                s.current = Some((i, w));
                after_action(s, format!("now on {sel}")).await
            }
            Builtin::Snapshot => Ok(vec![text(s.snapshot().await?)]),
            Builtin::Click => {
                let (w, label, sel) = s.target(&a).await?;
                let mut c = w.locator(&sel).click().mode(s.policy.input_mode);
                if a.get("double").and_then(Value::as_bool) == Some(true) {
                    c = c.count(2);
                }
                let button = match str_arg(&a, "button") {
                    Some("right") => MouseButton::Right,
                    Some("middle") => MouseButton::Middle,
                    _ => MouseButton::Left,
                };
                c.button(button).await.map_err(e)?;
                s.record(
                    if a.get("double").and_then(Value::as_bool) == Some(true) { "dblclick" } else { "click" },
                    Some(label),
                    Some(sel.clone()),
                    json!({ "button": str_arg(&a, "button").unwrap_or("left") }),
                );
                after_action(s, format!("clicked {sel}")).await
            }
            Builtin::Hover => {
                let (w, label, sel) = s.target(&a).await?;
                w.locator(&sel).hover().mode(s.policy.input_mode).await.map_err(e)?;
                s.record("hover", Some(label), Some(sel.clone()), Value::Null);
                after_action(s, format!("hovering {sel}")).await
            }
            Builtin::Drag => {
                let from = s.target(&json!({ if str_arg(&a, "from").is_some_and(|f| s.refs.contains_key(f)) { "ref" } else { "selector" }: str_arg(&a, "from") })).await?;
                let to = s.target(&json!({ if str_arg(&a, "to").is_some_and(|f| s.refs.contains_key(f)) { "ref" } else { "selector" }: str_arg(&a, "to") })).await?;
                let target = to.0.locator(&to.2);
                from.0.locator(&from.2).drag_to(&target).mode(s.policy.input_mode).await.map_err(e)?;
                s.record("drag_to", Some(from.1), Some(from.2.clone()), json!({ "target": to.2 }));
                after_action(s, "dragged".into()).await
            }
            Builtin::Scroll => {
                let (w, label, sel) = s.target(&a).await?;
                let (dx, dy) = (a.get("dx").and_then(Value::as_f64).unwrap_or(0.0), a.get("dy").and_then(Value::as_f64).unwrap_or(0.0));
                w.locator(&sel).scroll(dx, dy).mode(s.policy.input_mode).await.map_err(e)?;
                s.record("scroll", Some(label), Some(sel), json!({ "dx": dx, "dy": dy }));
                after_action(s, "scrolled".into()).await
            }
            Builtin::Type | Builtin::Fill => {
                let (w, label, sel) = s.target(&a).await?;
                let t = str_arg(&a, "text").ok_or("give `text`")?.to_string();
                let loc = w.locator(&sel);
                if *self == Builtin::Fill {
                    loc.fill(&t).mode(s.policy.input_mode).await.map_err(e)?;
                    s.record("fill", Some(label.clone()), Some(sel.clone()), json!({ "text": t }));
                } else {
                    loc.type_text(&t).mode(s.policy.input_mode).await.map_err(e)?;
                    s.record("type", Some(label.clone()), Some(sel.clone()), json!({ "text": t }));
                    if a.get("submit").and_then(Value::as_bool) == Some(true) {
                        loc.press("Enter").mode(s.policy.input_mode).await.map_err(e)?;
                        s.record("press", Some(label), Some(sel.clone()), json!({ "keys": "Enter" }));
                    }
                }
                after_action(s, format!("typed into {sel}")).await
            }
            Builtin::PressKey => {
                let keys = str_arg(&a, "keys").ok_or("give `keys`")?.to_string();
                if a.get("ref").is_some() || a.get("selector").is_some() {
                    let (w, label, sel) = s.target(&a).await?;
                    w.locator(&sel).press(&keys).mode(s.policy.input_mode).await.map_err(e)?;
                    s.record("press", Some(label), Some(sel), json!({ "keys": keys }));
                } else {
                    let w = s.window()?.clone();
                    w.keyboard().mode(s.policy.input_mode).press(&keys).await.map_err(e)?;
                    let label = crate::snapshot::window_label(&w.info().await.map_err(e)?);
                    s.record("window_press", Some(label), None, json!({ "keys": keys }));
                }
                after_action(s, format!("pressed {keys}")).await
            }
            Builtin::Screenshot => {
                let mut shot = if a.get("ref").is_some() || a.get("selector").is_some() {
                    let (w, _, sel) = s.target(&a).await?;
                    w.locator(&sel).screenshot()
                } else {
                    s.window()?.screenshot()
                };
                if let Some(src) = str_arg(&a, "source") {
                    shot = shot.source(CaptureSource::from_str(src).map_err(e)?);
                }
                let img = shot.await.map_err(e)?;
                let mut out = Vec::new();
                if let Some(name) = str_arg(&a, "save_as") {
                    let p = s.policy.output(name)?;
                    raywright::imaging::save_png(&img, &p).map_err(e)?;
                    out.push(text(format!("saved {}", p.display())));
                }
                out.push(png(&img, s.policy.max_image_px)?);
                Ok(out)
            }
            Builtin::Expect => {
                let (w, label, sel) = s.target(&a).await?;
                let what = str_arg(&a, "assertion").ok_or("give `assertion`")?;
                let value = a.get("value").map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()));
                let mut x = expect(w.locator(&sel));
                if let Some(t) = ms_arg(&a, "timeout_ms") {
                    x = x.timeout(t);
                }
                let need = || value.clone().ok_or_else(|| format!("{what} needs `value`"));
                let r = match what {
                    "visible" => x.to_be_visible().await,
                    "hidden" => x.to_be_hidden().await,
                    "enabled" => x.to_be_enabled().await,
                    "disabled" => x.to_be_disabled().await,
                    "checked" => x.to_be_checked().await,
                    "unchecked" => x.not().to_be_checked().await,
                    "text" => x.to_have_text(&need()?).await,
                    "contains" => x.to_contain_text(&need()?).await,
                    "value" => x.to_have_value(&need()?).await,
                    "count" => x.to_have_count(need()?.parse().map_err(|_| "count needs a number")?).await,
                    other => return Err(format!("unknown assertion `{other}`")),
                };
                let matcher = match what {
                    "visible" => "to_be_visible",
                    "hidden" => "to_be_hidden",
                    "enabled" => "to_be_enabled",
                    "disabled" => "to_be_disabled",
                    "checked" | "unchecked" => "to_be_checked",
                    "text" => "to_have_text",
                    "contains" => "to_contain_text",
                    "value" => "to_have_value",
                    _ => "to_have_count",
                };
                match r {
                    Ok(()) => {
                        let mut args = json!({ "matcher": matcher, "not": what == "unchecked" });
                        match (what, &value) {
                            ("count", Some(v)) => args["count"] = json!(v.parse::<u64>().unwrap_or(0)),
                            (_, Some(v)) => args["value"] = json!(v),
                            _ => {}
                        }
                        s.record("expect", Some(label), Some(sel.clone()), args);
                        Ok(vec![text(format!("passed: {sel} {what}{}", value.map(|v| format!(" {v:?}")).unwrap_or_default()))])
                    }
                    Err(err) => Ok(vec![text(format!("FAILED: {err}"))]),
                }
            }
            Builtin::WaitFor => {
                let (w, _, sel) = s.target(&a).await?;
                let state = match str_arg(&a, "state").unwrap_or("visible") {
                    "attached" => WaitState::Attached,
                    "detached" => WaitState::Detached,
                    "hidden" => WaitState::Hidden,
                    _ => WaitState::Visible,
                };
                w.locator(&sel).wait_for(state, ms_arg(&a, "timeout_ms")).await.map_err(e)?;
                after_action(s, format!("{sel} is {state:?}")).await
            }
            Builtin::BridgeCall => {
                let name = str_arg(&a, "name").ok_or("give `name`")?.to_string();
                let args = a.get("args").cloned().unwrap_or(Value::Null);
                let r = s.app()?.bridge().await.map_err(e)?.call(&name, args.clone()).await.map_err(e)?;
                s.record("bridge_call", None, None, json!({ "name": name, "args": args }));
                Ok(vec![text(serde_json::to_string_pretty(&r).unwrap_or_default())])
            }
            Builtin::RecordStart => {
                if s.recording.is_some() {
                    return Err("already recording; record_stop first".into());
                }
                let opts = record_options(s, &a);
                let h = if a.get("ref").is_some() || a.get("selector").is_some() {
                    let (w, _, sel) = s.target(&a).await?;
                    w.locator(&sel).record(opts).await.map_err(e)?
                } else {
                    s.window()?.record(opts).await.map_err(e)?
                };
                let id = s.recording_id();
                s.recording = Some((id.clone(), h));
                Ok(vec![text(format!("recording {id}; call record_stop when done"))])
            }
            Builtin::RecordStop => {
                let (id, h) = s.recording.take().ok_or("not recording")?;
                let rec = h.stop().await.map_err(e)?;
                finish_recording(s, id, rec, 6)
            }
            Builtin::RecordAction => {
                let (w, label, sel) = s.target(&a).await?;
                let settle = ms_arg(&a, "settle_ms").unwrap_or(Duration::from_millis(300));
                let opts = record_options(s, &a).until_settled(settle);
                let mode = s.policy.input_mode;
                let loc = w.locator(&sel);
                let action = str_arg(&a, "action").unwrap_or("click").to_string();
                let (keys, txt) = (str_arg(&a, "keys").map(str::to_string), str_arg(&a, "text").map(str::to_string));
                let act = async {
                    match action.as_str() {
                        "press_key" => loc.press(keys.as_deref().unwrap_or("Enter")).mode(mode).await,
                        "type" => loc.type_text(txt.as_deref().unwrap_or_default()).mode(mode).await,
                        _ => loc.click().mode(mode).await,
                    }
                };
                let (r, rec) = w.record_during(opts, act).await.map_err(e)?;
                r.map_err(e)?;
                let (step, args) = match action.as_str() {
                    "press_key" => ("press", json!({ "keys": keys })),
                    "type" => ("type", json!({ "text": txt })),
                    _ => ("click", json!({})),
                };
                s.record(step, Some(label), Some(sel), args);
                let id = s.recording_id();
                let n = a.get("keyframes").and_then(Value::as_u64).unwrap_or(6) as usize;
                finish_recording(s, id, rec, n)
            }
            Builtin::RecordingFrames => {
                let id = str_arg(&a, "id").ok_or("give `id`")?;
                let rec = s.recordings.get(id).ok_or_else(|| format!("no recording {id}"))?;
                let mut out = Vec::new();
                for f in a.get("frames").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).take(12) {
                    let frame = rec.frame(f as usize).ok_or_else(|| format!("{id} has {} frames", rec.len()))?;
                    out.push(text(format!("frame {f} at {} ms", frame.t.as_millis())));
                    out.push(png(&frame.image, s.policy.max_image_px)?);
                }
                Ok(out)
            }
            Builtin::GenerateTest => {
                let name = str_arg(&a, "name").unwrap_or("recorded");
                let launch = s.launch.clone().ok_or("nothing to generate: launch an app first")?;
                Ok(vec![text(raywright::codegen::to_rust(name, &launch, &s.steps))])
            }
        }
    }
}

fn record_options(s: &Session, a: &Value) -> RecordOptions {
    let mut o = RecordOptions::from_config(&s.rw.config().record);
    if let Some(fps) = a.get("fps").and_then(Value::as_f64) {
        o = o.fps(fps);
    }
    if let Some(d) = ms_arg(a, "max_duration_ms") {
        o = o.max_duration(d);
    }
    if let Some(step) = ms_arg(a, "manual_clock_ms") {
        o = o.manual_clock(step);
    }
    o
}

/// Saves a recording, keeps it for `recording_frames`, and answers with a summary, the
/// contact sheet and the `n` frames that changed most.
fn finish_recording(s: &mut Session, id: String, rec: Recording, n: usize) -> ToolResult {
    let dir: PathBuf = s.policy.output(&format!("recordings/{id}"))?;
    rec.save_dir(&dir).map_err(e)?;
    let a = rec.analysis();
    let keys = rec.keyframes(n);
    let summary = json!({
        "id": id,
        "frames": a.frames,
        "duration_ms": a.duration_ms,
        "effective_fps": (a.effective_fps * 10.0).round() / 10.0,
        "first_change_ms": a.first_change_ms,
        "last_change_ms": a.last_change_ms,
        "settled_ms": a.settled_ms,
        "changed_region": a.changed_region,
        "dropped_frames": a.dropped_frames,
        "keyframes": keys,
        "folder": dir.display().to_string(),
    });
    let mut out = vec![text(serde_json::to_string_pretty(&summary).unwrap_or_default()), png(&rec.contact_sheet(s.policy.max_image_px), s.policy.max_image_px)?];
    for k in keys {
        if let Some(f) = rec.frame(k) {
            out.push(text(format!("frame {k} at {} ms", f.t.as_millis())));
            out.push(png(&f.image, s.policy.max_image_px.min(800))?);
        }
    }
    s.recordings.insert(id, rec);
    Ok(out)
}
