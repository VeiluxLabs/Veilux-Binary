use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::info;
use veilux_rpc::server::RpcServer;
use veilux_rpc::types::{codes, RpcRequest, RpcResponse};

use crate::eth::{eth_balance, eth_code, eth_nonce, eth_receipt, eth_tx};
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

fn parse_hash(s: &str) -> Option<[u8; 32]> {
    let clean = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(clean).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut h = [0u8; 32];
    h.copy_from_slice(&bytes);
    Some(h)
}

fn first_str(req: &RpcRequest, idx: usize) -> Option<String> {
    req.params
        .as_array()
        .and_then(|a| a.get(idx))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn block_hash_at(n: &Node, height: u64) -> String {
    n.blocks
        .iter()
        .find(|b| b.height == height)
        .map(|b| format!("0x{}", b.hash().to_hex().trim_start_matches("0x")))
        .unwrap_or_else(|| format!("0x{}", hex::encode([0u8; 32])))
}

fn eth_block_json(n: &Node, b: &veilux_kernel::Block) -> serde_json::Value {
    serde_json::json!({
        "number": hex_quantity(b.height as u128),
        "hash": format!("0x{}", b.hash().to_hex().trim_start_matches("0x")),
        "parentHash": format!("0x{}", b.parent.to_hex().trim_start_matches("0x")),
        "stateRoot": format!("0x{}", b.state_root.to_hex().trim_start_matches("0x")),
        "timestamp": hex_quantity(b.timestamp as u128),
        "transactions": [],
        "gasLimit": hex_quantity(n.limits.max_block_gas as u128),
        "gasUsed": "0x0",
        "miner": format!("0x{}", hex::encode([0u8; 20])),
        "difficulty": "0x0",
        "totalDifficulty": "0x0",
        "nonce": "0x0000000000000000",
        "extraData": "0x",
        "size": "0x0",
        "uncles": [],
    })
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
        "eth_getCode" => {
            let Some(addr) = first_str(&req, 0).and_then(|s| parse_addr(&s)) else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "bad address");
            };
            let n = state.node.lock().await;
            ok(id, format!("0x{}", hex::encode(eth_code(&n.state, &addr))))
        }
        "eth_call" => {
            let to = req
                .params
                .as_array()
                .and_then(|a| a.first())
                .and_then(|o| o.get("to"))
                .and_then(|v| v.as_str())
                .and_then(parse_addr);
            let data = req
                .params
                .as_array()
                .and_then(|a| a.first())
                .and_then(|o| o.get("data"))
                .and_then(|v| v.as_str())
                .map(|s| hex::decode(s.strip_prefix("0x").unwrap_or(s)).unwrap_or_default())
                .unwrap_or_default();
            let Some(to) = to else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "missing 'to'");
            };
            let n = state.node.lock().await;
            match n.eth_call(&to, data) {
                Ok(out) => ok(id, format!("0x{}", hex::encode(out))),
                Err(e) => RpcResponse::err(id, codes::COMMAND_REJECTED, e.to_string()),
            }
        }
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
                    drop(n);
                    let to_str = match applied.to {
                        Some(a) => format!("0x{}", hex::encode(a)),
                        None => "<deploy>".to_string(),
                    };
                    let created = applied
                        .contract_address
                        .map(|a| format!("0x{}", hex::encode(a)))
                        .unwrap_or_default();
                    info!(
                        from = %format!("0x{}", hex::encode(applied.from)),
                        to = %to_str,
                        contract = %created,
                        value = applied.value,
                        nonce = applied.nonce,
                        gas_used = applied.gas_used,
                        logs = applied.logs.len(),
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
                    let gas = r.get("gas_used").and_then(|v| v.as_u64()).unwrap_or(21_000);
                    let status = r.get("status").and_then(|v| v.as_u64()).unwrap_or(1);
                    let contract = r
                        .get("contract_address")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let bhash = block_hash_at(&n, bn);
                    let logs = r
                        .get("logs")
                        .cloned()
                        .unwrap_or(serde_json::Value::Array(vec![]));
                    let receipt = serde_json::json!({
                        "transactionHash": h,
                        "transactionIndex": "0x0",
                        "blockNumber": hex_quantity(bn as u128),
                        "blockHash": bhash,
                        "from": r.get("from").cloned().unwrap_or(serde_json::Value::Null),
                        "to": r.get("to").cloned().unwrap_or(serde_json::Value::Null),
                        "cumulativeGasUsed": hex_quantity(gas as u128),
                        "gasUsed": hex_quantity(gas as u128),
                        "contractAddress": contract,
                        "logs": logs,
                        "logsBloom": format!("0x{}", hex::encode([0u8; 256])),
                        "status": if status == 0 { "0x0" } else { "0x1" },
                    });
                    ok(id, receipt)
                }
                None => ok(id, serde_json::Value::Null),
            }
        }
        "eth_getTransactionByHash" => {
            let Some(hash) = first_str(&req, 0).and_then(|s| parse_hash(&s)) else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "bad hash");
            };
            let n = state.node.lock().await;
            match eth_tx(&n.state, &hash) {
                Some(t) => {
                    let bn = t.get("block_number").and_then(|v| v.as_u64()).unwrap_or(0);
                    let value: u128 = t
                        .get("value")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    let gas_price: u128 = t
                        .get("gas_price")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    let gas = t.get("gas").and_then(|v| v.as_u64()).unwrap_or(21_000);
                    let nonce = t.get("nonce").and_then(|v| v.as_u64()).unwrap_or(0);
                    let tx = serde_json::json!({
                        "hash": t.get("hash").cloned().unwrap_or(serde_json::Value::Null),
                        "from": t.get("from").cloned().unwrap_or(serde_json::Value::Null),
                        "to": t.get("to").cloned().unwrap_or(serde_json::Value::Null),
                        "nonce": hex_quantity(nonce as u128),
                        "value": hex_quantity(value),
                        "gas": hex_quantity(gas as u128),
                        "gasPrice": hex_quantity(gas_price),
                        "input": t.get("input").cloned().unwrap_or(serde_json::Value::Null),
                        "blockNumber": hex_quantity(bn as u128),
                        "blockHash": block_hash_at(&n, bn),
                        "transactionIndex": "0x0",
                    });
                    ok(id, tx)
                }
                None => ok(id, serde_json::Value::Null),
            }
        }
        "eth_getLogs" => {
            let n = state.node.lock().await;
            let want_addr = req
                .params
                .as_array()
                .and_then(|a| a.first())
                .and_then(|o| o.get("address"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_lowercase());
            let mut out: Vec<serde_json::Value> = Vec::new();
            for (_key, raw) in n.state.iter_prefix("eth/receipt/0x") {
                let Ok(r) = serde_json::from_slice::<serde_json::Value>(raw) else {
                    continue;
                };
                let bn = r.get("block_number").and_then(|v| v.as_u64()).unwrap_or(0);
                if let Some(serde_json::Value::Array(logs)) = r.get("logs") {
                    for lg in logs {
                        let addr = lg
                            .get("address")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_lowercase())
                            .unwrap_or_default();
                        if let Some(w) = &want_addr {
                            if &addr != w {
                                continue;
                            }
                        }
                        let mut entry = lg.clone();
                        if let Some(obj) = entry.as_object_mut() {
                            obj.insert(
                                "blockNumber".into(),
                                serde_json::json!(hex_quantity(bn as u128)),
                            );
                            obj.insert(
                                "blockHash".into(),
                                serde_json::json!(block_hash_at(&n, bn)),
                            );
                        }
                        out.push(entry);
                    }
                }
            }
            ok(id, out)
        }
        "eth_getBlockByNumber" => {
            let n = state.node.lock().await;
            let want = first_str(&req, 0).unwrap_or_else(|| "latest".to_string());
            let target = match want.as_str() {
                "latest" | "pending" | "safe" | "finalized" => Some(n.head().height),
                "earliest" => Some(0),
                hexnum => u64::from_str_radix(hexnum.strip_prefix("0x").unwrap_or(hexnum), 16).ok(),
            };
            match target.and_then(|h| n.blocks.iter().find(|b| b.height == h)) {
                Some(b) => ok(id, eth_block_json(&n, b)),
                None => ok(id, serde_json::Value::Null),
            }
        }
        "eth_getBlockByHash" => {
            let Some(hash) = first_str(&req, 0) else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "missing hash");
            };
            let clean = hash.strip_prefix("0x").unwrap_or(&hash).to_lowercase();
            let n = state.node.lock().await;
            match n
                .blocks
                .iter()
                .find(|b| b.hash().to_hex().trim_start_matches("0x") == clean)
            {
                Some(b) => ok(id, eth_block_json(&n, b)),
                None => ok(id, serde_json::Value::Null),
            }
        }
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
