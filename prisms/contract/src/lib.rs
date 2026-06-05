pub mod vm;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use veilux_kernel::{
    Command, Event, Hash, PartyId, Prism, PrismError, PrismInfo, PrismOutput, StateTree, Visibility,
};

use vm::{ExecContext, Vm};

const CODE_PREFIX: &str = "contract/code/";
const STORE_PREFIX: &str = "contract/store/";
const MAX_CODE_SIZE: usize = 48 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContractMeta {
    pub address: Hash,
    pub deployer: PartyId,
    pub code: Vec<u8>,
    pub code_size: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ContractCommand {
    Deploy {
        code: Vec<u8>,
    },
    Call {
        address: Hash,
        args: Vec<u64>,
        value: u64,
        gas_limit: u64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContractEvent {
    Deployed {
        address: Hash,
        code_size: usize,
    },
    Called {
        address: Hash,
        return_value: Option<u64>,
        gas_used: u64,
        logs: Vec<u64>,
        reverted: bool,
    },
}

#[derive(Default)]
pub struct ContractPrism;

impl ContractPrism {
    pub fn new() -> Self {
        ContractPrism
    }

    fn code_key(addr: &Hash) -> String {
        format!("{CODE_PREFIX}{}", addr.to_hex())
    }

    fn store_key(addr: &Hash) -> String {
        format!("{STORE_PREFIX}{}", addr.to_hex())
    }

    fn load_storage(state: &StateTree, addr: &Hash) -> HashMap<u64, u64> {
        state
            .get_json(&Self::store_key(addr))
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    fn event(cmd: &Command, payload: ContractEvent) -> Event {
        Event {
            source_command: cmd.id(),
            prism: "contract".into(),
            visibility: cmd.visibility.clone(),
            payload: serde_json::to_vec(&payload).unwrap_or_default(),
        }
    }
}

impl Prism for ContractPrism {
    fn info(&self) -> PrismInfo {
        PrismInfo {
            name: "contract",
            description: "PhotonVM: deterministic stack-based smart contract execution",
            version: "1.0",
        }
    }

    fn handle(&self, command: &Command, state: &mut StateTree) -> Result<PrismOutput, PrismError> {
        let cmd: ContractCommand = serde_json::from_slice(&command.payload)
            .map_err(|e| PrismError::InvalidPayload(e.to_string()))?;

        match cmd {
            ContractCommand::Deploy { code } => {
                if code.len() > MAX_CODE_SIZE {
                    return Err(PrismError::LimitExceeded(format!(
                        "code size {} exceeds max {}",
                        code.len(),
                        MAX_CODE_SIZE
                    )));
                }
                let address = Hash::commit(
                    "contract/address",
                    &[
                        command.submitter.0.as_bytes(),
                        &command.nonce.to_le_bytes(),
                        &code,
                    ],
                );
                let meta = ContractMeta {
                    address,
                    deployer: command.submitter.clone(),
                    code_size: code.len(),
                    code,
                };
                state
                    .put_json(Self::code_key(&address), &meta)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;

                let cost = 32_000 + meta.code_size as u64 * 200;
                Ok(PrismOutput::single(
                    Self::event(
                        command,
                        ContractEvent::Deployed {
                            address,
                            code_size: meta.code_size,
                        },
                    ),
                    cost,
                ))
            }

            ContractCommand::Call {
                address,
                args,
                value,
                gas_limit,
            } => {
                let meta: ContractMeta = state
                    .get_json(&Self::code_key(&address))
                    .map_err(|e| PrismError::Internal(e.to_string()))?
                    .ok_or_else(|| {
                        PrismError::NotFound(format!("contract {}", address.to_hex()))
                    })?;

                let mut storage = Self::load_storage(state, &address);

                let caller_hash = {
                    let h = command.submitter.id_hash();
                    u64::from_be_bytes(h.as_bytes()[..8].try_into().unwrap())
                };
                let ctx = ExecContext {
                    caller_hash,
                    call_value: value,
                    args,
                };

                let capped_gas = gas_limit.min(10_000_000);
                let result = {
                    let mut machine = Vm::new(&meta.code, capped_gas, &mut storage, &ctx);
                    machine.run()
                };

                match result {
                    Ok(exec) => {
                        state
                            .put_json(Self::store_key(&address), &storage)
                            .map_err(|e| PrismError::Internal(e.to_string()))?;
                        Ok(PrismOutput::single(
                            Self::event(
                                command,
                                ContractEvent::Called {
                                    address,
                                    return_value: exec.return_value,
                                    gas_used: exec.gas_used,
                                    logs: exec.logs,
                                    reverted: false,
                                },
                            ),
                            exec.gas_used,
                        ))
                    }
                    Err(_e) => Ok(PrismOutput::single(
                        Self::event(
                            command,
                            ContractEvent::Called {
                                address,
                                return_value: None,
                                gas_used: capped_gas,
                                logs: vec![],
                                reverted: true,
                            },
                        ),
                        500,
                    )),
                }
            }
        }
    }

    fn estimate(&self, command: &Command, _state: &StateTree) -> u64 {
        match serde_json::from_slice::<ContractCommand>(&command.payload) {
            Ok(ContractCommand::Deploy { code }) => 32_000 + code.len() as u64 * 200,
            Ok(ContractCommand::Call { gas_limit, .. }) => gas_limit.min(100_000),
            Err(_) => 1_000,
        }
    }
}

pub fn deploy_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    code: Vec<u8>,
) -> Command {
    let payload = serde_json::to_vec(&ContractCommand::Deploy { code }).unwrap_or_default();
    Command {
        prism: "contract".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

pub fn call_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    address: Hash,
    args: Vec<u64>,
    value: u64,
    gas_limit: u64,
) -> Command {
    let payload = serde_json::to_vec(&ContractCommand::Call {
        address,
        args,
        value,
        gas_limit,
    })
    .unwrap_or_default();
    Command {
        prism: "contract".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vm::{ADD, PUSH8, RETURN, SLOAD, SSTORE};

    fn deployed_address(out: &PrismOutput) -> Hash {
        match serde_json::from_slice::<ContractEvent>(&out.events[0].payload).unwrap() {
            ContractEvent::Deployed { address, .. } => address,
            _ => panic!("expected Deployed"),
        }
    }

    #[test]
    fn deploy_and_call_adder() {
        let p = ContractPrism::new();
        let mut s = StateTree::new();

        let mut code = vec![PUSH8];
        code.extend_from_slice(&10u64.to_be_bytes());
        code.push(PUSH8);
        code.extend_from_slice(&20u64.to_be_bytes());
        code.push(ADD);
        code.push(RETURN);

        let dep = deploy_command(PartyId::new("alice"), Visibility::Public, 0, code);
        let addr = deployed_address(&p.handle(&dep, &mut s).unwrap());

        let call = call_command(
            PartyId::new("bob"),
            Visibility::Public,
            0,
            addr,
            vec![],
            0,
            1_000_000,
        );
        let out = p.handle(&call, &mut s).unwrap();
        match serde_json::from_slice::<ContractEvent>(&out.events[0].payload).unwrap() {
            ContractEvent::Called {
                return_value,
                reverted,
                ..
            } => {
                assert!(!reverted);
                assert_eq!(return_value, Some(30));
            }
            _ => panic!("expected Called"),
        }
    }

    #[test]
    fn persistent_storage_across_calls() {
        let p = ContractPrism::new();
        let mut s = StateTree::new();

        let mut code = vec![PUSH8];
        code.extend_from_slice(&7u64.to_be_bytes());
        code.push(PUSH8);
        code.extend_from_slice(&1u64.to_be_bytes());
        code.push(SSTORE);
        code.push(PUSH8);
        code.extend_from_slice(&1u64.to_be_bytes());
        code.push(SLOAD);
        code.push(RETURN);

        let dep = deploy_command(PartyId::new("alice"), Visibility::Public, 0, code);
        let addr = deployed_address(&p.handle(&dep, &mut s).unwrap());

        let call = call_command(
            PartyId::new("alice"),
            Visibility::Public,
            1,
            addr,
            vec![],
            0,
            1_000_000,
        );
        let out = p.handle(&call, &mut s).unwrap();
        match serde_json::from_slice::<ContractEvent>(&out.events[0].payload).unwrap() {
            ContractEvent::Called { return_value, .. } => assert_eq!(return_value, Some(7)),
            _ => panic!(),
        }
        assert!(s.contains(&format!("{}{}", STORE_PREFIX, addr.to_hex())));
    }

    #[test]
    fn reverting_call_is_recorded() {
        let p = ContractPrism::new();
        let mut s = StateTree::new();
        let code = vec![0xEE];
        let dep = deploy_command(PartyId::new("alice"), Visibility::Public, 0, code);
        let addr = deployed_address(&p.handle(&dep, &mut s).unwrap());
        let call = call_command(
            PartyId::new("alice"),
            Visibility::Public,
            1,
            addr,
            vec![],
            0,
            1_000_000,
        );
        let out = p.handle(&call, &mut s).unwrap();
        match serde_json::from_slice::<ContractEvent>(&out.events[0].payload).unwrap() {
            ContractEvent::Called { reverted, .. } => assert!(reverted),
            _ => panic!(),
        }
    }
}
