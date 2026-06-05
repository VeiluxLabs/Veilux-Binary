//! VEILUX JSON-RPC contract and server.
//!
//! This crate defines the wire types shared by the node's RPC server and the
//! `veilux-sdk` client, plus a featherweight HTTP/1.1 JSON-RPC server (behind
//! the `server` feature) with no heavyweight web framework.
//!
//! ## Methods
//! - `veilux_nodeInfo` -> [`types::NodeInfo`]
//! - `veilux_submit` ([`types::SubmitParams`]) -> [`types::SubmitResult`]
//! - `veilux_blockNumber` -> u64
//! - `veilux_getBlockByNumber` (u64) -> [`types::BlockView`]
//! - `veilux_getState` ([`types::StateQuery`]) -> [`types::StateResult`]
//! - `veilux_estimate` ([`types::SubmitParams`]) -> [`types::EstimateResult`]

pub mod types;

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "server")]
pub mod ws;

pub use types::*;

/// RPC method-name constants, shared by server and client to avoid typos.
pub mod method {
    pub const NODE_INFO: &str = "veilux_nodeInfo";
    pub const SUBMIT: &str = "veilux_submit";
    pub const BLOCK_NUMBER: &str = "veilux_blockNumber";
    pub const GET_BLOCK_BY_NUMBER: &str = "veilux_getBlockByNumber";
    pub const GET_STATE: &str = "veilux_getState";
    pub const ESTIMATE: &str = "veilux_estimate";

    // Explorer namespace — read-heavy queries for indexers and dashboards.
    pub const EXPLORER_STATS: &str = "explorer_stats";
    pub const EXPLORER_RECENT_BLOCKS: &str = "explorer_recentBlocks";
    pub const EXPLORER_BLOCK_BY_HASH: &str = "explorer_blockByHash";
    pub const EXPLORER_SEARCH_COMMAND: &str = "explorer_searchCommand";
    pub const EXPLORER_LIST_BY_PRISM: &str = "explorer_listByPrism";
    pub const EXPLORER_STATE_PREFIX: &str = "explorer_statePrefix";

    // Contract verification (explorer "verify source" feature).
    pub const CONTRACT_GET_CODE: &str = "contract_getCode";
    pub const CONTRACT_VERIFY: &str = "contract_verify";
    pub const CONTRACT_GET_VERIFICATION: &str = "contract_getVerification";
}
