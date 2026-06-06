# Changelog

All notable changes to **VEILUX** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.8.0] - 2026-06-07

### Added
- **Name Service Prism (VNS) — an ENS-like naming add-on.** Register
  human-readable `name.veil` names that resolve to a `PartyId`/address, the tenth
  Prism shipped. Features: registration with optional expiry, renewal, ownership
  transfer, a settable resolution target, arbitrary text records (avatar, url,
  etc., capped), forward `resolve`, `lookup`, and `reverse_lookup`, and
  reclaiming of expired names. Strict label validation (lowercase `a-z0-9-`,
  1–63 chars, no leading/trailing hyphen). Expiry is driven by a new `chain/now`
  block-timestamp written into state on commit. Exposed over JSON-RPC as
  `veilux_resolveName`, `veilux_lookupName`, and `veilux_reverseName`, and
  demonstrated live in `veilux demo`. Six unit tests cover registration,
  resolution, reverse lookup, expiry/reclaim, ownership enforcement, and label
  validation.

## [0.7.6] - 2026-06-06

### Fixed
- **`eth_call` now routes directly to EVM precompiles.** A call to a precompile
  address (`0x01`–`0x05`) via `eth_call` previously returned empty because the
  handler only executed deployed contract bytecode. It now detects precompile
  targets and runs them directly, matching the behaviour of Geth/standard
  Ethereum nodes (e.g. `eth_call` to `0x03` with `"abc"` returns the `ripemd160`
  digest). Precompiles invoked from within contract frames were already correct;
  this only affects direct top-level calls. Tests:
  `eth_call_routes_to_ripemd160_precompile`,
  `eth_call_routes_to_identity_precompile`.

## [0.7.5] - 2026-06-06

### Added
- **`ripemd160` EVM precompile (address `0x03`)** — completes the common
  Ethereum precompile set alongside `ecrecover` (0x01), `sha256` (0x02),
  `identity` (0x04), and `modexp` (0x05). Output follows the Ethereum layout
  (20-byte digest left-padded into a 32-byte word). Test:
  `ripemd160_known_vector`.

### Changed
- **Durable persistence (fsync).** All on-disk writes now flush to stable
  storage before returning: `append_block` and `append_pending` `fsync` the log
  file, and every atomic snapshot (`save_state`, `save_private_state`,
  `save_private_commitments`, `rewrite_pending`) writes to a temp file, `fsync`s
  it, renames into place, then `fsync`s the directory. This prevents block,
  state, mempool, and private-commitment loss or torn writes on power failure or
  OS crash. Test: `durable_writes_leave_no_tmp_files`.

### Docs
- Corrected stale production-readiness claims: inter-contract EVM calls
  (`CALL`/`CALLCODE`/`DELEGATECALL`/`STATICCALL`/`CREATE`/`CREATE2`) and the
  `ripemd160` precompile are **shipped** (they were still listed as missing).
  Only BN/BLS pairing precompiles and a fork-exact gas schedule remain on the EVM
  gap list. Updated `README.md` and `docs/evm-compat.md` accordingly.

## [0.7.4] - 2026-06-06

### Security
- **CRITICAL: fixed an empty-signature consensus vote bypass** found in a full
  internal audit (`docs/audit-2026-06.md`). `Aurora::add_vote` only verified a
  vote's Ed25519 signature when the signature field was non-empty, so a network
  peer could submit votes attributed to any validator with an empty signature and
  have them counted toward the finality quorum unverified — enough to fabricate a
  2/3+ quorum and finalize an arbitrary block (total BFT-safety loss). The fix
  splits the trusted local path (`add_local_vote`, accepts only the node's own
  self-votes) from the strict network path (`add_vote`, now requires a valid
  signature from a known validator). Honest multi-node consensus is unchanged
  (votes are signed before broadcast); verified live on a 4-validator network.
  Tests: `rejects_unsigned_network_vote`,
  `rejects_forged_signature_for_another_validator`.

### Added
- **`docs/audit-2026-06.md`** — a full internal security audit report covering
  consensus, ingress, crypto, block validity, privacy, economics, transport, and
  every Prism, with findings (1 critical fixed here, 1 high fixed in 0.7.3) and an
  honest statement that an independent third-party audit is still required.

## [0.7.3] - 2026-06-06

### Security
- **Fixed a forged-majority quorum-slash vulnerability** introduced with quorum
  arbitration in 0.7.2 (found in internal audit before any release was used). The
  `staking.slash_quorum` handler trusted a caller-supplied stakeholder count and
  "majority" set, so an attacker could sign attestations with throwaway identities
  and slash an honest stakeholder. The `QuorumFraudProof` now carries the full
  `PrivateEnvelope`; the handler verifies the envelope commitment, derives the
  real stakeholder set/count from it, requires every attester to be a genuine
  stakeholder, and reconstructs each signed message from the bound commitment so
  signatures cannot be replayed across contexts. New adversarial test
  `forged_majority_cannot_slash_an_honest_stakeholder` proves a sybil majority
  cannot slash an honest party; see `docs/security.md` §2.15.

## [0.7.2] - 2026-06-06

### Added
- **Quorum arbitration for confidential-state divergence.** When *different*
  stakeholders of a confidential transaction report conflicting private roots,
  the node now resolves it by majority: `AttestationBook::canonical_root` picks
  the root signed by a strict majority of the stakeholder set, and every minority
  signer is slashed via a new `QuorumFraudProof` / `staking.slash_quorum` command.
  The proof carries the majority's signed attestations plus the offender's own
  contradicting signed attestation, so the staking prism verifies every signature,
  confirms the majority meets the quorum threshold, and burns the minority liar's
  stake — the offender is identified by unforgeable cryptographic majority, not by
  trusting any single node. A tie with no majority is flagged but never
  auto-slashed (no provable liar). End-to-end test
  `quorum_minority_liar_is_slashed_end_to_end` (4 stakeholders, 3 honest + 1 liar)
  confirms the liar's stake is actually reduced; `quorum_majority_decides_canonical_root_and_flags_minority`
  and `no_majority_means_no_canonical_root_and_no_offenders` cover the arbitration
  logic. This closes the last open item from the privacy-hardening roadmap.

## [0.7.1] - 2026-06-06

### Added
- **On-chain slashing for confidential-state divergence.** A stakeholder that
  signs two conflicting `(commitment, private_root)` attestations for the same
  confidential transaction is now punished: the receiving node turns the two
  signed attestations into a staking `EquivocationProof`
  (`Node::private_divergence_proof`) and auto-submits a `staking.slash`, gossiped
  to the network — the same unforgeable double-signature mechanism already used
  for consensus equivocation, so the liar's stake is burned. This closes the last
  open privacy-hardening item: a divergence is no longer merely detected and
  logged, it is penalized on-chain. End-to-end test
  `private_divergence_yields_a_valid_slash_proof` builds a real conflicting-root
  proof and confirms the staking prism reduces the offender's voting power.

## [0.7.0] - 2026-06-06

### Added
- **X25519 key-wrapped confidential shares (no more shared-seed assumption).**
  A `PrivateEnvelope` now carries an ephemeral X25519 public key; each
  stakeholder's sealed share is encrypted under a key derived from the ECDH of
  that ephemeral with the stakeholder's *static* X25519 key (derived from its
  seed). Opening a share therefore requires the recipient's X25519 **secret key**
  — holding the right party name with the wrong key fails to decrypt
  (`wrong_key_cannot_open_even_if_named_stakeholder`).
- **Signed private-root attestations + divergence detection.** After executing a
  confidential transaction, a stakeholder signs a `(commitment, private_root)`
  attestation (`RootAttestation`, Ed25519) and gossips it
  (`NetMessage::PrivateRoot`). Every node keeps an `AttestationBook`; if two
  stakeholders report different private roots for the same commitment it is
  flagged (`AttestationOutcome::Divergence`) and logged as a
  `PRIVATE-ROOT DIVERGENCE` warning. `veilux_privateRoot` now also reports the
  attestation count and a `consistent` flag. **Verified live on a 3-validator
  network:** a confidential tx submitted to one validator was gossiped so both
  stakeholder validators (alice, bob) executed it to **byte-identical private
  state** and cross-attested without divergence, while the non-stakeholder
  validator held no private state.

### Notes
- A quorum/slashing penalty for a flagged private-root divergence is the next
  step; today divergence is cryptographically detected and surfaced, not yet
  punished on-chain.

## [0.6.5] - 2026-06-06

### Added
- **Confidential transactions gossip across the network (private execution goes
  multi-node).** A new `NetMessage::Private` carries a `PrivateEnvelope` over the
  P2P transport. A validator's tx-ingress now accepts `veilux_submitPrivate`;
  the envelope is applied locally and **gossiped** to every peer. Each receiving
  validator that hosts a stakeholder party decrypts its sealed share and executes
  the inner command into its private state; validators that are not parties record
  only the commitment. Validators can host parties with
  `veilux validator --host-party NAME:PASSPHRASE`. **Verified live on a 3-validator
  network:** a confidential token tx submitted to one validator's RPC propagated
  by gossip so the two stakeholder validators (alice, bob) each executed it into
  their private state (the secret amount `7777` present only there) while the
  third, non-stakeholder validator held just the commitment — no `private_state.json`
  at all. The plaintext never appears on the wire (`private_envelope_roundtrip`
  test asserts this).

## [0.6.4] - 2026-06-06

### Added
- **Live private-execution tooling.** `veilux serve --host-party NAME:PASSPHRASE`
  (repeatable) makes a dev node a confidential-transaction **stakeholder**, and a
  new `veilux seal-private` subcommand builds a `PrivateEnvelope` JSON from the
  CLI. This made it possible to verify the privacy model end to end over real
  RPC: submitting the same envelope to a stakeholder node and an outsider node,
  the stakeholder executed it (private root changed) while the outsider recorded
  only the commitment (private root stayed zero), and the public `state_root` was
  unchanged on both. The plaintext never appears in the serialized envelope.

### Fixed
- **Private replay protection now survives restart.** The applied
  private-commitment set is persisted (`private_commitments.json`) and reloaded
  on startup, so a confidential envelope that was already applied is rejected
  with `DuplicatePrivateCommitment` even after the node restarts (previously the
  in-memory set was lost on restart and replay only failed incidentally inside
  the prism). Regression test `private_state_and_replay_guard_survive_restart`.

## [0.6.3] - 2026-06-06

### Added
- **Private execution — confidential transactions whose data stays on
  stakeholder nodes (Canton-style ordering/validation separation).** New
  `veil/src/private.rs` introduces the `PrivateEnvelope`: a public `commitment`
  plus one ChaCha20-Poly1305 sealed share of the inner `Command` per stakeholder.
  Only the commitment and opaque sealed shares are ordered on the global chain —
  a non-stakeholder (including non-party validators) can prove a confidential
  transaction happened but cannot decrypt it or learn its effects. A node hosting
  a stakeholder keyring decrypts its share, executes the inner command against a
  separate **private state tree**, and advances a **private state root** the
  global public `state_root` never reflects. Exposed via the `veilux_submitPrivate`
  and `veilux_privateRoot` RPC methods and persisted separately
  (`private_state.json`). Tests prove a stakeholder executes (private root
  changes, public root unchanged) while an outsider records only the commitment
  (private root stays empty), the plaintext never appears in the serialized
  envelope, and tampering is rejected by the commitment check.
- **EVM precompiles** at addresses `0x01`/`0x02`/`0x04`/`0x05`: `ecrecover`
  (secp256k1), `sha256`, `identity`, and `modexp` (U256-range), wired into the
  `CALL`/`STATICCALL` path so standard contracts (e.g. OpenZeppelin ECDSA
  recovery) work. `ripemd160` and the BN/BLS pairing precompiles are not yet
  implemented (documented in `docs/evm-compat.md`).

## [0.6.2] - 2026-06-06

### Added
- **Full EVM execution — inter-contract calls and contract creation.** The
  `veilux-evm` interpreter now implements `CALL`, `CALLCODE`, `DELEGATECALL`,
  `STATICCALL`, `CREATE`, and `CREATE2`, plus `EXTCODESIZE`/`EXTCODECOPY`/
  `EXTCODEHASH`, `RETURNDATASIZE`/`RETURNDATACOPY`, `SELFBALANCE`, and
  `SELFDESTRUCT`. The `Host` trait gained code/nonce/transfer/snapshot/revert, and
  the node's `StateHost` implements them against the `StateTree`. This makes real
  multi-contract Solidity work: a contract can call another contract and use its
  return value, `DELEGATECALL` runs library/proxy code against the caller's
  storage (and leaves the library's own storage untouched), `STATICCALL` forbids
  state mutation, value-bearing calls isolate storage, and `CREATE`/`CREATE2`
  deploy children at the canonical addresses. Sub-call failures roll back via a
  state snapshot while the caller continues; call depth is bounded at 64 so deep
  recursion can never overflow the native stack. Verified end to end through the
  node: a deployed contract `CALL`s a second deployed contract over RPC-applied
  transactions and returns its result. New tests:
  `contract_calls_another_contract_and_reads_return`,
  `delegatecall_runs_callee_code_in_caller_storage`,
  `create2_deploys_at_deterministic_address`, `static_call_blocks_sstore`,
  `call_depth_is_bounded`, and `eth::tests::inter_contract_call_works_end_to_end`.

### Security
- **`STATICCALL` is enforced at the opcode level** — `SSTORE`, `LOG*`, `CREATE*`,
  and `SELFDESTRUCT` inside a static context revert (`StaticViolation`),
  preventing read-only calls from mutating state.
- Sub-call gas is forwarded from the caller's remaining budget and the overall
  30M cap still applies, so nested calls cannot escape the DoS bound.

## [0.6.1] - 2026-06-06

### Added
- **Wallet/indexer RPC completeness.** The `eth_*` shim gained
  `eth_getTransactionByHash`, `eth_getBlockByHash`, and `eth_getLogs` (with an
  optional `address` filter). Receipts now carry the **real `blockHash`** and the
  actual emitted **logs** (previously empty/zero), and `eth_getBlockByNumber`
  accepts `latest`/`earliest`/`pending` tags and any height. Each applied eth
  transaction now seals a real anchor block, so `eth_blockNumber` advances and
  blocks are addressable by their true hash — fixing a bug where the post-tx
  `produce_block()` silently failed on the empty native mempool and the height
  never moved.

### Security
- **EVM denial-of-service hardening (audit §2.14).** EVM gas is now **clamped to
  30M** for deploys, calls, and `eth_call` regardless of the transaction's
  declared `gas_limit`, so attacker bytecode with `gas_limit = u64::MAX` and an
  infinite loop terminates with *out of gas* in milliseconds instead of hanging
  the node (verified live: ~40 ms, node stays responsive). Returned contract code
  is capped at 24,576 bytes (EIP-170).
- **RLP integer-overflow panic fixed.** A crafted length prefix in a raw
  transaction could make `start + len` overflow `usize` and panic — a remotely
  reachable crash via `eth_sendRawTransaction`. All RLP length math now uses
  `checked_add` and returns a decode error. Added adversarial/fuzz regression
  tests (infinite loop, memory bomb, bad jumpdest, stack underflow, truncated and
  random RLP, oversized code, replayed nonce, wrong chain id).

## [0.6.0] - 2026-06-06

### Added
- **Full EVM bytecode execution — run real Solidity contracts.** The new
  `veilux-evm` crate gained a from-scratch, dependency-light EVM (no `revm`): a
  complete 256-bit integer (`U256` with signed/unsigned arithmetic, shifts,
  `EXP`, `SIGNEXTEND`) and an `Interpreter` covering the mainstream opcode set —
  arithmetic, `KECCAK256`, environment/context (`CALLER`, `CALLVALUE`,
  `CALLDATA*`, `CODECOPY`, `NUMBER`, `TIMESTAMP`, `CHAINID`, `GAS`), memory,
  `SLOAD`/`SSTORE`, `JUMP`/`JUMPI` with jumpdest analysis, `PUSH1`–`PUSH32`,
  `DUP`/`SWAP`, `LOG0`–`LOG4`, and `RETURN`/`REVERT`, with metered gas and bounded
  memory. The node implements the EVM `Host` against its `StateTree`, so
  `eth_sendRawTransaction` now supports **contract deployment** (`to = null`
  runs init code and stores the returned runtime code at the
  `keccak256(rlp(sender, nonce))` address) and **contract calls** (runs runtime
  code with real persistent storage), plus read-only `eth_call` and `eth_getCode`.
  Verified live over RPC: deploying the classic selector-dispatched storage
  contract, calling `store(424242)`, then `eth_call retrieve()` returns `424242`
  via genuine `SSTORE`/`SLOAD` and ABI return encoding; the receipt reports the
  contract address, gas used, and `status 0x1`, and the nonce advances correctly.
- **Persistent mempool.** Accepted-but-not-yet-included transactions are now
  written to `mempool.jsonl` and replayed (re-validated) on startup, so pending
  work survives a node restart instead of being silently dropped. The log is
  rewritten after each committed block (and after poison-command pruning) to keep
  only still-pending transactions. Regression test
  `pending_transactions_survive_restart`.
- **Encrypted P2P transport (confidential, forward-secret gossip).** The
  `--secure` handshake now also performs an **authenticated X25519 ephemeral key
  exchange** — the ephemeral key is signed under each validator's long-term
  Ed25519 identity, so a man-in-the-middle cannot substitute its own key. Both
  sides derive per-direction **ChaCha20-Poly1305** keys (BLAKE3-XOF KDF) and every
  gossip frame (proposals, votes, blocks, commands) is sealed with a counter
  nonce; a frame that fails to decrypt (tamper/reorder/replay) drops the peer.
  Forward-secret: ephemeral keys are discarded on disconnect. Verified live: a
  4-validator secure network finalizes in lockstep at byte-identical hashes
  (`power=300 quorum=267`) entirely over the encrypted channel.

### Changed
- `eth_getTransactionReceipt` now reports `contractAddress` and `gasUsed`; the
  `EthApplied.to` field is `Option` (a deploy has no `to`). See
  `docs/evm-compat.md` for the updated scope (single-contract execution today;
  inter-contract `CALL`/`CREATE` is the next step).

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

[Unreleased]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.8.0...HEAD
[0.8.0]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.7.6...v0.8.0
[0.7.6]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.7.5...v0.7.6
[0.7.5]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.7.4...v0.7.5
[0.7.4]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.7.3...v0.7.4
[0.7.3]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.7.2...v0.7.3
[0.7.2]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.7.1...v0.7.2
[0.7.1]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.7.0...v0.7.1
[0.7.0]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.6.5...v0.7.0
[0.6.5]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.6.4...v0.6.5
[0.6.4]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.6.3...v0.6.4
[0.6.3]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.6.2...v0.6.3
[0.6.2]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.6.1...v0.6.2
[0.6.1]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/VeiluxLabs/Veilux-Binary/compare/v0.5.1...v0.6.0
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
