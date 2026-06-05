use prism_token::{move_balance, native_token_id};
use veilux_evm::{decode_legacy_tx, EvmError};
use veilux_kernel::{PartyId, StateTree};

use crate::node::{Node, NodeError};

pub fn eth_party(addr: &[u8; 20]) -> PartyId {
    PartyId::new(format!("eth:0x{}", hex::encode(addr)))
}

fn nonce_key(addr: &[u8; 20]) -> String {
    format!("eth/nonce/0x{}", hex::encode(addr))
}

fn receipt_key(hash: &[u8; 32]) -> String {
    format!("eth/receipt/0x{}", hex::encode(hash))
}

pub fn eth_receipt(state: &StateTree, hash: &[u8; 32]) -> Option<serde_json::Value> {
    state
        .get_json::<serde_json::Value>(&receipt_key(hash))
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

#[derive(Debug, thiserror::Error)]
pub enum EthError {
    #[error("decode: {0}")]
    Decode(#[from] EvmError),
    #[error("wrong chain id: tx signed for {got:?}, node chain is {expected}")]
    WrongChainId { got: Option<u64>, expected: u64 },
    #[error("bad nonce: tx nonce {got}, account nonce {expected}")]
    BadNonce { got: u64, expected: u64 },
    #[error("contract creation / data calls are not supported by the native EVM shim")]
    Unsupported,
    #[error("node: {0}")]
    Node(String),
}

pub struct EthApplied {
    pub hash: [u8; 32],
    pub from: [u8; 20],
    pub to: [u8; 20],
    pub value: u128,
    pub nonce: u64,
}

impl Node {
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

        let to = tx.to.ok_or(EthError::Unsupported)?;
        if !tx.data.is_empty() {
            return Err(EthError::Unsupported);
        }

        let expected = eth_nonce(&self.state, &tx.from);
        if tx.nonce != expected {
            return Err(EthError::BadNonce {
                got: tx.nonce,
                expected,
            });
        }

        let from_party = eth_party(&tx.from);
        let to_party = eth_party(&to);
        move_balance(
            &mut self.state,
            &native_token_id(),
            &from_party,
            &to_party,
            tx.value,
        )
        .map_err(|e| EthError::Node(e.to_string()))?;

        self.state
            .put_json(nonce_key(&tx.from), &(expected + 1).to_string())
            .map_err(|e| EthError::Node(NodeError::Store(e.to_string()).to_string()))?;

        let block_number = self.head().height + 1;
        let receipt = serde_json::json!({
            "from": format!("0x{}", hex::encode(tx.from)),
            "to": format!("0x{}", hex::encode(to)),
            "value": tx.value.to_string(),
            "nonce": tx.nonce,
            "block_number": block_number,
            "status": 1u8,
        });
        self.state
            .put_json(receipt_key(&tx.hash), &receipt)
            .map_err(|e| EthError::Node(e.to_string()))?;

        if let Some(store) = &self.store {
            store
                .save_state(&self.state)
                .map_err(|e| EthError::Node(e.to_string()))?;
        }

        Ok(EthApplied {
            hash: tx.hash,
            from: tx.from,
            to,
            value: tx.value,
            nonce: tx.nonce,
        })
    }
}
