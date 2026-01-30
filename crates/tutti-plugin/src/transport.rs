//! IPC transport layer
//!
//! Handles message passing between host and bridge processes via Unix sockets
//! or Windows named pipes.

use crate::error::Result;
use crate::protocol::{BridgeMessage, HostMessage};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

/// Message transport for IPC
pub enum MessageTransport {
    #[cfg(unix)]
    Unix(UnixStream),
    #[cfg(windows)]
    WindowsClient(tokio::net::windows::named_pipe::NamedPipeClient),
    #[cfg(windows)]
    WindowsServer(NamedPipeServer),
}

impl MessageTransport {
    /// Create transport from existing Unix stream
    #[cfg(unix)]
    pub fn new(stream: UnixStream) -> Self {
        Self::Unix(stream)
    }

    /// Create transport from existing Windows client pipe
    #[cfg(windows)]
    pub fn from_client(pipe: tokio::net::windows::named_pipe::NamedPipeClient) -> Self {
        Self::WindowsClient(pipe)
    }

    /// Create transport from existing Windows server pipe
    #[cfg(windows)]
    pub fn from_server(pipe: NamedPipeServer) -> Self {
        Self::WindowsServer(pipe)
    }

    /// Connect to socket path (Unix) or named pipe (Windows)
    #[cfg(unix)]
    pub async fn connect(socket_path: &std::path::Path) -> Result<Self> {
        let stream = UnixStream::connect(socket_path).await?;
        Ok(Self::Unix(stream))
    }

    /// Connect to named pipe (Windows client)
    #[cfg(windows)]
    pub async fn connect(pipe_name: &std::path::Path) -> Result<Self> {
        let client = ClientOptions::new().open(pipe_name)?;
        Ok(Self::WindowsClient(client))
    }

    /// Get mutable reference to the underlying I/O stream
    #[cfg(unix)]
    fn stream_mut(&mut self) -> &mut UnixStream {
        match self {
            Self::Unix(s) => s,
        }
    }

    /// Get mutable reference to the underlying I/O stream (Windows)
    #[cfg(windows)]
    fn stream_mut(&mut self) -> &mut dyn tokio::io::AsyncWrite {
        match self {
            Self::WindowsClient(c) => c,
            Self::WindowsServer(s) => s,
        }
    }

    /// Get mutable reference for reading
    #[cfg(unix)]
    fn read_stream_mut(&mut self) -> &mut UnixStream {
        match self {
            Self::Unix(s) => s,
        }
    }

    /// Get mutable reference for reading (Windows)
    #[cfg(windows)]
    fn read_stream_mut(&mut self) -> &mut dyn tokio::io::AsyncRead {
        match self {
            Self::WindowsClient(c) => c,
            Self::WindowsServer(s) => s,
        }
    }

    /// Send a host message
    pub async fn send_host_message(&mut self, msg: &HostMessage) -> Result<()> {
        let data = bincode::serialize(msg)?;
        let len = data.len() as u32;

        #[cfg(unix)]
        {
            let stream = self.stream_mut();
            stream.write_u32(len).await?;
            stream.write_all(&data).await?;
        }

        #[cfg(windows)]
        {
            use tokio::io::AsyncWriteExt;
            match self {
                Self::WindowsClient(c) => {
                    c.write_u32(len).await?;
                    c.write_all(&data).await?;
                }
                Self::WindowsServer(s) => {
                    s.write_u32(len).await?;
                    s.write_all(&data).await?;
                }
            }
        }

        Ok(())
    }

    /// Receive a bridge message
    pub async fn recv_bridge_message(&mut self) -> Result<BridgeMessage> {
        #[cfg(unix)]
        {
            let stream = self.read_stream_mut();
            let len = stream.read_u32().await? as usize;
            let mut data = vec![0u8; len];
            stream.read_exact(&mut data).await?;
            let msg = bincode::deserialize(&data)?;
            Ok(msg)
        }

        #[cfg(windows)]
        {
            use tokio::io::AsyncReadExt;
            let (len, data) = match self {
                Self::WindowsClient(c) => {
                    let len = c.read_u32().await? as usize;
                    let mut data = vec![0u8; len];
                    c.read_exact(&mut data).await?;
                    (len, data)
                }
                Self::WindowsServer(s) => {
                    let len = s.read_u32().await? as usize;
                    let mut data = vec![0u8; len];
                    s.read_exact(&mut data).await?;
                    (len, data)
                }
            };
            let msg = bincode::deserialize(&data)?;
            Ok(msg)
        }
    }

    /// Send a bridge message
    pub async fn send_bridge_message(&mut self, msg: &BridgeMessage) -> Result<()> {
        let data = bincode::serialize(msg)?;
        let len = data.len() as u32;

        #[cfg(unix)]
        {
            let stream = self.stream_mut();
            stream.write_u32(len).await?;
            stream.write_all(&data).await?;
        }

        #[cfg(windows)]
        {
            use tokio::io::AsyncWriteExt;
            match self {
                Self::WindowsClient(c) => {
                    c.write_u32(len).await?;
                    c.write_all(&data).await?;
                }
                Self::WindowsServer(s) => {
                    s.write_u32(len).await?;
                    s.write_all(&data).await?;
                }
            }
        }

        Ok(())
    }

    /// Receive a host message
    pub async fn recv_host_message(&mut self) -> Result<HostMessage> {
        #[cfg(unix)]
        {
            let stream = self.read_stream_mut();
            let len = stream.read_u32().await? as usize;
            let mut data = vec![0u8; len];
            stream.read_exact(&mut data).await?;
            let msg = bincode::deserialize(&data)?;
            Ok(msg)
        }

        #[cfg(windows)]
        {
            use tokio::io::AsyncReadExt;
            let (len, data) = match self {
                Self::WindowsClient(c) => {
                    let len = c.read_u32().await? as usize;
                    let mut data = vec![0u8; len];
                    c.read_exact(&mut data).await?;
                    (len, data)
                }
                Self::WindowsServer(s) => {
                    let len = s.read_u32().await? as usize;
                    let mut data = vec![0u8; len];
                    s.read_exact(&mut data).await?;
                    (len, data)
                }
            };
            let msg = bincode::deserialize(&data)?;
            Ok(msg)
        }
    }
}

/// Server-side transport listener
pub struct TransportListener {
    #[cfg(unix)]
    listener: UnixListener,
    #[cfg(windows)]
    pipe_name: std::path::PathBuf,
}

impl TransportListener {
    /// Bind to socket path (Unix) or prepare named pipe (Windows)
    #[cfg(unix)]
    pub async fn bind(socket_path: &std::path::Path) -> Result<Self> {
        // Remove existing socket if it exists
        let _ = std::fs::remove_file(socket_path);
        let listener = UnixListener::bind(socket_path)?;
        Ok(Self { listener })
    }

    /// Prepare named pipe path (Windows)
    #[cfg(windows)]
    pub async fn bind(pipe_name: &std::path::Path) -> Result<Self> {
        Ok(Self {
            pipe_name: pipe_name.to_path_buf(),
        })
    }

    /// Accept a connection
    #[cfg(unix)]
    pub async fn accept(&self) -> Result<MessageTransport> {
        let (stream, _) = self.listener.accept().await?;
        Ok(MessageTransport::Unix(stream))
    }

    /// Accept a connection (Windows) - creates server pipe and waits for client
    #[cfg(windows)]
    pub async fn accept(&self) -> Result<MessageTransport> {
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&self.pipe_name)?;

        // Wait for client to connect
        server.connect().await?;

        Ok(MessageTransport::WindowsServer(server))
    }
}

/// Client-side transport connector
pub struct TransportConnector;

impl TransportConnector {
    /// Connect to socket path (Unix) or named pipe (Windows)
    pub async fn connect(path: &std::path::Path) -> Result<MessageTransport> {
        MessageTransport::connect(path).await
    }
}
