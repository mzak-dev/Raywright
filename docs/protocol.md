# Bridge protocol

An app that draws its own UI embeds `raywright-bridge` so Raywright can see inside its
windows. The protocol is JSON-RPC 2.0, one JSON message per line (`\n`), over any byte
stream. Types live in `raywright-protocol`; this page is the reference for implementing
the server in another language.

## Finding the bridge

When Raywright launches an app with `BridgeMode::Auto` (the default), it picks an address
and passes it in `RAYWRIGHT_BRIDGE`. The app listens there and prints one line to stderr:

```
raywright-bridge: listening pipe://raywright-1234-0-18f2a
```

Addresses are URIs:

| URI | Transport |
|---|---|
| `pipe://name` | Windows named pipe `\\.\pipe\name`, current user only |
| `tcp://127.0.0.1:port` | loopback TCP (port 0: the OS picks one, read it from the announce line) |
| `unix:///path.sock` | reserved for Linux and macOS |
| `memory://name` | in-process, tests |

An app started without the variable runs no bridge, unless it chooses an address itself
(`Bridge::builder(..).addr(..)`); `App::connect_bridge(addr)` then attaches to it.

## Methods

| Method | Params | Result | Capability |
|---|---|---|---|
| `hello` | `{}` | `{ app, version, protocol: 1, capabilities: [..], commands: [..] }` | always |
| `windows` | `{}` | `[WindowDesc]` | always |
| `elements` | `{ window }` | `[ElementDesc]` in paint order | `elements` |
| `capture` | `{ window }` | `{ png: base64, width, height, frame_seq }` | `capture` |
| `input` | `{ window, event }` | `null` | `input` |
| `call` | `{ name, args }` | any JSON | `call` |
| `clock` | `{ mode: "real" \| "manual" }` | `null` | `manual-clock` |
| `clock_step` | `{ ms, window? }` | `{ frame_seq, elapsed_ms }` | `manual-clock` |

Notification from the app: `frame { window, seq }` after a window presents a frame
(capability `frame-events`).

```json
{"jsonrpc":"2.0","id":3,"method":"elements","params":{"window":197234}}
{"jsonrpc":"2.0","id":3,"result":[{"key":"clock.face","role":"img","rect":{"x":0,"y":0,"w":160,"h":160}},
  {"key":"clock.menu","action":"menu.open","role":"button","text":"Menu","rect":{"x":130,"y":4,"w":24,"h":24}}]}
```

### WindowDesc

`id` is the native handle as a number (an HWND on Windows), so Raywright matches it with
the OS window list. `client` is the client area on screen in physical px, `scale` turns
logical px into physical. `kind`, `name` and `attrs` are the app's own words, e.g.
`kind: "settings"`.

### ElementDesc

| Field | |
|---|---|
| `key` | unique in the window while the element exists |
| `action` | what clicking it does (`media.play_pause`) |
| `text`, `role` | role names follow ARIA: `button`, `text`, `img`, `slider`, `textbox` |
| `rect`, `clip` | logical px relative to the client area; `clip` is the visible part |
| `parent` | the `key` of the containing element |
| `enabled`, `visible` | default `true` |
| `attrs` | anything else, matched by `bridge=[name=value]`; `attrs.name` is the accessible name (`role=button[name="Play"]`) when it differs from `text` |

### Input events

`{ "type": "mouse_move" | "mouse_down" | "mouse_up", x, y, button? }`,
`{ "type": "wheel", x, y, dx, dy }`, `{ "type": "key_down" | "key_up", key }` with W3C key
values, `{ "type": "text", text }`.

### Manual clock

After `clock { mode: "manual" }` the app's animation time stands still. `clock_step` adds
`ms` to it, makes the app draw, and answers once the frame showing the new time is drawn,
with that frame's `seq`. A recording then captures exactly one picture per step, the same
on every machine. Answer `clock_step` with integer-millisecond time if the app rounds
positions, so steps don't drift.

## Errors and evolution

Standard JSON-RPC codes; `-32601` for an unknown method (Raywright then treats the
capability as missing), `-32602` for bad params, `-32000` for the app's own errors.

Fields are only ever added, with defaults; clients ignore unknown fields. A new method
comes with a new capability name. `protocol` changes only for a breaking change.

## In Rust

```rust
let handle = Bridge::builder("myapp", env!("CARGO_PKG_VERSION"))
    .capabilities([capability::ELEMENTS, capability::CAPTURE, capability::CALL])
    .commands(commands.names())
    .start(move |req: Request, reply: Responder| {
        // Hand req to the UI thread; call reply.ok(..) or reply.err(..) from there.
        proxy.send_event(UserEvent::Automation(req, reply)).ok();
    })?;
let frames = handle.notifier(); // frames.frame(window, seq) after each present
```

The server uses plain threads, no async runtime: an app pays for nothing it doesn't use.
