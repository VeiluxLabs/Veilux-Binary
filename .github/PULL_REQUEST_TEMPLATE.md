# Summary

<!-- What does this PR do and why? Link any related issues (e.g. Closes #123). -->

## Type of change

- [ ] feat — new feature
- [ ] fix — bug fix
- [ ] docs — documentation only
- [ ] refactor / perf — no behavior change
- [ ] chore / ci — tooling
- [ ] test — adding or improving tests

## Checklist

- [ ] Targets the `develop` branch
- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo build --workspace` and `cargo test --workspace` pass
- [ ] (if SDK touched) `cd sdk-ts && npm run build` passes
- [ ] New behavior is covered by tests
- [ ] Determinism preserved (no floats/RNG/time/network in `Prism::handle`)
- [ ] Docs updated where relevant
- [ ] `CHANGELOG.md` updated under `[Unreleased]` for user-visible changes

## How was this tested?

<!-- Commands run, scenarios checked, nodes spun up, etc. -->

## Notes for reviewers

<!-- Anything that needs special attention, tradeoffs, or follow-ups. -->
