use std::collections::{HashMap, HashSet};

use veilux_consensus::ValidatorSet;
use veilux_kernel::PartyId;

pub struct ViewCoordinator {
    height: u64,
    votes: HashMap<u32, HashSet<PartyId>>,
}

impl ViewCoordinator {
    pub fn new(height: u64) -> Self {
        ViewCoordinator {
            height,
            votes: HashMap::new(),
        }
    }

    pub fn reset(&mut self, height: u64) {
        self.height = height;
        self.votes.clear();
    }

    pub fn record(
        &mut self,
        height: u64,
        view: u32,
        voter: PartyId,
        vset: &ValidatorSet,
    ) -> Option<u32> {
        if height != self.height {
            return None;
        }
        if !vset.is_validator(&voter) {
            return None;
        }
        self.votes.entry(view).or_default().insert(voter);
        self.quorum_view(vset)
    }

    fn quorum_view(&self, vset: &ValidatorSet) -> Option<u32> {
        let quorum = vset.quorum_threshold();
        self.votes
            .iter()
            .filter(|(_, voters)| {
                let power: u64 = voters
                    .iter()
                    .filter_map(|p| vset.get(p))
                    .map(|v| v.stake)
                    .sum();
                power >= quorum
            })
            .map(|(view, _)| *view)
            .max()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veilux_consensus::Validator;

    fn vset(n: u8) -> ValidatorSet {
        let mut s = ValidatorSet::new();
        for i in 1..=n {
            s.add(Validator::new(
                PartyId::new(format!("v{i}")),
                vec![i; 32],
                100,
            ));
        }
        s
    }

    #[test]
    fn no_quorum_below_two_thirds() {
        let s = vset(4);
        let mut vc = ViewCoordinator::new(1);
        assert_eq!(vc.record(1, 1, PartyId::new("v1"), &s), None);
        assert_eq!(vc.record(1, 1, PartyId::new("v2"), &s), None);
    }

    #[test]
    fn quorum_at_two_thirds() {
        let s = vset(4);
        let mut vc = ViewCoordinator::new(1);
        vc.record(1, 1, PartyId::new("v1"), &s);
        vc.record(1, 1, PartyId::new("v2"), &s);
        assert_eq!(vc.record(1, 1, PartyId::new("v3"), &s), Some(1));
    }

    #[test]
    fn ignores_other_heights_and_nonvalidators() {
        let s = vset(4);
        let mut vc = ViewCoordinator::new(1);
        assert_eq!(vc.record(2, 1, PartyId::new("v1"), &s), None);
        assert_eq!(vc.record(1, 1, PartyId::new("ghost"), &s), None);
    }
}
