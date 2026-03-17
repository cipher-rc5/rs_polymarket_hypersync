# Release Runbook

Use this checklist for every release.

## 1) Pre-release checks

1. Ensure branch is up to date with main.
2. Run strict quality gates:

```bash
just check-strict
```

3. Run a bounded smoke test:

```bash
FROM_BLOCK=84023890 TO_BLOCK_EXCL=84023910 FOLLOW_TAIL=false cargo run --quiet
```

Expected smoke outcome:

- Process starts and prints runtime settings.
- Stream completes without panic.
- Summary prints CTF/NRA/EXCH totals.

## 2) Version + changelog

1. Bump `version` in `Cargo.toml`.
2. Update changelog entry (or release notes draft) with:
   - behavior changes
   - env var additions/removals
   - migration notes

## 3) CI gate

1. Open PR.
2. Require green CI (`fmt`, `check`, `clippy`, `test`).
3. Merge only after required checks pass.

## 4) Tag and publish

1. Create annotated git tag:

```bash
git tag -a vX.Y.Z -m "Release vX.Y.Z"
```

2. Push tag:

```bash
git push origin vX.Y.Z
```

3. Publish release notes in GitHub Releases.

## 5) Rollback plan

If regressions appear:

1. Revert to previous known-good tag.
2. Redeploy previous binary artifact.
3. Keep generated storage artifacts (`DATA_DIR`) for postmortem.
4. Record incident details and add a regression test before re-release.

## Support matrix

- Rust toolchain: `1.93`
- Rust edition: `2024`
