# VEILUX Architecture

VEILUX is a modular blockchain with three layers, each in its own crate:

```
┌─────────────────────────────────────────────────────────────┐
│  node  (binary: `veilux`)                                     │
│  assembles everything; drives block production                │
└───────────────┬───────────────────────────┬──────────────────┘
                │                            │
        ┌───────▼────────┐          ┌────────▼─────────┐
        │  Prisms        │          │  Veil            │
        │  (add-ons)     │          │  (privacy)       │
        │  ┌──────────┐  │          │  views           │
        │  │ ai       │  │          │  projection      │
        │  ├──────────┤  │          │  sub-ledgers     │
        │  │ storage  │  │          └────────┬─────────┘
        │  └──────────┘  │                   │
        └───────┬────────┘                   │
                │                            │
        ┌───────▼────────────────────────────▼─────────┐
        │  kernel  (Photon — featherweight core)        │
        │  types · prism trait · cascade · state · crypto│
        └────────────────────────────────────────────────┘
```

## Why "featherweight"?

The kernel deliberately excludes everything that usually bloats a chain:

- **No EVM in the core.** EVM (or WASM, or anything) would be just another
  Prism. The base node doesn't pay for what it doesn't run.
- **Tiny dependency set.** The kernel depends only on `serde`, `blake3`,
  `thiserror`, `hex`. No async runtime, no networking, no database.
- **Size-optimized release profile.** `opt-level = "z"`, `lto = true`,
  `codegen-units = 1`, `panic = "abort"`, `strip = true`.

You scale *up* by installing Prisms, not *down* by disabling a monolith.

## Data flow

1. A client builds a `Command { prism, submitter, visibility, payload, nonce }`.
2. The node puts it in the mempool and later calls `produce_block`.
3. The `Cascade` routes each command to its Prism's `handle`, which mutates the
   `StateTree` and returns `PrismOutput { events, derived_commands, cost }`.
4. Derived commands re-enter the cascade (bounded by `MAX_CASCADE_DEPTH = 8`).
5. The block commits a Merkle root over event **commitments** and the new
   **state root**.
6. `Veil::project_block` seals each event into per-party `EncryptedView`s and
   appends them to per-party `SubLedger`s.

## The cascade (composability)

The cascade is what makes capabilities compose without tight coupling. The AI
Prism doesn't depend on the Storage crate — it just emits a `storage` command
as JSON. If the Storage Prism is installed, the result is persisted; if not,
the AI Prism keeps the result inline. Prisms communicate through commands and
events, never direct calls.

## Privacy (Veil) in depth

### The problem
A normal blockchain replicates all data to all nodes. That's the opposite of
what enterprises (and many AI workloads on sensitive data) want.

### The Canton-style solution
- **One virtual shared ledger, many private projections.** Every event has a
  `Visibility`. The global Merkle root is over *commitments* (`Hash`), which
  reveal nothing about contents but let all nodes agree on history.
- **Per-party encrypted views.** For each stakeholder, the event is encrypted
  with `ChaCha20-Poly1305` under a key derived from the party's secret seed and
  the view id. The commitment is bound into the AEAD associated data, so a
  ciphertext can't be moved to a different commitment.
- **Sub-ledgers.** Each party's node maintains a `SubLedger`: the decrypted
  events it can see, plus the `validated_root` it has checked against the global
  chain.

### What an outsider learns
- That a transaction occurred (its commitment is in the root).
- Its `Visibility` shape and rough size.
- **Not** the payload, the model used, the inputs, or the outputs.

## State model

`StateTree` is a `BTreeMap<String, Vec<u8>>` whose root is a Merkle root over
`commit("kv", [key, value])` leaves. Deterministic ordering (`BTreeMap`) means
all nodes compute the same `state_root`. Prisms namespace their keys:

| Prism    | Key prefix          |
|----------|---------------------|
| ai       | `ai/models/<id>`    |
| storage  | `storage/blob/<cid>`, `storage/pin/<cid>` |

## Determinism requirements

Every Prism must be deterministic: same `(state, command)` → same output on
every node. The reference Prisms achieve this by deriving all "AI" outputs from
`BLAKE3` of `(weights_hash, input)` rather than from floating-point model
execution, so validators always agree. In production, non-deterministic model
execution would be wrapped in a verifiable-compute scheme (ZK or
optimistic-challenge) and only the *commitment* would go on-chain.

## Consensus

Consensus is intentionally pluggable and out of scope for the core types. The
`Block` carries a `proposer` and chains via `parent`; a BFT or PoS engine can be
added as orchestration in the node layer (or as a Prism that validates proposer
rotation) without touching the kernel.
