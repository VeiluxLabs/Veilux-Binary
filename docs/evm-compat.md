# VEILUX EVM Compatibility (eth_* RPC)

VEILUX is not an EVM chain — its native execution is the Prism/cascade model
with Ed25519 signatures. But to be reachable by the existing Ethereum tooling
ecosystem (MetaMask, ethers.js, web3.js, hardware wallets), a node can expose an
**Ethereum-compatible JSON-RPC shim** that speaks `eth_*` and understands real
secp256k1-signed, EIP-155 transactions.

```bash
veilux serve --addr 127.0.0.1:8645 \
  --eth-rpc 127.0.0.1:8652 \
  --genesis genesis.json
```

Then in MetaMask → *Add network manually*:

| Field | Value |
|-------|-------|
| RPC URL | `http://127.0.0.1:8652` |
| Chain ID | the genesis `chain_id` (e.g. `777`) |
| Currency symbol | `LUX` |

## How it works

- **Addresses.** An Ethereum address `0xabc…` maps to the VEILUX party
  `eth:0xabc…`, which holds native-token (LUX) balance like any other account.
- **Signatures.** `eth_sendRawTransaction` receives an RLP-encoded, secp256k1
  signed legacy/EIP-155 transaction. The shim (`veilux-evm` crate) RLP-decodes
  it, rebuilds the EIP-155 signing hash (keccak256), and **recovers the sender
  address** from the signature — exactly as Ethereum does. Verified against the
  canonical EIP-155 mainnet test vector.
- **Replay protection.** The transaction's `chain_id` (from the `v` value) must
  match the node's chain id, so a tx signed for another chain is rejected.
- **Application.** A value transfer moves native LUX from the recovered sender
  party to the recipient party, increments the sender's eth nonce, writes a
  receipt, and produces a block. Sender balance, recipient balance, nonce, and
  receipt are then queryable via the standard methods.

## Supported methods

| Method | Notes |
|--------|-------|
| `eth_chainId`, `net_version` | the node's chain id |
| `eth_blockNumber` | current height |
| `eth_getBalance` | native LUX balance of an eth address |
| `eth_getTransactionCount` | account nonce (for tx ordering) |
| `eth_gasPrice`, `eth_estimateGas` | base price / fixed 21000 transfer cost |
| `eth_sendRawTransaction` | submit a signed value transfer; returns the tx hash |
| `eth_getTransactionReceipt` | poll for confirmation (status `0x1`) |
| `eth_getBlockByNumber` | head block summary |
| `web3_clientVersion`, `eth_syncing`, `net_listening` | wallet handshake |

## Limitations (honest scope)

- **Value transfers only.** Contract creation and `data`/contract calls are
  rejected (`Unsupported`) — the shim does not run EVM bytecode. VEILUX smart
  contracts use the PhotonVM Prism, not EVM bytecode. A full EVM execution layer
  (running Solidity bytecode) would be a separate, much larger Prism.
- **Legacy + EIP-155 transactions only.** Typed-envelope transactions
  (EIP-2718/1559, `0x02…`) are rejected; configure wallets to use legacy gas.
- **Global chain-id uniqueness is a social convention.** To avoid clashes,
  register the chosen `chain_id` on community lists (e.g. chainlist.org /
  ethereum-lists); there is no on-chain registry that enforces uniqueness.

## Security

The shim reuses the same value-conservation guarantees as the Token Prism:
`move_balance` debits the sender and credits the recipient atomically and cannot
create value. Sender authenticity rests on secp256k1 recovery (a forged or
wrong-chain signature recovers a different/zero address and fails). The reserved
`eth:` party namespace contains `:` but not `/`, so it does not collide with the
internal system-account guard (`/`).
