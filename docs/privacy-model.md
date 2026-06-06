# VEILUX Privacy Model — Banking-Grade Confidentiality

> Deep dive into the Veil layer: how VEILUX's **VeilLedger** design delivers
> privacy strong enough for regulated finance, and how it reconciles
> confidentiality with mandatory oversight.

---

## 1. The institutional privacy problem

Public blockchains replicate every transaction to every node. For a bank that is
a non-starter: trading strategies, counterparty identities, balances, and client
PII cannot be broadcast to competitors and the public.

Yet the answer is not "hide everything". Regulated institutions need the
opposite of secrecy from their regulator: a provable, complete, tamper-evident
record. As industry analysis puts it, institutional privacy is about *selective
disclosure and data sovereignty* — protecting data from competitors while giving
regulators a glass-box view ([Chainlink, 2026](https://chain.link/article/institutional-blockchain-privacy-solutions)).
Content was rephrased for compliance with licensing restrictions.

VEILUX targets both at once:
- **Confidentiality** from everyone who is not a stakeholder.
- **Provability** that a transaction happened, to everyone.
- **Selective disclosure** to auditors/regulators, enforced by cryptography.

---

## 2. The VeilLedger design principles

VEILUX's privacy layer, **VeilLedger**, is built on two design principles that
are well established in confidential-ledger research for regulated finance:

1. **Sub-transaction privacy.** A transaction is decomposed into views; each
   participant receives only the views it is a stakeholder of. No party sees the
   whole transaction unless entitled to.
2. **Separation of ordering from validation.** The ordering layer routes and
   sequences *blinded commitments* between nodes but never sees contents;
   validation happens locally at the participants who actually hold the data.

The baseline trust model is therefore one of known participants exchanging
encrypted messages whose commitments are globally ordered — as opposed to a pure
math-based (ZK) model. VeilLedger uses this message/projection model as its
foundation and adds optional cryptographic hardening (see §7) so deployments can
dial the trust assumption toward "trust the math" where required.

> Prior art and background. The general approach to institutional ledger privacy
> — selective disclosure, data sovereignty, and a "glass-box" view for
> regulators — is discussed across the industry; see the References in §9 for
> background reading. All such material was summarized/rephrased for compliance
> with licensing restrictions.

---

## 3. Veil architecture

Three layers, each a module in the `veil` crate:

```
            ┌────────────────────────────────────────────┐
  event ──► │ projection.rs  split block into per-party   │
            │                encrypted VIEWS              │
            └───────────────┬─────────────────────────────┘
                            │ EncryptedView (per party)
            ┌───────────────▼─────────────────────────────┐
            │ view.rs       ChaCha20-Poly1305 seal/open   │
            │               + public COMMITMENT           │
            └───────────────┬─────────────────────────────┘
                            │ decrypt locally
            ┌───────────────▼─────────────────────────────┐
            │ ledger.rs     per-party SUB-LEDGER          │
            │               (only what the party may see) │
            └──────────────────────────────────────────────┘

  identity.rs    Ed25519 signing identity (who submitted)
  disclosure.rs  scoped view grants (who may audit)
```

### 3.1 The commitment trick (single shared root, hidden contents)

Every event has two faces:

- **Commitment** (`Event::commitment()` → a BLAKE3 hash): public, reveals
  nothing about the payload. All events' commitments form the block's
  `events_root` (a Merkle root). **Every node computes the same root**, even
  non-stakeholders, because commitments are not encrypted.
- **Ciphertext** (`EncryptedView`): the event sealed with ChaCha20-Poly1305 to a
  specific party.

This is what lets VEILUX maintain one logically shared ledger while hiding
contents: consensus is over commitments, confidentiality is per-view.

### 3.2 Views and keys

For each stakeholder party of an event, `projection.rs` produces an
`EncryptedView`:

- Key derivation: `key = BLAKE3(domain ‖ party_seed ‖ view_id)` where
  `view_id = event.commitment()`. Each `(event, party)` pair gets a unique key.
- Nonce derivation: `nonce = BLAKE3("…/nonce" ‖ commitment ‖ party)[..12]`.
  Because each `(key, nonce)` pair is used exactly once, ChaCha20-Poly1305's
  security requirements are met.
- AEAD binding: the commitment is the **associated data**, so a ciphertext
  cannot be transplanted onto a different commitment.

### 3.3 Sub-ledgers

Each party's node keeps a `SubLedger`: the ordered list of events it could
decrypt, plus the `validated_root` it checked against the global chain. This is
the party's private, faithful projection of the one shared ledger.

### 3.4 Private execution (data stays on stakeholder nodes)

The projection model above still re-executes every command on every node to
converge the global `state_root`. For workloads where even the *inputs* must
never leave the stakeholders' servers, VeilLedger adds a **private execution**
path (`veil/src/private.rs`), modeled on the Canton-style separation of ordering
from validation:

- A confidential transaction is wrapped in a **`PrivateEnvelope`**: a public
  `commitment` plus one ChaCha20-Poly1305 **sealed share** per stakeholder
  (each share encrypts the *same* inner `Command` under a key derived from that
  stakeholder's secret seed and a per-tx salt). The commitment binds the salt,
  the stakeholder set, and every share, so it is tamper-evident.
- Only the **commitment and the opaque sealed shares** are ordered on the global
  chain. Non-stakeholders — including every validator that is not a party —
  witness *that* a confidential transaction occurred (and can prove it via the
  commitment) but can neither decrypt the inner command nor learn its effects.
- A node that **hosts a stakeholder keyring** decrypts its share, executes the
  inner command against a separate **private state tree**, and advances a
  **private state root** that only stakeholders can compute. The global public
  `state_root` is never touched by a confidential transaction.

This is exactly the institutional requirement: the ordering layer sees blinded
commitments, while validation/execution happens locally at the participants who
actually hold the data. The node exposes it via `veilux_submitPrivate` (apply an
envelope) and `veilux_privateRoot` (read the local private root + count). Tests
prove a stakeholder node executes (private root changes, public root unchanged)
while a non-stakeholder node records only the commitment (private root stays
empty), and that a tampered envelope is rejected.

> **Hardening status.** Sealed shares are now **X25519 key-wrapped**: each
> envelope carries an ephemeral X25519 public key, and a recipient's share key is
> derived from the ECDH of that ephemeral with the recipient's static X25519 key
> (derived from its seed) — so opening a share requires the recipient's *secret
> key*, not a shared seed (a wrong key for the right party name fails to decrypt).
> Stakeholders also **sign a `(commitment, private_root)` attestation** after
> executing; these are gossiped (`NetMessage::PrivateRoot`) so any divergence in
> the confidential state between stakeholders is detected and flagged
> (`AttestationOutcome::Divergence`). A stakeholder that signs **two conflicting
> private roots for the same commitment** is now **slashed on-chain**: the node
> turns the two signed attestations into a staking `EquivocationProof` and submits
> a `staking.slash` (the same unforgeable double-sign mechanism used for consensus
> equivocation), burning the offender's stake. When **different** stakeholders
> disagree, **quorum arbitration** decides: if a strict majority of the
> stakeholder set signed one root, that root is canonical and every minority
> signer is slashed via a `QuorumFraudProof` (`staking.slash_quorum`) carrying the
> majority's signatures plus the offender's own contradicting signature — the liar
> is identified by cryptographic majority, not by trusting any single node. A tie
> with no majority is flagged but not auto-slashed (no provable liar). Verified
> live on a 3-validator network (honest case: agreement, no false slash); both
> slash paths are covered end to end by
> `private_divergence_yields_a_valid_slash_proof` and
> `quorum_minority_liar_is_slashed_end_to_end`.

#### Try it live

```bash
# 1. start a stakeholder node (hosts party "alice") and an outsider node
veilux serve --addr 127.0.0.1:8680 --host-party alice:alice-pass --datadir ./stake
veilux serve --addr 127.0.0.1:8690 --datadir ./outsider

# 2. build a confidential transaction sealed to alice + bob
veilux seal-private --submitter alice --nonce 0 --salt round1 \
  --party alice:alice-pass --party bob:bob-pass --token Secret:SEC:5000 > env.json

# 3. submit the SAME envelope to both nodes via veilux_submitPrivate
#    stakeholder -> {"executed":true,  "private_root":"0x…"}   (executes locally)
#    outsider    -> {"executed":false, "private_root":"0x0…"}  (records commitment only)
# both nodes' public state_root is unchanged; veilux_privateRoot reads the local root
```

Verified end to end: the stakeholder advances its private root while the outsider
learns nothing but the commitment, the public root never moves, and a replayed
envelope is rejected (`DuplicatePrivateCommitment`) even across a restart.

---

## 4. What different observers learn

| Observer | Sees tx happened? | Sees `Visibility` shape? | Sees rough size? | Sees contents? |
|----------|:----:|:----:|:----:|:----:|
| Stakeholder party | ✅ | ✅ | ✅ | ✅ |
| Non-stakeholder node | ✅ (commitment in root) | ✅ | ✅ | ❌ |
| Authorized auditor (grant) | ✅ | ✅ | ✅ | ✅ (only in-scope) |
| Outside world | ✅ (root only) | ❌ | ❌ | ❌ |

The deliberate residual leakage to a non-stakeholder is **metadata**: that
*some* event occurred, its visibility *shape*, and an approximate size. §7
describes how to reduce even that.

---

## 5. Selective disclosure (the compliance half)

`disclosure.rs` implements **view grants**, modeled on the view-key pattern used
by privacy chains for compliance: viewing access flows through keys, and sharing
a viewing key lets a third party decrypt exactly the activity that key covers
([Inco, 2025](https://www.inco.org/blog/programmable-view-access);
[Aleo, 2025](https://aleo.org/post/aleo-view-key-compliance)).
Content was rephrased for compliance with licensing restrictions.

A `DisclosureGrant` is a capability that:
- names the **grantor** (data owner) and **grantee** (auditor/regulator),
- carries a **scope** — `All`, `Prism("token")`, `HeightRange{from,to}`, or an
  explicit set of event commitments,
- contains the per-event view keys for only the in-scope events,
- records a **justification** (legal basis) for the audit trail.

`audit_open` then decrypts exactly the in-scope views and nothing else. A
regulator handed a `Prism("token")` grant can review all token transfers for a
period without ever seeing the party's unrelated AI inferences.

This separates **signing identity** (Ed25519, "who acted", in `identity.rs`)
from **viewing capability** ("who may read"), so a party can grant read access
without ever exposing its signing key.

---

## 6. Threat model (privacy-specific)

| Adversary | Goal | Mitigation |
|-----------|------|------------|
| Curious validator | Read others' tx contents | Validators order commitments, never hold non-stakeholder keys |
| Malicious peer | Forge a view onto a real commitment | AEAD binds commitment as associated data; forgery fails |
| Replaying client | Resubmit an old command | Per-party strictly-increasing nonce (kernel + node) |
| Impersonator | Submit as another party | Ed25519 signature + trust-on-first-use key binding |
| Over-broad auditor | Read beyond mandate | Grants are scoped; `audit_open` filters by scope |
| Metadata analyst | Infer activity from sizes/timing | Padding & batching (§7), roadmap item |

---

## 7. Hardening roadmap (toward "trust the math")

The shipped Veil layer is a message/projection privacy model. These
optional upgrades reduce trust assumptions and residual leakage:

1. **Wrapped keys via X25519.** ✅ Shipped. Private-execution sealed shares use
   an ephemeral-static X25519 ECDH per envelope, so a recipient needs its secret
   key (not a shared seed) to open its share. (The passphrase-seeded *view* keys
   for the projection model can adopt the same wrapping next.)
2. **ZK proof of valid state transition.** Attach a succinct proof that an
   encrypted event corresponds to a valid Prism transition, so non-stakeholders
   verify *correctness* without seeing contents (the Prividium-style "trust the
   math" direction).
3. **TEE-backed validation.** Run Prism execution inside attested Trusted
   Execution Environments (Intel TDX, AMD SEV-SNP) so even the executing node
   cannot read data in use — the confidential-computing approach now used for
   private AI inference and finance workloads
   ([Chainlink, 2026](https://chain.link/article/confidential-computing-blockchain);
   [Phala, 2025](https://phala.com/de/learn/Confidential-Computing-in-Finance)).
   Content was rephrased for compliance with licensing restrictions.
4. **Metadata defenses.** Fixed-size view padding and per-epoch batching to blunt
   size/timing correlation.
5. **Proof of reserves / solvency** without disclosure, via range proofs over
   committed balances.

---

## 8. Mapping to banking requirements

| Requirement | VEILUX mechanism |
|-------------|------------------|
| Client data confidential from competitors | Per-party encrypted views |
| Counterparties see only shared deals | `Visibility::Parties` + projection |
| Regulator full, provable view | Scoped `DisclosureGrant` + commitment proofs |
| Non-repudiation of who acted | Ed25519 signed commands |
| Tamper-evident audit trail | Merkle `events_root` + `state_root` chain |
| Need-to-know enforced by crypto, not policy | AEAD per-view sealing |
| Data residency / sovereignty | A node only holds keys for parties it hosts |

---

## 9. References (background reading)

These are general industry sources on confidential ledgers, selective
disclosure, and confidential computing. They are listed as prior-art background
only; VeilLedger is VEILUX's own implementation.

- Confidential ledger privacy & technical primers (industry overviews)
- Messari — confidential multi-party ledger analysis: https://messari.io/
- L2BEAT — institutional privacy vs ZK approaches: https://l2beat.com/
- Chainlink — Institutional Blockchain Privacy; Confidential Computing for Blockchain: https://chain.link/article/institutional-blockchain-privacy-solutions , https://chain.link/article/confidential-computing-blockchain
- Inco — Programmable View Access: https://www.inco.org/blog/programmable-view-access
- Aleo — View Key & Compliance: https://aleo.org/post/aleo-view-key-compliance
- Phala — Confidential Computing in Finance: https://phala.com/de/learn/Confidential-Computing-in-Finance

*All external content referenced above was rephrased/summarized for compliance
with licensing restrictions; see links for originals.*
