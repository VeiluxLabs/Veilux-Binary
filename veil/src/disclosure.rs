use serde::{Deserialize, Serialize};

use veilux_kernel::{Hash, PartyId};

use crate::view::{EncryptedView, ViewError, ViewKeyring};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GrantScope {
    All,
    Prism(String),
    HeightRange { from: u64, to: u64 },
    Events(Vec<Hash>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DisclosureGrant {
    pub grantor: PartyId,
    pub grantee: PartyId,
    pub scope: GrantScope,
    pub keys: Vec<(Hash, [u8; 32])>,
    pub justification: String,
}

impl DisclosureGrant {
    pub fn covers(&self, commitment: &Hash) -> bool {
        self.keys.iter().any(|(c, _)| c == commitment)
    }

    pub fn disclosed_count(&self) -> usize {
        self.keys.len()
    }
}

pub struct AuditableEntry<'a> {
    pub height: u64,
    pub prism: &'a str,
    pub view: &'a EncryptedView,
}

pub fn grant_disclosure(
    grantor_keyring: &ViewKeyring,
    grantee: PartyId,
    scope: GrantScope,
    entries: &[AuditableEntry<'_>],
    justification: impl Into<String>,
) -> DisclosureGrant {
    let mut keys = Vec::new();
    for entry in entries {
        if !scope_matches(&scope, entry) {
            continue;
        }
        let key = grantor_keyring.key_for(&entry.view.commitment);
        keys.push((entry.view.commitment, key));
    }
    DisclosureGrant {
        grantor: grantor_keyring.party().clone(),
        grantee,
        scope,
        keys,
        justification: justification.into(),
    }
}

pub fn audit_open(
    grant: &DisclosureGrant,
    views: &[EncryptedView],
) -> Result<Vec<veilux_kernel::Event>, ViewError> {
    let mut out = Vec::new();
    for view in views {
        if let Some((_, key)) = grant.keys.iter().find(|(c, _)| *c == view.commitment) {
            let event = view.open(key)?;
            out.push(event);
        }
    }
    Ok(out)
}

fn scope_matches(scope: &GrantScope, entry: &AuditableEntry<'_>) -> bool {
    match scope {
        GrantScope::All => true,
        GrantScope::Prism(p) => entry.prism == p,
        GrantScope::HeightRange { from, to } => entry.height >= *from && entry.height <= *to,
        GrantScope::Events(set) => set.contains(&entry.view.commitment),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veilux_kernel::{Event, Visibility};

    fn make_view(keyring: &ViewKeyring, prism: &str, payload: &[u8]) -> EncryptedView {
        let event = Event {
            source_command: Hash::digest(payload),
            prism: prism.into(),
            visibility: Visibility::Parties(vec![keyring.party().clone()]),
            payload: payload.to_vec(),
        };
        keyring.seal(&event).unwrap()
    }

    #[test]
    fn grant_scoped_to_prism_only_discloses_that_prism() {
        let bank = ViewKeyring::from_passphrase(PartyId::new("bank"), "bank-seed");
        let token_view = make_view(&bank, "token", b"transfer 1000");
        let ai_view = make_view(&bank, "ai", b"private inference");

        let entries = vec![
            AuditableEntry {
                height: 1,
                prism: "token",
                view: &token_view,
            },
            AuditableEntry {
                height: 1,
                prism: "ai",
                view: &ai_view,
            },
        ];

        let grant = grant_disclosure(
            &bank,
            PartyId::new("regulator"),
            GrantScope::Prism("token".into()),
            &entries,
            "AML quarterly review",
        );

        assert_eq!(grant.disclosed_count(), 1);
        let opened = audit_open(&grant, &[token_view.clone(), ai_view.clone()]).unwrap();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].payload, b"transfer 1000");
    }

    #[test]
    fn grant_does_not_cover_out_of_scope_events() {
        let bank = ViewKeyring::from_passphrase(PartyId::new("bank"), "bank-seed");
        let ai_view = make_view(&bank, "ai", b"secret");
        let entries = vec![AuditableEntry {
            height: 5,
            prism: "ai",
            view: &ai_view,
        }];
        let grant = grant_disclosure(
            &bank,
            PartyId::new("regulator"),
            GrantScope::HeightRange { from: 1, to: 3 },
            &entries,
            "out of range",
        );
        assert_eq!(grant.disclosed_count(), 0);
        assert!(!grant.covers(&ai_view.commitment));
    }
}
