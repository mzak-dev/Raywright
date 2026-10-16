# Recording animations

A recording is pictures of a window taken in quick succession, with what changed between
them. Use it to check that an animation ends, ends in time and touches only what it should,
or to hand the whole thing to a person or a model.

```rust
let opts = RecordOptions::default().fps(60.0).until_settled(ms(300));
let button = win.get_by_role("button", "Expand");
let (clicked, rec) = win.record_during(opts, async { button.click().await }).await?;
clicked?;

expect(&rec).to_settle_within(ms(400))?;
rec.save_dir("target/raywright/expand")?;   // frames/, manifest.json, contact.png, anim.gif
let key = rec.keyframes(6);                 // the 6 frames that differ most
```

Or start and stop it yourself: `let h = win.record(opts).await?; ...; let rec = h.stop().await?;`.

## Frame sources

| `RecordOptions` | How | Use when |
|---|---|---|
| `.fps(n)` (default 30) | a picture every tick; late ticks are skipped and noted | any app |
| `.frame_synced()` | one picture per `frame` notification from the bridge | the app has a bridge with `frame-events` |
| `.manual_clock(step)` | stop the app's clock, then step, wait for the frame, capture | the app has `manual-clock`; the same frames on every machine |

A manual-clock recording is deterministic, so its keyframes can be snapshots:
`expect(&rec).to_match_keyframes("expand", 6, rw.config())?`.

`.capture(CaptureSource::App)` takes pictures from the app (through its bridge) instead
of the screen: nothing in front of the window gets in.

## Analysis

Each frame gets the share of pixels that changed since the one before (above
`change_threshold`) and the box around them. From those:

- `settled_at()`: when change stopped, for at least `until_settled`;
- `keyframes(n)`: the n frames that differ most from their neighbours;
- `analysis()`: frames, duration, last change, dropped ticks.

Assertions, on `expect(&rec)`: `to_settle_within`, `to_be_still`, `to_have_frame_count`,
`to_change_only_within(rect)`, `to_match_keyframes`.

## Files

`save_dir` writes:

- `frames/0000.png`…
- `manifest.json`: per frame its time, change and change box; settled time; notes. A model
  can read the motion from this alone.
- `contact.png`: all frames (or a spread of them) as one labelled grid, scaled to fit, for
  one image a model can look at.
- `anim.gif`: for people and the HTML report.

Other formats are an `Exporter`.

## From the command line and MCP

`raywright record --window "title=My App" --fps 60 --duration 2s --out rec/`.

MCP: `record_action` does one action and records until the window settles, then returns
the contact sheet, keyframes and the manifest summary. `record_start` / `record_stop` and
`recording_frames` cover the rest. See [mcp.md](mcp.md).
