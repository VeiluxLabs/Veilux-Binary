use std::collections::HashMap;

use tracing::{debug, info, warn};
use veilux_consensus::{Aurora, CommitOutcome, Vote, VoteKind};
use veilux_kernel::{Block, Hash, PartyId};

#[derive(Debug, Clone)]
pub enum Action {
    Broadcast(Outbound),
    Commit(Hash),
    None,
}

#[derive(Debug, Clone)]
pub enum Outbound {
    Proposal { round: u32, block: Box<Block> },
    Vote(Box<Vote>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Prevoted,
    Precommitted,
    Committed,
}

pub struct RoundMachine {
    pub height: u64,
    pub round: u32,
    pub phase: Phase,
    seen_blocks: HashMap<Hash, Block>,
    own_block: Option<Hash>,
    my_votes: Vec<Vote>,
}

impl RoundMachine {
    pub fn new_round(height: u64, round: u32) -> Self {
        RoundMachine {
            height,
            round,
            phase: Phase::Idle,
            seen_blocks: HashMap::new(),
            own_block: None,
            my_votes: Vec::new(),
        }
    }

    pub fn block(&self, hash: &Hash) -> Option<&Block> {
        self.seen_blocks.get(hash)
    }

    pub fn own_proposed_block(&self) -> Option<&Block> {
        self.own_block
            .as_ref()
            .and_then(|h| self.seen_blocks.get(h))
    }

    /// Votes this node has cast, for periodic re-broadcast so late-joining or
    /// lossy peers eventually receive them (gossip recovery).
    pub fn my_votes(&self) -> &[Vote] {
        &self.my_votes
    }

    fn make_vote(&self, me: &PartyId, block_hash: Hash, kind: VoteKind) -> Vote {
        Vote {
            height: self.height,
            round: self.round,
            block_hash,
            voter: me.clone(),
            kind,
            signature: vec![],
        }
    }

    /// Cast our own vote: count it into our own engine AND return it for
    /// broadcast. This is essential — votes are not looped back over the
    /// network, so a node must self-count to ever reach quorum.
    fn cast_own_vote(
        &mut self,
        me: &PartyId,
        block_hash: Hash,
        kind: VoteKind,
        aurora: &mut Aurora,
    ) -> (Vec<Action>, Option<CommitOutcome>) {
        let vote = self.make_vote(me, block_hash, kind);
        let outcome = aurora.add_vote(&vote).ok();
        self.my_votes.push(vote.clone());
        (
            vec![Action::Broadcast(Outbound::Vote(Box::new(vote)))],
            outcome,
        )
    }

    pub fn on_local_proposal(
        &mut self,
        block: Block,
        me: &PartyId,
        aurora: &mut Aurora,
    ) -> Vec<Action> {
        let hash = block.hash();
        self.seen_blocks.insert(hash, block.clone());
        self.own_block = Some(hash);

        let mut actions = vec![Action::Broadcast(Outbound::Proposal {
            round: self.round,
            block: Box::new(block),
        })];

        // Only prevote once (first time we propose this height); subsequent
        // ticks just re-broadcast the proposal for late peers.
        if aurora.validators.is_validator(me) && self.phase == Phase::Idle {
            self.phase = Phase::Prevoted;
            info!(height = self.height, %hash, "proposing block to network");
            let (vote_actions, _) = self.cast_own_vote(me, hash, VoteKind::Prevote, aurora);
            actions.extend(vote_actions);
        }
        actions
    }

    pub fn on_proposal(
        &mut self,
        block: Block,
        me: &PartyId,
        aurora: &mut Aurora,
        parent_hash: Hash,
    ) -> Vec<Action> {
        if block.height != self.height {
            return vec![Action::None];
        }
        if let Err(e) = aurora.verify_proposal(&block, parent_hash, self.round) {
            warn!(error = %e, "rejected invalid proposal");
            return vec![Action::None];
        }
        let hash = block.hash();
        self.seen_blocks.insert(hash, block);

        if aurora.validators.is_validator(me) && self.phase == Phase::Idle {
            self.phase = Phase::Prevoted;
            debug!(height = self.height, %hash, "casting prevote");
            let (actions, _) = self.cast_own_vote(me, hash, VoteKind::Prevote, aurora);
            return actions;
        }
        vec![Action::None]
    }

    pub fn on_vote(&mut self, vote: Vote, me: &PartyId, aurora: &mut Aurora) -> Vec<Action> {
        if vote.height != self.height {
            return vec![Action::None];
        }
        let block_hash = vote.block_hash;
        let kind = vote.kind;

        let outcome = match aurora.add_vote(&vote) {
            Ok(o) => o,
            Err(e) => {
                debug!(error = %e, "vote rejected");
                return vec![Action::None];
            }
        };

        let mut actions = Vec::new();
        self.collect_commit(&outcome, &mut actions);

        if kind == VoteKind::Prevote
            && self.phase == Phase::Prevoted
            && aurora.has_prevote_quorum(self.height, &block_hash)
            && aurora.validators.is_validator(me)
        {
            self.phase = Phase::Precommitted;
            debug!(height = self.height, %block_hash, "prevote quorum -> precommit");
            let (pc_actions, pc_outcome) =
                self.cast_own_vote(me, block_hash, VoteKind::Precommit, aurora);
            actions.extend(pc_actions);
            if let Some(o) = pc_outcome {
                self.collect_commit(&o, &mut actions);
            }
        }

        if actions.is_empty() {
            actions.push(Action::None);
        }
        actions
    }

    fn collect_commit(&mut self, outcome: &CommitOutcome, actions: &mut Vec<Action>) {
        if let CommitOutcome::Committed {
            block_hash, power, ..
        } = outcome
        {
            if self.phase != Phase::Committed {
                self.phase = Phase::Committed;
                info!(height = self.height, %block_hash, power, "finalized via network quorum");
                actions.push(Action::Commit(*block_hash));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veilux_consensus::{ConsensusConfig, Validator, ValidatorSet};

    fn validators(names: &[&str]) -> ValidatorSet {
        let mut vs = ValidatorSet::new();
        for (i, n) in names.iter().enumerate() {
            vs.add(Validator::new(
                PartyId::new(*n),
                vec![(i + 1) as u8; 32],
                100,
            ));
        }
        vs
    }

    fn block_at(height: u64, parent: Hash, proposer: &str) -> Block {
        let mut b = Block::genesis(PartyId::new(proposer), 1);
        b.height = height;
        b.parent = parent;
        b.events_root = b.compute_events_root();
        b
    }

    #[test]
    fn four_validators_reach_finality() {
        let names = ["v1", "v2", "v3", "v4"];
        let vs = validators(&names);
        let parent = Hash::ZERO;
        let height = 1;
        let proposer = vs.proposer_for(height, 0).unwrap();
        let block = block_at(height, parent, &proposer.0);
        let block_hash = block.hash();

        let mut engines: Vec<Aurora> = names
            .iter()
            .map(|n| {
                Aurora::new(
                    ConsensusConfig::default(),
                    vs.clone(),
                    Some(PartyId::new(*n)),
                )
            })
            .collect();
        let mut machines: Vec<RoundMachine> = names
            .iter()
            .map(|_| RoundMachine::new_round(height, 0))
            .collect();

        let mut prevotes = Vec::new();
        for (i, n) in names.iter().enumerate() {
            let acts = if PartyId::new(*n) == proposer {
                machines[i].on_local_proposal(block.clone(), &PartyId::new(*n), &mut engines[i])
            } else {
                machines[i].on_proposal(block.clone(), &PartyId::new(*n), &mut engines[i], parent)
            };
            for a in acts {
                if let Action::Broadcast(Outbound::Vote(v)) = a {
                    prevotes.push(*v);
                }
            }
        }
        assert_eq!(prevotes.len(), 4, "each validator casts one prevote");

        let mut precommits = Vec::new();
        let mut commits = 0;
        for (i, n) in names.iter().enumerate() {
            for pv in &prevotes {
                if pv.voter == PartyId::new(*n) {
                    continue;
                }
                for a in machines[i].on_vote(pv.clone(), &PartyId::new(*n), &mut engines[i]) {
                    match a {
                        Action::Broadcast(Outbound::Vote(v)) => precommits.push((i, *v)),
                        Action::Commit(_) => commits += 1,
                        _ => {}
                    }
                }
            }
        }
        assert!(precommits.len() >= 4, "each validator casts a precommit");

        for (i, n) in names.iter().enumerate() {
            for (src, pc) in &precommits {
                if *src == i {
                    continue;
                }
                for a in machines[i].on_vote(pc.clone(), &PartyId::new(*n), &mut engines[i]) {
                    if let Action::Commit(h) = a {
                        assert_eq!(h, block_hash);
                        commits += 1;
                    }
                }
            }
        }
        assert!(commits >= 4, "all validators commit; got {commits}");
        assert!(machines.iter().all(|m| m.phase == Phase::Committed));
    }
}
