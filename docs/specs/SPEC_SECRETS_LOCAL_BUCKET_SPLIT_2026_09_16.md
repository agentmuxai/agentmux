# SPEC: Split local-machine credentials out of `services/infra` into `services/local`

Status: **Phase 2 executed.** `services/local` created (via `aws secretsmanager
create-secret`, matching how `services/infra`/`dev`/`qa`/`prod` were all
originally provisioned out-of-band — none are CDK-managed) and populated with
the 7 migrated keys plus full `$schema_version`/`$changelog`/`$environment`/
`$last_updated`/`$updated_by` metadata. Originals remain untouched in
`services/infra` pending Phase 3/4.

`secrets verify services/local` will fail until `a5af/dev-tools#370` merges
(the CLI's `$environment` validator only recognized `dev`/`qa`/`prod`/`infra`;
that PR adds `local`) — not a data problem, a tooling-version gap. Companion
doc PR: `a5af/shared-infrastructure#475` (documents `services/infra` and
`services/local` as cross-cutting buckets outside the three-environment
deployment model).

## Problem

`services/infra` (AWS Secrets Manager, via the `secrets` CLI /
`@a5af/secrets`) mixes two different kinds of credential:

1. **Shared service/org infra** — GH admin PATs, per-agent workflow keys,
   OAuth client secrets for prod/dev/qa, Apple signing certs, ReAgent's
   jekt signing key, `muxbus-api-key`, `org-admin-key`,
   `package-publisher-key`, Discord webhooks, etc. Consumed by CI, release
   tooling, and multiple agents across machines.
2. **Local-machine credentials** — per-host or per-person account
   passwords for machines on the operator's own network, identified by a
   `<hostname>-...-password` or `<person>-...-password` naming pattern:
   - `area54-shareuser-password`
   - `narko-shareuser-password`
   - `gamerlove-shareuser-password`
   - `gamerlove-windows-vm-vmx-password`
   - `starpower-asafebgi-password`
   - `charlie-yas-password`
   - `cornelia-asafebgi-password`

Category 2 has nothing to do with shared service infra and doesn't belong
in the same bucket an agent reaches for when it needs (say) a GH token.
Mixing them risks a future agent misreading a personal machine credential
as production infra, or vice versa. Splitting them into a dedicated
`services/local` bucket removes that ambiguity.

### Not included: `amramebgi`

Initially flagged as a candidate (bare key, no obvious infra role), but
inspection showed it's a nested object `{ "github-pages-pat": "<40-char
token>" }` — a GitHub Pages deployment PAT for a project/site named
"amramebgi", not a local-machine or personal-account credential. It stays
in `services/infra` for now. Separate follow-up, out of scope for this
migration: rename the key to something self-describing (e.g.
`amramebgi-github-pages-pat`, flattened to match the convention used by
`gh-token-agent1`, `discord-webhook-asafebgi`, etc.). No consumer of the
current name was found in this repo, but that doesn't rule out something
outside it depending on the current key — confirm before renaming.

## Proposed change

### 1. Create `services/local`

Created via `secrets update services/local`, **not** a raw write — this is
the CLI's changelog-protocol-enforced path, so the bucket's creation and
the initial migration both land in `$changelog` from the start, matching
`services/infra`/`services/dev`/`services/prod`/`services/qa`'s existing
schema:

- `$schema_version`
- `$changelog`
- `$environment`
- `$last_updated`
- `$updated_by`

The first changelog entry documents this migration itself (source bucket,
keys moved, this spec's filename) — not a bare "created bucket" line.

### 2. Migrate the eight local-machine keys listed above

Copied into `services/local` under the same key names (no renaming, so
existing naming convention/searchability is preserved).

### 3. Do NOT delete from `services/infra` in the same step

Originals stay in `services/infra` as a read-only fallback until:

- every consumer of each key has been located and confirmed to either not
  exist, or to have been repointed at `services/local`, and
- there's explicit, separate sign-off for the deletion step specifically.

This spec covers steps 1–2 only. Deletion is a distinct, later action —
**at present, nothing in the codebase was found referencing these eight
keys directly** (no hits for `shareuser-password`, `area54`, `narko-`,
`gamerlove-`, `starpower-`, `charlie-yas`, `cornelia-asafebgi`, or
`amramebgi` outside this new spec and `services/infra` itself), which is
encouraging but not a substitute for the sign-off gate above — an agent's
own local scripts or an operator's personal tooling outside this repo
could still reference the old path.

### 4. Docs update

- `docs/agent-identity-bootstrap.md` and `CLAUDE.md`'s `gh-agent.sh`
  section reference `services/infra`'s `gh-token-<agent>` — those are
  category-1 keys and are **unaffected** by this migration.
- Add a short note (this repo's `CLAUDE.md` or a new
  `docs/secrets-buckets.md`, TBD at execution time) documenting the
  `services/local` convention: local-machine/personal-account credentials
  go there, not `services/infra`, going forward.

## Execution phases

1. **This spec** (done).
2. Create `services/local` + migrate the 8 keys via `secrets update`
   (changelog-enforced). Originals untouched in `services/infra`.
3. Grep the full repo and any operator-side scripts/tooling outside this
   repo for direct references to the migrated key names, to confirm no
   consumer breaks.
4. Remove originals from `services/infra` — **only after explicit
   go-ahead**, separate from the go-ahead for phase 2.
5. Docs update per §4 above.

Phases 2 onward require explicit confirmation before execution, per phase.
