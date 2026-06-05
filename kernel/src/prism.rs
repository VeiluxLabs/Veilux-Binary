use crate::state::StateTree;
use crate::types::{Command, Event};

#[derive(Debug, thiserror::Error)]
pub enum PrismError {
    #[error("invalid payload: {0}")]
    InvalidPayload(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("resource limit exceeded: {0}")]
    LimitExceeded(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("prism internal error: {0}")]
    Internal(String),
}

#[derive(Clone, Debug)]
pub struct PrismInfo {
    pub name: &'static str,
    pub description: &'static str,
    pub version: &'static str,
}

#[derive(Default)]
pub struct PrismOutput {
    pub events: Vec<Event>,
    pub derived_commands: Vec<Command>,
    pub cost: u64,
}

impl PrismOutput {
    pub fn single(event: Event, cost: u64) -> Self {
        PrismOutput {
            events: vec![event],
            derived_commands: vec![],
            cost,
        }
    }
}

pub trait Prism: Send + Sync {
    fn info(&self) -> PrismInfo;

    fn name(&self) -> &str {
        self.info().name
    }

    fn handle(&self, command: &Command, state: &mut StateTree) -> Result<PrismOutput, PrismError>;

    fn estimate(&self, _command: &Command, _state: &StateTree) -> u64 {
        1_000
    }
}
