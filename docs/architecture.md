# Architecture

## Layers

```
raywright-protocol      the bridge's wire format (serde only)
      ▲          ▲
      │          └── raywright-bridge       in the app: std threads, no async runtime
      │
raywright-core          locators, auto-waiting, expect, snapshots, recording, traces,
      ▲          ▲      config, registries, and the driver traits. No platform code.
      │          └── raywright-mock         an in-memory driver and demo app
      │
raywright-windows       the Windows driver (UI Automation, SendInput, BitBlt, WinEvents)
      ▲
raywright               the facade: core + the platform's driver, prelude, #[raywright::test]
      ▲          ▲
raywright-cli    raywright-mcp
```

CI enforces the arrows that matter: `raywright-core` never depends on a platform crate, and
`raywright-bridge` never on tokio.

## How a click flows

1. `win.get_by_role("button", "OK").click()` builds a `Click` with a `Locator`; nothing
   happens until it is awaited.
2. `perform` (core `api/action.rs`) runs the retry loop (`retry.rs`) until the action's
   timeout:
   - the locator resolves: each selector step runs its engine over a `NodeSet`, the
     window's element trees, read lazily from every `ElementSource` once per attempt;
   - strict mode: no match retries, more than one fails at once (`Error::Ambiguous`);
   - the action's `Check`s run (visible, stable across two attempts, enabled, nothing in
     front). A check that doesn't pass yet is the reason shown if time runs out.
3. Hooks run `before` (trace pictures, script recording).
4. In native mode the element's own API is tried (`ElementSource::perform`, e.g. UI
   Automation Invoke); otherwise an `InputBatch` goes to the driver's `InputDriver`: OS input
   behind a process-wide lock (and a named mutex on Windows), or posted window messages.
5. Hooks run `after` (slow motion last).

Nothing is retried after the action started, so nothing is clicked twice.

## Async without blocking

Every public call is async on tokio. Blocking platform work runs elsewhere:

- UI Automation lives on one COM (MTA) worker thread that takes jobs over a channel;
- window events come from a `SetWinEventHook` thread with its own message loop;
- input, capture and window listing run in `spawn_blocking`;
- the bridge client is tokio I/O; the bridge server in the app is plain threads.

Time only flows through `tokio::time`, so the core's tests run on paused time.

## Where things are

| | |
|---|---|
| `core/src/api` | `Raywright`, `App`, `Window`, `Locator`, actions, mouse and keyboard, screenshots |
| `core/src/selector` | parsing, the engines, `NodeSet`/`QueryCx` |
| `core/src/driver` | the traits a platform implements; `OsProcess` |
| `core/src/bridge` | client, `BridgeSource`, `BridgeCapture`, transports |
| `core/src/record` | streams, analysis, exporters, recording assertions |
| `core/src/snapshot` | pixelmatch, the snapshot store, `to_have_screenshot` |
| `core/src/trace` | the trace hook, zip and HTML |
| `core/src/conformance.rs` | the checks a driver must pass, against the demo app |
