pub mod cascade;
pub mod crypto;
pub mod prism;
pub mod state;
pub mod types;

pub use cascade::{Cascade, CascadeError, CascadeReceipt};
pub use crypto::{merkle_root as merkle_root_of, Hash};
pub use prism::{Prism, PrismError, PrismInfo, PrismOutput};
pub use state::{StateError, StateTree};
pub use types::{Block, Command, Event, PartyId, SignedCommand, Visibility};

pub const PROTOCOL_VERSION: &str = "photon/1.0";

pub const TOKEN_TICKER: &str = "LUX";

pub const TOKEN_SUBUNIT: &str = "ray";

pub const TOKEN_DECIMALS: u32 = 18;

pub fn lux(whole: u64) -> u128 {
    (whole as u128) * 10u128.pow(TOKEN_DECIMALS)
}
