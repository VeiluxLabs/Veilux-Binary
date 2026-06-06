# VEILUX Security & Exploitation Review

This document records the threat model, the exploitation checks performed against
the node and add-ons, what is already mitigated, and what remains out of scope
for the current implementation. It answers the question: *can the node and its
add-ons be safely run, and where are the sharp edges?*

---

## 1. Trust boundaries

```
   external client ──(SignedCommand)──► node.submit_signed ──► mempool
                                            │  verify sig, bind key,
                                            │  check nonce, size, routing
                                            ▼
                                         Cascade.apply ──► Prism.handle (state)
                                            │
                                            ▼
                                         produce_block ──► Veil projection
```

The single ingress for untrusted input is `Node::submit_signed`. Everything past
it has already been authenticated and bounds-checked. Derived commands (produced
by Prisms during a cascade) are trusted because they originate from
deterministic, already-vetted code — not from the network.

---

## 2. Exploitation checks performed

Each item below was implemented and is covered by a test or an enforced runtime
check.

### 2.1 Authentication & impersonation
- **Check:** Can a client submit a command as a party they don't control?
- **Mitigation:** Every external command is a `SignedCommand`; `verify_signed`
  validates an Ed25519 signature over canonical `signing_bytes`. The first valid
  key for a party is bound in the account registry (trust-on-first-use); later
  commands must match (`KeyMismatch` otherwise).
- **Tests:** `identity::tests::sign_and_verify_roundtrip`,
  `tampered_payload_fails`, `tampered_nonce_fails`.

### 2.2 Replay / ordering
- **Check:** Can an old signed command be replayed?
- **Mitigation:** Per-party strictly-increasing `nonce`, enforced in
  `submit_signed` (`BadNonce`). The nonce is inside `signing_bytes`, so it can't
  be altered without invalidating the signature.

### 2.3 Tampering in flight
- **Check:** Can a relay change `prism`, `visibility`, or `payload`?
- **Mitigation:** All of these are folded into `signing_bytes` with domain
  separation, so any change breaks verification.

### 2.4 Resource exhaustion (DoS)
- **Check:** Oversized payloads, mempool flooding, unbounded blocks, cascade
  bombs.
- **Mitigations:**
  - `Limits::max_payload_bytes` (1 MiB) rejects giant commands (`TooLarge`).
  - `Limits::max_mempool` applies backpressure (`MempoolFull`).
  - `Limits::max_block_commands` bounds work per block.
  - `MAX_CASCADE_DEPTH = 8` bounds Prism→Prism fan-out (`DepthExceeded`).
  - Storage `MAX_BLOB_SIZE` (1 MiB) bounds a single blob.

### 2.5 Routing to a missing handler
- **Check:** A command targeting an uninstalled Prism.
- **Mitigation:** `submit_signed` rejects unknown prisms early
  (`UnknownPrism`); the cascade also re-checks (`CascadeError::UnknownPrism`).
- **Test:** `cascade::tests::unknown_prism_errors`.

### 2.6 Privacy leakage
- **Check:** Can a non-stakeholder decrypt an event? Can a view be forged onto a
  real commitment?
- **Mitigations:** ChaCha20-Poly1305 per-view sealing; commitment bound as AEAD
  associated data; keys derived per `(party, view)`.
- **Tests:** `view::tests::wrong_party_cannot_open`,
  `commitment_is_independent_of_recipient`,
  `projection::tests::party_only_sees_own_views`.

### 2.7 P2P transport authentication & encryption
- **Check:** On a real network each validator runs on its own server. Can a
  stranger who can reach a validator's port inject votes/blocks/commands or read
  gossip?
- **Mitigation:** Optional authenticated, **end-to-end encrypted** transport
  (`veilux validator --secure`). Every peer connection (inbound and outbound)
  must pass a **mutual Ed25519 signed handshake** before any `NetMessage` is
  accepted: each side signs the other's fresh per-connection challenge
  (domain-separated with `veilux/net-handshake/v1`), and a peer whose party is
  unknown or whose key does not match the registered validator key is dropped.
  The same handshake carries a signed **X25519 ephemeral public key** (folded
  into the signed proof, so a man-in-the-middle cannot substitute its own), from
  which both sides derive per-direction **ChaCha20-Poly1305** keys (BLAKE3-XOF
  KDF). After the handshake every gossip frame is sealed with a counter nonce; a
  frame that fails to decrypt (tamper/reorder/replay) drops the peer. The
  exchange is forward-secret — ephemeral keys are discarded on disconnect, so a
  later identity-key compromise cannot decrypt captured past traffic. Inbound
  connections are additionally screened by an optional **IP allowlist**
  (`--allow-ip`).
- **Tests:** `auth::tests::mutual_handshake_succeeds_between_known_validators`
  (also asserts the encrypted frame round-trips), `unknown_peer_is_rejected`,
  `forged_key_for_known_party_is_rejected`, `ip_allowlist_enforced_when_set`.
  Verified live: a secure 4-validator mesh finalizes at byte-identical hashes
  entirely over the encrypted channel while an unlisted intruder is rejected at
  the handshake.

### 2.8 Consensus divergence via nondeterminism
- **Check:** Could a Prism produce different output on different nodes (forking
  state)?
- **Mitigation:** Prisms are required to be deterministic; the reference AI
  Prism derives all outputs from BLAKE3 instead of floats/RNG. `StateTree` uses
  a `BTreeMap` for deterministic ordering of the `state_root`.
- **Tests:** `prism_ai` determinism test; `state::tests::root_is_order_independent`.

### 2.9 Integer overflow
- **Check:** Cost/refcount/nonce arithmetic overflow.
- **Status:** Costs use `u64` with values far below overflow; refcounts use
  checked decrement guarded by a zero-check. Release profile uses
  `panic = "abort"`, so any unexpected overflow aborts rather than wraps in a
  debug build. **Recommended hardening:** switch hot arithmetic to
  `saturating_*`/`checked_*` (roadmap).

### 2.10 System-account impersonation (found & fixed)
- **Check:** Pooled funds live in keyless system accounts (`staking/escrow`,
  `staking/rewards`). Under trust-on-first-use key binding, could an attacker
  submit a `token.transfer` as one of these accounts (binding it to their own
  key) and drain the pool?
- **Fix:** `submit_signed` now rejects any submitter whose name contains `/` or
  is empty (`ReservedAccount`). Real parties are flat labels; every system and
  state-derived account uses `/`, so they can never originate external commands.
- **Tests:** `node::tests::system_account_cannot_submit_commands`,
  `normal_party_still_submits`.

### 2.11 Slashing vs. reward accounting (found & fixed)
- **Check:** Slashing reduced a validator's bonded stake and burned escrow, but
  did it update the reward-pool weight? If not, a slashed validator keeps
  earning reward shares on the burned amount (phantom stake) and `total_stake`
  drifts, letting them siphon the pool.
- **Fix:** the slash handler now calls `update_lock(offender, -burned)`, which
  harvests pending rewards and reduces both the offender's reward weight and the
  global `total_stake`.
- **Test:** `staking::tests::slashed_stake_stops_earning_rewards`.

### 2.12 Private blob plaintext in public state (found & fixed)
- **Check:** The Storage Prism wrote raw blob bytes to public state at
  `storage/blob/<cid>` regardless of visibility, so a `Visibility::Parties`
  blob's contents were readable by anyone via `explorer_statePrefix` /
  `veilux_getState` — contradicting the privacy claim.
- **Fix:** plaintext is stored in public state **only for `Public` blobs**. A
  private put keeps just the size/commitment pin record on-chain; the bytes ride
  in the sealed event payload (`StoredPrivate`) delivered to stakeholders via
  Veil. The AI Prism's large-result offload propagates the inference's
  visibility, so private inference outputs are covered too.
- **Tests:** `storage::tests::private_blob_bytes_never_enter_public_state`,
  `public_blob_still_readable`.

### 2.13 Poison-command liveness DoS (found & fixed)
- **Check:** A command valid at submit time can fail at execution (e.g. its
  balance was spent by an earlier command in the same block). `assemble_block`
  used `?`, so one failing command aborted the **entire block** — a cheap way to
  stall block production indefinitely.
- **Fix:** `assemble_block` now executes each candidate against a probe clone and
  **skips** any that fail (dropping them from the mempool), so a poison command
  can never block the good ones. `commit_block` stays strict (a failure there
  means a lying proposer and the block is rejected).
- **Test:** `node::tests::poison_command_does_not_stall_block_production`.

### 2.14 EVM bytecode DoS & RLP panic (found & fixed)
- **Check:** `eth_sendRawTransaction` accepts attacker-controlled bytecode and an
  attacker-chosen `gas_limit`. Could a transaction hang the node (infinite loop),
  exhaust memory, bloat state with huge code, or panic the RLP decoder?
- **Fixes:**
  - **Unbounded gas.** The EVM previously ran with the tx's declared gas
    (`gas_limit.max(1_000_000)`), so `gas_limit = u64::MAX` plus a
    `JUMPDEST;PUSH;JUMP` loop would spin while holding the node lock. Gas is now
    **clamped to 30M** for deploys, calls, and `eth_call`, so a loop hits *out of
    gas* in milliseconds. Verified live: the malicious deploy returns in ~40 ms
    and the node stays responsive.
  - **State bloat.** Returned contract code is capped at **24,576 bytes**
    (EIP-170).
  - **RLP integer-overflow panic.** A crafted length prefix could make
    `start + len` overflow `usize` and panic (a remotely reachable crash). All
    length math now uses `checked_add` and returns a decode error instead.
- **Tests:** `vm::tests::infinite_loop_halts_on_out_of_gas`,
  `memory_bomb_is_bounded`, `jump_to_non_jumpdest_is_rejected`,
  `stack_underflow_does_not_panic`, `rlp::tests::truncated_length_prefix_does_not_panic`,
  `fuzz_random_bytes_never_panic`, `eth::tests::infinite_loop_deploy_is_rejected_not_hung`,
  `oversized_contract_code_is_rejected`, `garbage_raw_tx_does_not_panic`,
  `replayed_nonce_rejected`, `wrong_chain_id_rejected`.

---

## 3. Residual risks / out of scope (current build)

These are **known limitations**, documented honestly rather than hidden:

| Area | Status | Notes |
|------|--------|-------|
| Consensus | Aurora BFT (stake-weighted) | Prevote/precommit with 2/3+ finality, deterministic proposer rotation, quorum-synchronized view-change failover. Multi-view finality and live 4-node operation verified. |
| Networking | TCP gossip + authenticated, encrypted handshake | Featherweight transport for proposals/votes/blocks/commands with optional mutual signed handshake, IP allowlist, and X25519/ChaCha20-Poly1305 end-to-end encryption (`--secure`). |
| Persistence | Append-only log + state snapshots + persistent mempool | Blocks, state, and pending transactions persist to disk and reload on restart. |
| Transport confidentiality | Encrypted under `--secure` | The handshake authenticates peers and establishes a forward-secret ChaCha20-Poly1305 channel; the open dev transport (no `--secure`) stays cleartext for local inspection. |
| View key exchange | Passphrase-seeded | Production needs X25519-wrapped keys (see privacy-model §7). |
| Real AI execution | Simulated (deterministic) | Real models need verifiable compute (TEE/ZK). |
| Metadata privacy | Partial | Sizes/timing observable to non-stakeholders. |
| Key management | Seeds in memory | No HSM/keystore integration yet. |

**Bottom line:** the node runs as a **multi-node, Byzantine-fault-tolerant,
privacy-preserving chain** — consensus, authenticated **and encrypted**
networking, and persistence (with a restart-safe mempool) are in place and
exercised by tests and live multi-node runs. The main remaining hardening items
are production key management (HSM/keystore) and verifiable AI execution. Treat
the current build as a solid testnet-grade core; complete the hardening checklist
before any value-bearing deployment.

---

## 4. Secure operation checklist

- [ ] Generate party signing keys with `PartyIdentity::generate` (OS RNG); never
      reuse seeds across parties.
- [ ] Keep view-keyring seeds and signing seeds in separate secured storage.
- [ ] Run only Prisms you trust; a malicious Prism can corrupt state for its own
      namespace and emit derived commands (bounded by cascade depth).
- [ ] Set `Limits` appropriately for your hardware before exposing any ingress.
- [ ] When adding a network layer, terminate untrusted input at
      `submit_signed` and never bypass it.
- [ ] Run validators with `--secure` and a curated `--peer`/`--allow-ip` set so
      only known validator keys (from allowlisted IPs) can join the gossip mesh.
      `--secure` also encrypts the channel (X25519 + ChaCha20-Poly1305), so
      authenticated payloads are no longer observable in cleartext.
- [ ] For banking deployments, require scoped `DisclosureGrant`s with recorded
      justifications for every audit access.

---

## 5. Reporting

Security issues should be reported privately to the maintainers before public
disclosure. Include a reproduction and the affected crate/version.
