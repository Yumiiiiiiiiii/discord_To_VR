use serde_json::Value;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

#[derive(Debug)]
pub enum IpcError {
    Disconnected,
    InvalidFrame,
}

/// Discord ローカル IPC (名前付きパイプ) と通信を行うクライアント
pub struct DiscordIPC {
    stream: Option<NamedPipeClient>,
}

impl DiscordIPC {
    pub fn new() -> Self {
        Self { stream: None }
    }

    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    pub fn close(&mut self) {
        self.stream = None;
    }

    /// Windows の名前付きパイプ (discord-ipc-0 〜 9) を順に探して接続を試行する
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

    /// Opcode + 長さ + JSON ペイロードをバイナリ形式で組み立ててパイプに送信する
    pub async fn send(&mut self, opcode: i32, payload: &Value) -> bool {
        if let Some(stream) = self.stream.as_mut() {
            let json_bytes = match serde_json::to_vec(payload) {
                Ok(b) => b,
                Err(_) => return false,
            };
            let mut buf = Vec::with_capacity(8 + json_bytes.len());
            buf.extend_from_slice(&opcode.to_le_bytes());
            buf.extend_from_slice(&(json_bytes.len() as i32).to_le_bytes());
            buf.extend_from_slice(&json_bytes);

            if stream.write_all(&buf).await.is_ok() && stream.flush().await.is_ok() {
                return true;
            }
            self.close();
        }
        false
    }

    /// パイプから 8 バイトのヘッダーを読み取り、続いて指定長の JSON データを1フレーム分受信する
    async fn recv_once(&mut self) -> Result<(i32, Value), IpcError> {
        let stream = self.stream.as_mut().ok_or(IpcError::Disconnected)?;
        let mut header = [0u8; 8];
        stream
            .read_exact(&mut header)
            .await
            .map_err(|_| IpcError::Disconnected)?;
        let opcode = i32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let length = i32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;

        if length > 16 * 1024 * 1024 {
            return Err(IpcError::InvalidFrame);
        }

        let mut payload = vec![0u8; length];
        stream
            .read_exact(&mut payload)
            .await
            .map_err(|_| IpcError::Disconnected)?;

        let json_val = serde_json::from_slice(&payload).map_err(|_| IpcError::InvalidFrame)?;
        Ok((opcode, json_val))
    }

    /// タイムアウト時間つきで1フレーム分の受信を試行する
    pub async fn recv_timeout(&mut self, duration: Duration) -> Result<Option<(i32, Value)>, IpcError> {
        if self.stream.is_none() {
            return Err(IpcError::Disconnected);
        }
        match tokio::time::timeout(duration, self.recv_once()).await {
            Ok(Ok(res)) => Ok(Some(res)),
            Ok(Err(e)) => {
                self.close();
                Err(e)
            }
            Err(_) => Ok(None),
        }
    }
}
