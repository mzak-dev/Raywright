# Extending Raywright

Everything below plugs in without changing Raywright. Register through
`Raywright::builder()` (or `rw.register_engine` at run time).

| To add | Implement | Register | Example |
|---|---|---|---|
| a selector engine | `selector::SelectorEngine` | `.engine(Arc::new(..))` | `examples/custom_selector_engine.rs` |
| an element source | `element::ElementSource` | `.source(..)` | `raywright-mock`'s `MockSource` |
| an action hook | `hooks::ActionHook` | `.hook(..)` | `codegen::ScriptRecorder`, `trace::TraceRecorder` |
| a matcher | an extension trait on `Expect<T>` calling `to_pass` | nothing | `examples/custom_matcher.rs` |
| an actionability check | `actionability::Check` | `.check(..)` on an action | `actionability::Stable` |
| an image comparator | `snapshot::ImageComparator` | `.comparator("name", ..)` | `snapshot::Pixelmatch` |
| a recording exporter | `record::Exporter` | `.exporter(..)` | `record::GifExporter` |
| a frame source | `record::CaptureStream` | a driver or your own `Recording::from_frames` | `record::IntervalStream` |
| a reporter | `report::Reporter` | `.reporter(..)` | `report::JunitReporter` |
| a bridge transport | `bridge::Transport` | `.transport(..)` | `bridge::TcpTransport` |
| an app profile | `profile::AppProfile` | `rw.launch_profile(&MyApp)` | `examples/app_profile.rs` |
| an MCP tool | `raywright_mcp::McpTool` | `RaywrightServer::new(..).tool(..)` | the built-ins in `raywright-mcp/src/tools.rs` |
| a platform | the traits in `driver` | one `cfg` in `raywright/src/platform.rs` | `raywright-windows`, `raywright-mock` |

## A selector engine

An engine gets the elements found by the previous step (`None` for the first) and returns
the ones it keeps or finds. Query engines look inside the input (`cx.within`); filter
engines narrow it down (`nth=`, `visible=`).

```rust
#[async_trait]
impl SelectorEngine for Near {
    fn name(&self) -> &str { "near" }
    async fn query(&self, cx: &QueryCx<'_>, input: Option<&[Found]>, body: &str) -> Result<Vec<Found>> { .. }
}
let rw = Raywright::platform().engine(Arc::new(Near)).build()?;
win.locator("role=button >> near=Count").click().await?;
```

## A driver

A platform implements `Driver`, `AppDriver`, `InputDriver`, `CaptureBackend` and at least
one `ElementSource`, then passes `raywright_core::conformance::run` against the demo app.
See [platforms.md](platforms.md).
