pub mod disclosure;
pub mod identity;
pub mod ledger;
pub mod projection;
pub mod view;

pub use disclosure::{audit_open, grant_disclosure, AuditableEntry, DisclosureGrant, GrantScope};
pub use identity::{verify_signed, IdentityError, PartyIdentity};
pub use ledger::{SubLedger, SubLedgerEntry};
pub use projection::{project_block, Projection};
pub use view::{EncryptedView, ViewError, ViewKeyring};

use veilux_kernel::Hash;

pub const VIEW_KEY_DOMAIN: &str = "veilux/veil/view-key/v1";

pub fn derive_view_key(party_seed: &[u8], view_id: &Hash) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(VIEW_KEY_DOMAIN.as_bytes());
    hasher.update(party_seed);
    hasher.update(view_id.as_bytes());
    *hasher.finalize().as_bytes()
}
