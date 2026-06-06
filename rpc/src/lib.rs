pub mod types;

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "server")]
pub mod ws;

pub use types::*;

pub mod method {
    pub const NODE_INFO: &str = "veilux_nodeInfo";
    pub const SUBMIT: &str = "veilux_submit";
    pub const BLOCK_NUMBER: &str = "veilux_blockNumber";
    pub const GET_BLOCK_BY_NUMBER: &str = "veilux_getBlockByNumber";
    pub const GET_STATE: &str = "veilux_getState";
    pub const ESTIMATE: &str = "veilux_estimate";
    pub const GET_ACCOUNT: &str = "veilux_getAccount";
    pub const SUBMIT_PRIVATE: &str = "veilux_submitPrivate";
    pub const PRIVATE_ROOT: &str = "veilux_privateRoot";

    pub const EXPLORER_STATS: &str = "explorer_stats";
    pub const EXPLORER_RECENT_BLOCKS: &str = "explorer_recentBlocks";
    pub const EXPLORER_BLOCK_BY_HASH: &str = "explorer_blockByHash";
    pub const EXPLORER_SEARCH_COMMAND: &str = "explorer_searchCommand";
    pub const EXPLORER_LIST_BY_PRISM: &str = "explorer_listByPrism";
    pub const EXPLORER_STATE_PREFIX: &str = "explorer_statePrefix";

    pub const CONTRACT_GET_CODE: &str = "contract_getCode";
    pub const CONTRACT_VERIFY: &str = "contract_verify";
    pub const CONTRACT_GET_VERIFICATION: &str = "contract_getVerification";
}
