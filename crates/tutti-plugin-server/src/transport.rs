//! IPC transport layer
//!
//! Handles message passing between host and bridge processes via Unix sockets
//! or Windows named pipes.

#![allow(dead_code)] // Used by server module

use tutti_plugin::protocol::{BridgeMessage, HostMessage};
use tutti_plugin::Result;

#[cfg(unix)]
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
};

#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

pub enum MessageTransport {
    #[cfg(unix)]
    Unix(UnixStream),
    #[cfg(windows)]
    WindowsClient(tokio::net::windows::named_pipe::NamedPipeClient),
    #[cfg(windows)]
    WindowsServer(NamedPipeServer),
}

impl MessageTransport {
    #[cfg(unix)]
    pub fn new(stream: UnixStream) -> Self {
        Self::Unix(stream)
    }

    #[cfg(windows)]
    pub fn from_client(pipe: tokio::net::windows::named_pipe::NamedPipeClient) -> Self {
        Self::WindowsClient(pipe)
    }

    #[cfg(windows)]
    pub fn from_server(pipe: NamedPipeServer) -> Self {
        Self::WindowsServer(pipe)
    }

    #[cfg(unix)]
    pub async fn connect(socket_path: &std::path::Path) -> Result<Self> {
        let stream = UnixStream::connect(socket_path).await?;
        Ok(Self::Unix(stream))
    }

    #[cfg(windows)]
    pub async fn connect(pipe_name: &std::path::Path) -> Result<Self> {
        let client = ClientOptions::new().open(pipe_name)?;
        Ok(Self::WindowsClient(client))
    }

    #[cfg(unix)]
    fn stream_mut(&mut self) -> &mut UnixStream {
        match self {
            Self::Unix(s) => s,
        }
    }

    #[cfg(unix)]
    fn read_stream_mut(&mut self) -> &mut UnixStream {
        match self {
            Self::Unix(s) => s,
        }
    }

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
            let data = match self {
                Self::WindowsClient(c) => {
                    let len = c.read_u32().await? as usize;
                    let mut data = vec![0u8; len];
                    c.read_exact(&mut data).await?;
                    data
                }
                Self::WindowsServer(s) => {
                    let len = s.read_u32().await? as usize;
                    let mut data = vec![0u8; len];
                    s.read_exact(&mut data).await?;
                    data
                }
            };
            let msg = bincode::deserialize(&data)?;
            Ok(msg)
        }
    }

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
            let data = match self {
                Self::WindowsClient(c) => {
                    let len = c.read_u32().await? as usize;
                    let mut data = vec![0u8; len];
                    c.read_exact(&mut data).await?;
                    data
                }
                Self::WindowsServer(s) => {
                    let len = s.read_u32().await? as usize;
                    let mut data = vec![0u8; len];
                    s.read_exact(&mut data).await?;
                    data
                }
            };
            let msg = bincode::deserialize(&data)?;
            Ok(msg)
        }
    }
}

pub struct TransportListener {
    #[cfg(unix)]
    listener: UnixListener,
    #[cfg(windows)]
    pipe_name: std::path::PathBuf,
}

impl TransportListener {
    #[cfg(unix)]
    pub async fn bind(socket_path: &std::path::Path) -> Result<Self> {
        let _ = std::fs::remove_file(socket_path);
        let listener = UnixListener::bind(socket_path)?;
        Ok(Self { listener })
    }

    #[cfg(windows)]
    pub async fn bind(pipe_name: &std::path::Path) -> Result<Self> {
        Ok(Self {
            pipe_name: pipe_name.to_path_buf(),
        })
    }

    #[cfg(unix)]
    pub async fn accept(&self) -> Result<MessageTransport> {
        let (stream, _) = self.listener.accept().await?;
        Ok(MessageTransport::Unix(stream))
    }

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

pub struct TransportConnector;

impl TransportConnector {
    pub async fn connect(path: &std::path::Path) -> Result<MessageTransport> {
        MessageTransport::connect(path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallvec::SmallVec;
    use std::path::PathBuf;
    use tutti_plugin::protocol::{
        BridgeMessage, HostMessage, NoteExpressionChanges, ParameterChanges, ParameterFlags,
        ParameterInfo, SampleFormat,
    };

    #[tokio::test]
    async fn test_host_message_roundtrip() {
        let (stream_a, stream_b) = tokio::net::UnixStream::pair().unwrap();
        let mut sender = MessageTransport::new(stream_a);
        let mut receiver = MessageTransport::new(stream_b);

        sender
            .send_host_message(&HostMessage::GetParameterList)
            .await
            .unwrap();

        let msg = receiver.recv_host_message().await.unwrap();
        assert!(
            matches!(msg, HostMessage::GetParameterList),
            "Expected GetParameterList, got {:?}",
            msg
        );
    }

    #[tokio::test]
    async fn test_bridge_message_roundtrip() {
        let (stream_a, stream_b) = tokio::net::UnixStream::pair().unwrap();
        let mut sender = MessageTransport::new(stream_a);
        let mut receiver = MessageTransport::new(stream_b);

        sender
            .send_bridge_message(&BridgeMessage::Ready)
            .await
            .unwrap();

        let msg = receiver.recv_bridge_message().await.unwrap();
        assert!(
            matches!(msg, BridgeMessage::Ready),
            "Expected Ready, got {:?}",
            msg
        );
    }

    #[tokio::test]
    async fn test_host_message_load_plugin_roundtrip() {
        let (stream_a, stream_b) = tokio::net::UnixStream::pair().unwrap();
        let mut sender = MessageTransport::new(stream_a);
        let mut receiver = MessageTransport::new(stream_b);

        let original = HostMessage::LoadPlugin {
            path: PathBuf::from("/tmp/test.clap"),
            sample_rate: 48000.0,
            block_size: 1024,
            preferred_format: SampleFormat::Float64,
            shm_name: "test_shm".to_string(),
        };

        sender.send_host_message(&original).await.unwrap();

        let msg = receiver.recv_host_message().await.unwrap();
        match msg {
            HostMessage::LoadPlugin {
                path,
                sample_rate,
                block_size,
                preferred_format,
                ..
            } => {
                assert_eq!(path, PathBuf::from("/tmp/test.clap"));
                assert_eq!(sample_rate, 48000.0);
                assert_eq!(block_size, 1024);
                assert_eq!(preferred_format, SampleFormat::Float64);
            }
            other => panic!("Expected LoadPlugin, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_bridge_message_parameter_list_roundtrip() {
        let (stream_a, stream_b) = tokio::net::UnixStream::pair().unwrap();
        let mut sender = MessageTransport::new(stream_a);
        let mut receiver = MessageTransport::new(stream_b);

        let original = BridgeMessage::ParameterList {
            parameters: vec![ParameterInfo {
                id: 42,
                name: "Cutoff".into(),
                unit: "Hz".into(),
                min_value: 20.0,
                max_value: 20000.0,
                default_value: 1000.0,
                step_count: 0,
                flags: ParameterFlags {
                    automatable: true,
                    read_only: false,
                    wrap: false,
                    is_bypass: false,
                    hidden: false,
                },
            }],
        };

        sender.send_bridge_message(&original).await.unwrap();

        let msg = receiver.recv_bridge_message().await.unwrap();
        match msg {
            BridgeMessage::ParameterList { parameters } => {
                assert_eq!(parameters.len(), 1);
                let p = &parameters[0];
                assert_eq!(p.id, 42);
                assert_eq!(p.name, "Cutoff");
                assert_eq!(p.unit, "Hz");
                assert_eq!(p.min_value, 20.0);
                assert_eq!(p.max_value, 20000.0);
                assert_eq!(p.default_value, 1000.0);
                assert_eq!(p.step_count, 0);
                assert!(p.flags.automatable);
                assert!(!p.flags.read_only);
                assert!(!p.flags.wrap);
                assert!(!p.flags.is_bypass);
                assert!(!p.flags.hidden);
            }
            other => panic!("Expected ParameterList, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_bridge_message_audio_processed_full_roundtrip() {
        let (stream_a, stream_b) = tokio::net::UnixStream::pair().unwrap();
        let mut sender = MessageTransport::new(stream_a);
        let mut receiver = MessageTransport::new(stream_b);

        let original = BridgeMessage::AudioProcessedFull(Box::new(
            tutti_plugin::protocol::AudioProcessedFullData {
                latency_us: 123,
                midi_output: SmallVec::new(),
                param_output: ParameterChanges::new(),
                note_expression_output: NoteExpressionChanges::new(),
            },
        ));

        sender.send_bridge_message(&original).await.unwrap();

        let msg = receiver.recv_bridge_message().await.unwrap();
        match msg {
            BridgeMessage::AudioProcessedFull(data) => {
                assert_eq!(data.latency_us, 123);
                assert!(data.midi_output.is_empty());
                assert!(data.param_output.is_empty());
                assert!(data.note_expression_output.is_empty());
            }
            other => panic!("Expected AudioProcessedFull, got {:?}", other),
        }
    }
}
