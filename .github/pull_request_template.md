<!-- What this changes and why, in a few lines. Stacked? Say what it sits on. -->



Closes #

### Checklist (delete lines that don't apply)

- [ ] `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace` and `python3 .dev/check-deps.py` pass (CLAUDE.md §12 rule 12)
- [ ] Small enough to review: about 800 lines of non-test code or less; bigger work is split into stacked PRs (#154)
- [ ] Base branch is `master`, an `integration/*` umbrella, or the PR this one is stacked on (CONTRIBUTING, Branch workflow)
- [ ] New or changed op or event: `docs/protocol.md` **and** `crates/mockd/fixtures/core.json` updated, Dev C told (§4, §9)
- [ ] A design choice CLAUDE.md doesn't answer: ADR in `docs/decisions/` (§12 rule 14)
- [ ] Touches `crates/core` or `crates/proto`: ADR and sign-off from Dev A and Dev B (§4)
- [ ] New module, `Collection`, `Source`, `Sink` or template: tested against the in-memory `Ctx` (§12 rule 13)
