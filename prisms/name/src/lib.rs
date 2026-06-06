use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use veilux_kernel::{
    Command, Event, Hash, PartyId, Prism, PrismError, PrismInfo, PrismOutput, StateTree, Visibility,
};

const REC_PREFIX: &str = "name/rec/";
const REV_PREFIX: &str = "name/rev/";
pub const ROOT_TLD: &str = "veil";
const MAX_LABEL_LEN: usize = 63;
const MAX_RECORDS: usize = 32;
const MAX_VALUE_LEN: usize = 320;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NameRecord {
    pub name: String,
    pub owner: PartyId,
    pub target: Option<PartyId>,
    pub registered_at: u64,
    pub expires_at: u64,
    pub records: BTreeMap<String, String>,
}

impl NameRecord {
    pub fn is_expired(&self, now: u64) -> bool {
        self.expires_at != 0 && now >= self.expires_at
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum NameCommand {
    Register {
        name: String,
        duration_secs: u64,
        target: Option<PartyId>,
    },
    Renew {
        name: String,
        duration_secs: u64,
    },
    Transfer {
        name: String,
        to: PartyId,
    },
    SetTarget {
        name: String,
        target: Option<PartyId>,
    },
    SetRecord {
        name: String,
        key: String,
        value: String,
    },
    ClearRecord {
        name: String,
        key: String,
    },
    Release {
        name: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NameEvent {
    Registered {
        name: String,
        owner: PartyId,
        expires_at: u64,
    },
    Renewed {
        name: String,
        expires_at: u64,
    },
    Transferred {
        name: String,
        from: PartyId,
        to: PartyId,
    },
    TargetSet {
        name: String,
        target: Option<PartyId>,
    },
    RecordSet {
        name: String,
        key: String,
    },
    RecordCleared {
        name: String,
        key: String,
    },
    Released {
        name: String,
    },
}

#[derive(Default)]
pub struct NamePrism;

impl NamePrism {
    pub fn new() -> Self {
        NamePrism
    }

    fn rec_key(name: &str) -> String {
        format!("{REC_PREFIX}{name}")
    }

    fn rev_key(target: &PartyId) -> String {
        format!("{REV_PREFIX}{}", target.0)
    }

    fn normalize(raw: &str) -> Result<String, PrismError> {
        let name = raw.trim().to_ascii_lowercase();
        let label = name.strip_suffix(&format!(".{ROOT_TLD}")).unwrap_or(&name);
        if label.is_empty() || label.len() > MAX_LABEL_LEN {
            return Err(PrismError::InvalidPayload(
                "name label must be 1..=63 chars".into(),
            ));
        }
        let ok = label
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !ok {
            return Err(PrismError::InvalidPayload(
                "name may only contain a-z, 0-9 and '-'".into(),
            ));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(PrismError::InvalidPayload(
                "name must not start or end with '-'".into(),
            ));
        }
        Ok(format!("{label}.{ROOT_TLD}"))
    }

    fn load(state: &StateTree, name: &str) -> Result<Option<NameRecord>, PrismError> {
        state
            .get_json::<NameRecord>(&Self::rec_key(name))
            .map_err(|e| PrismError::Internal(e.to_string()))
    }

    fn store(state: &mut StateTree, rec: &NameRecord) -> Result<(), PrismError> {
        state
            .put_json(Self::rec_key(&rec.name), rec)
            .map_err(|e| PrismError::Internal(e.to_string()))
    }

    fn set_reverse(
        state: &mut StateTree,
        target: &Option<PartyId>,
        name: &str,
    ) -> Result<(), PrismError> {
        if let Some(t) = target {
            state
                .put_json(Self::rev_key(t), &name.to_string())
                .map_err(|e| PrismError::Internal(e.to_string()))?;
        }
        Ok(())
    }

    fn clear_reverse(state: &mut StateTree, target: &Option<PartyId>, name: &str) {
        if let Some(t) = target {
            let key = Self::rev_key(t);
            if state.get_json::<String>(&key).ok().flatten().as_deref() == Some(name) {
                state.remove(&key);
            }
        }
    }

    fn now(state: &StateTree) -> u64 {
        state
            .get_json::<u64>("chain/now")
            .ok()
            .flatten()
            .unwrap_or(0)
    }

    fn event(cmd: &Command, payload: NameEvent) -> Event {
        Event {
            source_command: cmd.id(),
            prism: "name".into(),
            visibility: cmd.visibility.clone(),
            payload: serde_json::to_vec(&payload).unwrap_or_default(),
        }
    }

    fn require_owner(rec: &NameRecord, caller: &PartyId, now: u64) -> Result<(), PrismError> {
        if rec.is_expired(now) {
            return Err(PrismError::NotFound("name has expired".into()));
        }
        if &rec.owner != caller {
            return Err(PrismError::Unauthorized("not the name owner".into()));
        }
        Ok(())
    }
}

impl Prism for NamePrism {
    fn info(&self) -> PrismInfo {
        PrismInfo {
            name: "name",
            description: "Name service (VNS): human-readable names that resolve to parties",
            version: "1.0",
        }
    }

    fn handle(&self, command: &Command, state: &mut StateTree) -> Result<PrismOutput, PrismError> {
        let cmd: NameCommand = serde_json::from_slice(&command.payload)
            .map_err(|e| PrismError::InvalidPayload(e.to_string()))?;
        let now = Self::now(state);
        let caller = command.submitter.clone();

        match cmd {
            NameCommand::Register {
                name,
                duration_secs,
                target,
            } => {
                let name = Self::normalize(&name)?;
                if let Some(existing) = Self::load(state, &name)? {
                    if !existing.is_expired(now) {
                        return Err(PrismError::InvalidPayload("name already taken".into()));
                    }
                    Self::clear_reverse(state, &existing.target, &name);
                }
                let expires_at = if duration_secs == 0 {
                    0
                } else {
                    now.saturating_add(duration_secs)
                };
                let rec = NameRecord {
                    name: name.clone(),
                    owner: caller.clone(),
                    target: target.clone(),
                    registered_at: now,
                    expires_at,
                    records: BTreeMap::new(),
                };
                Self::store(state, &rec)?;
                Self::set_reverse(state, &target, &name)?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        NameEvent::Registered {
                            name,
                            owner: caller,
                            expires_at,
                        },
                    ),
                    5_000,
                ))
            }

            NameCommand::Renew {
                name,
                duration_secs,
            } => {
                let name = Self::normalize(&name)?;
                let mut rec = Self::load(state, &name)?
                    .ok_or_else(|| PrismError::NotFound("name not registered".into()))?;
                if rec.owner != caller {
                    return Err(PrismError::Unauthorized("not the name owner".into()));
                }
                let base = if rec.is_expired(now) {
                    now
                } else {
                    rec.expires_at
                };
                rec.expires_at = if rec.expires_at == 0 {
                    0
                } else {
                    base.saturating_add(duration_secs)
                };
                Self::store(state, &rec)?;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        NameEvent::Renewed {
                            name,
                            expires_at: rec.expires_at,
                        },
                    ),
                    2_000,
                ))
            }

            NameCommand::Transfer { name, to } => {
                let name = Self::normalize(&name)?;
                let mut rec = Self::load(state, &name)?
                    .ok_or_else(|| PrismError::NotFound("name not registered".into()))?;
                Self::require_owner(&rec, &caller, now)?;
                let from = rec.owner.clone();
                rec.owner = to.clone();
                Self::store(state, &rec)?;
                Ok(PrismOutput::single(
                    Self::event(command, NameEvent::Transferred { name, from, to }),
                    1_500,
                ))
            }

            NameCommand::SetTarget { name, target } => {
                let name = Self::normalize(&name)?;
                let mut rec = Self::load(state, &name)?
                    .ok_or_else(|| PrismError::NotFound("name not registered".into()))?;
                Self::require_owner(&rec, &caller, now)?;
                Self::clear_reverse(state, &rec.target, &name);
                rec.target = target.clone();
                Self::store(state, &rec)?;
                Self::set_reverse(state, &target, &name)?;
                Ok(PrismOutput::single(
                    Self::event(command, NameEvent::TargetSet { name, target }),
                    1_000,
                ))
            }

            NameCommand::SetRecord { name, key, value } => {
                let name = Self::normalize(&name)?;
                if key.is_empty() || key.len() > 64 {
                    return Err(PrismError::InvalidPayload(
                        "record key must be 1..=64 chars".into(),
                    ));
                }
                if value.len() > MAX_VALUE_LEN {
                    return Err(PrismError::LimitExceeded("record value too long".into()));
                }
                let mut rec = Self::load(state, &name)?
                    .ok_or_else(|| PrismError::NotFound("name not registered".into()))?;
                Self::require_owner(&rec, &caller, now)?;
                if !rec.records.contains_key(&key) && rec.records.len() >= MAX_RECORDS {
                    return Err(PrismError::LimitExceeded("too many records".into()));
                }
                rec.records.insert(key.clone(), value);
                Self::store(state, &rec)?;
                Ok(PrismOutput::single(
                    Self::event(command, NameEvent::RecordSet { name, key }),
                    900,
                ))
            }

            NameCommand::ClearRecord { name, key } => {
                let name = Self::normalize(&name)?;
                let mut rec = Self::load(state, &name)?
                    .ok_or_else(|| PrismError::NotFound("name not registered".into()))?;
                Self::require_owner(&rec, &caller, now)?;
                rec.records.remove(&key);
                Self::store(state, &rec)?;
                Ok(PrismOutput::single(
                    Self::event(command, NameEvent::RecordCleared { name, key }),
                    700,
                ))
            }

            NameCommand::Release { name } => {
                let name = Self::normalize(&name)?;
                let rec = Self::load(state, &name)?
                    .ok_or_else(|| PrismError::NotFound("name not registered".into()))?;
                if rec.owner != caller {
                    return Err(PrismError::Unauthorized("not the name owner".into()));
                }
                Self::clear_reverse(state, &rec.target, &name);
                state.remove(&Self::rec_key(&name));
                Ok(PrismOutput::single(
                    Self::event(command, NameEvent::Released { name }),
                    500,
                ))
            }
        }
    }

    fn estimate(&self, command: &Command, _state: &StateTree) -> u64 {
        match serde_json::from_slice::<NameCommand>(&command.payload) {
            Ok(NameCommand::Register { .. }) => 5_000,
            Ok(NameCommand::Renew { .. }) => 2_000,
            Ok(NameCommand::Transfer { .. }) => 1_500,
            Ok(_) => 1_000,
            Err(_) => 1_000,
        }
    }
}

pub fn id_of(name: &str) -> Hash {
    Hash::commit("name/id", &[name.as_bytes()])
}

pub fn lookup(state: &StateTree, name: &str) -> Option<NameRecord> {
    let name = NamePrism::normalize(name).ok()?;
    state
        .get_json::<NameRecord>(&NamePrism::rec_key(&name))
        .ok()
        .flatten()
}

pub fn resolve(state: &StateTree, name: &str) -> Option<PartyId> {
    let now = NamePrism::now(state);
    let rec = lookup(state, name)?;
    if rec.is_expired(now) {
        return None;
    }
    rec.target.or(Some(rec.owner))
}

pub fn reverse_lookup(state: &StateTree, target: &PartyId) -> Option<String> {
    let name: String = state.get_json(&NamePrism::rev_key(target)).ok().flatten()?;
    let rec = lookup(state, &name)?;
    let now = NamePrism::now(state);
    if rec.is_expired(now) || rec.target.as_ref() != Some(target) {
        return None;
    }
    Some(name)
}

pub fn register_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    name: &str,
    duration_secs: u64,
    target: Option<PartyId>,
) -> Command {
    let payload = serde_json::to_vec(&NameCommand::Register {
        name: name.to_string(),
        duration_secs,
        target,
    })
    .unwrap_or_default();
    Command {
        prism: "name".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_now(s: &mut StateTree, now: u64) {
        s.put_json("chain/now", &now).unwrap();
    }

    fn reg(name: &str, who: &str, dur: u64, target: Option<&str>) -> Command {
        register_command(
            PartyId::new(who),
            Visibility::Public,
            0,
            name,
            dur,
            target.map(PartyId::new),
        )
    }

    #[test]
    fn register_and_resolve() {
        let p = NamePrism::new();
        let mut s = StateTree::new();
        set_now(&mut s, 100);
        p.handle(&reg("alice", "party:alice", 0, None), &mut s)
            .unwrap();
        assert_eq!(resolve(&s, "alice.veil"), Some(PartyId::new("party:alice")));
        assert_eq!(resolve(&s, "alice"), Some(PartyId::new("party:alice")));
    }

    #[test]
    fn target_overrides_owner_and_reverse_works() {
        let p = NamePrism::new();
        let mut s = StateTree::new();
        set_now(&mut s, 0);
        p.handle(
            &reg("treasury", "party:dao", 0, Some("party:vault")),
            &mut s,
        )
        .unwrap();
        assert_eq!(
            resolve(&s, "treasury.veil"),
            Some(PartyId::new("party:vault"))
        );
        assert_eq!(
            reverse_lookup(&s, &PartyId::new("party:vault")),
            Some("treasury.veil".to_string())
        );
    }

    #[test]
    fn cannot_take_active_name() {
        let p = NamePrism::new();
        let mut s = StateTree::new();
        set_now(&mut s, 0);
        p.handle(&reg("brand", "party:a", 1000, None), &mut s)
            .unwrap();
        let dup = reg("brand", "party:b", 1000, None);
        assert!(p.handle(&dup, &mut s).is_err());
    }

    #[test]
    fn expired_name_can_be_reclaimed_and_stops_resolving() {
        let p = NamePrism::new();
        let mut s = StateTree::new();
        set_now(&mut s, 0);
        p.handle(&reg("temp", "party:a", 100, Some("party:a")), &mut s)
            .unwrap();
        set_now(&mut s, 100);
        assert_eq!(resolve(&s, "temp.veil"), None);
        p.handle(&reg("temp", "party:b", 100, Some("party:b")), &mut s)
            .unwrap();
        assert_eq!(resolve(&s, "temp.veil"), Some(PartyId::new("party:b")));
    }

    #[test]
    fn only_owner_can_transfer_and_set_records() {
        let p = NamePrism::new();
        let mut s = StateTree::new();
        set_now(&mut s, 0);
        p.handle(&reg("co", "party:owner", 0, None), &mut s)
            .unwrap();

        let bad = Command {
            prism: "name".into(),
            submitter: PartyId::new("party:intruder"),
            visibility: Visibility::Public,
            payload: serde_json::to_vec(&NameCommand::SetRecord {
                name: "co".into(),
                key: "url".into(),
                value: "https://x".into(),
            })
            .unwrap(),
            nonce: 1,
        };
        assert!(p.handle(&bad, &mut s).is_err());

        let xfer = Command {
            prism: "name".into(),
            submitter: PartyId::new("party:owner"),
            visibility: Visibility::Public,
            payload: serde_json::to_vec(&NameCommand::Transfer {
                name: "co".into(),
                to: PartyId::new("party:new"),
            })
            .unwrap(),
            nonce: 1,
        };
        p.handle(&xfer, &mut s).unwrap();
        assert_eq!(lookup(&s, "co").unwrap().owner, PartyId::new("party:new"));
    }

    #[test]
    fn rejects_invalid_labels() {
        let p = NamePrism::new();
        let mut s = StateTree::new();
        set_now(&mut s, 0);
        for bad in ["-lead", "trail-", "white space", "emoji😀", ""] {
            assert!(
                p.handle(&reg(bad, "party:a", 0, None), &mut s).is_err(),
                "label '{bad}' should be rejected"
            );
        }
    }
}
