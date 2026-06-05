# VEILUX Add-ons (Prisms) — Specifications

Every capability in VEILUX is a **Prism**: a module that implements the kernel's
`Prism` trait. This document specifies each shipped Prism and the contract for
writing your own.

---

## The Prism contract

```rust
pub trait Prism: Send + Sync {
    fn info(&self) -> PrismInfo;
    fn name(&self) -> &str { self.info().name }
    fn handle(&self, command: &Command, state: &mut StateTree)
        -> Result<PrismOutput, PrismError>;
    fn estimate(&self, command: &Command, state: &StateTree) -> u64 { 1_000 }
}
```

Rules every Prism must follow:

1. **Determinism.** Given the same `(state, command)`, `handle` must produce the
   same output on every node. No wall-clock, no RNG, no network, no floats whose
   result varies across platforms.
2. **State namespacing.** Use a unique key prefix (`name/...`) so Prisms never
   collide in the shared `StateTree`.
3. **Bounded work.** Respect input sizes; return `PrismError::LimitExceeded`
   instead of doing unbounded work.
4. **Visibility passthrough.** Copy `command.visibility` onto emitted events so
   the Veil layer can seal them correctly.

`PrismOutput` carries `events`, `derived_commands` (the cascade), and `cost`.

---

## Prism: `ai` — AI Add-on

**Crate:** `prisms/ai` · **Routing name:** `ai` · **Version:** 1.0

### Purpose
On-chain AI model registry plus deterministic, verifiable inference. Heavy model
weights stay off-chain (only a content hash is recorded); inference is computed
deterministically so every validator agrees on the result.

### State layout

| Key | Value |
|-----|-------|
| `ai/models/<model_id_hex>` | JSON `ModelRecord` |

### Commands (`AiCommand`)

```jsonc
// Register a model
{ "op": "register",
  "name": "sentiment-v1",
  "kind": "classification",        // classification | regression | embedding | text
  "weights_hash": "0x...",          // content hash of off-chain weights
  "version": "1.0",
  "dimensions": 3 }

// Run inference
{ "op": "infer",
  "model_id": "0x...",
  "input": [ ...bytes... ] }
```

### Events (`AiEvent`)

```jsonc
{ "kind": "model_registered", "model_id": "0x...", "name": "..." }

{ "kind": "inference_committed",
  "model_id": "0x...",
  "input_hash": "0x...",
  "output_hash": "0x...",
  "output": [ ...bytes... ] | null,   // null when offloaded to storage
  "cost": 8000 }
```

### Model kinds & gas

| Kind | Base cost | Output |
|------|-----------|--------|
| `classification` | 8,000 | `{ class, confidence }` |
| `regression` | 6,000 | `{ value }` (fixed-point ×1e6) |
| `embedding` | 12,000 + 4/dim | `{ dims, values[] }` (fixed-point ×1e6) |
| `text` | 20,000 | deterministic templated bytes |

### Cascade behavior
If an inference output exceeds **256 bytes**, the AI Prism emits a derived
`storage.put` command so the Storage Prism persists the full result, and the
event stores `output: null` plus the `output_hash`. This is a live example of
Prism→Prism cascade.

### Determinism note
The reference implementation derives outputs from `BLAKE3(weights_hash ‖ input)`
rather than running a real neural net, guaranteeing identical results across
nodes. For production-grade real models, see `docs/roadmap.md` → *Verifiable
Compute Prism* (TEE attestation or ZK proof of correct execution).

---

## Prism: `storage` — Storage Add-on (cascade target)

**Crate:** `prisms/storage` · **Routing name:** `storage` · **Version:** 1.0

### Purpose
Content-addressed blob storage with reference-counted pinning. Identical content
is stored once (natural dedup). Acts as the cascade sink for large AI results.

### State layout

| Key | Value |
|-----|-------|
| `storage/blob/<cid_hex>` | raw bytes |
| `storage/pin/<cid_hex>` | JSON `PinRecord { refcount, size, owner }` |

### Commands (`StorageCommand`)

```jsonc
// Put a blob (key is advisory; CID is the canonical content hash)
{ "key": "result.bin", "data": [ ...bytes... ] }

// Pin / Unpin an existing blob
{ "op": "pin",   "cid": "0x..." }
{ "op": "unpin", "cid": "0x..." }
```

### Events (`StorageEvent`)

```jsonc
{ "kind": "stored",   "cid": "0x...", "size": 1234, "key": "result.bin" }
{ "kind": "pinned",   "cid": "0x...", "refcount": 2 }
{ "kind": "unpinned", "cid": "0x...", "refcount": 0, "removed": true }
```

### Limits & gas

| Op | Cost | Limit |
|----|------|-------|
| put | 100 + 2/byte | `MAX_BLOB_SIZE = 1 MiB` |
| pin | 200 | blob must exist |
| unpin | 150 | GCs blob when refcount hits 0 |

### Privacy
On the public state, only a size-revealing pin record exists per CID. The blob
bytes travel inside the per-party encrypted view (via Veil), so non-stakeholders
learn a blob's size but not its content.

---

## Prism: `token` — Fungible Tokens (ERC-20-like)

**Crate:** `prisms/token` · **Routing name:** `token` · **Version:** 1.0

### Purpose
Create and manage fungible tokens with balances, transfers, allowances, mint and
burn. Amounts are `u128`, serialized as decimal strings (safe for JSON/JS).

### State layout

| Key | Value |
|-----|-------|
| `token/meta/<token_id>` | JSON `TokenMeta` |
| `token/bal/<token_id>/<party>` | balance (decimal string) |
| `token/allow/<token_id>/<owner>/<spender>` | allowance (decimal string) |

### Commands (`TokenCommand`)

```jsonc
{ "op": "create", "name": "Gold", "symbol": "GLD", "decimals": 18, "initial_supply": "1000000", "mintable": true }
{ "op": "transfer", "token_id": "0x...", "to": "bob", "amount": "250" }
{ "op": "approve", "token_id": "0x...", "spender": "carol", "amount": "300" }
{ "op": "transfer_from", "token_id": "0x...", "from": "alice", "to": "dave", "amount": "200" }
{ "op": "mint", "token_id": "0x...", "to": "bob", "amount": "1000" }   // owner only, if mintable
{ "op": "burn", "token_id": "0x...", "amount": "50" }
```

### Rules & gas
- Transfer/transfer_from check balance and (for the latter) allowance.
- Mint requires `mintable == true` and `submitter == owner`.
- Overflow-checked addition; burn reduces total supply.
- Gas: create 5,000 · transfer ~1,000–1,200 · approve 800 · mint 2,000 · burn 1,500.

---

## Prism: `nft` — Non-Fungible Tokens (ERC-721-like)

**Crate:** `prisms/nft` · **Routing name:** `nft` · **Version:** 1.0

### Purpose
Collections of unique tokens with mint, transfer, approve and burn. Each token
carries a `metadata_uri` (e.g. IPFS) and a `content_hash` for integrity.

### State layout

| Key | Value |
|-----|-------|
| `nft/coll/<collection_id>` | JSON `Collection` |
| `nft/token/<collection_id>/<index>` | JSON `NftToken` |
| `nft/owner/<collection_id>/<index>` | owner `PartyId` |
| `nft/approve/<collection_id>/<index>` | approved spender `PartyId` |

### Commands (`NftCommand`)

```jsonc
{ "op": "create_collection", "name": "Art", "symbol": "ART", "max_supply": 100 }
{ "op": "mint", "collection_id": "0x...", "to": "alice", "metadata_uri": "ipfs://...", "content_hash": "0x..." }
{ "op": "transfer", "collection_id": "0x...", "token_index": 0, "to": "bob" }
{ "op": "approve", "collection_id": "0x...", "token_index": 0, "spender": "carol" }
{ "op": "burn", "collection_id": "0x...", "token_index": 0 }
```

### Rules & gas
- Mint requires `submitter == collection.creator` and respects `max_supply`.
- Transfer allowed by owner or approved spender; approval clears on transfer.
- Gas: create 5,000 · mint 3,000 · transfer 1,200 · approve 800 · burn 1,000.

---

## Prism: `contract` — PhotonVM Smart Contracts

**Crate:** `prisms/contract` · **Routing name:** `contract` · **Version:** 1.0

### Purpose
A deterministic, stack-based virtual machine (PhotonVM) for general smart
contracts. Lightweight and fully reproducible across nodes — no floats, bounded
gas, explicit jump-dest validation.

### State layout

| Key | Value |
|-----|-------|
| `contract/code/<address>` | JSON `ContractMeta` (deployer + bytecode) |
| `contract/store/<address>` | JSON persistent storage map `u64 -> u64` |

### Commands (`ContractCommand`)

```jsonc
{ "op": "deploy", "code": [ ...bytecode... ] }
{ "op": "call", "address": "0x...", "args": [1,2,3], "value": 0, "gas_limit": 1000000 }
```

### Instruction set (PhotonVM v1)

| Opcode | Hex | Effect |
|--------|-----|--------|
| STOP | 0x00 | halt |
| ADD/SUB/MUL/DIV/MOD | 0x01–0x05 | arithmetic (wrapping; DIV/MOD trap on 0) |
| LT/GT/EQ/ISZERO | 0x10–0x13 | comparisons |
| AND/OR/NOT | 0x16/0x17/0x19 | bitwise |
| CALLER/CALLVALUE/ARG | 0x33–0x35 | execution context |
| POP/DUP/SWAP | 0x50–0x52 | stack ops |
| SLOAD/SSTORE | 0x54/0x55 | persistent storage (key on top for SSTORE) |
| JUMP/JUMPI/JUMPDEST | 0x56/0x57/0x5b | control flow (validated dests) |
| PUSH8 | 0x60 | push next 8 bytes as u64 |
| LOG | 0xa0 | emit a log value |
| RETURN/REVERT | 0xf3/0xfd | return value / revert |

### Gas & limits
- Per-opcode gas (e.g. SSTORE 5,000, SLOAD 200, arithmetic 5).
- `MAX_CODE_SIZE = 48 KiB`, stack depth ≤ 1024, gas capped at 10,000,000/call.
- A reverting call is recorded as an event with `reverted: true`; storage is not
  committed for reverted calls.

---

---

## Prism: `bridge` — Cross-Chain Transfers

**Crate:** `prisms/bridge` · **Routing name:** `bridge` · **Version:** 1.0

### Purpose
Move tokens between VEILUX and foreign chains (Cosmos, Solana, Ethereum, or a
custom chain) using a **guardian/relayer quorum** trust model (like Wormhole). A
registered set of guardians watches both sides and signs attestations; the
bridge accepts an inbound transfer once a quorum of valid Ed25519 signatures is
present.

### State layout

| Key | Value |
|-----|-------|
| `bridge/config/<chain>` | JSON `BridgeConfig` (guardians, quorum, admin) |
| `bridge/seq/<chain>` | next expected inbound sequence (anti-replay) |
| `bridge/outseq/<chain>` | outbound sequence counter |
| `token/bal/<id>/<party>` | reuses Token Prism balances (bridged value is real) |

### Commands (`BridgeCommand`)

```jsonc
// Register/update a foreign chain (admin-gated after creation)
{ "op": "register_chain", "chain": "cosmos", "guardians": ["<hex pubkey>", ...], "quorum": 2 }

// Outbound: lock VEILUX tokens to send abroad
{ "op": "send", "chain": "solana", "recipient": "<foreign addr>", "token_id": [..32 bytes..], "amount": "1000" }

// Inbound: redeem a guardian-attested transfer (mints wrapped tokens)
{ "op": "redeem", "transfer": { ...InboundTransfer... }, "signatures": [{ "public_key": "...", "signature": "..." }] }
```

### Flows
- **Outbound (`send`)**: debits the sender, emits `OutboundLocked` with a
  sequence + digest. Off-chain relayers mint/release on the foreign chain.
- **Inbound (`redeem`)**: verifies a guardian quorum signed the transfer digest,
  enforces a strictly-increasing per-chain sequence (anti-replay), then credits
  the wrapped token to the recipient.

### Security model
- Trust is in the guardian set (relayers), not pure math — documented honestly.
- Replay protection via per-chain sequence numbers.
- Duplicate-guardian signatures are not double-counted.
- Deterministic signature verification, so every validator agrees.
- Gas: register 5,000 · send 3,000 · redeem 8,000.

### Supported chains
`cosmos`, `solana`, `ethereum`, `custom`. Foreign addresses are opaque
hex/strings interpreted by each chain's relayers, so adding a new chain needs no
kernel change.

---

## Writing your own Prism

```rust
use veilux_kernel::{Command, Event, Prism, PrismError, PrismInfo, PrismOutput, StateTree};

pub struct CounterPrism;

impl Prism for CounterPrism {
    fn info(&self) -> PrismInfo {
        PrismInfo { name: "counter", description: "increments a per-party counter", version: "1.0" }
    }

    fn handle(&self, cmd: &Command, state: &mut StateTree) -> Result<PrismOutput, PrismError> {
        let key = format!("counter/{}", cmd.submitter);
        let current: u64 = state.get_json(&key)
            .map_err(|e| PrismError::Internal(e.to_string()))?
            .unwrap_or(0);
        let next = current + 1;
        state.put_json(&key, &next).map_err(|e| PrismError::Internal(e.to_string()))?;

        let event = Event {
            source_command: cmd.id(),
            prism: "counter".into(),
            visibility: cmd.visibility.clone(),
            payload: next.to_le_bytes().to_vec(),
        };
        Ok(PrismOutput::single(event, 500))
    }
}
```

Install and use it:

```rust
cascade.install(Box::new(CounterPrism));
```

That is the entire integration surface. No kernel changes, no forks.

### Checklist before shipping a Prism
- [ ] Deterministic across platforms (no floats/RNG/time/network in `handle`)
- [ ] Unique state-key prefix
- [ ] Input size bounds with `LimitExceeded`
- [ ] `estimate` returns a realistic cost
- [ ] Visibility copied onto events
- [ ] Unit tests for happy path + each error path
