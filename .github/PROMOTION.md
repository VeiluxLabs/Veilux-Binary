# VEILUX — promotion & submission kit

Ready-to-paste copy for sharing the project. **Positioning rule:** VEILUX is an
open-source **research / engineering** project. It has **not** been audited by an
independent third party and has not run a public mainnet. Never market it to
traders/investors or imply it is production-ready or a financial product. The
audience is **developers and engineers**.

---

## 0. GitHub repo hygiene (do this first)

- **About box:** "A featherweight, privacy-first, AI-native Layer-1 blockchain in Rust. Deploy Solidity, move private value between parties, run on-chain AI. Testnet-grade, unaudited."
- **Topics:** `blockchain` `layer1` `rust` `evm` `privacy` `bft-consensus` `ai` `cryptography` `web3` `smart-contracts`
- **Enable GitHub Pages:** Settings → Pages → Source = GitHub Actions (the `Docs (GitHub Pages)` workflow deploys `/docs`). Site: `https://veiluxlabs.github.io/Veilux-Binary/`
- **Releases:** already automated — each `vX.Y.Z` tag attaches binaries for Linux/macOS/Windows.
- **Add a demo GIF** to the top of the README (record `veilux demo`).

---

## 1. Hacker News — "Show HN"

**Title (≤ 80 chars):**
`Show HN: VEILUX – a privacy-first, AI-native Layer-1 blockchain in Rust`

**URL:** https://github.com/VeiluxLabs/Veilux-Binary

**First comment (post immediately after submitting):**

> I built VEILUX, a Layer-1 blockchain from scratch in Rust. It runs as a single
> binary: BFT consensus with proposer failover, a from-scratch EVM (deploy and
> call Solidity over `eth_*`, so MetaMask/ethers.js work), and a privacy layer
> where each transaction is sealed per-party — every node agrees on a Merkle root
> of blinded data, but only stakeholders can decrypt their own events. There's
> also on-chain slashing for equivocation, nine pluggable "Prisms" (token, NFT,
> AI inference, storage, etc.), and Rust + TypeScript SDKs.
>
> I want to be upfront: this is **testnet-grade and unaudited**. I did a full
> internal security audit (writeup in the repo, including one critical consensus
> bug I found and fixed), but an independent third-party audit and a public
> testnet under adversarial load are still required before anyone secures real
> value with it. I'm sharing it as an engineering project, not a coin.
>
> Happy to go deep on the privacy model (closest in spirit to Canton's
> per-participant ledgers), the consensus, or the EVM. Feedback welcome.

---

## 2. Reddit

### r/rust
**Title:** `VEILUX: a from-scratch Layer-1 blockchain in Rust (BFT + EVM + per-party privacy)`
Lead with the Rust angle: workspace of small crates, no `//` line-comments style,
1.85 toolchain, from-scratch U256/EVM interpreter and RLP, minimal deps. Link the
repo and the architecture doc. Flair: "Project".

### r/CryptoTechnology
**Title:** `Built a privacy-first L1 with per-party sealed ledgers (Canton-style) + EVM compatibility`
Focus on the privacy model and honest limitations. This sub bans price/shilling —
keep it purely technical.

### r/ethdev
**Title:** `An EVM-compatible chain in Rust where you can also move balances privately`
Focus on `eth_*` compatibility, precompiles, deploy via MetaMask, and where it
diverges from mainnet (simplified gas, legacy+EIP-155 txs only).

**Reddit rules:** read each sub's self-promotion policy; comment in threads before
posting your own project; never cross-post the identical text.

---

## 3. Lobste.rs
Tag: `rust`, `distributed`, `crypto`. Same Show-HN style intro. Lobsters is
small but high-signal and strict about low-effort self-promotion — only post if
you'll stick around to answer questions.

---

## 4. dev.to / Hashnode article

**Title:** `Building a privacy-first Layer-1 blockchain from scratch in Rust`

Outline:
1. Why another L1 — the per-party privacy gap (Ethereum is fully public, Canton is permissioned/JVM).
2. Architecture: cascade + Prisms, the VeilLedger privacy model.
3. The from-scratch EVM: U256, interpreter, RLP, precompiles, inter-contract calls.
4. Consensus: Aurora BFT, the critical empty-signature vote bug I found and fixed.
5. Honest status: what's done vs what needs an external audit.

Cross-post to Hashnode with a canonical URL pointing back to dev.to.

---

## 5. Awesome lists (open a PR adding VEILUX)

- `rust-unofficial/awesome-rust` (Applications → Blockchain)
- `yjjnls/awesome-blockchain`
- `sobolevn/awesome-cryptography` (if it fits the privacy/crypto criteria)

Entry: `[VEILUX](https://github.com/VeiluxLabs/Veilux-Binary) — featherweight, privacy-first, AI-native Layer-1 in Rust (testnet-grade, unaudited).`

---

## 6. Other channels

- **This Week in Rust** — submit the repo / a blog post via their PR process.
- **r/rust "what's everyone working on this week"** weekly thread — low-friction.
- **Mastodon / X** with `#rustlang #blockchain` and a link to the demo GIF.

---

## Do-not

- Don't post to investing/trading subs or Discords.
- Don't claim "audited", "production-ready", "mainnet", or imply token value.
- Don't paste identical text across many subs in a short window (spam filters).
