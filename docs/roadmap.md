# VEILUX Roadmap — Future Add-ons (Prisms)

Because every capability in VEILUX is a Prism, the roadmap is literally a list of
add-ons. This document proposes the Prisms a future-facing chain needs — grounded
in where the industry is heading (modular execution/DA/settlement, account &
chain abstraction, ZK coprocessing, restaked security, confidential computing)
and in needs that are still under-served today.

Sources informing this roadmap are listed in §“References”. Content was rephrased
for compliance with licensing restrictions.

---

## Tier 0 — Make it a real network (foundational)

Status legend: ✅ implemented · 🔜 next.

| Prism / Module | Status | What it adds |
|----------------|--------|--------------|
| **Consensus engine (Aurora)** | ✅ | stake-weighted BFT: validator set, deterministic proposer rotation, prevote/precommit, 2/3+ quorum, equivocation detection, jailing |
| **Persistence (store)** | ✅ | append-only block log + atomic state snapshots; chain reloads on restart |
| **Network layer** | ✅ | lightweight TCP gossip transport for blocks, votes, and commands (no heavy libp2p) |
| **`token` Prism** | ✅ | fungible LUX-style accounts, transfers, mint/burn |
| **Multi-validator live finality** | ✅ | networked BFT: proposer broadcasts proposal, validators prevote→precommit, blocks finalize at 2/3+ across nodes (verified live with 3–4 validators) |
| **State re-execution for non-proposers** | ✅ | blocks carry their commands; every node re-executes and verifies events_root + state_root, so all nodes converge on byte-identical authenticated state |
| **Proposer failover (view change)** | ✅ | quorum-synchronized view changes: a stalled height advances proposer only when 2/3+ stake signs a view-change, so leaders can fail without halting the chain (verified by killing the proposer mid-run) |
| **Block sync on join** | ✅ | `RequestBlocks`/`Blocks` gossip catches a lagging or restarted node up to the network head |
| **JSON-RPC API + Rust SDK** | ✅ | `veilux serve` exposes a JSON-RPC endpoint; `veilux-sdk` wraps it with identity, command builders, and a typed client |

---

## Tier 1 — The modern expected feature set

Aligned with the dominant 2025–2026 design direction: a modular stack where
execution, data availability, and settlement are distinct layers
([IdeaSoft, 2026](https://ideasoft.io/blog/how-to-build-a-secure-dapp-checklist/);
[2Z thesis, OneKey, 2025](https://onekey.so/blog/ecosystem/unlocking-alpha-the-case-for-2z-token/)).

| Prism | Capability | Notes |
|-------|------------|-------|
| **Account Abstraction Prism** | programmable accounts, gasless/sponsored tx, session keys, social recovery | Mirrors EIP-4337 / EIP-7702 direction; UX comparable to web2 ([OKX, 2025](https://www.okx.com/en-us/learn/eip-sdk-implementation)) |
| **Staking & Governance Prism** ✅ | bond native LUX, delegate, stake-weighted on-chain proposals & voting, **slashing** | Shipped: escrowed stake, delegation, proposal lifecycle, equivocation slashing burns 20% of self-stake on signed double-sign evidence (`prisms/staking`). Next: auto-route consensus equivocation into slash evidence + reward accrual |
| **Fee market / gas economics** ✅ | per-command fees charged at execution, split burn + proposer reward | Shipped: deterministic `fee = gas × price`, configurable burn fraction, capped at payer balance, configured at genesis (node-level). Next: dynamic base-fee (EIP-1559 style) |
| **Oracle Prism** ✅ | quorum-attested external data feeds (prices, AI outputs, facts) | Shipped: reporter set + signature quorum + round anti-replay (`prisms/oracle`) |
| **Data Availability Prism** | erasure-coded blobs + DA sampling so light nodes verify availability without full download | A dedicated DA layer keeps nodes light ([OurCryptoTalk, 2025](https://ourcryptotalk.com/learn/da-layer-in-crypto-data-availability)) |
| **ZK Coprocessor Prism** | offload heavy compute, verify a succinct proof on-chain | "Trust the math" path; complements Veil's message-based privacy |
| **Restaking / Shared Security Prism** | let LUX stake secure external services (oracles, bridges, DA) | Programmable security as a primitive |
| **Chain Abstraction Prism** | one UX across many chains; intents routed/settled behind the scenes | Account + chain abstraction together approach car-like ease of use ([Coinbureau](https://coinbureau.com/analysis/unifying-ethereum/)) |
| **Bridge Prism** ✅ | guardian-attested cross-chain transfers (Cosmos, Solana, EVM, custom) | Shipped: relayer quorum, anti-replay, wrapped tokens. Next: light-client / ZK verification to reduce relayer trust |
| **EVM/WASM Prism** | run existing smart contracts as just-another-Prism | Opt-in; the core never pays for it |

---

## Tier 2 — AI-native chain (VEILUX's differentiator)

VEILUX is AI-native by design; these Prisms make that real and verifiable.

| Prism | Capability |
|-------|------------|
| **Verifiable Compute Prism** | run real models in TEEs (Intel TDX / AMD SEV-SNP) with remote attestation, or attach ZK proofs of correct inference — so results are trustworthy without re-execution ([Chainlink, 2026](https://chain.link/article/confidential-computing-blockchain); [Phala, 2025](https://phala.com/de/learn/Confidential-Computing-in-Finance)) |
| **Model Marketplace Prism** | stake-weighted model registry, royalties per inference, versioning, reputation |
| **Federated Learning Prism** | coordinate private multi-party model training; only gradients/commitments on-chain, raw data stays in each party's sub-ledger |
| **AI Agent Identity Prism** | first-class on-chain identities, spending limits, and capability scopes for autonomous AI agents transacting on the user's behalf |
| **Data-Provenance Prism** | content credentials / watermark attestations so AI training data and outputs have verifiable origin |
| **Inference Oracle Prism** | bring off-chain model outputs on-chain with a quorum + proof, for models too large to run deterministically |

---

## Tier 3 — Privacy & compliance (deepening the Veil)

Builds on `docs/privacy-model.md` §7.

| Prism | Capability |
|-------|------------|
| **X25519 Key-Exchange Prism** | wrap view keys to grantee public keys; remove shared-seed assumption |
| **Proof-of-Reserves Prism** | prove solvency/holdings via range proofs without revealing balances |
| **Compliance Prism** | policy-as-code (AML/KYC thresholds, travel rule) enforced at submission, with scoped regulator grants auto-issued |
| **Metadata-Privacy Prism** | fixed-size view padding + epoch batching to defeat size/timing analysis |
| **Confidential Token Prism** ✅ | encrypted balances and amounts with selective auditor disclosure | Shipped: shielded note/commitment pool, value-conserving transfers, selective disclosure (`prisms/confidential`). Next: ZK proof of conservation so validators are blind to amounts too |

---

## Tier 4 — Forward bets (under-served / "not yet thought of")

Higher-risk, higher-reward Prisms for problems the space hasn't fully solved.

| Prism | Problem it solves |
|-------|-------------------|
| **Post-Quantum Prism** | swap Ed25519/X25519 for ML-DSA/ML-KEM (NIST PQC) before quantum attacks are practical; the kernel keeps crypto behind traits to make this a drop-in |
| **Time-Lock / VDF Prism** | verifiable delay functions for fair ordering and MEV resistance; encrypt-to-the-future for sealed-bid auctions |
| **Intent & Solver Prism** | users express *what* they want; a competitive solver market finds *how*, settled atomically |
| **Programmable Privacy Prism** | per-field visibility policies attached to data that travel with it across Prisms and chains |
| **Reversible/Recoverable Tx Prism** | bounded dispute window with multi-party arbitration for regulated finance (escape from "code is law" rigidity) |
| **Carbon/Resource Accounting Prism** | meter and attest the energy/compute footprint of each block — increasingly a compliance requirement |
| **Decentralized Identity (DID/VC) Prism** | reusable, privacy-preserving credentials; present a proof of a claim without revealing the underlying document |
| **AI Safety / Kill-Switch Prism** | on-chain governance to pause or rate-limit autonomous agents exhibiting anomalous behavior |
| **Cross-Prism Composability Bus** | typed, capability-checked message passing between Prisms beyond the linear cascade (a DAG of capabilities) |

---

## How a new Prism graduates

```
idea → spec (docs/add-ons.md format) → reference Prism crate under prisms/
     → determinism + security review (docs/security.md checklist)
     → tests green → install in node cascade → ship
```

No kernel fork is ever required: that is the whole point of the Prism model.

---

## References

- IdeaSoft — Secure dApp checklist 2026: https://ideasoft.io/blog/how-to-build-a-secure-dapp-checklist/
- OneKey — modular DA / ZK / restaking thesis: https://onekey.so/blog/ecosystem/unlocking-alpha-the-case-for-2z-token/
- OKX — EIP-7702 / account abstraction: https://www.okx.com/en-us/learn/eip-sdk-implementation
- Coinbureau — Account & Chain Abstraction: https://coinbureau.com/analysis/unifying-ethereum/
- OurCryptoTalk — Data Availability layer: https://ourcryptotalk.com/learn/da-layer-in-crypto-data-availability
- Chainlink — Confidential Computing for Blockchain: https://chain.link/article/confidential-computing-blockchain
- Phala — Confidential Computing in Finance: https://phala.com/de/learn/Confidential-Computing-in-Finance

*All external content above was rephrased/summarized for compliance with
licensing restrictions; see links for originals.*
