use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::info;
use veilux_rpc::types::{
    codes, BlockNotification, BlockView, ChainStats, CommandLocation, ContractCode, EstimateResult,
    EventView, NodeInfo, RpcRequest, RpcResponse, StateEntry, StatePrefixResult, StateResult,
    SubmitResult, VerificationRecord, VerifyResult,
};
use veilux_rpc::ws::WsHub;
use veilux_rpc::{method, server::RpcServer};

use crate::node::Node;

pub async fn serve_rpc(
    node: Arc<Mutex<Node>>,
    listen_addr: String,
    hub: Arc<WsHub>,
) -> std::io::Result<()> {
    let server = RpcServer::new(listen_addr.clone());
    info!(addr = %listen_addr, "starting VEILUX dev RPC node");

    server
        .serve(move |req| {
            let node = Arc::clone(&node);
            let hub = Arc::clone(&hub);
            async move { dispatch(node, hub, req).await }
        })
        .await
}

async fn dispatch(node: Arc<Mutex<Node>>, hub: Arc<WsHub>, req: RpcRequest) -> RpcResponse {
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
                    let _ = n.produce_block();
                    let mempool_len = n.mempool.len();
                    let head = n.head();
                    let notif = BlockNotification::new(
                        head.height,
                        head.hash().to_hex(),
                        head.state_root.to_hex(),
                        head.commands.len(),
                        head.events.len(),
                        head.timestamp,
                    );
                    drop(n);
                    hub.publish(serde_json::to_string(&notif).unwrap_or_default());
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

        method::EXPLORER_STATS => {
            let n = node.lock().await;
            ok(id, build_stats(&n))
        }

        method::EXPLORER_RECENT_BLOCKS => {
            let limit = req
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(20)
                .min(200) as usize;
            let n = node.lock().await;
            let blocks: Vec<BlockView> =
                n.blocks.iter().rev().take(limit).map(block_view).collect();
            ok(id, blocks)
        }

        method::EXPLORER_BLOCK_BY_HASH => {
            let hash = req.params.get("hash").and_then(|v| v.as_str());
            let Some(hash) = hash else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "missing 'hash'");
            };
            let n = node.lock().await;
            match n.blocks.iter().find(|b| b.hash().to_hex() == hash) {
                Some(b) => ok(id, block_view(b)),
                None => RpcResponse::err(id, codes::INVALID_PARAMS, "block not found"),
            }
        }

        method::EXPLORER_SEARCH_COMMAND => {
            let cid = req.params.get("command_id").and_then(|v| v.as_str());
            let Some(cid) = cid else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "missing 'command_id'");
            };
            let n = node.lock().await;
            ok(id, search_command(&n, cid))
        }

        method::EXPLORER_LIST_BY_PRISM => {
            let prism = req.params.get("prism").and_then(|v| v.as_str());
            let Some(prism) = prism else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "missing 'prism'");
            };
            let limit = req
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(50)
                .min(500) as usize;
            let n = node.lock().await;
            let mut out: Vec<EventView> = Vec::new();
            for block in n.blocks.iter().rev() {
                for ev in &block.events {
                    if ev.prism == prism {
                        out.push(event_view(block.height, ev));
                        if out.len() >= limit {
                            break;
                        }
                    }
                }
                if out.len() >= limit {
                    break;
                }
            }
            ok(id, out)
        }

        method::EXPLORER_STATE_PREFIX => {
            let prefix = req
                .params
                .get("prefix")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let limit = req
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(100)
                .min(1000) as usize;
            let n = node.lock().await;
            let all: Vec<(String, Vec<u8>)> = n
                .state
                .iter_prefix(prefix)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let total = all.len();
            let entries: Vec<StateEntry> = all
                .into_iter()
                .take(limit)
                .map(|(key, v)| StateEntry {
                    key,
                    value_hex: hex::encode(v),
                })
                .collect();
            ok(
                id,
                StatePrefixResult {
                    prefix: prefix.to_string(),
                    total,
                    entries,
                },
            )
        }

        method::CONTRACT_GET_CODE => {
            let addr = req.params.get("address").and_then(|v| v.as_str());
            let Some(addr) = addr else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "missing 'address'");
            };
            let n = node.lock().await;
            ok(id, read_contract_code(&n, addr))
        }

        method::CONTRACT_GET_VERIFICATION => {
            let addr = req.params.get("address").and_then(|v| v.as_str());
            let Some(addr) = addr else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "missing 'address'");
            };
            let n = node.lock().await;
            let key = format!("contract/verified/{addr}");
            match n.state.get_json::<VerificationRecord>(&key) {
                Ok(Some(rec)) => ok(id, serde_json::json!({ "found": true, "record": rec })),
                _ => ok(id, serde_json::json!({ "found": false })),
            }
        }

        method::CONTRACT_VERIFY => {
            let vr: Result<veilux_rpc::types::VerifyRequest, _> =
                serde_json::from_value(req.params.clone());
            let Ok(vr) = vr else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "invalid verify request");
            };
            let mut n = node.lock().await;
            let code = read_contract_code(&n, &vr.address);
            if !code.found {
                return ok(
                    id,
                    VerifyResult {
                        verified: false,
                        message: "contract not found on chain".into(),
                        code_hash: String::new(),
                    },
                );
            }
            let submitted = vr.bytecode_hex.trim_start_matches("0x").to_lowercase();
            let onchain = code.bytecode_hex.trim_start_matches("0x").to_lowercase();
            if submitted != onchain {
                return ok(
                    id,
                    VerifyResult {
                        verified: false,
                        message: "submitted bytecode does not match deployed bytecode".into(),
                        code_hash: code.code_hash.clone(),
                    },
                );
            }
            let height = n.head().height;
            let record = VerificationRecord {
                address: vr.address.clone(),
                name: vr.name,
                source: vr.source,
                compiler: vr.compiler,
                abi: vr.abi,
                code_hash: code.code_hash.clone(),
                verified_at_height: height,
            };
            let key = format!("contract/verified/{}", vr.address);
            if let Err(e) = n.state.put_json(key, &record) {
                return RpcResponse::err(id, codes::INTERNAL_ERROR, e.to_string());
            }
            ok(
                id,
                VerifyResult {
                    verified: true,
                    message: "contract verified".into(),
                    code_hash: code.code_hash,
                },
            )
        }

        other => RpcResponse::err(
            id,
            codes::METHOD_NOT_FOUND,
            format!("unknown method: {other}"),
        ),
    }
}

fn block_view(b: &veilux_kernel::Block) -> BlockView {
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
    }
}

fn event_view(height: u64, ev: &veilux_kernel::Event) -> EventView {
    let (visibility, redacted, stakeholders) = match &ev.visibility {
        veilux_kernel::Visibility::Public => ("public".to_string(), false, 0usize),
        veilux_kernel::Visibility::Parties(p) => ("parties".to_string(), true, p.len()),
    };

    let (payload_json, payload_hex) = if redacted {
        (None, None)
    } else {
        match serde_json::from_slice::<serde_json::Value>(&ev.payload) {
            Ok(j) => (Some(j), None),
            Err(_) => (None, Some(hex::encode(&ev.payload))),
        }
    };

    EventView {
        block_height: height,
        prism: ev.prism.clone(),
        commitment: ev.commitment().to_hex(),
        source_command: ev.source_command.to_hex(),
        visibility,
        redacted,
        stakeholders,
        payload_json,
        payload_hex,
    }
}

fn build_stats(n: &Node) -> ChainStats {
    let mut total_commands = 0u64;
    let mut total_events = 0u64;
    let mut events_by_prism: std::collections::BTreeMap<String, u64> = Default::default();
    for b in &n.blocks {
        total_commands += b.commands.len() as u64;
        for ev in &b.events {
            total_events += 1;
            *events_by_prism.entry(ev.prism.clone()).or_insert(0) += 1;
        }
    }
    let head = n.head();
    ChainStats {
        height: head.height,
        total_blocks: n.blocks.len() as u64,
        total_commands,
        total_events,
        head_hash: head.hash().to_hex(),
        state_root: n.state.root().to_hex(),
        state_entries: n.state.len(),
        events_by_prism,
    }
}

fn search_command(n: &Node, command_id_hex: &str) -> CommandLocation {
    for block in n.blocks.iter().rev() {
        for c in &block.commands {
            if c.id().to_hex() == command_id_hex {
                let events: Vec<EventView> = block
                    .events
                    .iter()
                    .filter(|e| e.source_command.to_hex() == command_id_hex)
                    .map(|e| event_view(block.height, e))
                    .collect();
                return CommandLocation {
                    found: true,
                    command_id: command_id_hex.to_string(),
                    block_height: Some(block.height),
                    block_hash: Some(block.hash().to_hex()),
                    prism: Some(c.prism.clone()),
                    submitter: Some(c.submitter.0.clone()),
                    events,
                };
            }
        }
    }
    CommandLocation {
        found: false,
        command_id: command_id_hex.to_string(),
        block_height: None,
        block_hash: None,
        prism: None,
        submitter: None,
        events: vec![],
    }
}

fn parse_submit(req: &RpcRequest) -> Option<veilux_rpc::types::SubmitParams> {
    serde_json::from_value(req.params.clone()).ok()
}

fn read_contract_code(n: &Node, address: &str) -> ContractCode {
    let key = format!("contract/code/{address}");
    let meta: Option<serde_json::Value> = n.state.get_json(&key).ok().flatten();
    let verified = n.state.contains(&format!("contract/verified/{address}"));
    match meta {
        Some(m) => {
            let code: Vec<u8> = m
                .get("code")
                .and_then(|c| c.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_u64().map(|n| n as u8))
                        .collect()
                })
                .unwrap_or_default();
            let deployer = m
                .get("deployer")
                .and_then(|d| d.as_str())
                .map(|s| s.to_string());
            let code_hash = veilux_kernel::Hash::digest(&code).to_hex();
            ContractCode {
                address: address.to_string(),
                found: true,
                deployer,
                bytecode_hex: hex::encode(&code),
                code_size: code.len(),
                code_hash,
                verified,
            }
        }
        None => ContractCode {
            address: address.to_string(),
            found: false,
            deployer: None,
            bytecode_hex: String::new(),
            code_size: 0,
            code_hash: String::new(),
            verified: false,
        },
    }
}

fn ok<T: serde::Serialize>(id: serde_json::Value, value: T) -> RpcResponse {
    match serde_json::to_value(value) {
        Ok(v) => RpcResponse::ok(id, v),
        Err(e) => RpcResponse::err(id, codes::INTERNAL_ERROR, e.to_string()),
    }
}
