# Raywright

**Playwright for desktop apps, in Rust.** Launch an app, find what is on screen by role, text
or the app's own keys, click and type with auto-waiting, compare screenshots, record
animations frame by frame, and trace every step. Async on tokio. Windows today; the core is
platform-free and other systems plug in as drivers.

```rust
use raywright::prelude::*;

#[raywright::test]
async fn increments() -> raywright::Result<()> {
    let rw = Raywright::new()?;
    let app = rw.launch(LaunchSpec::exe("demo-app.exe")).await?;
    let win = app.window("title=\"Raywright Demo\"").await?;

    win.get_by_role("button", "Increment").click().await?;
    expect(win.get_by_text("Count:")).to_have_text("Count: 1").await?;
    expect(&win).to_have_screenshot("after-one-click").await?;

    app.close().await?;
    Ok(())
}
```

## Install

```toml
[dev-dependencies]
raywright = { git = "https://github.com/mzak-dev/Raywright", tag = "v0.1.0" }
```

Tools (inspector TUI, recorder, trace viewer, MCP server for Claude Code):

```sh
cargo install --git https://github.com/mzak-dev/Raywright --tag v0.1.0 raywright-cli
cargo install --git https://github.com/mzak-dev/Raywright --tag v0.1.0 raywright-mcp
```

## What it does

| | Playwright | Raywright |
|---|---|---|
| Start | `chromium.launch()` | `rw.launch(LaunchSpec::exe(..))`, `rw.launch_named("profile")`, `rw.attach(..)` |
| Page | `Page` | `Window`: `app.window("title=Settings")` |
| Find | `getByRole`, `getByText`, CSS | `get_by_role`, `get_by_text`, `get_by_key`, `get_by_action`, `role=`, `text=`, `uia=`, `bridge=`, chains with `>>` |
| Act | `click`, `fill`, `press`, `dragTo` | `click`, `dblclick`, `fill`, `type_text`, `press`, `drag_to`, `scroll`, `check`, all auto-waiting and strict |
| Assert | `expect(..).toBeVisible()` | `expect(..).to_be_visible()`, `to_have_text`, `to_have_count`, `.not()`, `.soft()` |
| Pictures | `toHaveScreenshot` | `to_have_screenshot` (pixelmatch, masks, diffs) |
| Animations | | `record`, `record_during`: frames, `manifest.json`, contact sheet, GIF; `to_settle_within` |
| Trace | trace viewer | `trace.zip` + `report.html`, `raywright show-trace` |
| Agents | Playwright MCP | `raywright-mcp` |

### Apps that draw their own UI

Games, wgpu or Skia apps have no accessibility tree. Add `raywright-bridge` to the app and
answer a few requests (windows, elements, a picture, your own commands); Raywright turns the
answers into locators, clicks and screenshots. With a manual animation clock, recordings are
the same frame for frame on every machine.

## Crates

| Crate | |
|---|---|
| `raywright` | What tests use: the core plus the platform's driver |
| `raywright-core` | Locators, auto-waiting, `expect`, snapshots, recording, tracing, the driver traits. No platform code |
| `raywright-windows` | The Windows driver: UI Automation, `SendInput`, screen capture |
| `raywright-mock` | An in-memory driver, to test Raywright and its extensions on any OS |
| `raywright-bridge` | For apps: show Raywright what you drew. No async runtime needed |
| `raywright-protocol` | The bridge's wire format |
| `raywright-macros` | `#[raywright::test]` |
| `raywright-cli` | `raywright` command and terminal UI |
| `raywright-mcp` | MCP server for Claude Code and other agents |

## Docs

- [Architecture](docs/architecture.md): layers, traits, how a call flows
- [Extending](docs/extending.md): your own selector engines, sources, hooks, matchers, profiles, exporters, MCP tools
- [Bridge protocol](docs/protocol.md)
- [Platforms](docs/platforms.md): where Linux and macOS drivers go
- [Recording](docs/recording.md) and [MCP](docs/mcp.md)
- [CONTEXT.md](CONTEXT.md): the words Raywright uses

## License

MIT
