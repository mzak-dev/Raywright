# Plugins (design)

> Status: proposed. Nothing below is implemented yet; this page is the plan the code will
> follow. The first plugin is Godot.

A plugin teaches Raywright one kind of app: how to start it, how to see inside it, what an
agent can ask it. Godot comes first; Unity, Flutter or Electron would be the same shape. A
plugin is one crate in any repository, composed with Raywright when a test or a tool is
built.

[extending.md](extending.md) lists the extension points one by one. A plugin is a named
bundle of them, plus what single extension points can't say: which apps it is for, its own
profile settings and its own MCP tools.

## What a plugin can add

| | Through | Godot |
|---|---|---|
| selector engines, element sources, hooks, transports, comparators, exporters, reporters | `PluginCx` in `setup` | `node=HUD/Start` |
| profiles in raywright.toml | `Plugin::profile` | `[profiles.game] plugin = "godot"`, `project = "."` |
| its own settings | `[plugins.<name>]` in raywright.toml | `bin`, allowed projects |
| knowing its apps | `Plugin::claims(&Hello)` | the bridge announces capability `godot` |
| a driver | `Plugin::driver` | the bridge driver: works on every OS, in the background |
| test helpers | extension traits in the plugin crate | `app.godot().await?.call(..)` |
| MCP tools and instructions | `McpPlugin` in `raywright-mcp` | `godot_tree`, `godot_inspect`, `godot_logs`... |

## In tests

```rust
let rw = Raywright::platform().plugin(raywright_godot::plugin()).build()?;
let app = rw.launch_named("game").await?;          // [profiles.game] plugin = "godot"
let win = app.first_window().await?;
win.locator("node=HUD/Start").click().await?;
app.godot().await?.action("jump", Tap).await?;      // GodotAppExt, from the plugin crate
```

```toml
[plugins.godot]
bin = "C:/tools/godot-4.7.exe"

[profiles.game]
plugin = "godot"
project = "."
scene = "res://levels/one.tscn"
```

## The core API: `raywright_core::plugin`

```rust
pub trait Plugin: Send + Sync + 'static {
    /// `godot`: the name in raywright.toml (`plugin = "godot"`, `[plugins.godot]`), in
    /// `--plugin-opt godot.key=value` and in tool names.
    fn name(&self) -> &str;
    fn version(&self) -> &str;

    /// Registers engines, sources, hooks and the rest. Runs once, in `build()`.
    fn setup(&self, cx: &mut PluginCx<'_>) -> Result<()>;

    /// A profile from a `[profiles.x]` table that says `plugin = "<name>"`; the table's
    /// other keys are the plugin's to read.
    fn profile(&self, name: &str, table: &toml::Table, cx: &PluginConfig) -> Result<Box<dyn AppProfile>> {
        Err(Error::Config(format!("plugin {} has no profiles", self.name())))
    }

    /// Is this app one of mine? Asked once the app's bridge said hello.
    fn claims(&self, hello: &Hello) -> bool {
        false
    }

    /// A driver to use instead of the platform's when the user picks this plugin's
    /// (`.driver_from_plugin("godot")`, `raywright-mcp --driver godot`).
    fn driver(&self) -> Option<Arc<dyn Driver>> {
        None
    }
}
```

- `PluginCx` is a narrow view of the builder: `engine`, `source`, `hook`, `transport`,
  `comparator`, `exporter`, `reporter`, and `config()` with the plugin's `[plugins.<name>]`
  table (`PluginConfig`, merged with `--plugin-opt` values). It can't touch the driver,
  timeouts or other plugins.
- `RaywrightBuilder::plugin(impl Plugin)`; two plugins with one name fail `build()`.
  `Raywright::plugins()` lists them, `Raywright::plugin(name)` finds one.
- `App::plugin() -> Option<Arc<dyn Plugin>>`: the plugin that claimed the app (the first
  whose `claims` is true), so tools and helpers can tell a Godot game from anything else.
- `ProfileSpec` gains `plugin: Option<String>` and keeps every other key it doesn't know in
  a flattened table. `launch_named` hands a plugin profile to that plugin's `profile()`;
  a profile naming a plugin that isn't registered is a config error that says which one.
- `Config.extra` already keeps unknown sections, so `[plugins.*]` needs no new field.

No API number: plugins are crates, so cargo checks they were built against a compatible
`raywright-core` (semver), and a mismatch fails to compile rather than at run time.

## In the MCP server: `raywright_mcp::McpPlugin`

```rust
pub trait McpPlugin: Send + Sync {
    /// The core half: registered on the server's Raywright.
    fn plugin(&self) -> Arc<dyn Plugin>;
    /// Tools named `<plugin>_*`, listed only when the plugin is enabled.
    fn tools(&self) -> Vec<Arc<dyn McpTool>>;
    /// Added to the server's instructions: when and how to use the tools.
    fn instructions(&self) -> &str { "" }
    /// What the plugin may start or do beyond the server's policy, from its options
    /// (Godot: the projects `godot_launch` may open; eval on or off). Default: nothing.
    fn policy(&self, opts: &PluginConfig) -> Result<PluginPolicy> { Ok(PluginPolicy::default()) }
}
```

- `RaywrightServer::new(rw, policy).plugin(Arc<dyn McpPlugin>)`.
- A plugin's tools run in the same `Session` as the built-ins: refs, steps and recordings
  are shared, so `snapshot`, `click` and `generate_test` work on a Godot game unchanged.
  Steps a plugin tool wants in the generated test go through `Session::record` as
  `bridge_call` (or a step kind the plugin's codegen knows).
- `Session::claimed()` returns the current app's plugin; a tool for another app's plugin
  answers "the current app is not a <name> app" instead of failing obscurely.
- New for every app, because plugins need it first: `screenshot { annotate: true }` draws
  each element's box and ref (`e3`) on the picture, so a model reads a picture and a
  snapshot as one thing. This makes `record::font` public.

### The binary

Rust has no stable ABI, so a plugin can't be loaded into a built `raywright-mcp`. Instead
the server's `main` moves into the library, and a plugin ships a binary of three lines:

```rust
fn main() {
    raywright_mcp::cli::main(vec![Arc::new(raywright_godot::McpGodot)]);
}
```

`cli::main` keeps every flag of `raywright-mcp` and adds `--plugin-opt <name>.<key>=<value>`
(repeatable; the same keys as `[plugins.<name>]`) and `--driver <plugin name>`.

```sh
claude mcp add godot -- raywright-godot-mcp --driver godot --plugin-opt godot.project=.
```

A plugin repository can also be a Claude Code plugin (`.claude-plugin/plugin.json`, an
`.mcp.json` that starts its binary, a skill describing the workflow), so installing it in
Claude Code is one step.

## Godot, the first plugin

Its own repository (`raywright-godot`), depending on this one by git or crates.io:

| Piece | Where |
|---|---|
| GDExtension in the game (godot-rust, Godot 4.7): the bridge protocol of [protocol.md](protocol.md) plus capability `godot` and commands `godot.*` (tree, inspect, get/set/call, logs, InputMap actions, waiting, sampling, scenes) | raywright-godot, feature `extension`, no tokio |
| `GodotPlugin`: `node=` engine, `GodotProject` profile, `claims` by capability, the bridge driver | raywright-godot, feature `test` |
| `GodotAppExt` and the typed `godot.*` client | raywright-godot, feature `test` |
| `McpGodot`: `godot_launch`, `godot_overview`, `godot_tree`, `godot_inspect`, `godot_get/set/call`, `godot_action`, `godot_wait`, `godot_watch`, `godot_time`, `godot_logs`... and `raywright-godot-mcp` | raywright-godot, feature `mcp` |
| the addon folder, a demo project, tests against a real Godot | raywright-godot |

Two pieces it needs belong here instead, because they help every app:

- **`bridge::BridgeDriver`** in raywright-core: a driver made only of an app's bridge (its
  windows, input and pictures). An app with a complete bridge then runs under Raywright on
  Linux and macOS too, and on Windows without taking the user's mouse. `GodotPlugin::driver`
  returns it.
- **annotated screenshots** in raywright-mcp (above).

## Work in this repository

1. `raywright-core`: the `plugin` module, `RaywrightBuilder::plugin`, plugin profiles,
   `App::plugin`, `[plugins.*]` settings.
2. `raywright-core`: `bridge::BridgeDriver`, tested against a bridge in a thread.
3. `raywright-mcp`: `McpPlugin`, `RaywrightServer::plugin`, `cli::main`, `--plugin-opt`,
   `--driver <plugin>`, `screenshot { annotate }`.
4. A plugin over the mock demo app in `raywright-mock` (`demo`: a `demo=` engine, a
   profile, one MCP tool), so the plumbing is tested on every OS without Godot; an example
   `examples/plugin.rs` next to the other extension examples.
5. Docs: this page loses "design", [extending.md](extending.md) and [mcp.md](mcp.md) point
   to it.

Then `raywright-godot` starts in its own repository against that API.

## Not now

- Plugins loaded at run time (dynamic libraries, or tool providers in other processes over
  JSON-RPC). See [ADR 0006](adr/0006-plugins-are-crates.md) for why.
- A plugin registry or `raywright plugin install`.
- Plugins for the inspector TUI (`raywright-cli`); its `[tui]` section stays as it is.
