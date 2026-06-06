use prism_token::{move_balance, native_token_id};
use std::time::{SystemTime, UNIX_EPOCH};
use veilux_evm::u256::U256;
use veilux_evm::vm::{CallContext, Host, Interpreter, Log};
use veilux_evm::{decode_legacy_tx, keccak256, EvmError};
use veilux_kernel::{Block, Hash, PartyId, StateTree};

use crate::node::Node;

const MAX_EVM_GAS: u64 = 30_000_000;
const MAX_CODE_SIZE: usize = 24_576;

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

fn txmeta_key(hash: &[u8; 32]) -> String {
    format!("eth/tx/0x{}", hex::encode(hash))
}

pub fn eth_tx(state: &StateTree, hash: &[u8; 32]) -> Option<serde_json::Value> {
    state
        .get_json::<serde_json::Value>(&txmeta_key(hash))
        .ok()
        .flatten()
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

#[derive(Debug)]
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
            gas_limit: MAX_EVM_GAS,
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
                    gas_limit: tx.gas_limit.clamp(21_000, MAX_EVM_GAS),
                };
                let out = Interpreter::new(&tx.data, &ctx, &mut host)
                    .run()
                    .map_err(|e| EthError::Vm(e.to_string()))?;
                if !out.success {
                    return Err(EthError::Reverted);
                }
                if out.return_data.len() > MAX_CODE_SIZE {
                    return Err(EthError::Vm(format!(
                        "contract code size {} exceeds limit {}",
                        out.return_data.len(),
                        MAX_CODE_SIZE
                    )));
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
                        gas_limit: tx.gas_limit.clamp(21_000, MAX_EVM_GAS),
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

        let log_json: Vec<serde_json::Value> = logs
            .iter()
            .enumerate()
            .map(|(i, l)| {
                serde_json::json!({
                    "address": created
                        .or(tx.to)
                        .map(|a| format!("0x{}", hex::encode(a)))
                        .unwrap_or_default(),
                    "topics": l
                        .topics
                        .iter()
                        .map(|t| format!("0x{}", hex::encode(t.to_big_endian())))
                        .collect::<Vec<_>>(),
                    "data": format!("0x{}", hex::encode(&l.data)),
                    "logIndex": format!("0x{:x}", i),
                })
            })
            .collect();

        let receipt = serde_json::json!({
            "from": format!("0x{}", hex::encode(tx.from)),
            "to": tx.to.map(|a| format!("0x{}", hex::encode(a))),
            "contract_address": created.map(|a| format!("0x{}", hex::encode(a))),
            "value": tx.value.to_string(),
            "nonce": tx.nonce,
            "block_number": block_number,
            "gas_used": gas_used,
            "logs": log_json,
            "status": 1u8,
        });
        new_state
            .put_json(receipt_key(&tx.hash), &receipt)
            .map_err(|e| EthError::Node(e.to_string()))?;

        let tx_meta = serde_json::json!({
            "hash": format!("0x{}", hex::encode(tx.hash)),
            "from": format!("0x{}", hex::encode(tx.from)),
            "to": tx.to.map(|a| format!("0x{}", hex::encode(a))),
            "nonce": tx.nonce,
            "value": tx.value.to_string(),
            "gas": tx.gas_limit,
            "gas_price": tx.gas_price.to_string(),
            "input": format!("0x{}", hex::encode(&tx.data)),
            "block_number": block_number,
        });
        new_state
            .put_json(txmeta_key(&tx.hash), &tx_meta)
            .map_err(|e| EthError::Node(e.to_string()))?;

        self.state = new_state;
        self.seal_anchor_block(timestamp)?;

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

    fn seal_anchor_block(&mut self, timestamp: u64) -> Result<(), EthError> {
        let parent = self.head();
        let ts = now_secs().max(parent.timestamp).max(timestamp);
        let mut block = Block {
            height: parent.height + 1,
            parent: parent.hash(),
            events_root: Hash::ZERO,
            state_root: self.state.root(),
            timestamp: ts,
            proposer: self.proposer.clone(),
            events: vec![],
            commands: vec![],
        };
        block.events_root = block.compute_events_root();
        self.blocks.push(block);
        if let Some(store) = &self.store {
            let last = self.blocks.last().expect("just pushed");
            store
                .append_block(last)
                .map_err(|e| EthError::Node(e.to_string()))?;
            store
                .save_state(&self.state)
                .map_err(|e| EthError::Node(e.to_string()))?;
        }
        Ok(())
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_token::seed_native_token;
    use veilux_evm::sign_legacy_tx;
    use veilux_kernel::Cascade;

    fn eth_node(chain_id: u64) -> Node {
        let cascade = Cascade::new();
        let mut n = Node::new(PartyId::new("eth-test"), cascade);
        n.chain_id = chain_id;
        let _ = seed_native_token(
            &mut n.state,
            "Veilux",
            "LUX",
            18,
            &PartyId::new("treasury"),
            &[],
        );
        n
    }

    #[test]
    fn infinite_loop_deploy_is_rejected_not_hung() {
        let mut n = eth_node(7);
        let sk = [9u8; 32];
        let loop_code = hex::decode("5b600056").unwrap();
        let raw = sign_legacy_tx(&sk, 0, 1, u64::MAX, None, 0, &loop_code, 7).unwrap();
        let result = n.eth_apply_raw(&raw);
        assert!(
            matches!(result, Err(EthError::Vm(_))),
            "an infinite-loop deploy with gas_limit=u64::MAX must be capped and rejected, not hang: {result:?}"
        );
        assert_eq!(n.head().height, 0, "a rejected tx must not seal a block");
    }

    #[test]
    fn oversized_contract_code_is_rejected() {
        let mut n = eth_node(7);
        let sk = [11u8; 32];
        let mut init = Vec::new();
        let big = MAX_CODE_SIZE + 100;
        init.push(0x61);
        init.extend_from_slice(&(big as u16).to_be_bytes());
        init.push(0x80);
        init.push(0x60);
        init.push(0x0a);
        init.push(0x60);
        init.push(0x00);
        init.push(0x39);
        init.push(0x60);
        init.push(0x00);
        init.push(0xf3);
        let raw = sign_legacy_tx(&sk, 0, 1, 5_000_000, None, 0, &init, 7).unwrap();
        let result = n.eth_apply_raw(&raw);
        assert!(
            matches!(result, Err(EthError::Vm(_))),
            "a deploy returning more than the code-size limit must be rejected: {result:?}"
        );
    }

    #[test]
    fn garbage_raw_tx_does_not_panic() {
        let mut n = eth_node(7);
        for bytes in [
            vec![],
            vec![0xff; 4],
            vec![0xf8, 0xff],
            vec![0xc0],
            (0u8..255).collect::<Vec<u8>>(),
        ] {
            let _ = n.eth_apply_raw(&bytes);
        }
    }

    #[test]
    fn wrong_chain_id_rejected() {
        let mut n = eth_node(7);
        let sk = [3u8; 32];
        let raw = sign_legacy_tx(&sk, 0, 1, 21_000, Some([0x22u8; 20]), 0, &[], 9).unwrap();
        assert!(matches!(
            n.eth_apply_raw(&raw),
            Err(EthError::WrongChainId { .. })
        ));
    }

    #[test]
    fn replayed_nonce_rejected() {
        let mut n = eth_node(7);
        let sk = [5u8; 32];
        let from = veilux_evm::address_from_secret(&sk).unwrap();
        let _ = prism_token::credit(
            &mut n.state,
            &native_token_id(),
            &eth_party(&from),
            1_000_000,
        );
        let raw = sign_legacy_tx(&sk, 0, 1, 21_000, Some([0x44u8; 20]), 100, &[], 7).unwrap();
        assert!(n.eth_apply_raw(&raw).is_ok());
        assert!(
            matches!(n.eth_apply_raw(&raw), Err(EthError::BadNonce { .. })),
            "replaying the same nonce must be rejected"
        );
    }
}
