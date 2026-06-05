# Changelog

All notable changes to **VEILUX** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Bridge Prism** (`prisms/bridge`) — guardian-attested cross-chain transfers
  to Cosmos, Solana, Ethereum, or a custom chain. Register a guardian set with a
  signature quorum; lock tokens outbound (`send`) and redeem guardian-attested
  inbound transfers (`redeem`) with anti-replay sequencing. Bridged value reuses
  Token Prism balances, so it is real, spendable token balance. Builders added
  to both the Rust and TypeScript SDKs.

## [0.3.0] - 2026-06-05

The developer-experience release: client SDKs, real-time subscriptions, and
contributor tooling.

### Added
- **TypeScript SDK** (`@veilux/sdk`) — build, sign, and submit commands from
  Node.js or the browser. Ed25519 signing and BLAKE3 hashing are byte-compatible
  with the Rust node, so TS-signed commands verify on-chain. Ships typed
  client, identity, command builders, and a runnable quickstart example.
  Published to npm.
- **WebSocket subscriptions** — `veilux serve` opens a WebSocket endpoint
  (RFC 6455, featherweight, no external library) that pushes a notification for
  every committed block. The TypeScript SDK exposes `subscribeBlocks()`.
- SDK id helpers (`tokenId`, `collectionId`, `contractAddress`), state-key
  helpers, `Client.tokenBalance()` and `Client.waitForHeight()` conveniences.
- Cross-language compatibility test suite asserting TS signing bytes, command
  ids, and Ed25519 signatures match the Rust node exactly.
- CI job that type-checks, builds, and tests the TypeScript SDK on every push.
- npm publish workflow that releases `@veilux/sdk` on version tags.
- Contributor tooling: `CONTRIBUTING.md`, PR template, issue templates, and a
  `develop` integration branch as the PR target.

### Fixed
- Release workflow no longer duplicates the changelog: builds upload artifacts,
  then a single publish job creates the release using the matching `CHANGELOG.md`
  section as the body.

## [0.2.0] - 2026-06-05

The "real network" release: VEILUX graduates from a local engine to a
multi-node, fault-tolerant chain with a developer-facing API.

### Added
- **Aurora consensus** (`consensus`) — stake-weighted Byzantine fault-tolerant
  engine: validator set, deterministic proposer selection, prevote/precommit
  voting with 2/3+ finality, equivocation detection, and validator jailing.
- **Persistence** (`store`) — append-only block log plus atomic state
  snapshots; chains reload from disk on restart.
- **Networking** (`network`) — featherweight TCP gossip transport for proposals,
  votes, blocks, commands, and view-change messages (no libp2p dependency).
- **Live multi-node finality** — the `veilux validator` command runs a
  networked BFT validator. Verified live with 3–4 nodes finalizing in lockstep.
- **State re-execution for non-proposers** — blocks now carry their commands;
  every node re-executes and verifies both `events_root` and `state_root`, so
  all nodes converge on byte-identical authenticated state.
- **Proposer failover** — quorum-synchronized view changes let the chain
  advance past a failed leader without honest nodes desyncing onto different
  proposers (verified by killing the proposer mid-run).
- **Block sync on join** — `RequestBlocks`/`Blocks` gossip catches a lagging or
  restarted node up to the network head.
- **JSON-RPC API** (`rpc`) — featherweight HTTP/1.1 JSON-RPC server plus shared
  contract types: `veilux_nodeInfo`, `veilux_submit`, `veilux_blockNumber`,
  `veilux_getBlockByNumber`, `veilux_getState`, `veilux_estimate`.
- **Rust SDK** (`veilux-sdk`) — typed client wrapping the RPC, with identity and
  command builders for every Prism, plus a quickstart example.
- **`veilux serve`** — a developer "dev node" that exposes the RPC endpoint and
  mines a block per submitted command for fast feedback.
- Contact details and dual MIT/Apache licensing surfaced in the README.

### Changed
- Privacy layer rebranded from "Canton-style" to VEILUX's own **VeilLedger**
  model across code and documentation; external references reduced to neutral
  background reading.
- `Block` now includes a `commands` field and a `commands_root` in its hash so
  blocks are independently re-executable.
- Author/contact metadata updated to `nathan <nathan@winnode.xyz>`.

### Fixed
- GitHub license detection now resolves cleanly to **MIT** (top-level `LICENSE`
  no longer ambiguous).
- Comment-stripped, `rustfmt`- and `clippy`-clean across the workspace under the
  pinned 1.85.0 toolchain.

## [0.1.0] - 2026-06-05

Initial public release.

### Added
- **Photon kernel** — featherweight core: content-addressed state, Merkle
  commitments, the `Prism` extension trait, and the cascade execution pipeline.
- **Veil privacy layer** — encrypted per-party views (ChaCha20-Poly1305),
  block projection, per-party sub-ledgers, Ed25519 identities, and scoped
  selective disclosure for auditors/regulators.
- **Prisms** — AI (model registry + verifiable inference, optional Ollama),
  Storage (content-addressed blobs), Token (ERC-20-like), NFT (ERC-721-like),
  and Contract (the PhotonVM smart-contract VM).
- **Node binary** with `info`, `demo`, and `run` commands.
- Multi-platform release pipeline (Linux gnu/musl, Windows, macOS Intel/ARM),
  Docker image, and full documentation set.
- Dual licensing under MIT OR Apache-2.0.

[Unreleased]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/VeiluxLabs/Veilux-Binary/releases/tag/v0.1.0
