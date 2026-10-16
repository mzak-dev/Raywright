use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use image::RgbaImage;
use raywright_protocol::{
    BridgeAddr, CallParams, CaptureParams, CaptureResult, ClockMode, ClockParams, ClockStepParams, ClockStepResult, ElementDesc, ElementsParams, FrameEvent, Hello, InputEvent, InputParams, Message, PROTOCOL_VERSION, RpcError, WindowDesc, methods,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::bridge::transport::Transport;
use crate::error::{Error, Result};

type Pending = Mutex<HashMap<u64, oneshot::Sender<std::result::Result<Value, RpcError>>>>;

/// A connection to one app's bridge. Cheap to clone; requests from many tasks interleave.
#[derive(Clone)]
pub struct BridgeClient {
    inner: Arc<Inner>,
}

struct Inner {
    addr: BridgeAddr,
    out: mpsc::UnboundedSender<String>,
    pending: Arc<Pending>,
    next_id: AtomicU64,
    frames: broadcast::Sender<FrameEvent>,
    notes: broadcast::Sender<(String, Value)>,
    closed: watch::Receiver<bool>,
    timeout: Duration,
    hello: Mutex<Hello>,
}

impl std::fmt::Debug for BridgeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BridgeClient").field("addr", &self.inner.addr.to_string()).field("app", &self.hello().app).finish()
    }
}

impl BridgeClient {
    /// Connects and says `hello`. `timeout` limits every request.
    pub async fn connect(transport: &dyn Transport, addr: &BridgeAddr, timeout: Duration) -> Result<BridgeClient> {
        let stream = transport.connect(addr).await.map_err(|e| Error::Bridge(format!("connect {addr}: {e}")))?;
        let (read, mut write) = tokio::io::split(stream);
        let (out, mut out_rx) = mpsc::unbounded_channel::<String>();
        let pending: Arc<Pending> = Arc::default();
        let (frames, _) = broadcast::channel(256);
        let (notes, _) = broadcast::channel(64);
        let (closed_tx, closed) = watch::channel(false);

        tokio::spawn(async move {
            while let Some(mut line) = out_rx.recv().await {
                line.push('\n');
                if write.write_all(line.as_bytes()).await.is_err() || write.flush().await.is_err() {
                    break;
                }
            }
        });

        let (p, f, n, o) = (pending.clone(), frames.clone(), notes.clone(), out.clone());
        tokio::spawn(async move {
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                match Message::decode(&line) {
                    Ok(Message::Response { id, result }) => {
                        if let Some(tx) = p.lock().unwrap().remove(&id) {
                            let _ = tx.send(result);
                        }
                    }
                    Ok(Message::Notification { method, params }) if method == methods::FRAME => {
                        if let Ok(ev) = serde_json::from_value::<FrameEvent>(params) {
                            let _ = f.send(ev);
                        }
                    }
                    Ok(Message::Notification { method, params }) => {
                        let _ = n.send((method, params));
                    }
                    Ok(Message::Request { id, method, .. }) => {
                        let _ = o.send(Message::err(id, RpcError::method_not_found(&method)).encode());
                    }
                    Err(e) => tracing::warn!("bridge: {e}"),
                }
            }
            let _ = closed_tx.send(true);
            for (_, tx) in p.lock().unwrap().drain() {
                let _ = tx.send(Err(RpcError::app("the bridge connection closed")));
            }
        });

        let client = BridgeClient { inner: Arc::new(Inner { addr: addr.clone(), out, pending, next_id: AtomicU64::new(1), frames, notes, closed, timeout, hello: Mutex::new(Hello::default()) }) };
        let hello: Hello = client.request(methods::HELLO, serde_json::json!({ "protocol": PROTOCOL_VERSION })).await?;
        if hello.protocol < 1 {
            return Err(Error::Bridge(format!("{} speaks bridge protocol {}, need 1 or later", hello.app, hello.protocol)));
        }
        *client.inner.hello.lock().unwrap() = hello;
        Ok(client)
    }

    pub fn addr(&self) -> &BridgeAddr {
        &self.inner.addr
    }

    pub fn hello(&self) -> Hello {
        self.inner.hello.lock().unwrap().clone()
    }

    pub fn has(&self, capability: &str) -> bool {
        self.inner.hello.lock().unwrap().has(capability)
    }

    pub fn is_closed(&self) -> bool {
        *self.inner.closed.borrow()
    }

    /// Resolves when the connection closes.
    pub async fn closed(&self) {
        let mut rx = self.inner.closed.clone();
        while !*rx.borrow_and_update() {
            if rx.changed().await.is_err() {
                return;
            }
        }
    }

    /// Any method, raw.
    pub async fn request_value(&self, method: &str, params: impl Serialize) -> Result<Value> {
        if self.is_closed() {
            return Err(Error::Bridge("the bridge connection closed".into()));
        }
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().unwrap().insert(id, tx);
        if self.inner.out.send(Message::request(id, method, params).encode()).is_err() {
            self.inner.pending.lock().unwrap().remove(&id);
            return Err(Error::Bridge("the bridge connection closed".into()));
        }
        match tokio::time::timeout(self.inner.timeout, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(Error::Rpc(e)),
            Ok(Err(_)) => Err(Error::Bridge("the bridge connection closed".into())),
            Err(_) => {
                self.inner.pending.lock().unwrap().remove(&id);
                Err(Error::Bridge(format!("`{method}` got no answer within {} ms", self.inner.timeout.as_millis())))
            }
        }
    }

    pub async fn request<T: DeserializeOwned>(&self, method: &str, params: impl Serialize) -> Result<T> {
        let v = self.request_value(method, params).await?;
        serde_json::from_value(v).map_err(|e| Error::Bridge(format!("`{method}` answered something unexpected: {e}")))
    }

    pub async fn windows(&self) -> Result<Vec<WindowDesc>> {
        self.request(methods::WINDOWS, Value::Null).await
    }

    pub async fn elements(&self, window: u64) -> Result<Vec<ElementDesc>> {
        self.request(methods::ELEMENTS, ElementsParams::new(window)).await
    }

    /// The window's own picture of its last frame, and that frame's number.
    pub async fn capture(&self, window: u64) -> Result<(RgbaImage, u64)> {
        let r: CaptureResult = self.request(methods::CAPTURE, CaptureParams::new(window)).await?;
        let png = r.png_bytes()?;
        let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png)?.to_rgba8();
        Ok((img, r.frame_seq))
    }

    pub async fn input(&self, window: u64, event: InputEvent) -> Result<()> {
        self.request_value(methods::INPUT, InputParams::new(window, event)).await.map(drop)
    }

    /// One of the app's own commands.
    pub async fn call(&self, name: &str, args: Value) -> Result<Value> {
        self.request_value(methods::CALL, CallParams::new(name, args)).await
    }

    pub async fn clock(&self, mode: ClockMode) -> Result<()> {
        self.request_value(methods::CLOCK, ClockParams::new(mode)).await.map(drop)
    }

    pub async fn clock_step(&self, ms: f64, window: Option<u64>) -> Result<ClockStepResult> {
        let mut p = ClockStepParams::new(ms);
        p.window = window;
        self.request(methods::CLOCK_STEP, p).await
    }

    /// `frame` notifications from now on.
    pub fn frames(&self) -> broadcast::Receiver<FrameEvent> {
        self.inner.frames.subscribe()
    }

    /// Other notifications from now on.
    pub fn notifications(&self) -> broadcast::Receiver<(String, Value)> {
        self.inner.notes.subscribe()
    }
}
