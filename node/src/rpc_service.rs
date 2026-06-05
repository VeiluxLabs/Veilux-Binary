use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::info;
use veilux_rpc::types::{
    codes, BlockView, EstimateResult, NodeInfo, RpcRequest, RpcResponse, StateResult, SubmitResult,
};
use veilux_rpc::{method, server::RpcServer};

use crate::node::Node;

/// Runs the JSON-RPC server over a shared, mutable node. This is the developer
/// entry point ("dev node"): submitted commands are applied and a block is
/// produced immediately, so clients get fast, deterministic feedback — similar
/// to a local Ethereum dev chain.
pub async fn serve_rpc(node: Arc<Mutex<Node>>, listen_addr: String) -> std::io::Result<()> {
    let server = RpcServer::new(listen_addr.clone());
    info!(addr = %listen_addr, "starting VEILUX dev RPC node");

    server
        .serve(move |req| {
            let node = Arc::clone(&node);
            async move { dispatch(node, req).await }
        })
        .await
}

async fn dispatch(node: Arc<Mutex<Node>>, req: RpcRequest) -> RpcResponse {
    let id = req.id.clone();
    match req.method.as_str() {
        method::NODE_INFO => {
            let n = node.lock().await;
            let info = NodeInfo {
                network: "veilux-dev".into(),
                protocol: veilux_kernel::PROTOCOL_VERSION.into(),
                token: veilux_kernel::TOKEN_TICKER.into(),
                height: n.head().height,
                head_hash: n.head().hash().to_hex(),
                state_root: n.state.root().to_hex(),
                prisms: n
                    .cascade
                    .installed()
                    .iter()
                    .map(|p| p.name.to_string())
                    .collect(),
            };
            ok(id, info)
        }

        method::BLOCK_NUMBER => {
            let n = node.lock().await;
            ok(id, n.head().height)
        }

        method::GET_BLOCK_BY_NUMBER => {
            let height = req.params.get("height").and_then(|v| v.as_u64());
            let Some(height) = height else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "missing 'height'");
            };
            let n = node.lock().await;
            match n.blocks.iter().find(|b| b.height == height) {
                Some(b) => ok(
                    id,
                    BlockView {
                        height: b.height,
                        hash: b.hash().to_hex(),
                        parent: b.parent.to_hex(),
                        state_root: b.state_root.to_hex(),
                        events_root: b.events_root.to_hex(),
                        proposer: b.proposer.0.clone(),
                        timestamp: b.timestamp,
                        command_count: b.commands.len(),
                        event_count: b.events.len(),
                    },
                ),
                None => RpcResponse::err(id, codes::INVALID_PARAMS, "block not found"),
            }
        }

        method::GET_STATE => {
            let key = req.params.get("key").and_then(|v| v.as_str());
            let Some(key) = key else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "missing 'key'");
            };
            let n = node.lock().await;
            let (found, value_hex) = match n.state.get(key) {
                Some(v) => (true, hex::encode(v)),
                None => (false, String::new()),
            };
            ok(
                id,
                StateResult {
                    key: key.to_string(),
                    found,
                    value_hex,
                },
            )
        }

        method::ESTIMATE => {
            let Some(params) = parse_submit(&req) else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "missing 'command'");
            };
            let n = node.lock().await;
            match n.estimate(&params.command.command) {
                Ok(cost) => ok(id, EstimateResult { cost }),
                Err(e) => RpcResponse::err(id, codes::COMMAND_REJECTED, e.to_string()),
            }
        }

        method::SUBMIT => {
            let Some(params) = parse_submit(&req) else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "missing 'command'");
            };
            let command_id = params.command.command.id().to_hex();
            let mut n = node.lock().await;
            match n.submit_signed(params.command) {
                Ok(()) => {
                    // Dev-node: produce a block immediately for fast feedback.
                    let _ = n.produce_block();
                    let mempool_len = n.mempool.len();
                    ok(
                        id,
                        SubmitResult {
                            accepted: true,
                            command_id,
                            mempool_len,
                        },
                    )
                }
                Err(e) => RpcResponse::err(id, codes::COMMAND_REJECTED, e.to_string()),
            }
        }

        other => RpcResponse::err(
            id,
            codes::METHOD_NOT_FOUND,
            format!("unknown method: {other}"),
        ),
    }
}

fn parse_submit(req: &RpcRequest) -> Option<veilux_rpc::types::SubmitParams> {
    serde_json::from_value(req.params.clone()).ok()
}

fn ok<T: serde::Serialize>(id: serde_json::Value, value: T) -> RpcResponse {
    match serde_json::to_value(value) {
        Ok(v) => RpcResponse::ok(id, v),
        Err(e) => RpcResponse::err(id, codes::INTERNAL_ERROR, e.to_string()),
    }
}
