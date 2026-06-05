pub mod auth;
pub mod message;

pub use auth::{AuthConfig, AuthError, PeerInfo, PeerKey};
pub use message::{NetMessage, ViewChange};

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::{debug, info, warn};

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("encode error: {0}")]
    Encode(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct NetConfig {
    pub node_id: String,
    pub listen_addr: String,
    pub bootstrap: Vec<String>,
    pub auth: Option<AuthConfig>,
}

impl std::fmt::Debug for NetConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetConfig")
            .field("node_id", &self.node_id)
            .field("listen_addr", &self.listen_addr)
            .field("bootstrap", &self.bootstrap)
            .field("auth", &self.auth.is_some())
            .finish()
    }
}

impl Default for NetConfig {
    fn default() -> Self {
        NetConfig {
            node_id: "node".into(),
            listen_addr: "127.0.0.1:30420".into(),
            bootstrap: vec![],
            auth: None,
        }
    }
}

pub struct Network {
    cfg: NetConfig,
    outbound: broadcast::Sender<String>,
    inbound_tx: mpsc::UnboundedSender<NetMessage>,
    peer_count: Arc<Mutex<usize>>,
}

pub struct NetHandle {
    pub inbound: mpsc::UnboundedReceiver<NetMessage>,
    pub net: Arc<Network>,
}

impl Network {
    pub fn spawn(cfg: NetConfig) -> NetHandle {
        let (outbound, _) = broadcast::channel::<String>(1024);
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel::<NetMessage>();
        let net = Arc::new(Network {
            cfg: cfg.clone(),
            outbound,
            inbound_tx,
            peer_count: Arc::new(Mutex::new(0)),
        });

        let listener_net = Arc::clone(&net);
        tokio::spawn(async move {
            if let Err(e) = listener_net.run_listener().await {
                warn!(error = %e, "listener stopped");
            }
        });

        for addr in cfg.bootstrap.iter().cloned() {
            let dial_net = Arc::clone(&net);
            tokio::spawn(async move {
                dial_net.dial(addr).await;
            });
        }

        NetHandle {
            inbound: inbound_rx,
            net,
        }
    }

    pub fn broadcast(&self, msg: &NetMessage) -> Result<(), NetError> {
        let line = msg.encode()?;
        let _ = self.outbound.send(line);
        debug!(kind = msg.kind(), "broadcast queued");
        Ok(())
    }

    pub async fn peer_count(&self) -> usize {
        *self.peer_count.lock().await
    }

    async fn run_listener(self: Arc<Self>) -> Result<(), NetError> {
        let listener = TcpListener::bind(&self.cfg.listen_addr).await?;
        let auth_mode = if self.cfg.auth.is_some() {
            "authenticated"
        } else {
            "open (dev)"
        };
        info!(addr = %self.cfg.listen_addr, node = %self.cfg.node_id, mode = auth_mode, "listening for peers");
        loop {
            let (stream, addr) = listener.accept().await?;
            if let Some(auth) = &self.cfg.auth {
                if !auth.allows_ip(&addr.ip()) {
                    warn!(%addr, "rejected inbound peer: ip not on allowlist");
                    continue;
                }
            }
            debug!(%addr, "peer connected (inbound)");
            let me = Arc::clone(&self);
            tokio::spawn(async move {
                me.handle_peer(stream, Some(addr), true).await;
            });
        }
    }

    async fn dial(self: Arc<Self>, addr: String) {
        loop {
            match TcpStream::connect(&addr).await {
                Ok(stream) => {
                    info!(%addr, "dialed peer");
                    let peer_addr = stream.peer_addr().ok();
                    Arc::clone(&self)
                        .handle_peer(stream, peer_addr, false)
                        .await;
                }
                Err(e) => {
                    debug!(%addr, error = %e, "dial failed, retrying");
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    async fn handle_peer(
        self: Arc<Self>,
        stream: TcpStream,
        peer_addr: Option<SocketAddr>,
        inbound: bool,
    ) {
        let (read_half, write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);
        let mut write_half = write_half;

        let mut session = None;
        if let Some(auth) = &self.cfg.auth {
            match auth::perform_handshake(auth, &mut reader, &mut write_half).await {
                Ok((peer, sess)) => {
                    info!(
                        party = %peer.party,
                        addr = ?peer_addr,
                        dir = if inbound { "inbound" } else { "outbound" },
                        "peer authenticated (encrypted channel established)"
                    );
                    session = Some(sess);
                }
                Err(e) => {
                    warn!(addr = ?peer_addr, error = %e, "handshake failed, dropping peer");
                    return;
                }
            }
        }

        {
            let mut c = self.peer_count.lock().await;
            *c += 1;
        }
        let mut rx = self.outbound.subscribe();
        let inbound_tx = self.inbound_tx.clone();
        let peer_count = Arc::clone(&self.peer_count);

        let (sender, receiver) = match session {
            Some(s) => {
                let (tx, rx) = s.split();
                (Some(tx), Some(rx))
            }
            None => (None, None),
        };

        let writer = tokio::spawn(async move {
            let mut sender = sender;
            while let Ok(line) = rx.recv().await {
                let payload = match &mut sender {
                    Some(tx) => match tx.seal(line.as_bytes()) {
                        Ok(ct) => hex::encode(ct),
                        Err(_) => break,
                    },
                    None => line,
                };
                if write_half.write_all(payload.as_bytes()).await.is_err() {
                    break;
                }
                if write_half.write_all(b"\n").await.is_err() {
                    break;
                }
            }
        });

        let mut receiver = receiver;
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            let plaintext = match &mut receiver {
                Some(rx) => {
                    let Ok(ct) = hex::decode(line.trim()) else {
                        debug!("dropping malformed encrypted frame");
                        continue;
                    };
                    match rx.open(&ct) {
                        Ok(pt) => match String::from_utf8(pt) {
                            Ok(s) => s,
                            Err(_) => {
                                warn!("decrypted frame is not valid utf-8, dropping peer");
                                break;
                            }
                        },
                        Err(_) => {
                            warn!("frame decryption failed (tamper/replay), dropping peer");
                            break;
                        }
                    }
                }
                None => line,
            };
            match NetMessage::decode(&plaintext) {
                Ok(msg) => {
                    let _ = inbound_tx.send(msg);
                }
                Err(e) => debug!(error = %e, "dropping malformed message"),
            }
        }

        writer.abort();
        {
            let mut c = peer_count.lock().await;
            *c = c.saturating_sub(1);
        }
        debug!("peer disconnected");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veilux_kernel::{Block, PartyId};

    #[tokio::test]
    async fn two_nodes_gossip_a_block() {
        let a_cfg = NetConfig {
            node_id: "a".into(),
            listen_addr: "127.0.0.1:39001".into(),
            bootstrap: vec![],
            auth: None,
        };
        let mut a = Network::spawn(a_cfg);

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let b_cfg = NetConfig {
            node_id: "b".into(),
            listen_addr: "127.0.0.1:39002".into(),
            bootstrap: vec!["127.0.0.1:39001".into()],
            auth: None,
        };
        let b = Network::spawn(b_cfg);

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        let blk = Block::genesis(PartyId::new("v1"), 42);
        b.net.broadcast(&NetMessage::Block(Box::new(blk))).unwrap();

        let recv = tokio::time::timeout(std::time::Duration::from_secs(2), a.inbound.recv()).await;
        assert!(recv.is_ok(), "node A should receive a gossiped message");
        let msg = recv.unwrap().unwrap();
        assert_eq!(msg.kind(), "block");
    }
}
