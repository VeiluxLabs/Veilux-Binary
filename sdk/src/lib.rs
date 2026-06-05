//! # VEILUX SDK
//!
//! A small, ergonomic Rust client for talking to a VEILUX node over JSON-RPC.
//! It bundles the three things an application developer needs:
//!
//! 1. **Identity** — generate/load an Ed25519 [`PartyIdentity`] and sign commands.
//! 2. **Builders** — construct commands for every shipped Prism (token, NFT,
//!    contract, storage, AI) through one import.
//! 3. **Client** — submit signed commands and query the chain.
//!
//! ```no_run
//! use veilux_sdk::{Client, PartyIdentity, builders, Visibility};
//!
//! let client = Client::new("http://127.0.0.1:8645");
//! let alice = PartyIdentity::from_seed("alice", &[1u8; 32]);
//!
//! let info = client.node_info()?;
//! println!("height = {}", info.height);
//!
//! let cmd = builders::token_create(alice.party().clone(), Visibility::Public, 0,
//!     "Gold", "GLD", 18, 1_000_000, true);
//! let res = client.submit(&alice.sign(cmd))?;
//! println!("accepted: {}", res.accepted);
//! # Ok::<(), veilux_sdk::SdkError>(())
//! ```

use serde_json::json;

pub use veilux_kernel::{Command, Hash, PartyId, SignedCommand, Visibility};
pub use veilux_rpc::types::{
    BlockView, ChainStats, CommandLocation, EstimateResult, EventView, NodeInfo, StateEntry,
    StatePrefixResult, StateResult, SubmitParams, SubmitResult,
};
pub use veilux_rpc::{method, RpcRequest, RpcResponse};
pub use veilux_veil::PartyIdentity;

#[derive(Debug, thiserror::Error)]
pub enum SdkError {
    #[error("transport error: {0}")]
    Transport(String),
    #[error("rpc error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("decode error: {0}")]
    Decode(String),
    #[error("missing result in response")]
    MissingResult,
}

/// A synchronous VEILUX RPC client.
pub struct Client {
    endpoint: String,
    next_id: std::cell::Cell<u64>,
}

impl Client {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Client {
            endpoint: endpoint.into(),
            next_id: std::cell::Cell::new(1),
        }
    }

    fn call<T: for<'de> serde::Deserialize<'de>>(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<T, SdkError> {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let req = RpcRequest::new(method, params, id);
        let body = serde_json::to_string(&req).map_err(|e| SdkError::Decode(e.to_string()))?;

        let resp_text = ureq::post(&self.endpoint)
            .set("Content-Type", "application/json")
            .send_string(&body)
            .map_err(|e| SdkError::Transport(e.to_string()))?
            .into_string()
            .map_err(|e| SdkError::Transport(e.to_string()))?;

        let resp: RpcResponse =
            serde_json::from_str(&resp_text).map_err(|e| SdkError::Decode(e.to_string()))?;

        if let Some(err) = resp.error {
            return Err(SdkError::Rpc {
                code: err.code,
                message: err.message,
            });
        }
        let result = resp.result.ok_or(SdkError::MissingResult)?;
        serde_json::from_value(result).map_err(|e| SdkError::Decode(e.to_string()))
    }

    /// Node metadata (network, height, installed prisms).
    pub fn node_info(&self) -> Result<NodeInfo, SdkError> {
        self.call(method::NODE_INFO, json!({}))
    }

    /// Current chain height.
    pub fn block_number(&self) -> Result<u64, SdkError> {
        self.call(method::BLOCK_NUMBER, json!({}))
    }

    /// Fetch a block by height.
    pub fn block_by_number(&self, height: u64) -> Result<BlockView, SdkError> {
        self.call(method::GET_BLOCK_BY_NUMBER, json!({ "height": height }))
    }

    /// Read a raw state value by key (hex-encoded bytes).
    pub fn get_state(&self, key: &str) -> Result<StateResult, SdkError> {
        self.call(method::GET_STATE, json!({ "key": key }))
    }

    /// Estimate the cost of a signed command without submitting it.
    pub fn estimate(&self, command: &SignedCommand) -> Result<EstimateResult, SdkError> {
        self.call(method::ESTIMATE, json!({ "command": command }))
    }

    /// Submit a signed command to the node's mempool.
    pub fn submit(&self, command: &SignedCommand) -> Result<SubmitResult, SdkError> {
        self.call(method::SUBMIT, json!({ "command": command }))
    }

    // ---- Explorer queries ----

    /// Chain-wide statistics (height, totals, per-prism event breakdown).
    pub fn explorer_stats(&self) -> Result<ChainStats, SdkError> {
        self.call(method::EXPLORER_STATS, json!({}))
    }

    /// The most recent blocks, newest first.
    pub fn explorer_recent_blocks(&self, limit: u64) -> Result<Vec<BlockView>, SdkError> {
        self.call(method::EXPLORER_RECENT_BLOCKS, json!({ "limit": limit }))
    }

    /// Look up a block by its hash.
    pub fn explorer_block_by_hash(&self, hash: &str) -> Result<BlockView, SdkError> {
        self.call(method::EXPLORER_BLOCK_BY_HASH, json!({ "hash": hash }))
    }

    /// Locate a command by id and return its block + produced events.
    pub fn explorer_search_command(&self, command_id: &str) -> Result<CommandLocation, SdkError> {
        self.call(
            method::EXPLORER_SEARCH_COMMAND,
            json!({ "command_id": command_id }),
        )
    }

    /// Recent events emitted by a given Prism.
    pub fn explorer_list_by_prism(
        &self,
        prism: &str,
        limit: u64,
    ) -> Result<Vec<EventView>, SdkError> {
        self.call(
            method::EXPLORER_LIST_BY_PRISM,
            json!({ "prism": prism, "limit": limit }),
        )
    }

    /// List state entries under a key prefix (e.g. "token/meta/").
    pub fn explorer_state_prefix(
        &self,
        prefix: &str,
        limit: u64,
    ) -> Result<StatePrefixResult, SdkError> {
        self.call(
            method::EXPLORER_STATE_PREFIX,
            json!({ "prefix": prefix, "limit": limit }),
        )
    }
}

/// Command builders for every shipped Prism, re-exported under one namespace so
/// applications need a single import.
pub mod builders {
    pub use prism_ai::{infer_command as ai_infer, register_command as ai_register};
    pub use prism_bridge::{
        register_chain_command as bridge_register_chain, send_command as bridge_send,
    };
    pub use prism_contract::{call_command as contract_call, deploy_command as contract_deploy};
    pub use prism_nft::create_collection_command as nft_create_collection;
    pub use prism_storage::put_command as storage_put;
    pub use prism_token::{create_command as token_create, transfer_command as token_transfer};
}
