//! The client side of the automation bridge (see `raywright-protocol`).

mod capture;
mod client;
mod source;
mod transport;

pub use capture::BridgeCapture;
pub use client::BridgeClient;
pub use source::BridgeSource;
pub use transport::{AsyncStream, BoxStream, TcpTransport, Transport};

/// Where an app's bridge connection stands.
#[derive(Clone, Debug)]
pub enum BridgeState {
    /// The app was not asked to start one.
    Disabled,
    /// Waiting for the app to listen.
    Pending,
    Connected(BridgeClient),
    /// Gave up; why.
    Failed(String),
}

impl BridgeState {
    pub fn client(&self) -> Option<&BridgeClient> {
        match self {
            BridgeState::Connected(c) => Some(c),
            _ => None,
        }
    }
}
