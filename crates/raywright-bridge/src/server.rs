use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use raywright_protocol::{ADDR_ENV, ANNOUNCE_PREFIX, BridgeAddr, FrameEvent, Hello, Message, RpcError, methods};
use serde::Serialize;
use serde_json::Value;

/// A request from Raywright.
#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub params: Value,
}

impl Request {
    /// The params as `T`; on mismatch, answer with `reply.err(e)`.
    pub fn params<T: for<'de> serde::Deserialize<'de>>(&self) -> Result<T, RpcError> {
        raywright_protocol::params(&self.params)
    }
}

/// Answers one request, from any thread, exactly once. Dropping it unanswered answers
/// with an internal error, so the client never waits forever.
pub struct Responder {
    id: u64,
    out: Sender<String>,
    answered: bool,
}

impl Responder {
    pub fn ok(mut self, result: impl Serialize) {
        self.answered = true;
        let _ = self.out.send(Message::ok(self.id, result).encode());
    }

    pub fn err(mut self, e: RpcError) {
        self.answered = true;
        let _ = self.out.send(Message::err(self.id, e).encode());
    }

    pub fn reply(self, r: Result<Value, RpcError>) {
        match r {
            Ok(v) => self.ok(v),
            Err(e) => self.err(e),
        }
    }

    /// "I don't do that": the client then does without this feature.
    pub fn not_found(self, method: &str) {
        self.err(RpcError::method_not_found(method));
    }
}

impl std::fmt::Debug for Responder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Responder").field("id", &self.id).field("answered", &self.answered).finish()
    }
}

impl Drop for Responder {
    fn drop(&mut self) {
        if !self.answered {
            let _ = self.out.send(Message::err(self.id, RpcError::new(raywright_protocol::codes::INTERNAL_ERROR, "the app dropped the request")).encode());
        }
    }
}

/// What the app does with requests (everything but `hello`, which the bridge answers).
pub trait Handler: Send + Sync + 'static {
    fn handle(&self, req: Request, reply: Responder);
}

impl<F: Fn(Request, Responder) + Send + Sync + 'static> Handler for F {
    fn handle(&self, req: Request, reply: Responder) {
        self(req, reply)
    }
}

struct Conn {
    out: Sender<String>,
    alive: Arc<AtomicBool>,
}

/// Pushes notifications (new frames) to every connected client. Cheap to clone.
#[derive(Clone, Default)]
pub struct Notifier {
    conns: Arc<Mutex<Vec<Conn>>>,
}

impl Notifier {
    /// `window` presented frame `seq`.
    pub fn frame(&self, window: u64, seq: u64) {
        self.notify(methods::FRAME, FrameEvent::new(window, seq));
    }

    pub fn notify(&self, method: &str, params: impl Serialize) {
        let line = Message::notification(method, params).encode();
        let mut conns = self.conns.lock().unwrap();
        conns.retain(|c| c.alive.load(Ordering::Relaxed) && c.out.send(line.clone()).is_ok());
    }

    /// Clients connected now.
    pub fn connections(&self) -> usize {
        let mut conns = self.conns.lock().unwrap();
        conns.retain(|c| c.alive.load(Ordering::Relaxed));
        conns.len()
    }
}

/// A running bridge. It keeps serving until the process ends or [`BridgeHandle::stop`].
pub struct BridgeHandle {
    addr: BridgeAddr,
    notifier: Notifier,
    stopped: Arc<AtomicBool>,
}

impl BridgeHandle {
    /// Where it listens (with the real port for `tcp://...:0`).
    pub fn addr(&self) -> &BridgeAddr {
        &self.addr
    }

    pub fn notifier(&self) -> Notifier {
        self.notifier.clone()
    }

    /// Stops accepting new clients.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

pub struct Bridge;

impl Bridge {
    pub fn builder(app: impl Into<String>, version: impl Into<String>) -> BridgeBuilder {
        BridgeBuilder { hello: Hello::new(app, version), addr: None, announce: true }
    }

    /// The address Raywright asked for (`RAYWRIGHT_BRIDGE`), if it launched this process.
    pub fn requested_addr() -> Option<BridgeAddr> {
        std::env::var(ADDR_ENV).ok().and_then(|s| BridgeAddr::from_str(&s).ok())
    }
}

pub struct BridgeBuilder {
    hello: Hello,
    addr: Option<BridgeAddr>,
    announce: bool,
}

pub(crate) type Halves = (Box<dyn Read + Send>, Box<dyn Write + Send>);

pub(crate) trait Listener: Send {
    fn accept(&mut self) -> io::Result<Halves>;
}

struct Tcp(TcpListener);

impl Listener for Tcp {
    fn accept(&mut self) -> io::Result<Halves> {
        let (s, _) = self.0.accept()?;
        s.set_nodelay(true)?;
        Ok((Box::new(s.try_clone()?), Box::new(s)))
    }
}

impl BridgeBuilder {
    pub fn capability(mut self, c: &str) -> Self {
        self.hello.capabilities.push(c.to_string());
        self
    }

    pub fn capabilities<'a>(mut self, cs: impl IntoIterator<Item = &'a str>) -> Self {
        self.hello.capabilities.extend(cs.into_iter().map(str::to_string));
        self
    }

    /// Names accepted by `call` (shown to clients; `call` is announced as a capability).
    pub fn commands<S: Into<String>>(mut self, names: impl IntoIterator<Item = S>) -> Self {
        self.hello.commands.extend(names.into_iter().map(Into::into));
        if !self.hello.has(raywright_protocol::capability::CALL) {
            self.hello.capabilities.push(raywright_protocol::capability::CALL.into());
        }
        self
    }

    /// Listen here instead of `RAYWRIGHT_BRIDGE` or the platform default.
    pub fn addr(mut self, addr: BridgeAddr) -> Self {
        self.addr = Some(addr);
        self
    }

    /// Print `raywright-bridge: listening <addr>` on stderr (on by default).
    pub fn announce(mut self, on: bool) -> Self {
        self.announce = on;
        self
    }

    pub fn start(self, handler: impl Handler) -> io::Result<BridgeHandle> {
        let addr = self.addr.or_else(Bridge::requested_addr).unwrap_or_else(|| BridgeAddr::platform_default(&format!("raywright-{}", std::process::id())));
        let (listener, actual): (Box<dyn Listener>, BridgeAddr) = match &addr {
            BridgeAddr::Tcp(a) => {
                let l = TcpListener::bind(a.as_str())?;
                let real = l.local_addr()?;
                (Box::new(Tcp(l)), BridgeAddr::Tcp(real.to_string()))
            }
            #[cfg(windows)]
            BridgeAddr::Pipe(name) => (Box::new(crate::pipe::PipeListener::new(name)?), addr.clone()),
            other => return Err(io::Error::new(io::ErrorKind::Unsupported, format!("this build can't listen on {other}"))),
        };
        let notifier = Notifier::default();
        let stopped = Arc::new(AtomicBool::new(false));
        let handler: Arc<dyn Handler> = Arc::new(handler);
        let hello = Arc::new(self.hello);
        {
            let (notifier, stopped) = (notifier.clone(), stopped.clone());
            thread::Builder::new().name("raywright-bridge".into()).spawn(move || accept_loop(listener, handler, hello, notifier, stopped))?;
        }
        if self.announce {
            eprintln!("{ANNOUNCE_PREFIX}{actual}");
        }
        Ok(BridgeHandle { addr: actual, notifier, stopped })
    }
}

fn accept_loop(mut listener: Box<dyn Listener>, handler: Arc<dyn Handler>, hello: Arc<Hello>, notifier: Notifier, stopped: Arc<AtomicBool>) {
    while !stopped.load(Ordering::SeqCst) {
        let (read, write) = match listener.accept() {
            Ok(h) => h,
            Err(e) => {
                eprintln!("raywright-bridge: accept failed: {e}");
                thread::sleep(std::time::Duration::from_millis(200));
                continue;
            }
        };
        if stopped.load(Ordering::SeqCst) {
            break;
        }
        let (out, out_rx) = mpsc::channel::<String>();
        let alive = Arc::new(AtomicBool::new(true));
        notifier.conns.lock().unwrap().push(Conn { out: out.clone(), alive: alive.clone() });
        let _ = thread::Builder::new().name("raywright-bridge-write".into()).spawn(move || {
            let mut w = io::BufWriter::new(write);
            for line in out_rx {
                if w.write_all(line.as_bytes()).and_then(|_| w.write_all(b"\n")).and_then(|_| w.flush()).is_err() {
                    break;
                }
            }
        });
        let (handler, hello) = (handler.clone(), hello.clone());
        let _ = thread::Builder::new().name("raywright-bridge-read".into()).spawn(move || {
            serve(read, out, handler.as_ref(), &hello);
            alive.store(false, Ordering::Relaxed);
        });
    }
}

fn serve(read: Box<dyn Read + Send>, out: Sender<String>, handler: &dyn Handler, hello: &Hello) {
    for line in BufReader::new(read).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        match Message::decode(&line) {
            Ok(Message::Request { id, method, params }) => {
                let reply = Responder { id, out: out.clone(), answered: false };
                if method == methods::HELLO {
                    reply.ok(hello);
                } else {
                    handler.handle(Request { method, params }, reply);
                }
            }
            Ok(_) => {}
            Err(e) => {
                let _ = out.send(Message::err(0, RpcError::new(raywright_protocol::codes::PARSE_ERROR, e.to_string())).encode());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raywright_protocol::{ElementDesc, Rect};
    use std::io::{BufRead, Write};
    use std::net::TcpStream;

    fn call(s: &mut TcpStream, r: &mut impl BufRead, id: u64, method: &str, params: Value) -> Message {
        writeln!(s, "{}", Message::request(id, method, params).encode()).unwrap();
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        Message::decode(&line).unwrap()
    }

    #[test]
    fn serves_hello_requests_and_notifications_over_tcp() {
        let h = Bridge::builder("t", "1.0")
            .capabilities([raywright_protocol::capability::ELEMENTS, raywright_protocol::capability::FRAME_EVENTS])
            .addr(BridgeAddr::Tcp("127.0.0.1:0".into()))
            .announce(false)
            .start(|req: Request, reply: Responder| match req.method.as_str() {
                "elements" => reply.ok(vec![ElementDesc::new("k", Rect::new(0.0, 0.0, 1.0, 1.0))]),
                "drop" => drop(reply),
                _ => reply.not_found(&req.method),
            })
            .unwrap();
        let BridgeAddr::Tcp(a) = h.addr() else { panic!() };
        assert!(!a.ends_with(":0"));
        let mut s = TcpStream::connect(a.as_str()).unwrap();
        let mut r = BufReader::new(s.try_clone().unwrap());

        let Message::Response { result: Ok(v), .. } = call(&mut s, &mut r, 1, "hello", Value::Null) else { panic!() };
        let hello: Hello = serde_json::from_value(v).unwrap();
        assert_eq!(hello.app, "t");
        assert!(hello.has("frame-events"));

        let Message::Response { id: 2, result: Ok(v) } = call(&mut s, &mut r, 2, "elements", Value::Null) else { panic!() };
        assert_eq!(v[0]["key"], "k");

        let Message::Response { result: Err(e), .. } = call(&mut s, &mut r, 3, "nope", Value::Null) else { panic!() };
        assert_eq!(e.code, raywright_protocol::codes::METHOD_NOT_FOUND);

        let Message::Response { id: 4, result: Err(e) } = call(&mut s, &mut r, 4, "drop", Value::Null) else { panic!() };
        assert!(e.message.contains("dropped"));

        // wait for the connection to be registered, then notify
        for _ in 0..100 {
            if h.notifier().connections() == 1 {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }
        h.notifier().frame(42, 7);
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        assert_eq!(Message::decode(&line).unwrap(), Message::notification("frame", FrameEvent::new(42, 7)));
    }
}
