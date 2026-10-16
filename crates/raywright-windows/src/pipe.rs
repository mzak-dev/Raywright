//! The client end of named-pipe bridges.

use std::io;
use std::time::Duration;

use async_trait::async_trait;
use raywright_core::bridge::{BoxStream, Transport};
use raywright_protocol::BridgeAddr;
use tokio::net::windows::named_pipe::ClientOptions;

const ERROR_PIPE_BUSY: i32 = 231;

pub struct NamedPipeTransport;

#[async_trait]
impl Transport for NamedPipeTransport {
    fn schemes(&self) -> &[&str] {
        &["pipe"]
    }

    async fn connect(&self, addr: &BridgeAddr) -> io::Result<BoxStream> {
        let path = addr.pipe_path().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{addr} is not a pipe address")))?;
        for _ in 0..40 {
            match ClientOptions::new().open(&path) {
                Ok(c) => return Ok(Box::new(c)),
                // every instance is serving another client; the server makes a new one soon
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => tokio::time::sleep(Duration::from_millis(25)).await,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, format!("{path} stayed busy")))
    }
}
