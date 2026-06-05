# Changelog

All notable changes to **VEILUX** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.1] - 2026-06-05

### Changed
- **~40–70× throughput improvement.** The release profile used `opt-level = "z"`
  (optimize for size), which crippled the elliptic-curve cryptography and made
  signature verification the bottleneck (~255 tx/s ingest). Switched to
  `opt-level = 3`. Measured single-node, in-memory throughput on a token-transfer
  workload jumped to **~18,000 tx/s serial ingest, ~33,000 tx/s execution, and
  ~11,700 tx/s end-to-end**. The release binary grows from ~0.9 MB to ~2.9 MB —
  a worthwhile trade for a >40× speedup (still tiny for a node).

### Added
- **Parallel batch signature verification** (`verify_signed_batch`, rayon) —
  ~44,000 tx/s verifying a block's commands across cores.
- **`veilux_getAccount` RPC** — returns an account's next nonce, key-bound
  status, bound public key, and native balance, so clients can build correctly
  ordered transactions and check inclusion without scraping raw state. Exposed in
  the Rust SDK (`get_account`).

## [0.5.0] - 2026-06-05

### Added
- **EVM compatibility — connect MetaMask & Ethereum tooling.** A node can now
  expose an Ethereum-compatible JSON-RPC endpoint with `veilux serve --eth-rpc
  ADDR`. It speaks the `eth_*` methods wallets need (`eth_chainId`, `net_version`,
  `eth_blockNumber`, `eth_getBalance`, `eth_getTransactionCount`, `eth_gasPrice`,
  `eth_estimateGas`, `eth_sendRawTransaction`, `eth_getTransactionReceipt`,
  `eth_getBlockByNumber`, `web3_clientVersion`). New `veilux-evm` crate implements
  keccak256, EIP-55 checksum addresses, an RLP codec, and **secp256k1 sender
  recovery** for legacy/EIP-155 raw transactions — verified against the canonical
  EIP-155 mainnet test vector. An Ethereum address `0x…` maps to the VEILUX party
  `eth:0x…` holding native LUX; a signed value transfer is recovered, chain-id
  checked, applied (sender debited, recipient credited, nonce bumped), and a
  receipt is written. Verified end-to-end: a real secp256k1-signed tx for a
  custom chain id transfers value and is queryable exactly like on Ethereum.
  Scope is value transfers only (no EVM bytecode execution); see
  `docs/evm-compat.md`.

## [0.4.0] - 2026-06-05

### Added
- **Chain-id replay protection (EIP-155 style).** Signatures are now bound to a
  chain via `SignedCommand.chain_id`: `signing_bytes_for_chain` appends a
  domain-separated `chain<id_le>` suffix for nonzero ids (id 0 stays
  byte-identical to the legacy scheme, so existing vectors and dev flows are
  unchanged). The node rejects any command whose `chain_id` differs from its own
  (`WrongChainId`), and the validator tx-ingress rejects it up front with a clear
  error. `chain_id` is set in the genesis spec and surfaced via `veilux_chainId`.
  The TypeScript SDK gained `signForChain` / `signingBytesForChain`; cross-language
  byte-compatibility is preserved (compat vectors still pass). Verified live: a
  command signed for the wrong chain is refused, the right one is finalized.

### Fixed
- **Single-validator (and self-quorum) liveness.** A validator that already held
  a prevote quorum from its own vote never advanced past the prevote phase,
  because the prevote→precommit transition only ran when a vote arrived over the
  network. A lone validator therefore produced **zero blocks** (it appeared
  "stuck/slow"). The round machine now self-checks quorum immediately after
  casting its own prevote, so a single validator finalizes on its own timer while
  multi-node finality is unchanged (the self-check only fires when quorum already
  exists). Regression test `single_validator_self_finalizes` added; the 4-node
  finality test still passes and a live 3-node run still finalizes at quorum 201.

## [0.3.9] - 2026-06-05

### Added
- **Validator transaction ingress.** `veilux validator --rpc ADDR` now opens a
  JSON-RPC endpoint that accepts `veilux_submit`, validates the signed command,
  adds it to the mempool, and **gossips it to the network** so the next proposer
  includes it — closing the gap where a multi-validator network had no way to
  receive user transactions (only the single-node `veilux serve` did). The
  endpoint also answers `veilux_blockNumber`, `veilux_chainId`, and
  `veilux_nodeInfo`. Verified end-to-end: a transfer submitted to one validator
  is BFT-finalized across all of them and applied to state (sender debited,
  recipient credited).
- **Configurable `chain_id` and `network` at genesis**, surfaced via
  `veilux_chainId` / `veilux_nodeInfo`.

### Changed
- **Block validity now enforces a gas limit and timestamp rules.** A block is
  rejected on commit unless its `timestamp` is `>= parent` and within 2 hours of
  local time (`BadTimestamp`), and its total gas is `<= Limits::max_block_gas`
  (default 30M) (`BlockGasExceeded`). Assembly stops including commands once the
  gas limit would be exceeded. These bound block work and prevent arbitrary
  proposer timestamps.

## [0.3.8] - 2026-06-05

### Security
- **Audit pass: four vulnerabilities found and fixed**, each with a regression test.
  - **System-account impersonation (critical).** Keyless pool accounts
    (`staking/escrow`, `staking/rewards`) could be impersonated via
    trust-on-first-use key binding and drained. `submit_signed` now rejects any
    submitter whose name contains `/` or is empty (`ReservedAccount`).
  - **Slashing bypassed reward accounting.** A slashed validator kept earning
    reward-pool shares on burned stake (phantom weight) and drifted
    `total_stake`. Slashing now reduces the offender's reward weight and the
    global stake total.
  - **Private blobs leaked in public state.** The Storage Prism wrote raw blob
    bytes to public state regardless of visibility, exposing `Visibility::Parties`
    payloads via state queries. Plaintext is now stored publicly only for
    `Public` blobs; private blob bytes ride in the Veil-sealed event payload.
  - **Poison-command liveness DoS.** A command that failed at execution aborted
    the whole block, stalling production. `assemble_block` now probes each
    command and skips failures (dropping them) instead of aborting.
- See `docs/security.md` §2.10–2.13 for the full write-up.

## [0.3.7] - 2026-06-05

### Added
- **Dynamic base fee (EIP-1559 style).** When a chain sets `fee_target_gas > 0`
  at genesis, the per-gas price is no longer fixed: after each block the base
  price (kept deterministically in state at `fee/base_price`) adjusts up to
  ~12.5% toward demand — rising when a block exceeds the target gas and falling
  (never below the genesis floor) when it is under. With `fee_target_gas = 0` the
  price stays fixed. Verified live: under sustained congestion the base price
  climbed block over block.
- **Staking reward pool with proportional claims.** New `fund_rewards` and
  `claim_rewards` staking commands implement a MasterChef-style reward
  accumulator: funded LUX is distributed across all current stake in proportion
  to each staker's locked amount, and a staker's pending reward is settled
  automatically on every stake/unstake/delegate/undelegate. Stake added after a
  funding event cannot back-claim earlier rewards. Lets a chain share validator
  fee income with delegators fairly.

## [0.3.6] - 2026-06-05

### Added
- **Automatic equivocation detection & slashing.** Each validator now watches
  incoming consensus votes and, the moment it sees a peer sign two different
  blocks for the same height/round, it builds signed equivocation evidence and
  submits a `staking.slash` command (gossiped to the network). Honest repeat
  votes are ignored, and each offender is reported at most once. This closes the
  loop from the manual slashing primitive to fully on-chain enforcement.

### Changed
- **README rewritten** to cover the full current system: nine Prisms, Aurora BFT
  with auto-slashing, authenticated transport, configurable genesis token, fees,
  staking, governance, and the confidential token.

## [0.3.5] - 2026-06-05

### Added
- **Transaction fees / gas market (node-level).** When a chain sets
  `fee_price_per_gas > 0` at genesis, every command is charged
  `fee = gas_used × price` in the native token during block execution. The fee is
  split by `fee_burn_bps`: a configurable fraction is **burned** (deflationary)
  and the rest is paid to the **block proposer** as a reward. The charge is
  computed deterministically by every node during re-execution and capped at the
  payer's balance, so it never causes disagreement. Disabled by default. Verified
  live: a transfer charged the submitter, credited the proposer half the fee, and
  reduced total supply by the burned half.
- **Validator slashing for equivocation.** The Staking Prism gained a `slash`
  command: anyone may submit two messages a validator signed for the same
  consensus slot; the prism verifies both signatures against the offender's key
  and that they differ — unforgeable proof of double-signing — then **burns** 20%
  of the offender's self-bonded stake and records the offence so it cannot be
  replayed. Forged or self-consistent evidence is rejected.
- **Native-token fee helpers** in the Token Prism (`collect_fee`, `burn_from`)
  with value-conserving burn/reward accounting, reused by the node fee engine and
  staking slashing.

## [0.3.4] - 2026-06-05

### Added
- **Staking & Governance Prism** (`prisms/staking`) — turn the validator set
  into an economic system. Bond the native token (LUX) as stake, delegate to
  other validators, and run **stake-weighted on-chain governance**: open
  proposals, cast weighted votes, and finalize after a voting period. Staked
  value is escrowed (unspendable until unstaked); each party votes once per
  proposal.
- **Oracle Prism** (`prisms/oracle`) — bring trusted off-chain data on-chain
  (asset prices, large AI model outputs, real-world facts) via a **reporter
  quorum**. A feed defines a fixed reporter set and a signature threshold; a
  value update is accepted only when enough reporters sign its digest, with
  strictly-advancing rounds for anti-replay.
- **Confidential Token Prism** (`prisms/confidential`) — a privacy-preserving
  token whose **amounts and balances never appear on the public ledger**. Uses a
  shielded note/commitment model: public state holds only note commitments and
  spent flags, while amounts/owners ride in the Veil encrypted view. Transfers
  are value-conserving (outputs sum to the spent note), and an owner can
  **selectively disclose** a note's opening to an auditor without revealing
  anything else.
- **Configurable native token at genesis (`ChainSpec`).** A new chain chooses its
  own token **name, symbol, decimals, and total supply** plus the initial
  allocation distribution via a genesis JSON (`--genesis spec.json` on both
  `veilux serve` and `veilux validator`); a sensible default is used otherwise.
  Genesis seeding is deterministic and idempotent, so every validator sharing a
  spec converges on byte-identical state. The token crate gained native-token
  helpers (`native_token_id`, `seed_native_token`, `credit`/`debit`/
  `move_balance`) reused by staking and fees.

## [0.3.3] - 2026-06-05

### Added
- **Authenticated P2P transport (peer confirmation + IP allowlist).** Validators
  can now require a mutual Ed25519 signed handshake on every peer connection
  before any consensus traffic is accepted. Each side proves it holds the secret
  key of a registered validator by signing the other's per-connection challenge
  (domain-separated, so observed/replayed traffic is useless); a peer whose party
  is unknown or whose key does not match the registered one is dropped. Inbound
  connections are additionally filtered by an optional **IP allowlist** chosen by
  the node owner. Enabled with `veilux validator --secure [--allow-ip <IP> ...]`;
  the transport stays open (dev mode) when `--secure` is omitted. Verified live:
  a secure 3-validator mesh finalizes normally while an unlisted "intruder" node
  is rejected at the handshake (`peer party … is not a known validator`) without
  disturbing finality.

### Fixed
- **Consensus liveness: multi-view finality.** Votes are now tallied per
  `(height, round)` instead of per height. Previously, after a single leader
  failure and view change, a validator's prevote for the new view's block was
  rejected by its own engine as equivocation against its earlier-view prevote,
  so a fresh quorum could never form and the chain rotated views forever
  without finalizing. Validators can now legitimately vote across views at the
  same height, so the network finalizes through leader failover. Verified live
  with 4 networked validators (byte-identical hashes across nodes) and surviving
  the loss of one validator. Regression test added.

## [0.3.2] - 2026-06-05

### Fixed
- **Privacy: explorer now redacts private events.** The `explorer_*` event views
  no longer expose the payload of non-public events. For `Visibility::Parties`
  events the node returns only the commitment and stakeholder count (`redacted:
  true`), honoring the VeilLedger model — public observers can prove a private
  event happened without seeing its contents. The web explorer renders a privacy
  notice instead of the payload.

## [0.3.1] - 2026-06-05

### Added
- **Contract verification** — verify a deployed PhotonVM contract's source
  against its on-chain bytecode. New RPC methods `contract_getCode`,
  `contract_verify`, and `contract_getVerification`; surfaced in both SDKs and
  in the web explorer's "Verify Contract" page. Mismatched bytecode is rejected.
- **Explorer UI v2** — motion and graphics (animated hero, entrance animations,
  live row inserts, count-up stats), nav tabs (Explorer / Verify Contract /
  API Docs), a built-in API documentation page, and contract detail pages with
  a verified badge and source view.
- **Web Explorer** (`explorer/`) — a modern, Etherscan/Blockscout-style block
  explorer UI: dashboard stats, latest blocks & transactions, universal search
  (height / hash / command id), block & transaction detail pages, prism activity
  filter, state browser, and live updates over WebSocket. Zero-build static site
  (plain HTML/CSS/JS).
- **CORS preflight** handling in the JSON-RPC server (`OPTIONS` → 204) so
  browser-based explorers and dApps on other origins can call a node directly.
- **Explorer API** — a read-heavy `explorer_*` JSON-RPC namespace for indexers,
  block explorers, and dashboards: `explorer_stats` (chain totals + per-prism
  event breakdown), `explorer_recentBlocks`, `explorer_blockByHash`,
  `explorer_searchCommand` (locate a command and its events),
  `explorer_listByPrism`, and `explorer_statePrefix` (list state under a key
  prefix). Surfaced in both the Rust and TypeScript SDKs.
- **Bridge Prism** (`prisms/bridge`) — guardian-attested cross-chain transfers
  to Cosmos, Solana, Ethereum, or a custom chain. Register a guardian set with a
  signature quorum; lock tokens outbound (`send`) and redeem guardian-attested
  inbound transfers (`redeem`) with anti-replay sequencing. Bridged value reuses
  Token Prism balances, so it is real, spendable token balance. Builders added
  to both the Rust and TypeScript SDKs.
  (height / hash / command id), block & transaction detail pages, prism activity
  filter, state browser, and live updates over WebSocket. Zero-build static site
  (plain HTML/CSS/JS).
- **CORS preflight** handling in the JSON-RPC server (`OPTIONS` → 204) so
  browser-based explorers and dApps on other origins can call a node directly.

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

[Unreleased]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.5.1...HEAD
[0.5.1]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.3.9...v0.4.0
[0.3.9]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.3.8...v0.3.9
[0.3.8]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.3.7...v0.3.8
[0.3.7]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.3.6...v0.3.7
[0.3.6]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.3.5...v0.3.6
[0.3.5]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.3.4...v0.3.5
[0.3.4]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.3.3...v0.3.4
[0.3.3]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/VeiluxLabs/Veilux-Binary/releases/tag/v0.1.0
