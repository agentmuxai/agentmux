# SPEC: Split local-machine credentials out of `services/infra` into `services/local`

**Status:** active — Phase 2 executed. `services/local` created (via `aws

> **No implementing PR in this repo (checked 2026-09-17).** The work this spec
> describes (`services/local` bucket creation) happened in the secrets infra, not
> in `agentmux` — the only commit touching this file is #3290, which added the
> spec itself and changed no source here. Cited as a gap rather than mis-attributed
> to #3290, which implemented nothing.

secretsmanager create-secret`, matching how `services/infra`/`dev`/`qa`/`prod`
were all originally provisioned out-of-band — none are CDK-managed) and
populated with the 7 migrated keys plus full
`$schema_version`/`$changelog`/`$environment`/`$last_updated`/`$updated_by`
metadata. Originals remain untouched in `services/infra` pending Phase 3/4.

`a5af/dev-tools#370` (adds `local` to the CLI's `$environment` enum) has
merged and published as `@a5af/secrets@1.1.8` — `secrets verify services/local`
now passes cleanly. Companion doc PR: `a5af/shared-infrastructure#475`
(documents `services/infra` and `services/local` as cross-cutting buckets
outside the three-environment deployment model).

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

**As planned, then revised once executed:** the original plan below was to
create the bucket via `secrets update services/local` (the CLI's
changelog-protocol-enforced path) end to end. In practice `secrets update`
refused to operate on a not-yet-existing secret ("can't find the specified
secret") — the `@a5af/secrets` CLI has no `create`/`init` subcommand at all;
every `services/*` bucket, including `infra`/`dev`/`qa`/`prod`, was
originally provisioned out-of-band the same way (confirmed via
`shared-infrastructure`: `services/{stage}` secrets are only ever
`Secret.fromSecretNameV2`-imported, never `new secretsmanager.Secret(...)`-
created, in that repo's CDK).

What was actually executed: `aws secretsmanager create-secret` (empty
placeholder), then — since `secrets update` still errored on the freshly
created secret pending `a5af/dev-tools#370`'s `$environment` fix — a
hand-built JSON payload replicating the exact schema shape read back from
an existing bucket (`services/qa`), written via
`aws secretsmanager put-secret-value`:

- `$schema_version`
- `$changelog`
- `$environment`
- `$last_updated`
- `$updated_by`

The first (and so far only) `$changelog` entry documents this migration
itself (source bucket, keys moved, this spec's filename). This is a
manual replication of the CLI's schema, not CLI-enforced — `secrets
verify`/`update` can't operate on this bucket until `a5af/dev-tools#370`
merges and publishes; once it does, treat all *future* changes to
`services/local` as going through `secrets update` normally, the same as
any other bucket.

### 2. Migrate the seven local-machine keys listed above

Copied into `services/local` under the same key names (no renaming, so
existing naming convention/searchability is preserved).

### 3. Do NOT delete from `services/infra` in the same step

Originals stay in `services/infra` as a read-only fallback until:

- every consumer of each key has been located and confirmed to either not
  exist, or to have been repointed at `services/local`, and
- there's explicit, separate sign-off for the deletion step specifically.

This spec covers steps 1–2 only. Deletion is a distinct, later action —
**at present, nothing in the codebase was found referencing these seven
keys directly** (no hits for `shareuser-password`, `area54`, `narko-`,
`gamerlove-`, `starpower-`, `charlie-yas`, or `cornelia-asafebgi` outside
this new spec and `services/infra` itself), which is
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
2. **Done.** Create `services/local` + migrate the 7 keys — executed via
   the raw-AWS-CLI fallback described in §1, not `secrets update` as
   originally planned (that path wasn't available until
   `a5af/dev-tools#370` shipped). Originals untouched in `services/infra`.
3. Grep the full repo and any operator-side scripts/tooling outside this
   repo for direct references to the migrated key names, to confirm no
   consumer breaks.
4. Remove originals from `services/infra` — **only after explicit
   go-ahead**, separate from the go-ahead for phase 2.
5. **Done.** Docs update per §4 above — `a5af/shared-infrastructure#475`.

Phases 2 onward require explicit confirmation before execution, per phase.
