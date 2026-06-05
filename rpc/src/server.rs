use std::future::Future;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{debug, info, warn};

use crate::types::{codes, RpcRequest, RpcResponse};

/// A minimal HTTP/1.1 JSON-RPC server with no heavyweight web framework.
/// It reads a single request, dispatches it to the handler, and writes one
/// JSON response. This keeps the dependency surface tiny, in line with the
/// Photon philosophy.
pub struct RpcServer {
    listen_addr: String,
}

impl RpcServer {
    pub fn new(listen_addr: impl Into<String>) -> Self {
        RpcServer {
            listen_addr: listen_addr.into(),
        }
    }

    /// Serve until the process ends. `handler` maps an [`RpcRequest`] to an
    /// [`RpcResponse`]; it is shared across connections via `Arc`.
    pub async fn serve<H, Fut>(self, handler: H) -> std::io::Result<()>
    where
        H: Fn(RpcRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = RpcResponse> + Send,
    {
        let listener = TcpListener::bind(&self.listen_addr).await?;
        info!(addr = %self.listen_addr, "JSON-RPC server listening");
        let handler = Arc::new(handler);

        loop {
            let (stream, peer) = listener.accept().await?;
            let handler = Arc::clone(&handler);
            tokio::spawn(async move {
                if let Err(e) = handle_conn(stream, handler).await {
                    debug!(%peer, error = %e, "rpc connection closed");
                }
            });
        }
    }
}

async fn handle_conn<H, Fut>(
    mut stream: tokio::net::TcpStream,
    handler: Arc<H>,
) -> std::io::Result<()>
where
    H: Fn(RpcRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = RpcResponse> + Send,
{
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];

    // Read until we have headers + full body (Content-Length).
    let body_start;
    let content_length;
    loop {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = find_header_end(&buf) {
            body_start = pos;
            content_length = parse_content_length(&buf[..pos]).unwrap_or(0);
            break;
        }
        if buf.len() > 1024 * 1024 {
            return write_response(
                &mut stream,
                &RpcResponse::err(
                    serde_json::Value::Null,
                    codes::INVALID_REQUEST,
                    "request too large",
                ),
            )
            .await;
        }
    }

    while buf.len() < body_start + content_length {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }

    let body = &buf[body_start..(body_start + content_length).min(buf.len())];

    let response = match serde_json::from_slice::<RpcRequest>(body) {
        Ok(req) => {
            debug!(method = %req.method, "rpc request");
            handler(req).await
        }
        Err(e) => {
            warn!(error = %e, "failed to parse rpc request");
            RpcResponse::err(serde_json::Value::Null, codes::PARSE_ERROR, e.to_string())
        }
    };

    write_response(&mut stream, &response).await
}

async fn write_response(
    stream: &mut tokio::net::TcpStream,
    response: &RpcResponse,
) -> std::io::Result<()> {
    let body = serde_json::to_vec(response).unwrap_or_default();
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.flush().await?;
    Ok(())
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

fn parse_content_length(headers: &[u8]) -> Option<usize> {
    let text = std::str::from_utf8(headers).ok()?;
    for line in text.lines() {
        if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            return rest.trim().parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_length() {
        let h = b"POST / HTTP/1.1\r\nContent-Length: 42\r\n\r\n";
        let end = find_header_end(h).unwrap();
        assert_eq!(parse_content_length(&h[..end]), Some(42));
    }

    #[test]
    fn header_end_detected() {
        let h = b"GET / HTTP/1.1\r\nHost: x\r\n\r\nBODY";
        assert_eq!(find_header_end(h), Some(h.len() - 4));
    }
}
