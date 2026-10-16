use std::io;

use async_trait::async_trait;
use raywright_protocol::BridgeAddr;
use tokio::io::{AsyncRead, AsyncWrite};

/// A duplex byte stream to a bridge.
pub trait AsyncStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> AsyncStream for T {}

pub type BoxStream = Box<dyn AsyncStream>;

/// Opens connections for some address schemes (`pipe`, `tcp`, `unix`, `memory`).
#[async_trait]
pub trait Transport: Send + Sync {
    fn schemes(&self) -> &[&str];
    async fn connect(&self, addr: &BridgeAddr) -> io::Result<BoxStream>;
}

/// Loopback TCP: works on every platform.
pub struct TcpTransport;

#[async_trait]
impl Transport for TcpTransport {
    fn schemes(&self) -> &[&str] {
        &["tcp"]
    }

    async fn connect(&self, addr: &BridgeAddr) -> io::Result<BoxStream> {
        match addr {
            BridgeAddr::Tcp(a) => {
                let s = tokio::net::TcpStream::connect(a.as_str()).await?;
                s.set_nodelay(true)?;
                Ok(Box::new(s))
            }
            other => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("tcp transport can't open {other}"))),
        }
    }
}
