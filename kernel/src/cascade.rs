use std::collections::HashMap;

use crate::prism::{Prism, PrismError, PrismInfo};
use crate::state::StateTree;
use crate::types::{Command, Event};

const MAX_CASCADE_DEPTH: usize = 8;

#[derive(Debug, thiserror::Error)]
pub enum CascadeError {
    #[error("no prism registered for '{0}'")]
    UnknownPrism(String),

    #[error("cascade depth limit ({0}) exceeded")]
    DepthExceeded(usize),

    #[error(transparent)]
    Prism(#[from] PrismError),
}

#[derive(Default)]
pub struct CascadeReceipt {
    pub events: Vec<Event>,
    pub total_cost: u64,
    pub depth: usize,
}

#[derive(Default)]
pub struct Cascade {
    prisms: HashMap<String, Box<dyn Prism>>,
    order: Vec<String>,
}

impl Cascade {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn install(&mut self, prism: Box<dyn Prism>) -> &mut Self {
        let name = prism.name().to_string();
        if !self.prisms.contains_key(&name) {
            self.order.push(name.clone());
        }
        self.prisms.insert(name, prism);
        self
    }

    pub fn has(&self, name: &str) -> bool {
        self.prisms.contains_key(name)
    }

    pub fn installed(&self) -> Vec<PrismInfo> {
        self.order
            .iter()
            .filter_map(|n| self.prisms.get(n))
            .map(|p| p.info())
            .collect()
    }

    pub fn estimate(&self, command: &Command, state: &StateTree) -> Result<u64, CascadeError> {
        let prism = self
            .prisms
            .get(&command.prism)
            .ok_or_else(|| CascadeError::UnknownPrism(command.prism.clone()))?;
        Ok(prism.estimate(command, state))
    }

    pub fn apply(
        &self,
        command: Command,
        state: &mut StateTree,
    ) -> Result<CascadeReceipt, CascadeError> {
        let mut receipt = CascadeReceipt::default();
        let mut frontier = vec![command];
        let mut depth = 0usize;

        while !frontier.is_empty() {
            if depth >= MAX_CASCADE_DEPTH {
                return Err(CascadeError::DepthExceeded(MAX_CASCADE_DEPTH));
            }

            let mut next_frontier = Vec::new();
            for cmd in frontier.drain(..) {
                let prism = self
                    .prisms
                    .get(&cmd.prism)
                    .ok_or_else(|| CascadeError::UnknownPrism(cmd.prism.clone()))?;

                let output = prism.handle(&cmd, state)?;
                receipt.total_cost += output.cost;
                receipt.events.extend(output.events);
                next_frontier.extend(output.derived_commands);
            }

            frontier = next_frontier;
            depth += 1;
        }

        receipt.depth = depth;
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prism::{PrismInfo, PrismOutput};
    use crate::types::{PartyId, Visibility};

    struct EchoPrism;
    impl Prism for EchoPrism {
        fn info(&self) -> PrismInfo {
            PrismInfo {
                name: "echo",
                description: "test",
                version: "0",
            }
        }
        fn handle(&self, cmd: &Command, _s: &mut StateTree) -> Result<PrismOutput, PrismError> {
            let event = Event {
                source_command: cmd.id(),
                prism: "echo".into(),
                visibility: Visibility::Public,
                payload: cmd.payload.clone(),
            };
            let derived = Command {
                prism: "echo2".into(),
                submitter: cmd.submitter.clone(),
                visibility: Visibility::Public,
                payload: b"derived".to_vec(),
                nonce: 0,
            };
            Ok(PrismOutput {
                events: vec![event],
                derived_commands: vec![derived],
                cost: 10,
            })
        }
    }

    struct Echo2Prism;
    impl Prism for Echo2Prism {
        fn info(&self) -> PrismInfo {
            PrismInfo {
                name: "echo2",
                description: "test",
                version: "0",
            }
        }
        fn handle(&self, cmd: &Command, _s: &mut StateTree) -> Result<PrismOutput, PrismError> {
            let event = Event {
                source_command: cmd.id(),
                prism: "echo2".into(),
                visibility: Visibility::Public,
                payload: cmd.payload.clone(),
            };
            Ok(PrismOutput::single(event, 5))
        }
    }

    #[test]
    fn cascade_chains_prisms() {
        let mut c = Cascade::new();
        c.install(Box::new(EchoPrism)).install(Box::new(Echo2Prism));

        let mut state = StateTree::new();
        let cmd = Command {
            prism: "echo".into(),
            submitter: PartyId::new("alice"),
            visibility: Visibility::Public,
            payload: b"hello".to_vec(),
            nonce: 1,
        };

        let receipt = c.apply(cmd, &mut state).unwrap();
        assert_eq!(receipt.events.len(), 2);
        assert_eq!(receipt.total_cost, 15);
        assert_eq!(receipt.depth, 2);
    }

    #[test]
    fn unknown_prism_errors() {
        let c = Cascade::new();
        let mut state = StateTree::new();
        let cmd = Command {
            prism: "ghost".into(),
            submitter: PartyId::new("bob"),
            visibility: Visibility::Public,
            payload: vec![],
            nonce: 0,
        };
        assert!(matches!(
            c.apply(cmd, &mut state),
            Err(CascadeError::UnknownPrism(_))
        ));
    }
}
