# VEILUX Consensus, Persistence & Networking (Tier 0)

This document covers the three foundational layers that turn the VEILUX engine
into a real chain: the **Aurora** BFT consensus, the **store** persistence
layer, and the **network** gossip transport. All three stay featherweight — no
libp2p, no RocksDB — consistent with the Photon philosophy.

---

## 1. Aurora — stake-weighted BFT consensus

Crate: `consensus` (`veilux-consensus`).

### Validator set
- Each validator has a `PartyId`, an Ed25519 `public_key`, and a `stake`.
- Voting power is stake-weighted; the quorum threshold is `2/3 * total + 1`.
- Validators can be jailed (deactivated) after missing `jail_threshold` slots.
- `ValidatorSet::hash()` gives a deterministic commitment to the active set.

### Proposer selection
Deterministic and stake-set-aware:

```
proposer_for(height, round) = active[(height*31 + round) % active.len()]
```

Every node computes the same proposer for a given height/round, with rounds
allowing fallback if a proposer is offline.

### Voting (two-phase BFT)
1. **Prevote** — validators prevote for a proposed block.
2. **Precommit** — once prevote quorum is seen, validators precommit.
3. **Commit** — when precommit power ≥ `2/3+1`, the block is final.

The `VoteSet` tallies stake-weighted power per block hash and **detects
equivocation** (a validator signing two different blocks at the same
height/round) — a slashable offense.

```rust
let outcome = aurora.add_vote(&vote)?;
if let CommitOutcome::Committed { height, block_hash, power, .. } = outcome {
    // block is final
}
```

### Safety properties
- **Agreement**: with < 1/3 stake Byzantine, no two conflicting blocks can both
  reach a 2/3+ precommit quorum.
- **Accountability**: equivocating validators are detectable from their two
  conflicting signed votes.

---

## 2. Store — persistence

Crate: `store` (`veilux-store`).

- **Block log** (`blocks.jsonl`): append-only, one JSON block per line. Simple,
  human-inspectable, crash-friendly.
- **State snapshot** (`state.json`): the authenticated `StateTree`, written
  atomically (temp file + rename) so a crash never leaves a half-written state.

```rust
let store = Store::open("./veilux-data")?;
let node = Node::with_store(proposer, cascade, store)?; // loads existing chain
let summary = node.produce_block()?;                    // appends + snapshots
```

On restart, `Node::with_store` reloads all blocks and the latest state, so the
chain continues from where it left off (verified by `veilux run` twice).

---

## 3. Network — gossip transport

Crate: `network` (`veilux-network`).

- Plain **TCP** with newline-delimited JSON messages — tiny and dependency-light
  (just `tokio`).
- A `broadcast` channel fans outbound messages to every connected peer; inbound
  messages arrive on an `mpsc` channel the node drains.
- Bootstrap peers are dialed with automatic retry; inbound peers are accepted by
  a listener.

### Message types (`NetMessage`)

| Variant | Purpose |
|---------|---------|
| `Hello` | handshake with node id + height |
| `Command` | a `SignedCommand` for the mempool |
| `Proposal` | a proposed block for a round |
| `Vote` | a consensus prevote/precommit |
| `Block` | a finalized block |
| `RequestBlocks` | ask a peer to send blocks from a height |

```rust
let handle = Network::spawn(NetConfig {
    node_id: "node-a".into(),
    listen_addr: "127.0.0.1:30420".into(),
    bootstrap: vec!["127.0.0.1:30421".into()],
});
handle.net.broadcast(&NetMessage::Block(Box::new(block)))?;
while let Some(msg) = handle.inbound.recv().await { /* handle */ }
```

A unit test spins up two nodes and verifies a block gossiped from one is
received by the other over real TCP.

---

## 4. How they fit together

```
            ┌────────────── network (TCP gossip) ──────────────┐
            │  inbound: Command / Proposal / Vote / Block        │
            ▼                                                    ▲
   ┌───────────────┐   produce/verify    ┌──────────────────┐   │ broadcast
   │  node (Node)  │◄───────────────────►│ consensus (Aurora)│   │
   │  cascade+veil │                     │ validators+votes  │───┘
   └───────┬───────┘                     └──────────────────┘
           │ append block + snapshot state
           ▼
     ┌───────────┐
     │  store    │  blocks.jsonl + state.json
     └───────────┘
```

Single-node persistent operation (`veilux run`) and **multi-node live BFT
finality** (`veilux validator`) are both implemented today. State re-execution
for non-proposers and proposer failover are the next steps — see
`docs/roadmap.md` Tier 0.

---

## 5. Run it

### Single persistent node

```bash
cargo run --bin veilux -- run ./veilux-data   # produce + persist a block
cargo run --bin veilux -- run ./veilux-data   # reloads chain, grows it
```

### Live 3-validator network (multi-node finality)

Open three terminals (shared seed strings so every node derives the same
validator public keys):

```bash
veilux validator --name v1 --seed v1seed --listen 127.0.0.1:33001 \
  --peer v2:v2seed --peer v3:v3seed --datadir ./d1 \
  --bootstrap 127.0.0.1:33002 --bootstrap 127.0.0.1:33003

veilux validator --name v2 --seed v2seed --listen 127.0.0.1:33002 \
  --peer v1:v1seed --peer v3:v3seed --datadir ./d2 \
  --bootstrap 127.0.0.1:33001 --bootstrap 127.0.0.1:33003

veilux validator --name v3 --seed v3seed --listen 127.0.0.1:33003 \
  --peer v1:v1seed --peer v2:v2seed --datadir ./d3 \
  --bootstrap 127.0.0.1:33001 --bootstrap 127.0.0.1:33002
```

You'll see `block committed by BFT quorum ... power=300 quorum=201` and all
three data directories grow the chain in lockstep with byte-identical blocks.

```bash
cargo test -p veilux-consensus -p veilux-store -p veilux-network
cargo test -p veilux-node driver   # deterministic 4-validator finality test
```
