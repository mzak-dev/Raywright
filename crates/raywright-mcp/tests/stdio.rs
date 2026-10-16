//! The server as an MCP client sees it: spawned over stdio, on the mock driver.

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use serde_json::{Value, json};

type Client = RunningService<RoleClient, ()>;

async fn call(c: &Client, name: &str, args: Value) -> CallToolResult {
    let Value::Object(map) = args else { panic!("args must be an object") };
    c.call_tool(CallToolRequestParams::new(name.to_string()).with_arguments(map)).await.unwrap()
}

fn texts(r: &CallToolResult) -> String {
    r.content.iter().filter_map(|c| if let ContentBlock::Text(t) = c { Some(t.text.clone()) } else { None }).collect::<Vec<_>>().join("\n")
}

fn images(r: &CallToolResult) -> usize {
    r.content.iter().filter(|c| matches!(c, ContentBlock::Image(_))).count()
}

/// The ref of the first snapshot line containing `needle`.
fn ref_of(snapshot: &str, needle: &str) -> String {
    let line = snapshot.lines().find(|l| l.contains(needle)).unwrap_or_else(|| panic!("no {needle:?} in\n{snapshot}"));
    let start = line.find("[ref=").unwrap() + 5;
    line[start..].split(']').next().unwrap().to_string()
}

#[tokio::test]
async fn an_agent_session_end_to_end() {
    let out = tempfile::tempdir().unwrap();
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_raywright-mcp"));
    cmd.args(["--driver", "mock", "--output-dir"]).arg(out.path());
    let c: Client = ().serve(TokioChildProcess::new(cmd).unwrap()).await.unwrap();

    let tools: Vec<String> = c.list_all_tools().await.unwrap().into_iter().map(|t| t.name.to_string()).collect();
    for t in ["app_launch", "snapshot", "click", "expect", "screenshot", "record_action", "generate_test"] {
        assert!(tools.contains(&t.to_string()), "{t} missing from {tools:?}");
    }

    // a program that isn't allowed is refused
    let r = call(&c, "app_launch", json!({ "exe": "cmd.exe" })).await;
    assert_eq!(r.is_error, Some(true));
    assert!(texts(&r).contains("not allowed"));

    let r = call(&c, "app_launch", json!({ "exe": "demo-app" })).await;
    let snap = texts(&r);
    assert!(r.is_error != Some(true), "{snap}");
    let increment = ref_of(&snap, "button \"Increment\"");

    let r = call(&c, "click", json!({ "ref": increment })).await;
    assert!(texts(&r).contains("\"Count: 1\""), "{}", texts(&r));

    let r = call(&c, "expect", json!({ "selector": "key=\"counter\"", "assertion": "text", "value": "Count: 1" })).await;
    assert!(texts(&r).starts_with("passed"), "{}", texts(&r));
    let r = call(&c, "expect", json!({ "selector": "key=\"counter\"", "assertion": "text", "value": "Count: 9", "timeout_ms": 200 })).await;
    assert!(texts(&r).starts_with("FAILED"), "{}", texts(&r));

    let r = call(&c, "fill", json!({ "selector": "role=textbox", "text": "hello" })).await;
    assert!(texts(&r).contains("value=\"hello\""), "{}", texts(&r));

    let r = call(&c, "screenshot", json!({ "save_as": "window.png" })).await;
    assert_eq!(images(&r), 1);
    assert!(out.path().join("window.png").exists());

    let r = call(&c, "record_action", json!({ "selector": "action=\"canvas.box\"", "action": "click", "keyframes": 3 })).await;
    let summary: Value = serde_json::from_str(texts(&r).split("\nframe ").next().unwrap()).unwrap();
    assert!(summary["frames"].as_u64().unwrap() > 2, "{summary}");
    assert!(summary["last_change_ms"].is_number(), "{summary}");
    assert!(images(&r) >= 2, "contact sheet and keyframes");
    let id = summary["id"].as_str().unwrap().to_string();
    let r = call(&c, "recording_frames", json!({ "id": id, "frames": [0] })).await;
    assert_eq!(images(&r), 1);

    let r = call(&c, "bridge_call", json!({ "name": "state" })).await;
    assert!(texts(&r).contains("\"clicks\": 1"), "{}", texts(&r));

    let r = call(&c, "generate_test", json!({ "name": "agent session" })).await;
    let code = texts(&r);
    assert!(code.contains("async fn agent_session()"), "{code}");
    assert!(code.contains(".click().await?;"), "{code}");
    assert!(code.contains(".fill(\"hello\").await?;"), "{code}");
    assert!(code.contains(".to_have_text(\"Count: 1\")"), "{code}");

    let r = call(&c, "app_close", json!({})).await;
    assert!(texts(&r).contains("closed"));
    assert!(out.path().join("session.log").exists());
    c.cancel().await.unwrap();
}
