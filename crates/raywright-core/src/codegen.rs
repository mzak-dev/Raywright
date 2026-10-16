//! Turning what was done (in the inspector, through MCP) into a Rust test.

use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Error;
use crate::hooks::{ActionCtx, ActionHook};

/// One step of a script.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Step {
    /// `click`, `fill`, `press`, `expect`, `window_press`, `bridge_call`...
    pub action: String,
    /// The window selector.
    pub window: Option<String>,
    pub selector: Option<String>,
    #[serde(default)]
    pub args: Value,
}

/// How the generated test starts the app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Launch {
    Profile(String),
    Exe { program: String, args: Vec<String> },
    Attach { window: String },
}

/// Collects steps as an [`ActionHook`] (successful actions only), plus anything pushed.
#[derive(Default)]
pub struct ScriptRecorder {
    steps: Mutex<Vec<Step>>,
}

impl ScriptRecorder {
    pub fn push(&self, step: Step) {
        self.steps.lock().unwrap().push(step);
    }
    pub fn steps(&self) -> Vec<Step> {
        self.steps.lock().unwrap().clone()
    }
    pub fn clear(&self) {
        self.steps.lock().unwrap().clear();
    }
}

#[async_trait]
impl ActionHook for ScriptRecorder {
    fn priority(&self) -> i32 {
        500
    }

    async fn after(&self, cx: &ActionCtx, outcome: std::result::Result<(), &Error>) {
        if outcome.is_ok() {
            self.push(Step { action: cx.action.to_string(), window: cx.window_label.clone(), selector: cx.selector.clone(), args: cx.args.clone() });
        }
    }
}

fn lit(s: &str) -> String {
    format!("{s:?}")
}

fn ident(name: &str) -> String {
    let mut s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert_str(0, "recorded_");
    }
    s
}

/// The Rust code of one action, given the variable holding its window.
fn line(step: &Step, win: &str) -> Option<String> {
    let loc = step.selector.as_deref().map(|s| format!("{win}.locator({})", lit(s)));
    let a = &step.args;
    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    Some(match step.action.as_str() {
        "click" => {
            let right = a.get("button").and_then(Value::as_str) == Some("right");
            format!("{}.{}().await?;", loc?, if right { "right_click" } else { "click" })
        }
        "dblclick" => format!("{}.dblclick().await?;", loc?),
        "hover" => format!("{}.hover().await?;", loc?),
        "fill" => format!("{}.fill({}).await?;", loc?, lit(&s("text"))),
        "type" => format!("{}.type_text({}).await?;", loc?, lit(&s("text"))),
        "press" => format!("{}.press({}).await?;", loc?, lit(&s("keys"))),
        "check" => format!("{}.check().await?;", loc?),
        "uncheck" => format!("{}.uncheck().await?;", loc?),
        "focus" => format!("{}.focus().await?;", loc?),
        "scroll" => format!("{}.scroll({:?}, {:?}).await?;", loc?, a.get("dx").and_then(Value::as_f64).unwrap_or(0.0), a.get("dy").and_then(Value::as_f64).unwrap_or(0.0)),
        "drag_to" => format!("{}.drag_to(&{win}.locator({})).await?;", loc?, lit(&s("target"))),
        "window_press" => format!("{win}.keyboard().press({}).await?;", lit(&s("keys"))),
        "window_type" => format!("{win}.keyboard().type_text({}).await?;", lit(&s("text"))),
        "expect" => {
            let matcher = s("matcher");
            let value = a.get("value").and_then(Value::as_str);
            let not = if a.get("not").and_then(Value::as_bool) == Some(true) { ".not()" } else { "" };
            match value {
                Some(v) => format!("expect({}){not}.{matcher}({}).await?;", loc?, lit(v)),
                None if matcher == "to_have_count" => format!("expect({}){not}.to_have_count({}).await?;", loc?, a.get("count").and_then(Value::as_u64).unwrap_or(0)),
                None => format!("expect({}){not}.{matcher}().await?;", loc?),
            }
        }
        "screenshot" => format!("{win}.screenshot().path({}).await?;", lit(&s("path"))),
        "bridge_call" => format!("app.bridge().await?.call({}, raywright::serde_json::json!({})).await?;", lit(&s("name")), a.get("args").cloned().unwrap_or(Value::Null)),
        _ => return Some(format!("// {} {}", step.action, step.selector.as_deref().unwrap_or(""))),
    })
}

/// A complete `#[raywright::test]` doing `steps`.
pub fn to_rust(test_name: &str, launch: &Launch, steps: &[Step]) -> String {
    let mut out = String::new();
    out.push_str("use raywright::prelude::*;\n\n#[raywright::test]\nasync fn ");
    out.push_str(&ident(test_name));
    out.push_str("() -> raywright::Result<()> {\n    let rw = Raywright::new()?;\n");
    out.push_str(&match launch {
        Launch::Profile(p) => format!("    let app = rw.launch_named({}).await?;\n", lit(p)),
        Launch::Exe { program, args } => {
            let args = if args.is_empty() { String::new() } else { format!(".args([{}])", args.iter().map(|a| lit(a)).collect::<Vec<_>>().join(", ")) };
            format!("    let app = rw.launch(LaunchSpec::exe({}){args}).await?;\n", lit(program))
        }
        Launch::Attach { window } => format!("    let app = rw.attach(AttachTarget::Window({}.into())).await?;\n", lit(window)),
    });
    let mut windows: Vec<String> = Vec::new();
    for step in steps {
        let label = step.window.clone().unwrap_or_else(|| "*".into());
        let idx = match windows.iter().position(|w| *w == label) {
            Some(i) => i,
            None => {
                windows.push(label.clone());
                let i = windows.len() - 1;
                let get = if label == "*" { "app.first_window().await?".to_string() } else { format!("app.window({}).await?", lit(&label)) };
                out.push_str(&format!("    let w{} = {get};\n", i + 1));
                i
            }
        };
        if let Some(l) = line(step, &format!("w{}", idx + 1)) {
            out.push_str("    ");
            out.push_str(&l);
            out.push('\n');
        }
    }
    if !matches!(launch, Launch::Attach { .. }) {
        out.push_str("    app.close().await?;\n");
    }
    out.push_str("    Ok(())\n}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn writes_a_test() {
        let steps = vec![
            Step { action: "click".into(), window: Some("title=Demo".into()), selector: Some("role=button[name*=\"Add\"]".into()), args: json!({"button": "left"}) },
            Step { action: "fill".into(), window: Some("title=Demo".into()), selector: Some("uia=Edit".into()), args: json!({"text": "hi \"there\""}) },
            Step { action: "expect".into(), window: Some("title=Demo".into()), selector: Some("text=Count".into()), args: json!({"matcher": "to_have_text", "value": "Count: 1"}) },
            Step { action: "window_press".into(), window: Some("title=Other".into()), selector: None, args: json!({"keys": "Control+S"}) },
        ];
        let code = to_rust("My Flow", &Launch::Profile("demo".into()), &steps);
        assert!(code.contains("async fn my_flow()"));
        assert!(code.contains("let app = rw.launch_named(\"demo\").await?;"));
        assert!(code.contains("let w1 = app.window(\"title=Demo\").await?;"));
        assert!(code.contains("w1.locator(\"role=button[name*=\\\"Add\\\"]\").click().await?;"));
        assert!(code.contains(".fill(\"hi \\\"there\\\"\")"));
        assert!(code.contains("expect(w1.locator(\"text=Count\")).to_have_text(\"Count: 1\").await?;"));
        assert!(code.contains("let w2 = app.window(\"title=Other\").await?;"));
        assert!(code.contains("w2.keyboard().press(\"Control+S\").await?;"));
        assert!(code.trim_end().ends_with("Ok(())\n}") || code.ends_with("Ok(())\n}\n"));
    }
}
