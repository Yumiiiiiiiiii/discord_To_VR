use serde_json::Value;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum IpcError {
    Disconnected,
    InvalidFrame,
}

/// Bytes survive receive timeouts; read_exact on a local buffer did not.
#[derive(Default)]
struct FrameBuffer {
    bytes: Vec<u8>,
}

impl FrameBuffer {
    fn next(&mut self) -> Result<Option<(i32, Vec<u8>)>, IpcError> {
        if self.bytes.len() < 8 {
            return Ok(None);
        }
        let opcode = i32::from_le_bytes(self.bytes[..4].try_into().unwrap());
        let length = u32::from_le_bytes(self.bytes[4..8].try_into().unwrap()) as usize;
        if length > MAX_FRAME_SIZE || !(0..=4).contains(&opcode) {
            return Err(IpcError::InvalidFrame);
        }
        if self.bytes.len() < 8 + length {
            return Ok(None);
        }
        let payload = self.bytes[8..8 + length].to_vec();
        self.bytes.drain(..8 + length);
        Ok(Some((opcode, payload)))
    }
}

pub struct DiscordIPC {
    stream: Option<NamedPipeClient>,
    frames: FrameBuffer,
}

impl Default for DiscordIPC {
    fn default() -> Self {
        Self::new()
    }
}

impl DiscordIPC {
    pub fn new() -> Self {
        Self {
            stream: None,
            frames: FrameBuffer::default(),
        }
    }

    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    #[cfg(test)]
    pub(super) fn from_stream(stream: NamedPipeClient) -> Self {
        Self {
            stream: Some(stream),
            frames: FrameBuffer::default(),
        }
    }

    pub fn close(&mut self) {
        self.stream = None;
        self.frames.bytes.clear();
    }

    pub async fn connect(&mut self) -> bool {
        self.close();
        for i in 0..10 {
            let pipe_path = format!(r"\\.\pipe\discord-ipc-{}", i);
            if let Ok(client) = ClientOptions::new().open(&pipe_path) {
                println!("✅ [Discord IPC] {} に接続しました！", pipe_path);
                self.stream = Some(client);
                return true;
            }
        }
        false
    }

    pub async fn send(&mut self, opcode: i32, payload: &Value) -> bool {
        match serde_json::to_vec(payload) {
            Ok(bytes) => self.send_bytes(opcode, &bytes).await,
            Err(_) => false,
        }
    }

    async fn send_bytes(&mut self, opcode: i32, bytes: &[u8]) -> bool {
        if bytes.len() > MAX_FRAME_SIZE {
            return false;
        }
        let Some(stream) = self.stream.as_mut() else {
            return false;
        };
        let mut buf = Vec::with_capacity(8 + bytes.len());
        buf.extend_from_slice(&opcode.to_le_bytes());
        buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(bytes);
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            stream.write_all(&buf).await?;
            stream.flush().await
        })
        .await;
        if matches!(result, Ok(Ok(()))) {
            true
        } else {
            // A timed-out write can be partial; never reuse that stream.
            self.close();
            false
        }
    }

    async fn recv_once(&mut self) -> Result<(i32, Vec<u8>), IpcError> {
        loop {
            if let Some(frame) = self.frames.next()? {
                return Ok(frame);
            }
            let stream = self.stream.as_mut().ok_or(IpcError::Disconnected)?;
            let mut chunk = [0u8; 8192];
            // AsyncReadExt::read is cancellation-safe. Store every consumed byte
            // before the next await, so a timeout never drops half a frame.
            let count = stream
                .read(&mut chunk)
                .await
                .map_err(|_| IpcError::Disconnected)?;
            if count == 0 {
                return Err(IpcError::Disconnected);
            }
            self.frames.bytes.extend_from_slice(&chunk[..count]);
        }
    }

    pub async fn recv_timeout(
        &mut self,
        duration: Duration,
    ) -> Result<Option<(i32, Value)>, IpcError> {
        if self.stream.is_none() {
            return Err(IpcError::Disconnected);
        }
        let deadline = tokio::time::Instant::now() + duration;
        loop {
            let received = tokio::time::timeout_at(deadline, self.recv_once()).await;
            let (opcode, bytes) = match received {
                Ok(Ok(frame)) => frame,
                Ok(Err(error)) => {
                    self.close();
                    return Err(error);
                }
                Err(_) => return Ok(None),
            };
            match opcode {
                2 => {
                    self.close();
                    return Err(IpcError::Disconnected);
                }
                3 => {
                    // Echo the exact ping payload, including non-JSON payloads.
                    if !self.send_bytes(4, &bytes).await {
                        return Err(IpcError::Disconnected);
                    }
                }
                4 => {}
                _ => match serde_json::from_slice(&bytes) {
                    Ok(value) => return Ok(Some((opcode, value))),
                    Err(_) => {
                        self.close();
                        return Err(IpcError::InvalidFrame);
                    }
                },
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut bytes = 1i32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn fragmented_header_and_payload_survive_multiple_reads() {
        let bytes = frame(br#"{"body":"hello"}"#);
        let mut buffer = FrameBuffer::default();
        for byte in &bytes[..bytes.len() - 1] {
            buffer.bytes.push(*byte);
            assert!(buffer.next().unwrap().is_none());
        }
        buffer.bytes.push(*bytes.last().unwrap());
        assert_eq!(buffer.next().unwrap().unwrap().1, br#"{"body":"hello"}"#);
        assert!(buffer.bytes.is_empty());
    }

    #[test]
    fn consecutive_frames_are_kept_separate() {
        let mut buffer = FrameBuffer {
            bytes: [frame(b"first"), frame(b"second")].concat(),
        };
        assert_eq!(buffer.next().unwrap().unwrap().1, b"first");
        assert_eq!(buffer.next().unwrap().unwrap().1, b"second");
        assert!(buffer.next().unwrap().is_none());
    }

    #[test]
    fn excessive_length_is_rejected_before_allocation() {
        let mut buffer = FrameBuffer {
            bytes: [1i32.to_le_bytes(), u32::MAX.to_le_bytes()].concat(),
        };
        assert!(matches!(buffer.next(), Err(IpcError::InvalidFrame)));
    }

    #[tokio::test]
    async fn timeout_preserves_partially_consumed_pipe_frame() {
        use tokio::net::windows::named_pipe::ServerOptions;
        let name = format!(r"\\.\pipe\discord-vr-test-{}", uuid::Uuid::new_v4());
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)
            .unwrap();
        let client = ClientOptions::new().open(&name).unwrap();
        server.connect().await.unwrap();
        let mut ipc = DiscordIPC::new();
        ipc.stream = Some(client);
        let bytes = frame(br#"{"body":"hello"}"#);
        server.write_all(&bytes[..5]).await.unwrap();
        assert!(ipc
            .recv_timeout(Duration::from_millis(30))
            .await
            .unwrap()
            .is_none());
        server.write_all(&bytes[5..]).await.unwrap();
        let (_, value) = ipc
            .recv_timeout(Duration::from_secs(1))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(value["body"], "hello");
    }

    #[tokio::test]
    async fn peer_ping_is_echoed_and_close_disconnects() {
        use tokio::net::windows::named_pipe::ServerOptions;
        let name = format!(r"\\.\pipe\discord-vr-test-{}", uuid::Uuid::new_v4());
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)
            .unwrap();
        let client = ClientOptions::new().open(&name).unwrap();
        server.connect().await.unwrap();
        let mut ipc = DiscordIPC::new();
        ipc.stream = Some(client);
        let mut ping = frame(b"non-json-ping");
        ping[..4].copy_from_slice(&3i32.to_le_bytes());
        server.write_all(&ping).await.unwrap();
        assert!(ipc
            .recv_timeout(Duration::from_millis(30))
            .await
            .unwrap()
            .is_none());
        let mut pong = vec![0; ping.len()];
        tokio::time::timeout(Duration::from_secs(1), server.read_exact(&mut pong))
            .await
            .unwrap()
            .unwrap();
        ping[..4].copy_from_slice(&4i32.to_le_bytes());
        assert_eq!(pong, ping);
        server
            .write_all(&[2i32.to_le_bytes(), 0i32.to_le_bytes()].concat())
            .await
            .unwrap();
        assert!(matches!(
            ipc.recv_timeout(Duration::from_secs(1)).await,
            Err(IpcError::Disconnected)
        ));
        assert!(!ipc.is_connected());
    }
}
