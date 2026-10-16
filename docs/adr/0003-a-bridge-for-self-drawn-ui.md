# A bridge inside the app for self-drawn UI

Apps that draw with a GPU (wgpu, Skia, games) show UI Automation one opaque window. Rather
than recognise pixels, such an app embeds `raywright-bridge`, a small JSON-RPC server that
reports its windows and elements from the app's own layout, returns its own frames, runs
its commands and can stop its animation clock.

The bridge is synchronous, has no tokio and depends only on `raywright-protocol`, so an
app adds it behind a feature flag for little cost. It only answers when Raywright asked
for it (`RAYWRIGHT_BRIDGE`), so release builds stay as they are.
