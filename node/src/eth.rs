use prism_token::{move_balance, native_token_id};
use veilux_evm::u256::U256;
use veilux_evm::vm::{CallContext, Host, Interpreter, Log};
use veilux_evm::{decode_legacy_tx, keccak256, EvmError};
use veilux_kernel::{PartyId, StateTree};

use crate::node::Node;

pub fn eth_party(addr: &[u8; 20]) -> PartyId {
    PartyId::new(format!("eth:0x{}", hex::encode(addr)))
}

fn addr_to_u256(addr: &[u8; 20]) -> U256 {
    let mut buf = [0u8; 32];
    buf[12..].copy_from_slice(addr);
    U256::from_big_endian(&buf)
}

fn u256_to_addr(v: &U256) -> [u8; 20] {
    let b = v.to_big_endian();
    let mut a = [0u8; 20];
    a.copy_from_slice(&b[12..]);
    a
}

fn value_u256(v: u128) -> U256 {
    U256::from_big_endian(&v.to_be_bytes())
}

fn nonce_key(addr: &[u8; 20]) -> String {
    format!("eth/nonce/0x{}", hex::encode(addr))
}

fn code_key(addr: &[u8; 20]) -> String {
    format!("eth/code/0x{}", hex::encode(addr))
}

fn storage_key(addr: &[u8; 20], slot: &U256) -> String {
    format!(
        "eth/store/0x{}/0x{}",
        hex::encode(addr),
        hex::encode(slot.to_big_endian())
    )
}

fn receipt_key(hash: &[u8; 32]) -> String {
    format!("eth/receipt/0x{}", hex::encode(hash))
}

pub fn eth_nonce(state: &StateTree, addr: &[u8; 20]) -> u64 {
    state
        .get_json::<String>(&nonce_key(addr))
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

pub fn eth_balance(state: &StateTree, addr: &[u8; 20]) -> u128 {
    prism_token::balance_of(state, &native_token_id(), &eth_party(addr))
}

pub fn eth_code(state: &StateTree, addr: &[u8; 20]) -> Vec<u8> {
    state
        .get_json::<String>(&code_key(addr))
        .ok()
        .flatten()
        .and_then(|s| hex::decode(s).ok())
        .unwrap_or_default()
}

pub fn eth_receipt(state: &StateTree, hash: &[u8; 32]) -> Option<serde_json::Value> {
    state
        .get_json::<serde_json::Value>(&receipt_key(hash))
        .ok()
        .flatten()
}

pub fn contract_address(deployer: &[u8; 20], nonce: u64) -> [u8; 20] {
    let nonce_bytes = if nonce == 0 {
        vec![]
    } else {
        nonce
            .to_be_bytes()
            .iter()
            .copied()
            .skip_while(|&b| b == 0)
            .collect()
    };
    let rlp = veilux_evm::rlp::encode_list(&[
        veilux_evm::rlp::encode_str(deployer),
        veilux_evm::rlp::encode_str(&nonce_bytes),
    ]);
    let h = keccak256(&rlp);
    let mut a = [0u8; 20];
    a.copy_from_slice(&h[12..]);
    a
}

struct StateHost<'a> {
    state: &'a mut StateTree,
    chain_id: u64,
    block_number: u64,
    timestamp: u64,
}

impl Host for StateHost<'_> {
    fn sload(&self, address: &U256, key: &U256) -> U256 {
        let addr = u256_to_addr(address);
        self.state
            .get_json::<String>(&storage_key(&addr, key))
            .ok()
            .flatten()
            .and_then(|s| hex::decode(s).ok())
            .map(|b| U256::from_big_endian(&b))
            .unwrap_or(U256::ZERO)
    }
    fn sstore(&mut self, address: &U256, key: U256, value: U256) {
        let addr = u256_to_addr(address);
        let _ = self.state.put_json(
            storage_key(&addr, &key),
            &hex::encode(value.to_big_endian()),
        );
    }
    fn balance(&self, address: &U256) -> U256 {
        let addr = u256_to_addr(address);
        let bal = prism_token::balance_of(self.state, &native_token_id(), &eth_party(&addr));
        U256::from_big_endian(&bal.to_be_bytes())
    }
    fn block_number(&self) -> u64 {
        self.block_number
    }
    fn block_timestamp(&self) -> u64 {
        self.timestamp
    }
    fn chain_id(&self) -> u64 {
        self.chain_id
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EthError {
    #[error("decode: {0}")]
    Decode(#[from] EvmError),
    #[error("wrong chain id: tx signed for {got:?}, node chain is {expected}")]
    WrongChainId { got: Option<u64>, expected: u64 },
    #[error("bad nonce: tx nonce {got}, account nonce {expected}")]
    BadNonce { got: u64, expected: u64 },
    #[error("evm execution error: {0}")]
    Vm(String),
    #[error("evm reverted")]
    Reverted,
    #[error("node: {0}")]
    Node(String),
}

pub struct EthApplied {
    pub hash: [u8; 32],
    pub from: [u8; 20],
    pub to: Option<[u8; 20]>,
    pub contract_address: Option<[u8; 20]>,
    pub value: u128,
    pub nonce: u64,
    pub gas_used: u64,
    pub logs: Vec<Log>,
}

impl Node {
    pub fn eth_call(&self, to: &[u8; 20], calldata: Vec<u8>) -> Result<Vec<u8>, EthError> {
        let code = eth_code(&self.state, to);
        if code.is_empty() {
            return Ok(vec![]);
        }
        let mut probe = self.state.clone();
        let mut host = StateHost {
            state: &mut probe,
            chain_id: self.chain_id,
            block_number: self.head().height,
            timestamp: self.head().timestamp,
        };
        let ctx = CallContext {
            caller: U256::ZERO,
            address: addr_to_u256(to),
            value: U256::ZERO,
            calldata,
            gas_limit: 50_000_000,
        };
        let out = Interpreter::new(&code, &ctx, &mut host)
            .run()
            .map_err(|e| EthError::Vm(e.to_string()))?;
        Ok(out.return_data)
    }

    pub fn eth_apply_raw(&mut self, raw: &[u8]) -> Result<EthApplied, EthError> {
        let tx = decode_legacy_tx(raw)?;

        if let Some(cid) = tx.chain_id {
            if cid != self.chain_id {
                return Err(EthError::WrongChainId {
                    got: Some(cid),
                    expected: self.chain_id,
                });
            }
        }

        let expected = eth_nonce(&self.state, &tx.from);
        if tx.nonce != expected {
            return Err(EthError::BadNonce {
                got: tx.nonce,
                expected,
            });
        }

        let block_number = self.head().height + 1;
        let timestamp = self.head().timestamp;
        let mut new_state = self.state.clone();
        let mut gas_used = 21_000u64;
        let mut logs: Vec<Log> = Vec::new();
        let mut created: Option<[u8; 20]> = None;

        match tx.to {
            None => {
                let new_addr = contract_address(&tx.from, tx.nonce);
                let mut host = StateHost {
                    state: &mut new_state,
                    chain_id: self.chain_id,
                    block_number,
                    timestamp,
                };
                let ctx = CallContext {
                    caller: addr_to_u256(&tx.from),
                    address: addr_to_u256(&new_addr),
                    value: value_u256(tx.value),
                    calldata: tx.data.clone(),
                    gas_limit: tx.gas_limit.max(1_000_000),
                };
                let out = Interpreter::new(&tx.data, &ctx, &mut host)
                    .run()
                    .map_err(|e| EthError::Vm(e.to_string()))?;
                if !out.success {
                    return Err(EthError::Reverted);
                }
                new_state
                    .put_json(code_key(&new_addr), &hex::encode(&out.return_data))
                    .map_err(|e| EthError::Node(e.to_string()))?;
                gas_used = gas_used.saturating_add(out.gas_used);
                logs = out.logs;
                created = Some(new_addr);
            }
            Some(to) => {
                let code = eth_code(&new_state, &to);
                if code.is_empty() {
                    if !tx.data.is_empty() {
                        return Err(EthError::Vm("call to account with no code".into()));
                    }
                    if tx.value > 0 {
                        move_balance(
                            &mut new_state,
                            &native_token_id(),
                            &eth_party(&tx.from),
                            &eth_party(&to),
                            tx.value,
                        )
                        .map_err(|e| EthError::Node(e.to_string()))?;
                    }
                } else {
                    if tx.value > 0 {
                        move_balance(
                            &mut new_state,
                            &native_token_id(),
                            &eth_party(&tx.from),
                            &eth_party(&to),
                            tx.value,
                        )
                        .map_err(|e| EthError::Node(e.to_string()))?;
                    }
                    let mut host = StateHost {
                        state: &mut new_state,
                        chain_id: self.chain_id,
                        block_number,
                        timestamp,
                    };
                    let ctx = CallContext {
                        caller: addr_to_u256(&tx.from),
                        address: addr_to_u256(&to),
                        value: value_u256(tx.value),
                        calldata: tx.data.clone(),
                        gas_limit: tx.gas_limit.max(1_000_000),
                    };
                    let out = Interpreter::new(&code, &ctx, &mut host)
                        .run()
                        .map_err(|e| EthError::Vm(e.to_string()))?;
                    if !out.success {
                        return Err(EthError::Reverted);
                    }
                    gas_used = gas_used.saturating_add(out.gas_used);
                    logs = out.logs;
                }
            }
        }

        new_state
            .put_json(nonce_key(&tx.from), &(expected + 1).to_string())
            .map_err(|e| EthError::Node(e.to_string()))?;

        let receipt = serde_json::json!({
            "from": format!("0x{}", hex::encode(tx.from)),
            "to": tx.to.map(|a| format!("0x{}", hex::encode(a))),
            "contract_address": created.map(|a| format!("0x{}", hex::encode(a))),
            "value": tx.value.to_string(),
            "nonce": tx.nonce,
            "block_number": block_number,
            "gas_used": gas_used,
            "status": 1u8,
        });
        new_state
            .put_json(receipt_key(&tx.hash), &receipt)
            .map_err(|e| EthError::Node(e.to_string()))?;

        self.state = new_state;
        if let Some(store) = &self.store {
            store
                .save_state(&self.state)
                .map_err(|e| EthError::Node(e.to_string()))?;
        }

        Ok(EthApplied {
            hash: tx.hash,
            from: tx.from,
            to: tx.to,
            contract_address: created,
            value: tx.value,
            nonce: tx.nonce,
            gas_used,
            logs,
        })
    }
}
