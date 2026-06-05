# Contributing to VEILUX

Thanks for your interest in contributing! VEILUX is an open, privacy-first,
AI-native modular blockchain, and we welcome issues, ideas, and pull requests.

## Branching model

We keep `main` releasable at all times and integrate work on `develop`.

| Branch | Purpose |
|--------|---------|
| `main` | Stable, released code. Protected. Tags (`vX.Y.Z`) are cut from here. |
| `develop` | Integration branch. **Open your PRs against `develop`.** |
| `feat/*`, `fix/*`, `docs/*`, `chore/*` | Your working branches. |

### Workflow

```bash
# 1. Fork the repo on GitHub, then clone your fork
git clone https://github.com/<you>/Veilux-Binary
cd Veilux-Binary

# 2. Start from develop
git checkout develop
git pull origin develop

# 3. Create a working branch
git checkout -b feat/my-new-prism

# 4. Make changes, commit, push
git commit -m "feat(prism): add my-new-prism"
git push -u origin feat/my-new-prism

# 5. Open a Pull Request targeting `develop`
```

## Commit messages

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(consensus): add proposer failover
fix(rpc): handle empty params gracefully
docs(readme): clarify install steps
chore(ci): bump toolchain
test(token): cover allowance overflow
```

Types: `feat`, `fix`, `docs`, `chore`, `test`, `refactor`, `perf`, `ci`.

## Before you open a PR

Run the same gates CI runs. All must pass:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test --workspace
```

For the TypeScript SDK:

```bash
cd sdk-ts
npm install
npm run build
```

Checklist:

- [ ] Code is formatted (`cargo fmt`) and clippy-clean (`-D warnings`).
- [ ] Tests pass, and new behavior has tests.
- [ ] Determinism preserved — Prisms must produce identical output on every node
      (no floats/RNG/time/network inside `Prism::handle`).
- [ ] Docs updated when behavior or APIs change.
- [ ] `CHANGELOG.md` updated under `[Unreleased]` for user-visible changes.

## Writing a Prism

Adding a capability? It's a Prism — no kernel fork needed. See
[`docs/add-ons.md`](docs/add-ons.md) for the trait, state-namespacing rules, gas
metering, and the pre-ship checklist.

## Project layout

See the workspace map in [`README.md`](README.md) and the deep dives in
[`docs/`](docs/) (architecture, consensus, privacy, RPC/SDK, security, roadmap).

## Reporting security issues

Please **do not** open public issues for security vulnerabilities. Email
**nathan@winnode.xyz** (Telegram [@Winnodexx](https://t.me/Winnodexx)) with
details and a reproduction. See [`docs/security.md`](docs/security.md).

## Code of Conduct

Be respectful, constructive, and inclusive. Harassment or abuse of any kind is
not tolerated. Maintainers may remove comments, commits, or contributors that
violate these norms.

## License

By contributing, you agree that your contributions are dual-licensed under
**MIT OR Apache-2.0**, the same terms as the project.

---

Questions? Open a [discussion or issue](https://github.com/VeiluxLabs/Veilux-Binary/issues)
or reach out on Telegram [@Winnodexx](https://t.me/Winnodexx).
