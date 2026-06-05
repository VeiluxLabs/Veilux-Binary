use serde::{Deserialize, Serialize};

use veilux_kernel::{Event, Hash, PartyId};

use crate::view::{EncryptedView, ViewError, ViewKeyring};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubLedgerEntry {
    pub height: u64,
    pub commitment: Hash,
    pub event: Event,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SubLedger {
    pub party: Option<PartyId>,
    pub entries: Vec<SubLedgerEntry>,
    pub validated_root: Hash,
    pub height: u64,
}

impl SubLedger {
    pub fn new(party: PartyId) -> Self {
        Self {
            party: Some(party),
            entries: Vec::new(),
            validated_root: Hash::ZERO,
            height: 0,
        }
    }

    pub fn apply_views(
        &mut self,
        height: u64,
        expected_root: Hash,
        views: &[EncryptedView],
        keyring: &ViewKeyring,
    ) -> Result<usize, ViewError> {
        let mut applied = 0;
        for view in views {
            let event = keyring.open(view)?;
            self.entries.push(SubLedgerEntry {
                height,
                commitment: view.commitment,
                event,
            });
            applied += 1;
        }
        self.height = height;
        self.validated_root = expected_root;
        Ok(applied)
    }

    pub fn entries_for_prism<'a>(
        &'a self,
        prism: &'a str,
    ) -> impl Iterator<Item = &'a SubLedgerEntry> {
        self.entries.iter().filter(move |e| e.event.prism == prism)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains_commitment(&self, commitment: &Hash) -> bool {
        self.entries.iter().any(|e| &e.commitment == commitment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::project_block;
    use veilux_kernel::{Block, Visibility};

    fn ev(parties: Vec<&str>, payload: &[u8]) -> Event {
        Event {
            source_command: Hash::digest(payload),
            prism: "token".into(),
            visibility: Visibility::Parties(parties.into_iter().map(PartyId::new).collect()),
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn sub_ledger_collects_only_visible_events() {
        let alice = ViewKeyring::from_passphrase(PartyId::new("alice"), "a");

        let mut block = Block::genesis(PartyId::new("alice"), 0);
        block.height = 7;
        block.events = vec![
            ev(vec!["alice", "bob"], b"shared"),
            ev(vec!["bob", "carol"], b"not-for-alice"),
        ];

        let proj = project_block(&block, std::slice::from_ref(&alice)).unwrap();
        let alice_views = proj.views_by_party.get(&PartyId::new("alice")).unwrap();

        let mut ledger = SubLedger::new(PartyId::new("alice"));
        let applied = ledger
            .apply_views(block.height, proj.events_root, alice_views, &alice)
            .unwrap();

        assert_eq!(applied, 1);
        assert_eq!(ledger.height, 7);
        assert_eq!(ledger.validated_root, proj.events_root);
        assert_eq!(ledger.entries[0].event.payload, b"shared");
    }
}
