# Changesets

This directory holds **pending changeset files** — one per PR that wants to
contribute to the next release. RFC #857 Phase 2 / spec
`docs/specs/SPEC_MULTI_AGENT_VERSION_COORDINATION_2026_05_15.md`.

## Why

Multiple agents commit to this repo concurrently. When every PR ran
`bump patch`, version files (`package.json`, `Cargo.toml`, `Cargo.lock`,
`package-lock.json`) became near-100% conflict surface. The
"version downgrade P1" finding from reagent fired on every forward-merge.

Changesets fix this: a feature PR adds **one new file** with a unique name
(`<timestamp>-<slug>.md`). That file never conflicts with another PR's
changeset. The version bump happens once, in a dedicated release commit.

## Author flow (in a feature PR)

```bash
task changeset -- patch "fix(auth): cancel in-flight session on selection swap"
# or:
scripts/changeset.sh patch "fix(auth): cancel in-flight session on selection swap"
```

This creates `.changesets/1747298400-fix-auth-cancel-in-flight.md`:

```markdown
---
type: patch
---

fix(auth): cancel in-flight session on selection swap
```

Then commit the changeset file alongside your code changes. **Do NOT run
`bump patch`** — the release step owns version bumps now.

Allowed types: `patch`, `minor`, `major`. If the changesets in a release
include any `major`, the release is major. Otherwise minor wins over patch.

## Release flow (separate PR, periodic or on-demand)

```bash
task release
```

This:

1. Scans `.changesets/*.md`, aggregates descriptions
2. Picks the highest bump type (major > minor > patch)
3. Runs `scripts/bump-wrapper.sh <type>` to bump version
4. Appends entries to `VERSION_HISTORY.md`
5. Deletes consumed `.changesets/*.md` files
6. Stages all changes (the bump commit + the changeset deletes)

The releaser then commits with a message like
`chore: release v0.33.897` and opens a PR. That PR is the **only** PR that
touches `package.json` / `Cargo.toml` / lockfiles.

## What if I'm fixing a small thing and don't need a release?

Then don't add a changeset. The PR ships without forcing a version bump.

## Conflict surface comparison

| Today (post-Phase 1) | With changesets |
|---|---|
| 4 version files modified per PR | 0 version files; 1 unique `.changesets/<id>.md` |
| `bump patch` collisions between agents | Filenames are unique by timestamp+slug |
| Lockfile rebuild per PR (~130 line diff) | Lockfile rebuild only in release PR |
| Forward-merge causes version-downgrade P1 | Forward-merge is conflict-free |
