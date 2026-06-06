<p align="center">
  <img src="https://raw.githubusercontent.com/VeiluxLabs/.github/main/VeiLux-labs.jpg" alt="VEILUX" width="100%">
</p>

<h1 align="center">VEILUX</h1>

[![CI](https://github.com/VeiluxLabs/Veilux-Binary/actions/workflows/ci.yml/badge.svg)](https://github.com/VeiluxLabs/Veilux-Binary/actions/workflows/ci.yml)
[![Release](https://github.com/VeiluxLabs/Veilux-Binary/actions/workflows/release.yml/badge.svg)](https://github.com/VeiluxLabs/Veilux-Binary/actions/workflows/release.yml)
[![npm](https://img.shields.io/npm/v/@veilux/sdk?label=%40veilux%2Fsdk&color=cb3837&logo=npm)](https://www.npmjs.com/package/@veilux/sdk)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

> **Veil** (privacy by default) + **Lux** (light / illumination) — a featherweight, privacy-first, AI-native modular blockchain.

Repository: **[github.com/VeiluxLabs/Veilux-Binary](https://github.com/VeiluxLabs/Veilux-Binary)** &nbsp;·&nbsp; TypeScript SDK: **[npmjs.com/package/@veilux/sdk](https://www.npmjs.com/package/@veilux/sdk)** &nbsp;·&nbsp; Releases: **[binaries](https://github.com/VeiluxLabs/Veilux-Binary/releases)**

VEILUX is built around three ideas:

1. **A featherweight core.** The *Photon* kernel knows almost nothing. It defines the data shapes, one extension trait (`Prism`), a pipeline (`Cascade`), and a content-addressed state. That's it. Everything heavy is an add-on you compile in only if you need it. Release binaries are built for speed (`opt-level = 3`, LTO, stripped) yet still tiny (~2.9 MB).

2. **Everything is a Prism (add-on).** A *Prism* is a self-contained capability. Nine ship today: **Token**, **NFT**, **Contract** (PhotonVM), **AI** (+ optional Ollama), **Storage**, **Bridge** (cross-chain), **Staking & Governance**, **Oracle**, and **Confidential Token**. They **cascade**: one Prism can trigger another (the AI Prism offloads large results to the Storage Prism automatically). Add your own by implementing one trait — no kernel fork.

3. **Privacy by ledger (VeilLedger).** The *Veil* layer gives you one logically shared ledger where **no participant sees data they aren't a stakeholder of**. Every node agrees on the same Merkle root of *blinded commitments*, while contents are sealed per-party into encrypted **views** and stored in per-party **sub-ledgers**.

```
            ╦  ╦╔═╗╦╦  ╦ ╦═╗ ╦
            ╚╗╔╝║╣ ║║  ║ ║╔╩╦╝
             ╚╝ ╚═╝╩╩═╝╚═╝╩ ╚═
   featherweight · privacy-first · AI-native
```

## Quickstart (60 seconds)

```bash
# build the single binary (Rust 1.85+)
cargo build --release --bin veilux

# 1. see the kernel + installed Prisms
./target/release/veilux info

# 2. run the end-to-end demo (private AI inference + token + NFT + contract + audit)
./target/release/veilux demo

# 3. start a dev node with JSON-RPC + WebSocket + an Ethereum-compatible endpoint
./target/release/veilux serve --eth-rpc 127.0.0.1:8652

# now point MetaMask at http://127.0.0.1:8652, or use the TypeScript SDK:
#   npm install @veilux/sdk
```

That's a full, self-contained chain node — no external database, no extra
services. For a real multi-validator BFT network see
[`docs/consensus-networking.md`](docs/consensus-networking.md).

## Native token

The native token's name, symbol, decimals, and total supply are **chosen at
genesis** (`--genesis spec.json`), so every network picks its own identity. The
defaults are:

| | |
|---|---|
| Name / Ticker | Veilux / `LUX` |
| Subunit | `ray` (1 LUX = 10¹⁸ ray) |
| Supply | set by genesis allocations |

The native token powers **staking**, **governance voting weight**, **transaction
fees**, and **block rewards**.

## What is VEILUX?

VEILUX is an original **Layer-1 blockchain**, written entirely in **Rust** and
compiled to a single, native, featherweight binary (`veilux`). It is not a fork
and does not embed a third-party EVM — it is built from the ground up around
three principles: **a tiny core, everything-as-a-module, and privacy by
default.**

### The technology

- **100% Rust, native binary.** Compiles straight to machine code (no
  interpreter, no VM runtime). Memory-safe with no garbage collector, and
  deterministic — every node computes identical results, which is what BFT
  consensus requires.
- **Featherweight.** Release binaries are built for speed (`opt-level=3`, LTO,
  stripped, `panic="abort"`) and are still only ~2.9 MB per platform. The
  core kernel depends on just four small crates — no heavyweight database or
  networking framework.
- **Multi-platform.** One codebase ships to six targets: Linux (glibc + static
  musl, x86_64 + ARM64), Windows x86_64, and macOS (Intel + Apple Silicon).
- **Modular by construction.** Capabilities are *Prisms* you compile in only
  when needed, so you never pay for what you don't run.

### How it works

| Layer | What it does |
|-------|--------------|
| **Photon** (kernel) | Data shapes, the `Prism` trait, the cascade pipeline, and a BLAKE3 content-addressed state. Deliberately minimal. |
| **Aurora** (consensus) | Stake-weighted Byzantine fault-tolerant consensus: 2/3+ finality, deterministic proposer selection, quorum-synchronized proposer failover, and equivocation detection that auto-submits slashing evidence. |
| **Veil** (privacy) | One logically shared ledger; each event is sealed per-party with ChaCha20-Poly1305, while all nodes agree on a Merkle root of blinded commitments. Includes scoped selective disclosure for auditors/regulators. |
| **Economics** | Configurable native token at genesis, gas-priced transaction fees split between a burn and a proposer reward, staking with delegation, and on-chain stake-weighted governance. |
| **Store** | Persistent block log + atomic state snapshots + a persistent mempool (pending transactions survive restarts); the chain reloads on restart. |
| **Network** | Lightweight TCP gossip for proposals, votes, blocks, and sync — with an optional authenticated, **end-to-end encrypted** transport (signed peer identity + X25519/ChaCha20-Poly1305 + IP allowlist). |

### Features

**Built-in Prisms (add-ons):**

| Prism | Capability |
|-------|------------|
| **Token** | Fungible tokens (ERC-20-style): transfer, approve, mint, burn; plus the native token + fee helpers |
| **NFT** | Non-fungible tokens & collections (ERC-721-style) |
| **Contract** | PhotonVM — a deterministic stack-based smart-contract VM |
| **AI** | On-chain model registry + deterministic inference, with optional local LLM execution via Ollama |
| **Storage** | Content-addressed blob storage with reference-counted pinning |
| **Bridge** | Guardian-attested cross-chain transfers to Cosmos, Solana, and EVM chains |
| **Staking & Governance** | Bond/delegate native LUX, stake-weighted proposals & voting, and equivocation **slashing** |
| **Oracle** | Quorum-attested external data feeds (prices, AI outputs, off-chain facts) |
| **Confidential** | Confidential token with hidden balances (note commitments) + selective auditor disclosure |

**Developer experience:**

- **JSON-RPC API** (`veilux serve`) — a local dev node, like Anvil/Ganache
- **Explorer API** — `explorer_*` query endpoints (stats, blocks, command search,
  per-prism events, state prefix) for indexers and dashboards
- **WebSocket subscriptions** — real-time block notifications
- **SDKs** — Rust (`veilux-sdk`) and TypeScript (`@veilux/sdk`, on npm), with
  byte-compatible signing so clients in either language verify on-chain
- **Web Explorer** — a modern, Etherscan-style block explorer UI in [`explorer/`](explorer)
  (static, zero-build); browse blocks, transactions, events, and state with live
  WebSocket updates
- **Full CI/CD** — multi-platform release binaries, Docker images, and automatic
  npm publishing

### What makes it different

1. **AI-native.** AI is a first-class Prism with deterministic, verifiable
   inference — not a bolt-on. It can also drive local LLMs through Ollama.
2. **Banking-grade privacy.** The VeilLedger model keeps data confidential from
   competitors while remaining provably transparent to authorized regulators,
   enforced by cryptography rather than policy. The Confidential Token Prism even
   hides balances and amounts from public observers, and **private execution**
   (`veilux_submitPrivate`) keeps a confidential transaction's inputs and effects
   on stakeholder nodes only — the global chain orders just a tamper-evident
   commitment, Canton-style.
3. **Featherweight & modular.** A tiny core plus opt-in Prisms means stronger
   security, smaller binaries, and long-term maintainability.
4. **Real economics & security.** Stake-weighted BFT with delegation,
   governance, gas-priced fees (burn + proposer reward), automatic slashing of
   equivocating validators, and an authenticated, end-to-end encrypted peer
   transport.
5. **Cross-chain by design.** The Bridge Prism connects VEILUX to other
   ecosystems out of the box.
6. **EVM-compatible — runs Solidity.** An optional `eth_*` JSON-RPC shim lets
   MetaMask, ethers.js, and other Ethereum tooling connect to a VEILUX node,
   send real secp256k1/EIP-155 signed transactions, and **deploy and call EVM
   contract bytecode** — the `veilux-evm` crate includes a from-scratch EVM
   interpreter (deploy, call, `eth_call`, receipts, `eth_getCode`). See
   `docs/evm-compat.md`.

### How VEILUX compares

No chain wins on every axis. Here is an honest positioning against well-known
designs — VEILUX's niche is **privacy + AI + a tiny modular core in one binary**,
not raw throughput records.

| | VEILUX | Ethereum L1 | Solana | Cosmos SDK chain | Canton/Fabric |
|---|---|---|---|---|---|
| Language / artifact | Rust, **one ~3 MB binary** | Go/Rust clients | Rust, large | Go, large | JVM/Go, heavy |
| Smart contracts | **EVM bytecode** + native PhotonVM + Prisms | EVM | SVM (BPF) | CosmWasm/modules | DAML/chaincode |
| Privacy | **Per-party encrypted sub-ledgers + private execution** (data stays on stakeholder nodes) | Public by default | Public by default | Public by default | Strong (per-party) |
| AI | **First-class AI Prism** (verifiable inference, Ollama) | none | none | none | none |
| Extending it | **Implement one trait, no fork** | hardforks/EIPs | core changes | modules | chaincode |
| Consensus | Aurora stake-weighted BFT | Gasper PoS | PoH+PoS | Tendermint BFT | varies |
| Wallet reach | **MetaMask/ethers via `eth_*`** | native | native | Keplr | n/a |
| Maturity | **testnet-grade, unaudited externally** | battle-tested mainnet | mainnet | mainnet | enterprise prod |

The honest takeaway: established chains are **production-proven**; VEILUX is a
**young, original codebase** whose differentiators (Canton-style privacy that
actually keeps data off the global ledger, AI as a native primitive, and a
featherweight one-binary modular core) are unusual to find together. Choose it
today for **research, private/permissioned deployments, and prototyping** those
differentiators — not yet for securing large public value (see readiness below).

### Production readiness (honest checklist)

**Works today, verified by tests + live multi-node runs:**

- ✅ Multi-node BFT finality, proposer failover, block sync, byte-identical state
- ✅ Persistence (blocks + state + **restart-safe mempool**)
- ✅ Authenticated **and end-to-end encrypted** validator transport (`--secure`)
- ✅ Signature/nonce/key-binding/replay protection at the single ingress
- ✅ EVM: deploy + call Solidity, inter-contract calls, CREATE/CREATE2, precompiles
- ✅ Privacy: per-party views, **private execution**, divergence detection +
  **on-chain slashing**
- ✅ Economics: configurable token, fees (burn + reward), staking, governance, slashing
- ✅ Rust + TypeScript SDKs, JSON-RPC, WebSocket, web explorer
- ✅ Internal security audit with findings fixed (one critical caught & fixed —
  see [`docs/audit-2026-06.md`](docs/audit-2026-06.md))

**Required before securing real value (not done yet):**

- 🔜 **Independent third-party security audit** (the single most important gap)
- 🔜 Public testnet with real adversarial load and incentive testing
- 🔜 Inter-contract EVM `CALL`/`CREATE` from within a running frame for complex
  DeFi (single-frame contracts work today)
- 🔜 HSM / keystore key management (keys are seed-derived in memory today)
- 🔜 Fork-exact EVM gas schedule + missing precompiles (`ripemd160`, BN/BLS pairings)
- 🔜 ZK-blind confidential transfers (amounts hidden but not yet zero-knowledge-proven)
- 🔜 Light clients, formal spec, and large-scale fuzzing

> **Bottom line:** the binary **runs and does what this README says** — you can
> launch a network, deploy Solidity, move private value, and slash cheaters
> right now. It is **not** yet hardened for a value-bearing public mainnet. Treat
> it as a strong **testnet-grade core** pending an external audit.

### Who it's for

Concrete things you can build on VEILUX today:

- **Confidential institutional settlement.** Two banks settle a trade as a
  `Parties([bankA, bankB])` private transaction: the amount and counterparties
  live only on their own nodes, the public chain holds a tamper-evident
  commitment, and a regulator can be handed a scoped disclosure grant — without
  exposing anything to competitors.
- **Verifiable on-chain AI.** Register a model and run deterministic inference
  through the AI Prism (or a local LLM via Ollama); the result is committed and
  auditable, with large outputs auto-offloaded to content-addressed storage.
- **EVM dApps with privacy options.** Deploy a normal Solidity contract via
  MetaMask, then move sensitive balances through the Confidential Token Prism or
  private execution when you need them off the public ledger.
- **Tokenized assets & private DAOs.** Issue a configurable native or custom
  token, run stake-weighted governance, and keep membership/holdings private.
- **Permissioned consortium chains.** Run `--secure` validators with an IP
  allowlist and signed-peer handshake so only known members join the mesh.

Primary audiences:

- **Financial institutions** building confidential finance and tokenized assets
- **Decentralized AI** applications needing verifiable on-chain inference
- **Enterprises** coordinating multi-party workflows over private-but-verifiable data
- **Web3 developers** shipping dApps with the Rust or TypeScript SDK

### Status

VEILUX is a fully functional chain: live multi-node BFT consensus with proposer
failover and auto-slashing, persistence (with a restart-safe mempool), an
authenticated **and encrypted** gossip network, privacy, nine Prisms, configurable
token economics with fees and staking, a from-scratch EVM execution layer
(deploy + call Solidity bytecode over `eth_*`), and JSON-RPC + WebSocket APIs with
Rust and TypeScript SDKs — all covered by tests and continuous integration. It has
not yet run a public mainnet; treat it as a testnet-grade core. See
`docs/security.md` for the honest threat model and remaining hardening items
(ZK-blind confidential transfers, HSM key management, inter-contract EVM calls),
and [`docs/audit-2026-06.md`](docs/audit-2026-06.md) for the **internal security
audit** (findings + fixes). An independent third-party audit is still required
before any value-bearing deployment.

## Workspace layout

```
veilux/
├── kernel/            # Photon: featherweight core (no EVM, ~zero heavy deps)
│   ├── crypto.rs      #   BLAKE3 hashing + Merkle roots
│   ├── types.rs       #   Command / Event / Block / PartyId / Visibility
│   ├── prism.rs       #   the Prism trait — the one extension point
│   ├── cascade.rs     #   the Prism pipeline / executor
│   └── state.rs       #   content-addressed authenticated state
├── veil/              # VeilLedger privacy
│   ├── view.rs        #   encrypted per-party views (ChaCha20-Poly1305)
│   ├── identity.rs    #   Ed25519 signing identities
│   ├── disclosure.rs  #   scoped selective disclosure (auditor/regulator)
│   ├── projection.rs  #   split a block into per-party views
│   └── ledger.rs      #   per-party sub-ledgers
├── consensus/         # Aurora — stake-weighted BFT (validators, votes, quorum)
├── store/             # append-only block log + state snapshots (persistence)
├── network/           # lightweight TCP gossip (blocks, votes, commands)
├── rpc/               # JSON-RPC contract types + featherweight HTTP server
├── sdk/               # veilux-sdk: Rust client (identity + builders + RPC)
├── sdk-ts/            # @veilux/sdk: TypeScript/JS client for web & Node
├── explorer/          # web block explorer (static, Etherscan-style)
├── prisms/
│   ├── ai/            # AI Prism: model registry + inference (+ optional Ollama)
│   ├── storage/       # Storage Prism: content-addressed blobs + pinning
│   ├── token/         # Token Prism: fungible + native token, fees, balances
│   ├── nft/           # NFT Prism: non-fungible tokens (ERC-721-like)
│   ├── contract/      # Contract Prism: PhotonVM smart contracts
│   ├── bridge/        # Bridge Prism: cross-chain transfers (Cosmos, Solana, EVM)
│   ├── staking/       # Staking & Governance Prism: stake, delegate, vote, slash
│   ├── oracle/        # Oracle Prism: quorum-attested external data feeds
│   └── confidential/  # Confidential Token Prism: hidden balances + disclosure
├── evm/               # EVM: 256-bit math, interpreter, keccak/secp256k1, RLP, eth_* tx
└── node/              # assembles kernel + veil + consensus + store + prisms + evm
                       #   (genesis token config, fee engine, auto-slash watcher)
```

## The cascade

A command is routed to its Prism, which emits events **and** optional derived
commands. Derived commands flow back into the pipeline (bounded depth), so
capabilities compose:

```
submit(ai.infer) ─► AI Prism ─► InferenceCommitted event
                                 └─(large result)─► storage.put ─► Storage Prism ─► Stored event
```

## Privacy model (the VeilLedger design)

- Each event declares a `Visibility`: `Public` or `Parties([...])`.
- The block commits to a Merkle root over **commitments** (blinded hashes), so
  every node — even non-stakeholders — agrees on the same global root.
- For each stakeholder, the event is sealed into an **EncryptedView**
  (ChaCha20-Poly1305, key derived per party + view).
- Each party keeps a **SubLedger**: the decrypted events it's entitled to, plus
  the validated global root.

Result: a non-stakeholder can prove a transaction *happened* without learning
*what* happened.

## Token economics, staking & governance

VEILUX ships a complete economic layer on top of the native token:

- **Configurable genesis.** A JSON spec sets the token name, symbol, decimals,
  initial allocations (= total supply), and the fee policy. Seeding is
  deterministic, so every validator with the same spec converges on identical
  state.

  ```jsonc
  {
    "token_name": "Veilux", "token_symbol": "LUX", "token_decimals": 18,
    "treasury": "treasury",
    "allocations": [
      { "party": "treasury",   "amount": 700000000 },
      { "party": "validators", "amount": 200000000 },
      { "party": "ecosystem",  "amount": 100000000 }
    ],
    "fee_price_per_gas": 1, "fee_burn_bps": 5000, "fee_target_gas": 50000
  }
  ```

- **Transaction fees.** When enabled, each command pays `fee = gas × price` in
  the native token. A configurable fraction is **burned** (deflationary) and the
  rest rewards the **block proposer** — the anti-spam and validator-incentive
  backbone. The charge is deterministic and capped at the payer's balance.

- **Staking & delegation.** Bond native LUX as validator stake or delegate it to
  another validator; voting power = self-bonded + delegated.

- **Governance.** Bonded stakers open proposals; everyone votes with
  stake-weighted power; proposals finalize after a voting period.

- **Slashing.** If a validator double-signs, any node that observes the two
  conflicting signed votes automatically submits **equivocation evidence**, and
  the offender's self-stake is slashed (burned). Forged evidence is rejected.

Run a chain with economics enabled:

```bash
veilux serve --genesis genesis.example.json
veilux validator --name v1 --seed v1 --listen 127.0.0.1:30421 \
  --peer v2:v2 --peer v3:v3 --secure --allow-ip 127.0.0.1 \
  --genesis genesis.example.json
```

See [`docs/add-ons.md`](docs/add-ons.md) for the Staking, Oracle, and
Confidential Prism specs, and the genesis/fee reference.

## Download (prebuilt binaries)

Every tagged release publishes ready-to-run binaries on the
[Releases page](https://github.com/VeiluxLabs/Veilux-Binary/releases):

| Platform | Asset |
|----------|-------|
| Linux x86_64 (glibc) | `veilux-<ver>-x86_64-unknown-linux-gnu.tar.gz` |
| Linux x86_64 (static musl) | `veilux-<ver>-x86_64-unknown-linux-musl.tar.gz` |
| Linux ARM64 | `veilux-<ver>-aarch64-unknown-linux-gnu.tar.gz` |
| Windows x86_64 | `veilux-<ver>-x86_64-pc-windows-msvc.zip` |
| macOS Intel | `veilux-<ver>-x86_64-apple-darwin.tar.gz` |
| macOS Apple Silicon | `veilux-<ver>-aarch64-apple-darwin.tar.gz` |

Each asset ships with a `.sha256` checksum. Example (Linux):

```bash
tar xzf veilux-v0.1.0-x86_64-unknown-linux-gnu.tar.gz
cd veilux-v0.1.0-x86_64-unknown-linux-gnu
./veilux info
```

Or pull the container image:

```bash
docker run --rm ghcr.io/veiluxlabs/veilux-binary:latest info
```

## Build & run

> Requires the Rust toolchain (`rustup`, stable 1.85+). The workspace compiles
> cleanly and all tests pass.

```bash
cd veilux
cargo build --release          # featherweight, size-optimized binaries
cargo test --workspace         # kernel/veil/prism unit tests (all green)
cargo run --bin veilux -- info # show kernel + installed prisms
cargo run --bin veilux -- demo # private AI + storage + audit demo
cargo run --bin veilux -- run  # persistent node (BFT consensus + disk store)
cargo run --bin veilux -- serve # dev RPC + WebSocket node (http :8645, ws :8646)
```

The `run` command opens a data directory (default `./veilux-data`), loads any
existing chain from disk, produces+persists a block, and reports the Aurora BFT
proposer slot. Re-running it shows the chain growing across restarts.

For a live multi-node BFT network, use `veilux validator` (see
`docs/consensus-networking.md`) — three validators reach 2/3+ finality over TCP
and stay byte-for-byte in sync.

See **`docs/INSTALL.md`** for a full setup, troubleshooting, and library quick-start.

## Documentation

| Doc | What's inside |
|-----|---------------|
| [`docs/INSTALL.md`](docs/INSTALL.md) | Install, build, run, troubleshoot, CI/CD, Docker |
| [`docs/architecture.md`](docs/architecture.md) | System design, cascade, state model |
| [`docs/add-ons.md`](docs/add-ons.md) | Per-Prism specs (all nine) + native token, fees & genesis + how to build your own |
| [`docs/consensus-networking.md`](docs/consensus-networking.md) | Aurora BFT consensus, authenticated transport, persistence, and gossip |
| [`docs/rpc-sdk.md`](docs/rpc-sdk.md) | JSON-RPC API + Rust & TypeScript SDKs for building applications |
| [`docs/evm-compat.md`](docs/evm-compat.md) | Ethereum-compatible `eth_*` RPC — connect MetaMask & ethers.js |
| [`docs/ai-ollama.md`](docs/ai-ollama.md) | Running real AI models via Ollama |
| [`docs/privacy-model.md`](docs/privacy-model.md) | Deep VeilLedger banking-grade privacy research |
| [`docs/security.md`](docs/security.md) | Threat model + exploitation review + what runs safely |
| [`docs/audit-2026-06.md`](docs/audit-2026-06.md) | **Internal security audit (June 2026)** — full manual review, findings & fixes |
| [`docs/roadmap.md`](docs/roadmap.md) | Future add-ons the chain needs next |
| [`CHANGELOG.md`](CHANGELOG.md) | Version history (Keep a Changelog format) |

## Writing your own Prism

```rust
use veilux_kernel::{Command, Event, Prism, PrismError, PrismInfo, PrismOutput, StateTree};

struct HelloPrism;

impl Prism for HelloPrism {
    fn info(&self) -> PrismInfo {
        PrismInfo { name: "hello", description: "says hi", version: "1.0" }
    }
    fn handle(&self, cmd: &Command, _state: &mut StateTree) -> Result<PrismOutput, PrismError> {
        let event = Event {
            source_command: cmd.id(),
            prism: "hello".into(),
            visibility: cmd.visibility.clone(),
            payload: b"hi".to_vec(),
        };
        Ok(PrismOutput::single(event, 100))
    }
}
```

Install it with `cascade.install(Box::new(HelloPrism))` and it's live. Full spec
and checklist in `docs/add-ons.md`.

## SDKs

Build apps against VEILUX in Rust or TypeScript:

- **TypeScript / JavaScript** — [`@veilux/sdk`](https://www.npmjs.com/package/@veilux/sdk)
  [![npm version](https://img.shields.io/npm/v/@veilux/sdk?color=cb3837&logo=npm)](https://www.npmjs.com/package/@veilux/sdk)
  [![npm downloads](https://img.shields.io/npm/dm/@veilux/sdk?color=cb3837)](https://www.npmjs.com/package/@veilux/sdk)

  Source in [`sdk-ts/`](sdk-ts). Byte-compatible signing with the node, command
  builders, typed RPC client, and WebSocket block subscriptions.

  ```bash
  npm install @veilux/sdk
  ```

  ```ts
  import { Client, PartyIdentity, builders, subscribeBlocks } from "@veilux/sdk";

  const client = new Client("http://127.0.0.1:8645");
  const alice = PartyIdentity.fromSeed("alice", new Uint8Array(32).fill(1));
  await client.submit(alice.sign(
    builders.tokenCreate("alice", "Public", 0, "Gold", "GLD", 18, 1_000_000n, true)));

  subscribeBlocks("ws://127.0.0.1:8646", { onBlock: (b) => console.log(b.height) });
  ```

- **Rust** — [`veilux-sdk`](sdk) crate. Same surface, native types.

See [`docs/rpc-sdk.md`](docs/rpc-sdk.md) for the full API.

## Contributing

Contributions are welcome! Open PRs against the **`develop`** branch. See
[`CONTRIBUTING.md`](CONTRIBUTING.md) for the branching model, commit conventions,
and the checks CI runs. Adding a capability is as simple as writing a Prism —
no kernel fork required (see [`docs/add-ons.md`](docs/add-ons.md)).

## License

Licensed under either of:

- MIT license ([LICENSE-MIT](LICENSE-MIT))
- Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.

## Contact

- Email: [nathan@winnode.xyz](mailto:nathan@winnode.xyz)

## Author

Created and maintained by **nathan**. Original work — © 2026 nathan.
