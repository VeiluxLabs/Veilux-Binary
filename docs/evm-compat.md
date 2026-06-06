# VEILUX EVM Compatibility (eth_* RPC + bytecode execution)

VEILUX is privacy-first and natively runs the Prism/cascade model with Ed25519
signatures. On top of that it ships a real **EVM execution layer** plus an
**Ethereum-compatible JSON-RPC shim** (`eth_*`), so it can both be reached by the
existing Ethereum tooling ecosystem (MetaMask, ethers.js, web3.js, hardware
wallets) and **run real Solidity contract bytecode** — deploy, call, and read.

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
- **Contract deployment.** A transaction with `to = null` runs its `data` as EVM
  init code; the bytecode it `RETURN`s is stored as the runtime code of the new
  contract. The contract address is `keccak256(rlp(sender, nonce))[12..]`, the
  same derivation Ethereum uses.
- **Contract calls.** A transaction to an address that holds code runs that
  runtime code with the tx `data` as calldata, executing genuine `SLOAD`/`SSTORE`
  against persisted contract storage. Contracts may **call other contracts**
  (`CALL`/`DELEGATECALL`/`STATICCALL`) and **deploy new ones** (`CREATE`/
  `CREATE2`); a sub-call that reverts rolls back through a state snapshot while
  the caller continues. `eth_call` runs the same path read-only against a
  throwaway copy of state (no block produced, no nonce change).
- **Application.** Value (if any) moves native LUX from the recovered sender to
  the recipient, the EVM executes, the sender's eth nonce increments, a receipt
  (with `contractAddress`, `gasUsed`, `status`) is written, and a block is
  produced.

## The EVM (`veilux-evm` crate)

A from-scratch, dependency-light interpreter (no `revm`):

- **`U256`** — full 256-bit integer: add/sub/mul, unsigned and signed div/mod,
  shifts (`SHL`/`SHR`/`SAR`), bitwise ops, `SIGNEXTEND`, `EXP`, comparisons.
- **`Interpreter`** — the mainstream opcode set: arithmetic (incl. signed
  `SDIV`/`SMOD`/`SLT`/`SGT`), `KECCAK256`, environment/context (`ADDRESS`,
  `CALLER`, `CALLVALUE`, `CALLDATA*`, `CODECOPY`, `EXTCODESIZE`/`EXTCODECOPY`/
  `EXTCODEHASH`, `RETURNDATASIZE`/`RETURNDATACOPY`, `NUMBER`, `TIMESTAMP`,
  `CHAINID`, `GAS`, `SELFBALANCE`), memory (`MLOAD`/`MSTORE`/`MSTORE8`), storage
  (`SLOAD`/`SSTORE`), control flow (`JUMP`/`JUMPI` with jumpdest analysis),
  `PUSH1`–`PUSH32`, `DUP1`–`DUP16`, `SWAP1`–`SWAP16`, `LOG0`–`LOG4`,
  **inter-contract `CALL`/`CALLCODE`/`DELEGATECALL`/`STATICCALL`**, **contract
  creation `CREATE`/`CREATE2`**, `SELFDESTRUCT`, and `RETURN`/`REVERT`. Gas is
  metered and bounded; memory is capped; call depth is bounded (64) so recursion
  can never overflow the native stack.
- **Call semantics** are real: `DELEGATECALL` runs the callee's code against the
  caller's storage and `msg.sender`/`msg.value` (the library/proxy pattern),
  `STATICCALL` forbids state changes (`SSTORE`/`LOG`/`CREATE`/`SELFDESTRUCT`
  revert), `CALL` transfers value and isolates storage, and a reverted sub-call
  rolls back via a state snapshot while the caller keeps running.
- **`CREATE`/`CREATE2`** deploy child contracts at the canonical addresses
  (`keccak(rlp(sender,nonce))` and `keccak(0xff++sender++salt++keccak(init))`),
  run the init code, and store the returned runtime — so factory and
  deterministic-deployment patterns work.
- **`Host` trait** — the node implements it (`StateHost`) to bridge EVM storage,
  balances, code, nonces, value transfers, and snapshot/revert to the VEILUX
  `StateTree`.

A 4-byte-selector dispatched storage contract (the classic Solidity
`store(uint256)` / `retrieve()`) runs end to end, and inter-contract calls work:
a deployed contract can `CALL` another deployed contract and return its result
(verified end to end through the node's `StateHost`), `DELEGATECALL` runs library
code against the caller's storage, and `CREATE2` deploys at the deterministic
address.

## Supported methods

| Method | Notes |
|--------|-------|
| `eth_chainId`, `net_version` | the node's chain id |
| `eth_blockNumber` | current height |
| `eth_getBalance` | native LUX balance of an eth address |
| `eth_getTransactionCount` | account nonce (for tx ordering) |
| `eth_getCode` | deployed runtime bytecode at an address |
| `eth_call` | read-only contract execution (no state change) |
| `eth_getTransactionByHash` | full transaction details by hash |
| `eth_getLogs` | event logs emitted by contracts (optional `address` filter) |
| `eth_gasPrice`, `eth_estimateGas` | base price / fixed 21000 transfer cost |
| `eth_sendRawTransaction` | submit a signed transfer, deploy, or contract call; returns the tx hash |
| `eth_getTransactionReceipt` | poll for confirmation (`status`, `contractAddress`, `gasUsed`, real `logs` + `blockHash`) |
| `eth_getBlockByNumber`, `eth_getBlockByHash` | block by height/tag or by hash |
| `web3_clientVersion`, `eth_syncing`, `net_listening` | wallet handshake |

## Limitations (honest scope)

- **Gas schedule is simplified.** Gas is metered and bounded (so it is
  DoS-safe), but the per-opcode costs are approximate rather than a byte-exact
  match of a specific Ethereum hard fork. Contracts run correctly; absolute gas
  numbers will differ from mainnet.
- **No precompiled contracts yet** (`ecrecover`, `sha256`, `modexp`, the BN/BLS
  pairing precompiles at addresses `0x01`–`0x0a`). Contracts that rely on them
  will revert when calling those addresses.
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
internal system-account guard (`/`). EVM execution runs against a trial copy of
state and is only committed if the transaction succeeds; a revert rolls back all
storage writes.

### Denial-of-service hardening

EVM bytecode is attacker-controlled, so the execution path is bounded against
abuse (each item is regression-tested):

- **Gas is capped at 30M regardless of the tx's declared `gas_limit`.** A
  transaction claiming `gas_limit = u64::MAX` with an infinite loop
  (`JUMPDEST; PUSH 0; JUMP`) is clamped and terminates with *out of gas* in
  milliseconds instead of hanging the node (`infinite_loop_deploy_is_rejected_not_hung`).
- **Contract code size is capped at 24,576 bytes** (EIP-170), so a deploy cannot
  bloat state with arbitrarily large runtime code (`oversized_contract_code_is_rejected`).
- **Memory is bounded** (1 MiB) and metered, so a huge `MSTORE` offset hits the
  cap rather than allocating unbounded (`memory_bomb_is_bounded`).
- **The RLP decoder is panic-free on arbitrary bytes.** Declared lengths are
  checked with `checked_add` before slicing, so a crafted length prefix can never
  trigger an integer-overflow panic or out-of-bounds slice; it returns a decode
  error (`truncated_length_prefix_does_not_panic`, `fuzz_random_bytes_never_panic`,
  `garbage_raw_tx_does_not_panic`).
- **`eth_call` is also gas-capped**, so a read-only call cannot spin while holding
  the node lock.

