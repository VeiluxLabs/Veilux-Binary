use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::info;
use veilux_rpc::server::RpcServer;
use veilux_rpc::types::{codes, RpcRequest, RpcResponse};

use crate::eth::{eth_balance, eth_nonce, eth_receipt};
use crate::node::Node;

pub struct EthRpcState {
    pub node: Arc<Mutex<Node>>,
    pub chain_id: u64,
}

pub async fn serve_eth_rpc(addr: String, state: Arc<EthRpcState>) -> std::io::Result<()> {
    let server = RpcServer::new(addr.clone());
    info!(%addr, chain_id = state.chain_id, "Ethereum-compatible JSON-RPC (eth_*) online");
    server
        .serve(move |req| {
            let state = Arc::clone(&state);
            async move { dispatch(state, req).await }
        })
        .await
}

fn hex_quantity(v: u128) -> String {
    format!("0x{:x}", v)
}

fn parse_addr(s: &str) -> Option<[u8; 20]> {
    let clean = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(clean).ok()?;
    if bytes.len() != 20 {
        return None;
    }
    let mut a = [0u8; 20];
    a.copy_from_slice(&bytes);
    Some(a)
}

fn first_str(req: &RpcRequest, idx: usize) -> Option<String> {
    req.params
        .as_array()
        .and_then(|a| a.get(idx))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

async fn dispatch(state: Arc<EthRpcState>, req: RpcRequest) -> RpcResponse {
    let id = req.id.clone();
    match req.method.as_str() {
        "eth_chainId" => ok(id, hex_quantity(state.chain_id as u128)),
        "net_version" => ok(id, state.chain_id.to_string()),
        "eth_syncing" => ok(id, false),
        "net_listening" => ok(id, true),
        "web3_clientVersion" => ok(id, format!("veilux/{}", env!("CARGO_PKG_VERSION"))),
        "eth_gasPrice" => {
            let n = state.node.lock().await;
            ok(id, hex_quantity(n.current_base_price().max(1)))
        }
        "eth_estimateGas" => ok(id, "0x5208"),
        "eth_blockNumber" => {
            let n = state.node.lock().await;
            ok(id, hex_quantity(n.head().height as u128))
        }
        "eth_getBalance" => {
            let Some(addr) = first_str(&req, 0).and_then(|s| parse_addr(&s)) else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "bad address");
            };
            let n = state.node.lock().await;
            ok(id, hex_quantity(eth_balance(&n.state, &addr)))
        }
        "eth_getTransactionCount" => {
            let Some(addr) = first_str(&req, 0).and_then(|s| parse_addr(&s)) else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "bad address");
            };
            let n = state.node.lock().await;
            ok(id, hex_quantity(eth_nonce(&n.state, &addr) as u128))
        }
        "eth_sendRawTransaction" => {
            let Some(raw_hex) = first_str(&req, 0) else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "missing raw tx");
            };
            let clean = raw_hex.strip_prefix("0x").unwrap_or(&raw_hex);
            let Ok(raw) = hex::decode(clean) else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "bad hex");
            };
            let mut n = state.node.lock().await;
            match n.eth_apply_raw(&raw) {
                Ok(applied) => {
                    let _ = n.produce_block();
                    drop(n);
                    info!(
                        from = %format!("0x{}", hex::encode(applied.from)),
                        to = %format!("0x{}", hex::encode(applied.to)),
                        value = applied.value,
                        nonce = applied.nonce,
                        "eth tx applied"
                    );
                    ok(id, format!("0x{}", hex::encode(applied.hash)))
                }
                Err(e) => RpcResponse::err(id, codes::COMMAND_REJECTED, e.to_string()),
            }
        }
        "eth_getTransactionReceipt" => {
            let Some(h) = first_str(&req, 0) else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "missing hash");
            };
            let clean = h.strip_prefix("0x").unwrap_or(&h);
            let Ok(bytes) = hex::decode(clean) else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "bad hash");
            };
            if bytes.len() != 32 {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "hash must be 32 bytes");
            }
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&bytes);
            let n = state.node.lock().await;
            match eth_receipt(&n.state, &hash) {
                Some(r) => {
                    let bn = r.get("block_number").and_then(|v| v.as_u64()).unwrap_or(0);
                    let receipt = serde_json::json!({
                        "transactionHash": h,
                        "transactionIndex": "0x0",
                        "blockNumber": hex_quantity(bn as u128),
                        "blockHash": format!("0x{}", hex::encode([0u8; 32])),
                        "from": r.get("from").cloned().unwrap_or(serde_json::Value::Null),
                        "to": r.get("to").cloned().unwrap_or(serde_json::Value::Null),
                        "cumulativeGasUsed": "0x5208",
                        "gasUsed": "0x5208",
                        "contractAddress": serde_json::Value::Null,
                        "logs": [],
                        "logsBloom": format!("0x{}", hex::encode([0u8; 256])),
                        "status": "0x1",
                    });
                    ok(id, receipt)
                }
                None => ok(id, serde_json::Value::Null),
            }
        }
        "eth_getBlockByNumber" => {
            let n = state.node.lock().await;
            let head = n.head();
            let block = serde_json::json!({
                "number": hex_quantity(head.height as u128),
                "hash": format!("0x{}", head.hash().to_hex().trim_start_matches("0x")),
                "parentHash": format!("0x{}", head.parent.to_hex().trim_start_matches("0x")),
                "timestamp": hex_quantity(head.timestamp as u128),
                "transactions": [],
                "gasLimit": hex_quantity(n.limits.max_block_gas as u128),
                "gasUsed": "0x0",
                "miner": format!("0x{}", hex::encode([0u8; 20])),
            });
            ok(id, block)
        }
        "eth_call" => ok(id, "0x"),
        other => RpcResponse::err(
            id,
            codes::METHOD_NOT_FOUND,
            format!("eth method not supported by the VEILUX shim: {other}"),
        ),
    }
}

fn ok<T: serde::Serialize>(id: serde_json::Value, result: T) -> RpcResponse {
    RpcResponse::ok(
        id,
        serde_json::to_value(result).unwrap_or(serde_json::Value::Null),
    )
}
