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

### 2.7 P2P transport authentication
- **Check:** On a real network each validator runs on its own server. Can a
  stranger who can reach a validator's port inject votes/blocks/commands or read
  gossip?
- **Mitigation:** Optional authenticated transport (`veilux validator --secure`).
  Every peer connection (inbound and outbound) must pass a **mutual Ed25519
  signed handshake** before any `NetMessage` is accepted: each side signs the
  other's fresh per-connection challenge (domain-separated with
  `veilux/net-handshake/v1`), and a peer whose party is unknown or whose key does
  not match the registered validator key is dropped. Inbound connections are
  additionally screened by an optional **IP allowlist** (`--allow-ip`). Because
  the challenge is random per connection, observing or replaying prior traffic
  does not let an attacker complete a handshake.
- **Tests:** `auth::tests::mutual_handshake_succeeds_between_known_validators`,
  `unknown_peer_is_rejected`, `forged_key_for_known_party_is_rejected`,
  `ip_allowlist_enforced_when_set`. Verified live: a secure 3-validator mesh
  finalizes while an unlisted intruder is rejected at the handshake.

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

---

## 3. Residual risks / out of scope (current build)

These are **known limitations**, documented honestly rather than hidden:

| Area | Status | Notes |
|------|--------|-------|
| Consensus | Aurora BFT (stake-weighted) | Prevote/precommit with 2/3+ finality, deterministic proposer rotation, quorum-synchronized view-change failover. Multi-view finality and live 4-node operation verified. |
| Networking | TCP gossip + authenticated handshake | Featherweight transport for proposals/votes/blocks/commands with optional mutual signed handshake and IP allowlist (`--secure`). No transport encryption yet (payloads are signed/authenticated but sent in cleartext). |
| Persistence | Append-only log + state snapshots | Blocks and state persist to disk and reload on restart. |
| Transport confidentiality | None | The handshake authenticates peers but does not encrypt the channel; run validators over a private network/VPN/WireGuard or add TLS for confidentiality (roadmap). |
| View key exchange | Passphrase-seeded | Production needs X25519-wrapped keys (see privacy-model §7). |
| Real AI execution | Simulated (deterministic) | Real models need verifiable compute (TEE/ZK). |
| Metadata privacy | Partial | Sizes/timing observable to non-stakeholders. |
| Key management | Seeds in memory | No HSM/keystore integration yet. |

**Bottom line:** the node runs as a **multi-node, Byzantine-fault-tolerant,
privacy-preserving chain** — consensus, authenticated networking, and
persistence are in place and exercised by tests and live multi-node runs. The
main remaining hardening items are transport-level confidentiality (encryption),
production key management (HSM/keystore), and verifiable AI execution. Treat the
current build as a solid testnet-grade core; complete the hardening checklist
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
- [ ] Until transport encryption lands, run the validator mesh over a private
      network or VPN (e.g. WireGuard) so authenticated payloads aren't observed
      in cleartext.
- [ ] For banking deployments, require scoped `DisclosureGrant`s with recorded
      justifications for every audit access.

---

## 5. Reporting

Security issues should be reported privately to the maintainers before public
disclosure. Include a reproduction and the affected crate/version.
