use serde::{Deserialize, Serialize};
use veilux_kernel::{Hash, SignedCommand};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
    pub id: serde_json::Value,
}

impl RpcRequest {
    pub fn new(method: impl Into<String>, params: serde_json::Value, id: u64) -> Self {
        RpcRequest {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
            id: serde_json::json!(id),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
    pub id: serde_json::Value,
}

impl RpcResponse {
    pub fn ok(id: serde_json::Value, result: serde_json::Value) -> Self {
        RpcResponse {
            jsonrpc: "2.0".into(),
            result: Some(result),
            error: None,
            id,
        }
    }

    pub fn err(id: serde_json::Value, code: i64, message: impl Into<String>) -> Self {
        RpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
            id,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

pub mod codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
    pub const COMMAND_REJECTED: i64 = -32000;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeInfo {
    pub network: String,
    pub protocol: String,
    pub token: String,
    pub height: u64,
    pub head_hash: String,
    pub state_root: String,
    pub prisms: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubmitParams {
    pub command: SignedCommand,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubmitResult {
    pub accepted: bool,
    pub command_id: String,
    pub mempool_len: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockView {
    pub height: u64,
    pub hash: String,
    pub parent: String,
    pub state_root: String,
    pub events_root: String,
    pub proposer: String,
    pub timestamp: u64,
    pub command_count: usize,
    pub event_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateQuery {
    pub key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateResult {
    pub key: String,
    pub found: bool,
    pub value_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EstimateResult {
    pub cost: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockNotification {
    #[serde(rename = "type")]
    pub kind: String,
    pub height: u64,
    pub hash: String,
    pub state_root: String,
    pub command_count: usize,
    pub event_count: usize,
    pub timestamp: u64,
}

impl BlockNotification {
    pub fn new(
        height: u64,
        hash: String,
        state_root: String,
        command_count: usize,
        event_count: usize,
        timestamp: u64,
    ) -> Self {
        BlockNotification {
            kind: "block".to_string(),
            height,
            hash,
            state_root,
            command_count,
            event_count,
            timestamp,
        }
    }
}

pub fn hash_hex(h: &Hash) -> String {
    h.to_hex()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChainStats {
    pub height: u64,
    pub total_blocks: u64,
    pub total_commands: u64,
    pub total_events: u64,
    pub head_hash: String,
    pub state_root: String,
    pub state_entries: usize,
    pub events_by_prism: std::collections::BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventView {
    pub block_height: u64,
    pub prism: String,
    pub commitment: String,
    pub source_command: String,
    pub visibility: String,
    pub redacted: bool,
    pub stakeholders: usize,
    pub payload_json: Option<serde_json::Value>,
    pub payload_hex: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommandLocation {
    pub found: bool,
    pub command_id: String,
    pub block_height: Option<u64>,
    pub block_hash: Option<String>,
    pub prism: Option<String>,
    pub submitter: Option<String>,
    pub events: Vec<EventView>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateEntry {
    pub key: String,
    pub value_hex: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatePrefixResult {
    pub prefix: String,
    pub total: usize,
    pub entries: Vec<StateEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContractCode {
    pub address: String,
    pub found: bool,
    pub deployer: Option<String>,
    pub bytecode_hex: String,
    pub code_size: usize,
    pub code_hash: String,
    pub verified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerifyRequest {
    pub address: String,
    pub name: String,
    pub source: String,
    pub bytecode_hex: String,
    pub compiler: String,
    #[serde(default)]
    pub abi: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerificationRecord {
    pub address: String,
    pub name: String,
    pub source: String,
    pub compiler: String,
    pub abi: String,
    pub code_hash: String,
    pub verified_at_height: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerifyResult {
    pub verified: bool,
    pub message: String,
    pub code_hash: String,
}
