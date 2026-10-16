# Platforms

Raywright drives Windows today. Everything outside `raywright-windows` builds and tests on
Linux and macOS too (CI checks it on every push), so a new platform is one new crate.

## What a platform provides

A crate `raywright-<os>` implementing the traits in `raywright_core::driver`:

| Trait | Job |
|---|---|
| `Driver` | spawn and attach to processes, list engines and element sources, pick input and capture |
| `AppDriver` | one app: its windows, events, closing |
| `InputDriver` | mouse, keys and text, for `InputMode::Os` and (if it can) `InputMode::Message` |
| `CaptureBackend` | pictures of the screen or a window |
| `ElementSource` | the platform's accessibility tree |

Then one line in `crates/raywright/src/platform.rs`:

```rust
#[cfg(target_os = "linux")]
return Ok(Arc::new(raywright_linux::LinuxDriver::new()?));
```

and its dependency under `[target.'cfg(target_os = "linux")'.dependencies]`.

A mode or capture source the platform can't do returns `Error::Unsupported`, never panics.

## Where each piece would come from

| | Windows (now) | Linux | macOS |
|---|---|---|---|
| Elements | UI Automation (`uia=`) | AT-SPI over D-Bus (`atspi=`) | AXUIElement (`ax=`) |
| Input | SendInput, PostMessage | XTest on X11; libei or the RemoteDesktop portal on Wayland | CGEvent |
| Capture | BitBlt, PrintWindow | XGetImage; the ScreenCast portal and PipeWire | ScreenCaptureKit |
| Window events | SetWinEventHook | AT-SPI events, X11 | AXObserver |
| Bridge transport | named pipe | Unix socket | Unix socket |
| Kill the app's tree | Job Object | process group, `prctl(PR_SET_PDEATHSIG)` | process group |

`role=`, `text=`, `title=` and `bridge=` selectors work everywhere, so tests written with
them run unchanged on a new platform. `ControlOrMeta+S` is Ctrl on Windows and Linux,
Cmd on macOS.

## Done means conformance

`raywright_core::conformance::run(&rw, &conformance::demo(program))` is the contract: launch, windows,
clicking, typing, screenshots, the bridge and a manual-clock recording. The mock driver
and the Windows driver pass it in CI; a new driver passes it on its own runner.
