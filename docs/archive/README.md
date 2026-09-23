# `docs/archive/`

**Status:** living
**Date:** 2026-09-18
**Related:** [`../specs/SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md`](../specs/SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md) (Phase 2), [`../specs/README.md`](../specs/README.md) (the `Status:` vocabulary)

Retired documents. A doc lands here when it is **finished and no longer describes
the system** — superseded, resolved, or about a thing that no longer exists — but
is still worth keeping for the reasoning it records.

## What goes here

- Dated one-off analyses and reports whose conclusions have landed.
- Handoffs and status snapshots from a completed effort.
- Specs for subsystems that were removed.

Archiving is not deletion. If the reasoning is worth nothing, delete the file;
if it is worth something, archive it. Do not archive a doc that still describes
current behaviour just because it is old — that is what `Status: living` is for.

## Conventions

- **Keep the filename.** Renaming on archive breaks every inbound citation, and
  `scripts/check-doc-links.mjs` will fail the PR that does it.
- **Leave a banner** at the top saying what superseded it and when, e.g.
  `> Archived 2026-06-17 — superseded by X. Kept for historical reference only.`
  A reader who lands here from a search needs to know that in the first line,
  not the fifth paragraph.
- **Set `**Status:**`** to `historical` or `superseded` (the closed vocabulary in
  `../specs/README.md`). `superseded` additionally requires a `**Superseded-by:**`
  line that resolves — `scripts/check-doc-status.sh` enforces this.
- **Repoint inbound links** in the same commit as the move. Files here sit one
  level below `docs/`, so a link that read `../../retro/x.md` from
  `docs/analysis/archive/` becomes `../retro/x.md` from here. Run
  `node scripts/check-doc-links.mjs --all` after moving anything.

## Why `docs/specs/archive/` still exists separately

The lifecycle audit asked for **one** archive, not four. Two of the four are gone
(`specs/archive/` went with the top-level `specs/` tree; `docs/analysis/archive/`
was folded in here on 2026-09-18). `docs/specs/archive/` remains, deliberately,
because it is load-bearing tooling rather than merely a folder:

- `scripts/gen-docs-index.sh` scans `docs/specs/` and excludes exactly that
  subtree — it is the mechanism by which a spec leaves the generated index while
  staying next to its siblings. Its generated preamble explains `archive/` to the
  reader in those terms.
- `scripts/check-docs-lifecycle.mjs` hardcodes the path in its `EXCLUDED` regex.
- `scripts/linux-apprun.sh` and `scripts/package-portable.sh` cite files inside
  it by path.

Moving it means changing the generated preamble, which means regenerating
`INDEX.md`. When this was written, `gen-docs-index.sh` was not reproducible
across platforms (a Windows run emitted ~21 extra status buckets and a different
row order than CI's Linux run), so that regeneration had to happen somewhere
matching CI. Since 2026-09-23 the generator is `scripts/gen-docs-index.mjs`,
checked byte-identical on Linux, macOS and Windows in CI
(`SPEC_DOCS_INDEX_GENERATOR_NODE_PORT_2026_09_23.md`), so it can be regenerated
anywhere; the reasons above for keeping `docs/specs/archive/` still hold.

This is the hardening spec's own escape hatch — *"or explicitly document why more
than one is needed if there's a real reason this audit didn't surface"* — being
taken deliberately, not the consolidation being forgotten. **Archived specs go in
`docs/specs/archive/`; everything else archived goes here.**
