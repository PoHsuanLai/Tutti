//! IPC transport layer
//!
//! Handles message passing between host and bridge processes via Unix sockets
//! or Windows named pipes.

use crate::error::Result;
use crate::protocol::{BridgeMessage, HostMessage};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

/// Message transport for IPC
pub struct MessageTransport {
    stream: UnixStream,
}

impl MessageTransport {
    /// Create transport from existing stream
    pub fn new(stream: UnixStream) -> Self {
        Self { stream }
    }

    /// Connect to socket path (client-side)
    pub async fn connect(socket_path: &std::path::Path) -> Result<Self> {
        let stream = UnixStream::connect(socket_path).await?;
        Ok(Self::new(stream))
    }

    /// Send a host message
    pub async fn send_host_message(&mut self, msg: &HostMessage) -> Result<()> {
        let data = bincode::serialize(msg)?;

        // Send length prefix (4 bytes)
        let len = data.len() as u32;
        self.stream.write_u32(len).await?;

        // Send data
        self.stream.write_all(&data).await?;

        Ok(())
    }

    /// Receive a bridge message
    pub async fn recv_bridge_message(&mut self) -> Result<BridgeMessage> {
        // Read length prefix
        let len = self.stream.read_u32().await? as usize;

        // Read data
        let mut data = vec![0u8; len];
        self.stream.read_exact(&mut data).await?;

        // Deserialize
        let msg = bincode::deserialize(&data)?;

        Ok(msg)
    }

    /// Send a bridge message
    pub async fn send_bridge_message(&mut self, msg: &BridgeMessage) -> Result<()> {
        let data = bincode::serialize(msg)?;

        let len = data.len() as u32;
        self.stream.write_u32(len).await?;
        self.stream.write_all(&data).await?;

        Ok(())
    }

    /// Receive a host message
    pub async fn recv_host_message(&mut self) -> Result<HostMessage> {
        let len = self.stream.read_u32().await? as usize;
        let mut data = vec![0u8; len];
        self.stream.read_exact(&mut data).await?;

        let msg = bincode::deserialize(&data)?;

        Ok(msg)
    }
}

/// Server-side transport listener
pub struct TransportListener {
    listener: UnixListener,
}

impl TransportListener {
    /// Bind to socket path
    pub async fn bind(socket_path: &std::path::Path) -> Result<Self> {
        // Remove existing socket if it exists
        let _ = std::fs::remove_file(socket_path);

        let listener = UnixListener::bind(socket_path)?;

        Ok(Self { listener })
    }

    /// Accept a connection
    pub async fn accept(&self) -> Result<MessageTransport> {
        let (stream, _) = self.listener.accept().await?;
        Ok(MessageTransport::new(stream))
    }
}

/// Client-side transport connector
pub struct TransportConnector;

impl TransportConnector {
    /// Connect to socket path
    pub async fn connect(socket_path: &std::path::Path) -> Result<MessageTransport> {
        let stream = UnixStream::connect(socket_path).await?;
        Ok(MessageTransport::new(stream))
    }
}
