//! Wire types of the Raywright automation bridge.
//!
//! An application that draws its own UI (a game engine, a wgpu or Skia app) has no
//! accessibility tree for an automation tool to read. The bridge closes that gap: the app
//! answers a few JSON-RPC 2.0 requests about its windows and the elements it drew, and
//! Raywright turns those answers into locators.
//!
//! Framing is one JSON message per line (`\n`-terminated, no embedded newlines), over any
//! duplex byte stream: a Windows named pipe, a Unix socket or loopback TCP. The address is a
//! URI ([`BridgeAddr`]).
//!
//! Evolution rules, so old clients and apps keep working together:
//! - fields are only ever added, and every added field has a default;
//! - an app answers a method it doesn't know with [`codes::METHOD_NOT_FOUND`] and the
//!   client degrades that feature instead of failing;
//! - optional features are announced in [`Hello::capabilities`].

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;

mod addr;
mod geometry;

pub use addr::{AddrError, BridgeAddr};
pub use geometry::Rect;

/// The version this crate speaks. A client accepts any app whose version is at least 1.
pub const PROTOCOL_VERSION: u32 = 1;

/// Printed by an app on stderr once it listens: `raywright-bridge: listening <addr>`.
pub const ANNOUNCE_PREFIX: &str = "raywright-bridge: listening ";

/// The environment variable a launcher sets to tell the app where to listen.
pub const ADDR_ENV: &str = "RAYWRIGHT_BRIDGE";

/// Method names.
pub mod methods {
    pub const HELLO: &str = "hello";
    pub const WINDOWS: &str = "windows";
    pub const ELEMENTS: &str = "elements";
    pub const CAPTURE: &str = "capture";
    pub const INPUT: &str = "input";
    pub const CALL: &str = "call";
    pub const CLOCK: &str = "clock";
    pub const CLOCK_STEP: &str = "clock_step";
    /// Notification, app → client.
    pub const FRAME: &str = "frame";
}

/// Optional features an app announces in [`Hello::capabilities`].
pub mod capability {
    pub const ELEMENTS: &str = "elements";
    pub const CAPTURE: &str = "capture";
    pub const FRAME_EVENTS: &str = "frame-events";
    pub const INPUT: &str = "input";
    pub const CALL: &str = "call";
    pub const MANUAL_CLOCK: &str = "manual-clock";
}

/// JSON-RPC error codes.
pub mod codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
    /// The app understood the request but could not do it (no such window, command failed).
    pub const APP_ERROR: i64 = -32000;
}

// ---------------------------------------------------------------------------------------
// Envelope
// ---------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        RpcError { code, message: message.into(), data: None }
    }
    pub fn method_not_found(method: &str) -> Self {
        Self::new(codes::METHOD_NOT_FOUND, format!("method not found: {method}"))
    }
    pub fn invalid_params(detail: impl fmt::Display) -> Self {
        Self::new(codes::INVALID_PARAMS, format!("invalid params: {detail}"))
    }
    pub fn app(message: impl Into<String>) -> Self {
        Self::new(codes::APP_ERROR, message)
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (code {})", self.message, self.code)
    }
}

impl std::error::Error for RpcError {}

/// One line on the wire.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// Has an `id`: the other side must answer.
    Request {
        id: u64,
        method: String,
        params: Value,
    },
    /// No `id`: fire and forget (used for [`methods::FRAME`]).
    Notification {
        method: String,
        params: Value,
    },
    Response {
        id: u64,
        result: Result<Value, RpcError>,
    },
}

#[derive(Serialize, Deserialize)]
struct Raw {
    jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodeError(pub String);

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bad bridge message: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

impl Message {
    pub fn request(id: u64, method: impl Into<String>, params: impl Serialize) -> Self {
        Message::Request { id, method: method.into(), params: to_value(params) }
    }
    pub fn notification(method: impl Into<String>, params: impl Serialize) -> Self {
        Message::Notification { method: method.into(), params: to_value(params) }
    }
    pub fn ok(id: u64, result: impl Serialize) -> Self {
        Message::Response { id, result: Ok(to_value(result)) }
    }
    pub fn err(id: u64, error: RpcError) -> Self {
        Message::Response { id, result: Err(error) }
    }

    /// The message as one line, without the trailing newline.
    pub fn encode(&self) -> String {
        let raw = match self.clone() {
            Message::Request { id, method, params } => Raw { jsonrpc: "2.0".into(), id: Some(id), method: Some(method), params: Some(params), result: None, error: None },
            Message::Notification { method, params } => Raw { jsonrpc: "2.0".into(), id: None, method: Some(method), params: Some(params), result: None, error: None },
            Message::Response { id, result: Ok(v) } => Raw { jsonrpc: "2.0".into(), id: Some(id), method: None, params: None, result: Some(v), error: None },
            Message::Response { id, result: Err(e) } => Raw { jsonrpc: "2.0".into(), id: Some(id), method: None, params: None, result: None, error: Some(e) },
        };
        // serde_json never writes raw newlines inside strings, so one message is one line.
        serde_json::to_string(&raw).expect("bridge messages always serialize")
    }

    pub fn decode(line: &str) -> Result<Message, DecodeError> {
        let raw: Raw = serde_json::from_str(line.trim()).map_err(|e| DecodeError(e.to_string()))?;
        if raw.jsonrpc != "2.0" {
            return Err(DecodeError(format!("jsonrpc must be \"2.0\", got {:?}", raw.jsonrpc)));
        }
        match (raw.id, raw.method) {
            (Some(id), Some(method)) => Ok(Message::Request { id, method, params: raw.params.unwrap_or(Value::Null) }),
            (None, Some(method)) => Ok(Message::Notification { method, params: raw.params.unwrap_or(Value::Null) }),
            (Some(id), None) => match (raw.result, raw.error) {
                (_, Some(e)) => Ok(Message::Response { id, result: Err(e) }),
                (r, None) => Ok(Message::Response { id, result: Ok(r.unwrap_or(Value::Null)) }),
            },
            (None, None) => Err(DecodeError("neither a method nor an id".into())),
        }
    }
}

fn to_value(v: impl Serialize) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// Parses `params` into `T`, turning a mismatch into [`codes::INVALID_PARAMS`].
pub fn params<T: for<'de> Deserialize<'de>>(params: &Value) -> Result<T, RpcError> {
    let v = if params.is_null() { Value::Object(Default::default()) } else { params.clone() };
    serde_json::from_value(v).map_err(RpcError::invalid_params)
}

// ---------------------------------------------------------------------------------------
// Methods
// ---------------------------------------------------------------------------------------

/// `hello` → [`Hello`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Hello {
    pub app: String,
    #[serde(default)]
    pub version: String,
    pub protocol: u32,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Names accepted by [`methods::CALL`].
    #[serde(default)]
    pub commands: Vec<String>,
}

impl Hello {
    pub fn new(app: impl Into<String>, version: impl Into<String>) -> Self {
        Hello { app: app.into(), version: version.into(), protocol: PROTOCOL_VERSION, capabilities: Vec::new(), commands: Vec::new() }
    }
    pub fn has(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }
}

/// One top-level window of the app, in `windows` → `Vec<WindowDesc>`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WindowDesc {
    /// The native window handle as a number (an HWND on Windows), so the client can match
    /// it with the OS window list.
    pub id: u64,
    #[serde(default)]
    pub title: String,
    /// What the window is for the app, e.g. `"instance"`, `"settings"`, `"menu"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The app's own name for it, e.g. the widget instance id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Logical → physical px.
    #[serde(default = "one")]
    pub scale: f64,
    /// The client area on screen, physical px. Element rects are relative to its origin.
    #[serde(default)]
    pub client: Rect,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, String>,
}

fn one() -> f64 {
    1.0
}

impl WindowDesc {
    pub fn new(id: u64, title: impl Into<String>) -> Self {
        WindowDesc { id, title: title.into(), scale: 1.0, ..Default::default() }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ElementsParams {
    pub window: u64,
}

impl ElementsParams {
    pub fn new(window: u64) -> Self {
        ElementsParams { window }
    }
}

/// One thing the app drew, in `elements` → `Vec<ElementDesc>`, in paint order (later is on top).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ElementDesc {
    /// Unique within the window for as long as the element exists.
    pub key: String,
    /// What clicking it does, e.g. `"media.play_pause"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// A role name as in [ARIA](https://www.w3.org/TR/wai-aria/#role_definitions): `button`, `text`, `slider`...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Logical px, relative to the window's client area.
    pub rect: Rect,
    /// The visible part of `rect` when it is scrolled or clipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<Rect>,
    /// The `key` of the element containing this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, String>,
}

fn yes() -> bool {
    true
}

impl ElementDesc {
    pub fn new(key: impl Into<String>, rect: Rect) -> Self {
        ElementDesc { key: key.into(), rect, enabled: true, visible: true, ..Default::default() }
    }
    pub fn action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }
    pub fn role(mut self, role: impl Into<String>) -> Self {
        self.role = Some(role.into());
        self
    }
    pub fn clip(mut self, clip: Rect) -> Self {
        self.clip = Some(clip);
        self
    }
    pub fn parent(mut self, parent: impl Into<String>) -> Self {
        self.parent = Some(parent.into());
        self
    }
    pub fn attr(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.attrs.insert(k.into(), v.into());
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CaptureParams {
    pub window: u64,
}

impl CaptureParams {
    pub fn new(window: u64) -> Self {
        CaptureParams { window }
    }
}

/// `capture` → the window's own picture of its last frame.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CaptureResult {
    /// PNG, base64 (standard alphabet, padded).
    pub png: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// The frame this picture shows.
    #[serde(default)]
    pub frame_seq: u64,
}

impl CaptureResult {
    pub fn from_png(png: &[u8], width: u32, height: u32, frame_seq: u64) -> Self {
        use base64::Engine;
        CaptureResult { png: base64::engine::general_purpose::STANDARD.encode(png), width, height, frame_seq }
    }
    pub fn png_bytes(&self) -> Result<Vec<u8>, DecodeError> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.decode(&self.png).map_err(|e| DecodeError(format!("capture png: {e}")))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    #[default]
    Left,
    Right,
    Middle,
}

/// Input the app should treat as if it came from the OS. Coordinates are logical px in the
/// window's client area.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum InputEvent {
    MouseMove {
        x: f64,
        y: f64,
    },
    MouseDown {
        x: f64,
        y: f64,
        #[serde(default)]
        button: Button,
    },
    MouseUp {
        x: f64,
        y: f64,
        #[serde(default)]
        button: Button,
    },
    Wheel {
        x: f64,
        y: f64,
        dx: f64,
        dy: f64,
    },
    /// A key by its [W3C `key` value](https://www.w3.org/TR/uievents-key/): `"a"`, `"Enter"`, `"Control"`.
    KeyDown {
        key: String,
    },
    KeyUp {
        key: String,
    },
    Text {
        text: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct InputParams {
    pub window: u64,
    pub event: InputEvent,
}

impl InputParams {
    pub fn new(window: u64, event: InputEvent) -> Self {
        InputParams { window, event }
    }
}

/// `call` runs one of the app's own commands (the bridge's `page.evaluate`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CallParams {
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

impl CallParams {
    pub fn new(name: impl Into<String>, args: Value) -> Self {
        CallParams { name: name.into(), args }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClockMode {
    /// Animations follow the wall clock (normal running).
    #[default]
    Real,
    /// Animation time stands still until `clock_step`.
    Manual,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ClockParams {
    #[serde(default)]
    pub mode: ClockMode,
}

impl ClockParams {
    pub fn new(mode: ClockMode) -> Self {
        ClockParams { mode }
    }
}

/// `clock_step` moves the manual animation clock and answers once the app drew the result.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ClockStepParams {
    pub ms: f64,
    /// Only this window needs to have drawn before the answer; all windows when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<u64>,
}

impl ClockStepParams {
    pub fn new(ms: f64) -> Self {
        ClockStepParams { ms, window: None }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ClockStepResult {
    /// The frame drawn with the new time.
    pub frame_seq: u64,
    /// Total manual time since `clock { mode: manual }`.
    #[serde(default)]
    pub elapsed_ms: f64,
}

impl ClockStepResult {
    pub fn new(frame_seq: u64, elapsed_ms: f64) -> Self {
        ClockStepResult { frame_seq, elapsed_ms }
    }
}

/// `frame` notification: a window presented a new frame.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FrameEvent {
    pub window: u64,
    pub seq: u64,
}

impl FrameEvent {
    pub fn new(window: u64, seq: u64) -> Self {
        FrameEvent { window, seq }
    }
}

/// Parses the address out of an app's announce line on stderr.
pub fn parse_announce(line: &str) -> Option<BridgeAddr> {
    let rest = line.trim().strip_prefix(ANNOUNCE_PREFIX)?;
    BridgeAddr::from_str(rest.trim()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn messages_round_trip_through_one_line() {
        let all = [
            Message::request(7, methods::ELEMENTS, ElementsParams::new(42)),
            Message::notification(methods::FRAME, FrameEvent::new(42, 9)),
            Message::ok(7, json!({"a": "line\nbreak"})),
            Message::err(7, RpcError::method_not_found("nope")),
        ];
        for m in all {
            let line = m.encode();
            assert!(!line.contains('\n'), "{line}");
            assert_eq!(Message::decode(&line).unwrap(), m);
        }
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(Message::decode("{}").is_err());
        assert!(Message::decode(r#"{"jsonrpc":"1.0","id":1,"method":"x"}"#).is_err());
        assert!(Message::decode(r#"{"jsonrpc":"2.0"}"#).is_err());
        assert!(Message::decode("not json").is_err());
    }

    #[test]
    fn a_response_without_result_is_null() {
        let m = Message::decode(r#"{"jsonrpc":"2.0","id":3}"#).unwrap();
        assert_eq!(m, Message::Response { id: 3, result: Ok(Value::Null) });
    }

    /// An app written against protocol 1 may leave out everything with a default.
    #[test]
    fn minimal_payloads_from_old_apps_still_parse() {
        let h: Hello = serde_json::from_value(json!({"app": "x", "protocol": 1})).unwrap();
        assert!(h.capabilities.is_empty());
        let w: WindowDesc = serde_json::from_value(json!({"id": 5})).unwrap();
        assert_eq!(w.scale, 1.0);
        let e: ElementDesc = serde_json::from_value(json!({"key": "k", "rect": {"x": 1, "y": 2, "w": 3, "h": 4}})).unwrap();
        assert!(e.enabled && e.visible);
        assert_eq!(e.rect, Rect::new(1.0, 2.0, 3.0, 4.0));
    }

    /// Fields a newer app sends that this version doesn't know are ignored.
    #[test]
    fn unknown_fields_are_ignored() {
        let e: ElementDesc = serde_json::from_value(json!({"key": "k", "rect": {"x": 0, "y": 0, "w": 1, "h": 1}, "future": [1, 2]})).unwrap();
        assert_eq!(e.key, "k");
    }

    #[test]
    fn input_events_are_tagged() {
        let v = serde_json::to_value(InputEvent::MouseDown { x: 1.0, y: 2.0, button: Button::Right }).unwrap();
        assert_eq!(v, json!({"type": "mouse_down", "x": 1.0, "y": 2.0, "button": "right"}));
    }

    #[test]
    fn params_treats_null_as_empty_object() {
        let p: ClockParams = params(&Value::Null).unwrap();
        assert_eq!(p.mode, ClockMode::Real);
        let e = params::<ElementsParams>(&json!({"window": "x"})).unwrap_err();
        assert_eq!(e.code, codes::INVALID_PARAMS);
    }

    #[test]
    fn capture_png_round_trips() {
        let c = CaptureResult::from_png(&[1, 2, 3, 250], 1, 1, 4);
        assert_eq!(c.png_bytes().unwrap(), vec![1, 2, 3, 250]);
    }

    #[test]
    fn announce_line_parses() {
        let a = parse_announce("raywright-bridge: listening tcp://127.0.0.1:4567\n").unwrap();
        assert_eq!(a, BridgeAddr::Tcp("127.0.0.1:4567".into()));
        assert!(parse_announce("something else").is_none());
    }
}
