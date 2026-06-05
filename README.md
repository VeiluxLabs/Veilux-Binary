# VEILUX

[![CI](https://github.com/VeiluxLabs/Veilux-Binary/actions/workflows/ci.yml/badge.svg)](https://github.com/VeiluxLabs/Veilux-Binary/actions/workflows/ci.yml)
[![Release](https://github.com/VeiluxLabs/Veilux-Binary/actions/workflows/release.yml/badge.svg)](https://github.com/VeiluxLabs/Veilux-Binary/actions/workflows/release.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

> **Veil** (privacy by default) + **Lux** (light / illumination) — a featherweight, privacy-first, AI-native modular blockchain.

Repository: **[github.com/VeiluxLabs/Veilux-Binary](https://github.com/VeiluxLabs/Veilux-Binary)**

VEILUX is built around three ideas:

1. **A featherweight core.** The *Photon* kernel knows almost nothing. It defines the data shapes, one extension trait (`Prism`), a pipeline (`Cascade`), and a content-addressed state. That's it. Everything heavy is an add-on you compile in only if you need it. Release binaries are optimized for size (`opt-level = "z"`, LTO, stripped).

2. **Everything is a Prism (add-on).** A *Prism* is a self-contained capability. Shipped Prisms: **AI** (+ optional Ollama), **Storage**, **Token** (ERC-20-like), **NFT** (ERC-721-like), and **Contract** (PhotonVM). They **cascade**: one Prism can trigger another (the AI Prism offloads large results to the Storage Prism automatically). Add your own by implementing one trait.

3. **Privacy by ledger (VeilLedger).** The *Veil* layer gives you one logically shared ledger where **no participant sees data they aren't a stakeholder of**. Every node agrees on the same Merkle root of *blinded commitments*, while contents are sealed per-party into encrypted **views** and stored in per-party **sub-ledgers**.

```
            ╦  ╦╔═╗╦╦  ╦ ╦═╗ ╦
            ╚╗╔╝║╣ ║║  ║ ║╔╩╦╝
             ╚╝ ╚═╝╩╩═╝╚═╝╩ ╚═
   featherweight · privacy-first · AI-native
```

## Token

| | |
|---|---|
| Ticker | `LUX` |
| Subunit | `lumen` (1 LUX = 10¹⁸ lumen) |

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
├── prisms/
│   ├── ai/            # AI Prism: model registry + inference (+ optional Ollama)
│   ├── storage/       # Storage Prism: content-addressed blobs + pinning
│   ├── token/         # Token Prism: fungible tokens (ERC-20-like)
│   ├── nft/           # NFT Prism: non-fungible tokens (ERC-721-like)
│   └── contract/      # Contract Prism: PhotonVM smart contracts
└── node/              # assembles kernel + veil + consensus + store + prisms
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
cargo run --bin veilux -- serve # dev RPC node at http://127.0.0.1:8645
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
| `docs/INSTALL.md` | Install, build, run, troubleshoot, CI/CD, Docker |
| `docs/architecture.md` | System design, cascade, state model |
| `docs/add-ons.md` | Per-Prism specs (AI, Storage, Token, NFT, Contract) + how to build your own |
| `docs/consensus-networking.md` | Aurora BFT consensus, persistence, and gossip transport |
| `docs/rpc-sdk.md` | JSON-RPC API + Rust SDK for building applications |
| `docs/ai-ollama.md` | Running real AI models via Ollama |
| `docs/privacy-model.md` | Deep VeilLedger banking-grade privacy research |
| `docs/security.md` | Threat model + exploitation review + what runs safely |
| `docs/roadmap.md` | Future add-ons the chain needs next |

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

## License

Licensed under either of:

- MIT license ([LICENSE-MIT](LICENSE-MIT))
- Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.

## Contact

- Telegram: [@Winnodexx](https://t.me/Winnodexx)
- Email: [nathan@winnode.xyz](mailto:nathan@winnode.xyz)

## Author

Created and maintained by **nathan**. Original work — © 2026 nathan.
