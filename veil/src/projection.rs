use std::collections::BTreeMap;

use veilux_kernel::{merkle_root_of, Block, Hash, PartyId, Visibility};

use crate::view::{EncryptedView, ViewError, ViewKeyring};

pub struct Projection {
    pub commitments: Vec<Hash>,
    pub events_root: Hash,
    pub views_by_party: BTreeMap<PartyId, Vec<EncryptedView>>,
}

impl Projection {
    pub fn view_count(&self) -> usize {
        self.views_by_party.values().map(|v| v.len()).sum()
    }
}

pub fn project_block(block: &Block, keyrings: &[ViewKeyring]) -> Result<Projection, ViewError> {
    let mut commitments = Vec::with_capacity(block.events.len());
    let mut views_by_party: BTreeMap<PartyId, Vec<EncryptedView>> = BTreeMap::new();

    for event in &block.events {
        let commitment = event.commitment();
        commitments.push(commitment);

        let stakeholders: Vec<PartyId> = match &event.visibility {
            Visibility::Public => keyrings.iter().map(|k| k.party().clone()).collect(),
            Visibility::Parties(set) => set.clone(),
        };

        for party in stakeholders {
            if let Some(keyring) = keyrings.iter().find(|k| *k.party() == party) {
                let view = keyring.seal(event)?;
                views_by_party.entry(party).or_default().push(view);
            }
        }
    }

    let events_root = merkle_root_of(&commitments);

    Ok(Projection {
        commitments,
        events_root,
        views_by_party,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use veilux_kernel::{Event, Visibility};

    fn event_for(parties: Vec<&str>, payload: &[u8]) -> Event {
        Event {
            source_command: Hash::digest(payload),
            prism: "token".into(),
            visibility: Visibility::Parties(parties.into_iter().map(PartyId::new).collect()),
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn party_only_sees_own_views() {
        let alice = ViewKeyring::from_passphrase(PartyId::new("alice"), "a");
        let bob = ViewKeyring::from_passphrase(PartyId::new("bob"), "b");

        let mut block = Block::genesis(PartyId::new("alice"), 0);
        block.events = vec![
            event_for(vec!["alice", "bob"], b"shared"),
            event_for(vec!["bob"], b"bob-only"),
        ];

        let proj = project_block(&block, std::slice::from_ref(&alice)).unwrap();
        let alice_views = proj.views_by_party.get(&PartyId::new("alice")).unwrap();
        assert_eq!(alice_views.len(), 1);

        let proj_bob = project_block(&block, std::slice::from_ref(&bob)).unwrap();
        let bob_views = proj_bob.views_by_party.get(&PartyId::new("bob")).unwrap();
        assert_eq!(bob_views.len(), 2);
    }

    #[test]
    fn events_root_matches_regardless_of_hosted_parties() {
        let alice = ViewKeyring::from_passphrase(PartyId::new("alice"), "a");
        let bob = ViewKeyring::from_passphrase(PartyId::new("bob"), "b");

        let mut block = Block::genesis(PartyId::new("alice"), 0);
        block.events = vec![
            event_for(vec!["alice", "bob"], b"shared"),
            event_for(vec!["bob"], b"bob-only"),
        ];

        let pa = project_block(&block, std::slice::from_ref(&alice)).unwrap();
        let pb = project_block(&block, std::slice::from_ref(&bob)).unwrap();
        assert_eq!(pa.events_root, pb.events_root);
    }
}
