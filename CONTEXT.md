# Raywright

Playwright for desktop apps: find, act on, picture and record what native windows show.

## Language

**App**:
A running program Raywright launched or attached to: its process, windows and bridge.
_Avoid_: browser, process (for the whole thing)

**Window**:
One top-level window of an App. What Playwright calls a page.
_Avoid_: page, form

**Element**:
Something inside a Window that an Element Source reports: a button, a label, a shape the app drew.
_Avoid_: control (that is one platform's word), widget

**Element Source**:
One kind of element tree: UI Automation, an app's bridge, the mock. Locators don't know which source an element came from.
_Avoid_: provider, backend

**Selector**:
Text that finds elements: `engine=body`, steps joined with ` >> `.

**Selector Engine**:
What runs one step of a selector (`text=`, `role=`, `uia=`, `bridge=`, `nth=`). Built-in or registered.

**Locator**:
A selector bound to a Window, run again on every use. Actions on it need exactly one match (strict mode).
_Avoid_: element handle (Raywright has none: nothing goes stale)

**Actionability**:
What an action waits for before it acts: visible, stable, enabled, editable, receives events. Each is a Check.

**Driver**:
Everything platform-specific behind traits: processes, windows, input, pictures, element sources. Windows today; the mock everywhere.
_Avoid_: backend, platform layer

**Bridge**:
A small JSON-RPC server inside an app that draws its own UI, telling Raywright its windows, elements and pictures, and running its commands.
_Avoid_: plugin, agent

**Capture Source**:
Where pictures come from: the composed screen, the window itself, or the app (through its bridge).

**Recording**:
Pictures of a window in quick succession, with what changed between them and when it came to rest.
_Avoid_: video (a recording is frames you can assert on)

**Manual Clock**:
An app's animation time, stopped and stepped by Raywright through the bridge, so a recording is the same on every machine.

**Profile**:
How to start one particular app and know it is ready: in code (`AppProfile`) or in raywright.toml.

**Hook**:
Code around every action: tracing, slow motion, writing a script.
_Avoid_: middleware, interceptor

**Trace**:
What a test did, action by action, with pictures before and after: `trace.zip` and `report.html`.
