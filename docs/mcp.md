# MCP server

`raywright-mcp` lets Claude Code (or any MCP client) drive a desktop app the way
Playwright MCP drives a browser: read a text snapshot of a window, act on elements by
reference, take screenshots, record animations and turn the session into a test.

```
cargo install --git https://github.com/mzak-dev/Raywright --tag v0.1.0 raywright-mcp
claude mcp add raywright -- raywright-mcp --allow-exe C:\path\to\app.exe
```

Or in the app's repository, `.mcp.json`:

```json
{ "mcpServers": { "raywright": { "command": "raywright-mcp",
  "args": ["--allow-exe", "target\\debug\\app.exe", "--profile-file", "tests\\e2e\\raywright.toml"] } } }
```

It drives a real desktop, so it runs where the app runs: on Windows. `--driver mock`
runs it anywhere against a built-in demo app.

## Tools

| Tool | |
|---|---|
| `app_launch`, `app_attach`, `app_close`, `app_list` | start an allowed exe or a profile; attach needs `--allow-attach` |
| `window_list`, `window_select` | windows of the current app; pick the one to act on |
| `snapshot` | the window's elements as an indented list with refs `e1`, `e2`… |
| `click`, `hover`, `drag`, `scroll`, `type`, `fill`, `press_key` | act on a `ref` or a `selector`, with auto-waiting; answers with a fresh snapshot |
| `screenshot` | the window or one element, as an image (scaled to `--max-image-px`) |
| `expect` | `visible`, `hidden`, `enabled`, `disabled`, `checked`, `text`, `contains`, `value`, `count` |
| `wait_for` | until a selector is attached, detached, visible or hidden |
| `bridge_call` | one of the app's own commands |
| `record_action` | do an action and record until the window settles; returns a contact sheet and keyframes |
| `record_start`, `record_stop`, `recording_frames` | longer recordings and single frames from them |
| `generate_test` | the session's steps as a `#[raywright::test]` |

Refs live until the next snapshot. Each one is stored as the shortest selector that finds
exactly that element, so `generate_test` writes selectors, never refs.

## Safety

| Flag | Default |
|---|---|
| `--allow-exe <path>` (repeatable) | nothing may be launched except these and the profiles |
| `--allow-attach` | off |
| `--input-mode os\|message` | `message`: input goes to the app's windows, the user keeps the mouse |
| `--output-dir` | `raywright-mcp-output`; every file the server writes goes here, plus `session.log` |
| `--max-image-px` | 1568 |
| `--profile-file` | a raywright.toml whose `[profiles.*]` can be launched by name |

The server runs no shell commands.

## Your own tools

```rust
RaywrightServer::new(rw, policy).tool(Arc::new(MyTool)).serve_stdio().await?;
```

`MyTool` implements `McpTool`: `name`, `description`, `schema` (JSON Schema of its input)
and `call(&mut Session, args)`.
