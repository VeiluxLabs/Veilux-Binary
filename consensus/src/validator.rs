use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use veilux_kernel::{Hash, PartyId};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Validator {
    pub party: PartyId,
    pub public_key: Vec<u8>,
    pub stake: u64,
    pub active: bool,
    pub missed: u64,
}

impl Validator {
    pub fn new(party: PartyId, public_key: Vec<u8>, stake: u64) -> Self {
        Validator {
            party,
            public_key,
            stake,
            active: true,
            missed: 0,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ValidatorSet {
    validators: BTreeMap<PartyId, Validator>,
}

impl ValidatorSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, v: Validator) {
        self.validators.insert(v.party.clone(), v);
    }

    pub fn get(&self, party: &PartyId) -> Option<&Validator> {
        self.validators.get(party)
    }

    pub fn is_validator(&self, party: &PartyId) -> bool {
        self.validators
            .get(party)
            .map(|v| v.active)
            .unwrap_or(false)
    }

    pub fn active(&self) -> Vec<&Validator> {
        let mut out: Vec<&Validator> = self.validators.values().filter(|v| v.active).collect();
        out.sort_by(|a, b| a.party.0.cmp(&b.party.0));
        out
    }

    pub fn active_count(&self) -> usize {
        self.validators.values().filter(|v| v.active).count()
    }

    pub fn total_stake(&self) -> u64 {
        self.validators
            .values()
            .filter(|v| v.active)
            .map(|v| v.stake)
            .sum()
    }

    pub fn quorum_threshold(&self) -> u64 {
        let total = self.total_stake();
        total * 2 / 3 + 1
    }

    pub fn proposer_for(&self, height: u64, round: u32) -> Option<PartyId> {
        let active = self.active();
        if active.is_empty() {
            return None;
        }
        let seed = height.wrapping_mul(31).wrapping_add(round as u64);
        let idx = (seed % active.len() as u64) as usize;
        Some(active[idx].party.clone())
    }

    pub fn record_proposed(&mut self, party: &PartyId) {
        if let Some(v) = self.validators.get_mut(party) {
            v.missed = 0;
        }
    }

    pub fn record_missed(&mut self, party: &PartyId, jail_threshold: u64) {
        if let Some(v) = self.validators.get_mut(party) {
            v.missed += 1;
            if v.missed >= jail_threshold {
                v.active = false;
            }
        }
    }

    pub fn hash(&self) -> Hash {
        let mut parts: Vec<Vec<u8>> = Vec::new();
        for v in self.active() {
            let mut p = Vec::new();
            p.extend_from_slice(v.party.0.as_bytes());
            p.extend_from_slice(&v.stake.to_le_bytes());
            parts.push(p);
        }
        let refs: Vec<&[u8]> = parts.iter().map(|p| p.as_slice()).collect();
        Hash::commit("validator-set", &refs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vs() -> ValidatorSet {
        let mut s = ValidatorSet::new();
        s.add(Validator::new(PartyId::new("v1"), vec![1; 32], 100));
        s.add(Validator::new(PartyId::new("v2"), vec![2; 32], 100));
        s.add(Validator::new(PartyId::new("v3"), vec![3; 32], 100));
        s.add(Validator::new(PartyId::new("v4"), vec![4; 32], 100));
        s
    }

    #[test]
    fn quorum_is_two_thirds_plus_one() {
        let s = vs();
        assert_eq!(s.total_stake(), 400);
        assert_eq!(s.quorum_threshold(), 267);
    }

    #[test]
    fn proposer_is_deterministic_and_rotates() {
        let s = vs();
        let p0 = s.proposer_for(0, 0).unwrap();
        let p0_again = s.proposer_for(0, 0).unwrap();
        assert_eq!(p0, p0_again);
        let seen: std::collections::HashSet<_> =
            (0..8).map(|h| s.proposer_for(h, 0).unwrap()).collect();
        assert!(seen.len() > 1);
    }

    #[test]
    fn jailing_after_misses() {
        let mut s = vs();
        for _ in 0..5 {
            s.record_missed(&PartyId::new("v1"), 5);
        }
        assert!(!s.is_validator(&PartyId::new("v1")));
        assert_eq!(s.active_count(), 3);
    }
}
