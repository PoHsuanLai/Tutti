//! Client-side IPC transport via Unix sockets or Windows named pipes.

use crate::error::Result;
use crate::protocol::{BridgeMessage, HostMessage};

#[cfg(unix)]
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

#[cfg(windows)]
use tokio::net::windows::named_pipe::ClientOptions;

/// IPC transport to plugin server.
pub struct MessageTransport {
    #[cfg(unix)]
    stream: UnixStream,
    #[cfg(windows)]
    pipe: tokio::net::windows::named_pipe::NamedPipeClient,
}

impl MessageTransport {
    #[cfg(unix)]
    pub async fn connect(socket_path: &std::path::Path) -> Result<Self> {
        let stream = UnixStream::connect(socket_path).await?;
        Ok(Self { stream })
    }

    #[cfg(windows)]
    pub async fn connect(pipe_name: &std::path::Path) -> Result<Self> {
        let pipe = ClientOptions::new().open(pipe_name)?;
        Ok(Self { pipe })
    }

    pub async fn send_host_message(&mut self, msg: &HostMessage) -> Result<()> {
        let data = bincode::serialize(msg)?;
        let len = data.len() as u32;

        #[cfg(unix)]
        {
            self.stream.write_u32(len).await?;
            self.stream.write_all(&data).await?;
        }

        #[cfg(windows)]
        {
            use tokio::io::AsyncWriteExt;
            self.pipe.write_u32(len).await?;
            self.pipe.write_all(&data).await?;
        }

        Ok(())
    }

    pub async fn recv_bridge_message(&mut self) -> Result<BridgeMessage> {
        #[cfg(unix)]
        {
            let len = self.stream.read_u32().await? as usize;
            let mut data = vec![0u8; len];
            self.stream.read_exact(&mut data).await?;
            Ok(bincode::deserialize(&data)?)
        }

        #[cfg(windows)]
        {
            use tokio::io::AsyncReadExt;
            let len = self.pipe.read_u32().await? as usize;
            let mut data = vec![0u8; len];
            self.pipe.read_exact(&mut data).await?;
            Ok(bincode::deserialize(&data)?)
        }
    }
}
