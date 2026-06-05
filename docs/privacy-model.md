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

1. **Wrapped keys via X25519.** Replace passphrase-seeded view keys with
   per-party key-agreement so grants wrap keys to a grantee's public key,
   removing any shared-seed assumption.
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
