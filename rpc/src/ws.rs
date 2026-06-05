use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub struct WsHub {
    tx: broadcast::Sender<String>,
}

impl WsHub {
    pub fn new() -> Arc<Self> {
        let (tx, _) = broadcast::channel(1024);
        Arc::new(WsHub { tx })
    }

    pub fn publish(&self, json: String) {
        let _ = self.tx.send(json);
    }

    pub async fn serve(self: Arc<Self>, listen_addr: String) -> std::io::Result<()> {
        let listener = TcpListener::bind(&listen_addr).await?;
        info!(addr = %listen_addr, "WebSocket subscription server listening");
        loop {
            let (stream, peer) = listener.accept().await?;
            let hub = Arc::clone(&self);
            tokio::spawn(async move {
                if let Err(e) = hub.handle(stream).await {
                    debug!(%peer, error = %e, "ws connection closed");
                }
            });
        }
    }

    async fn handle(self: Arc<Self>, mut stream: tokio::net::TcpStream) -> std::io::Result<()> {
        let mut buf = Vec::with_capacity(2048);
        let mut tmp = [0u8; 1024];
        loop {
            let n = stream.read(&mut tmp).await?;
            if n == 0 {
                return Ok(());
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
            if buf.len() > 16 * 1024 {
                return Ok(());
            }
        }

        let headers = String::from_utf8_lossy(&buf);
        let Some(key) = extract_ws_key(&headers) else {
            let _ = stream
                .write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n")
                .await;
            return Ok(());
        };

        let accept = compute_accept(&key);
        let response = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
        );
        stream.write_all(response.as_bytes()).await?;

        write_text_frame(&mut stream, r#"{"type":"subscribed"}"#).await?;

        let mut rx = self.tx.subscribe();
        loop {
            match rx.recv().await {
                Ok(json) => {
                    if write_text_frame(&mut stream, &json).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!(skipped, "ws subscriber lagged; some notifications dropped");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
        Ok(())
    }
}

impl Default for WsHub {
    fn default() -> Self {
        let (tx, _) = broadcast::channel(1024);
        WsHub { tx }
    }
}

fn extract_ws_key(headers: &str) -> Option<String> {
    for line in headers.lines() {
        if line.to_ascii_lowercase().starts_with("sec-websocket-key:") {
            let idx = line.find(':')?;
            return Some(line[idx + 1..].trim().to_string());
        }
    }
    None
}

fn compute_accept(key: &str) -> String {
    use sha1_smol::Sha1;
    let mut hasher = Sha1::new();
    hasher.update(key.as_bytes());
    hasher.update(WS_GUID.as_bytes());
    base64_encode(&hasher.digest().bytes())
}

async fn write_text_frame(stream: &mut tokio::net::TcpStream, text: &str) -> std::io::Result<()> {
    let payload = text.as_bytes();
    let mut frame = Vec::with_capacity(payload.len() + 10);
    frame.push(0x81);
    let len = payload.len();
    if len < 126 {
        frame.push(len as u8);
    } else if len < 65536 {
        frame.push(126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }
    frame.extend_from_slice(payload);
    stream.write_all(&frame).await
}

fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
    }

    #[test]
    fn ws_accept_rfc_example() {
        let accept = compute_accept("dGhlIHNhbXBsZSBub25jZQ==");
        assert_eq!(accept, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn extracts_key_case_insensitive() {
        let h = "GET / HTTP/1.1\r\nSec-WebSocket-Key: abc123==\r\n\r\n";
        assert_eq!(extract_ws_key(h), Some("abc123==".to_string()));
    }
}
