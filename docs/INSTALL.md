# VEILUX — Installation & Running Guide

This guide takes you from a clean machine to a running VEILUX node, then shows
how to run tests, build size-optimized release binaries, and operate the node.

---

## 1. Prerequisites

| Requirement | Minimum | Notes |
|-------------|---------|-------|
| Rust toolchain | 1.85.0 (stable) | Install via [rustup](https://rustup.rs) |
| OS | Windows 10+, Linux, macOS | Tested on Windows (cmd/PowerShell) |
| RAM | 512 MB | The node is featherweight; the kernel itself is tiny |
| Disk | ~300 MB | Mostly the Rust build cache (`target/`) |

### Install Rust

Windows (PowerShell):

```powershell
winget install Rustlang.Rustup
# or download rustup-init.exe from https://rustup.rs
rustup default stable
```

Linux / macOS:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup default stable
```

Verify:

```bash
cargo --version    # expect 1.85.0 or newer
rustc --version
```

---

## 2. Get the code

```bash
cd veilux
```

The workspace layout:

```
veilux/
├── kernel/            Photon core (tiny)
├── veil/              VeilLedger privacy
├── prisms/ai/         AI add-on
├── prisms/storage/    Storage add-on
└── node/              the `veilux` binary
```

---

## 3. Build

### Development build (fast, unoptimized)

```bash
cargo build
```

### Release build (featherweight, size-optimized)

```bash
cargo build --release
```

The release profile is tuned for a small binary:

```toml
[profile.release]
opt-level = "z"      # optimize for size
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

The resulting binary is at `target/release/veilux` (`veilux.exe` on Windows).

---

## 4. Run the tests

```bash
cargo test --workspace
```

Expected: kernel, veil, and prism suites all green (23+ tests), covering:

- Merkle determinism and state roots (kernel)
- Encrypted view seal/open + wrong-party rejection (veil)
- Per-party projection and sub-ledgers (veil)
- Ed25519 sign/verify, tamper and replay rejection (veil)
- Scoped selective disclosure to an auditor (veil)
- Prism cascade and content-addressed storage dedup (prisms)

---

## 5. Run the node

### Show kernel info + installed Prisms

```bash
cargo run --bin veilux -- info
```

### Run the end-to-end privacy demo

```bash
cargo run --bin veilux -- demo
```

This:
1. Registers an AI model (public).
2. Runs a **private** inference visible only to `alice`.
3. Shows `bob` agreeing on the global root but unable to decrypt contents.
4. Issues a **scoped disclosure grant** to a `regulator` and audits one event.

### Control logging

```bash
# Windows PowerShell
$env:RUST_LOG="info"; cargo run --bin veilux -- demo

# Linux/macOS
RUST_LOG=debug cargo run --bin veilux -- demo
```

---

## 6. Quick start as a library

Add a Prism and run your own command in three steps:

```rust
use veilux_kernel::{Cascade, PartyId};
use veilux_veil::PartyIdentity;
use prism_storage::{put_command, StoragePrism};

// 1. assemble a node
let mut cascade = Cascade::new();
cascade.install(Box::new(StoragePrism::new()));
let mut node = Node::new(PartyId::new("validator-0"), cascade);

// 2. create an identity and sign a command
let alice = PartyIdentity::from_seed("alice", &[1u8; 32]);
let cmd = put_command(PartyId::new("alice"), Visibility::Public, 0, "note.txt", b"hello".to_vec());
node.submit_signed(alice.sign(cmd))?;

// 3. produce a block
let summary = node.produce_block()?;
println!("height {} root {}", summary.height, summary.state_root);
```

---

## 7. Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| `cargo: command not found` | Rust not on PATH | Restart shell after `rustup`; check `~/.cargo/bin` |
| `linker not found` (Windows) | Missing MSVC build tools | Install "Desktop development with C++" via Visual Studio Build Tools |
| `BadNonce` on submit | nonce reused or out of order | Use a strictly increasing nonce per party |
| `KeyMismatch` on submit | party already bound to another key | Use the same signing key for a given party |
| Unicode looks garbled in console | Windows code page | `chcp 65001` for UTF-8, or ignore (cosmetic) |

---

## 8. Continuous integration & releases

VEILUX ships with GitHub Actions workflows in `.github/workflows/`:

| Workflow | Trigger | What it does |
|----------|---------|--------------|
| `ci.yml` | push / PR | fmt check, clippy (`-D warnings`), build + test on Linux, Windows, macOS |
| `release.yml` | tag `v*` | cross-builds binaries for Linux (gnu+musl, amd64+arm64), Windows, macOS (intel+arm) and attaches them to the GitHub Release |
| `docker.yml` | push / tag | builds and pushes a multi-arch (amd64+arm64) image to GHCR |

### Cut a release

```bash
git tag v0.1.0
git push origin v0.1.0
```

This triggers `release.yml`, producing downloadable archives per platform.

### Build with Ollama (real AI models)

```bash
cargo build --release --features ollama
```

See `docs/ai-ollama.md` for configuring custom models.

### Docker

```bash
docker build -t veilux .
docker run --rm veilux info
```

---

## 9. Next steps

- Read `docs/architecture.md` for the system design.
- Read `docs/privacy-model.md` for the banking-grade privacy research.
- Read `docs/add-ons.md` for each Prism's spec and how to build your own.
- Read `docs/ai-ollama.md` for running real AI models via Ollama.
- Read `docs/security.md` for the threat model and exploitation review.
- Read `docs/roadmap.md` for future add-ons.
