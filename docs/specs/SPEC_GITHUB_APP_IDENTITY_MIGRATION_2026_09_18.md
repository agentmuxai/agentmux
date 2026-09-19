# SPEC: Migrate agent GitHub authentication from long-lived PATs to GitHub App installation tokens

**Date:** 2026-09-18
**Author:** Agenty (agent, `~/.agentmux/agents/agenty-0629j`), per direct request from the human operator
**Status:** draft — proof-of-concept built and tested against live data; nothing wired into
the actual auth path yet. One human action item (§3) is a hard prerequisite before
this can go further for the `agenty` identity specifically.
**Related:** the private `shared-infrastructure` repo's credential inventory
standard (§2 — the per-agent PAT catalog this replaces) and its key-rotation
strategy report (the original
"move automation off PATs" recommendation), `docs/specs/archive/SPEC_TRUST_CENTER_MODALS_IDENTITY_MEMORY_SEED_2026_06_19.md`
(the original design that provisioned the App credentials this spec finally wires up),
`docs/specs/SPEC_MUXBUS_GITHUB_REVIEW_NOTIFICATIONS_2026_06_20.md` (already anticipates
`agent{x|y|a-g|1-5}-workflow[bot]` as a recognized identity pattern — review routing
was designed with this migration in mind, it just never got built).

---

## 1. What already exists (confirmed by direct testing, not assumed)

`services/infra`'s `agent-configs.<agent>` already holds `github_app_id` +
`github_app_installation_id` for **`agent1`–`agent5`, `agentx`, `agenty`** — six
of the eight agent identities in that map. A matching `<agent>-workflow-key`
(RSA private key, PEM) exists for each of those six. **`agento`** has a
`github_login` field instead (PAT-based, no App). **`agenta`** has no entry in
`agent-configs` at all.

**Confirmed: zero code anywhere in `agentmux` (Rust backend, shell scripts,
frontend) reads `github_app_id`/`github_app_installation_id` or mints an
installation token.** `scripts/gh-agent.sh` — the actual auth path every agent
uses today, including this one, all session — goes straight to the long-lived
`gh-token-<agent>` PAT and never touches the App credentials at all. The June
2026 design (`SPEC_TRUST_CENTER_MODALS_IDENTITY_MEMORY_SEED_2026_06_19.md`)
provisioned the credentials; the wiring to actually use them was never built.
This spec is that wiring.

## 2. Proof of concept — built and tested against live `agenty` credentials

`scripts/github-app-token.py` (new, on this branch): given an agent name,
reads its `github_app_id`/`github_app_installation_id`/`<agent>-workflow-key`
from `services/infra`, mints a GitHub App JWT, exchanges it for a real
installation access token, and prints **only the token** to stdout (all
diagnostics to stderr, no secret ever logged).

**Confirmed working end to end for `agenty`:**
- `agenty-workflow` is a real, active GitHub App owned by `a5af`, installed
  with `repository_selection: all`.
- Minted a genuine installation token, 1-hour expiry, auto-generated — nothing
  to rotate, nothing that can be embedded in a stale git remote URL the way
  `gh-token-agenty` was.
- Verified the token can read repo contents (`contents: write` is granted) —
  the core capability needed for git push.

**A JWT-construction gotcha worth documenting so nobody re-discovers it the
hard way:** GitHub enforces `exp - iat <= 600` seconds **exactly**, not
`exp - actual_current_time <= 600`. A common pattern elsewhere is to backdate
`iat` by ~60s as a clock-drift buffer; combined with a full 600s expiry, that
overshoots the limit and GitHub rejects the JWT with `"Expiration time claim
('exp') is too far in the future"` — a confusing error for what's actually an
off-by-60-seconds problem, not a real clock or "too far in the future" issue.
Confirmed by testing: `iat = now, exp = now + 540` works reliably; `iat = now
- 60, exp = now + 600` does not.

**Confirmed gap for `agenty` specifically:** the App's permission grant is
missing `pull_requests`. A live probe (`POST /repos/.../pulls` with an
intentionally-invalid branch, to distinguish a permission failure from a data
validation failure) returned `403 Resource not accessible by integration` —
a real, confirmed permission gap, not a bug in the request. **This is isolated
to `agenty`** — `agent1` and `agentx`'s Apps were checked the same way and
both already have `pull_requests` in their granted permission set. Whatever
process created `agenty-workflow` produced a narrower grant than the others;
this needs the one-time fix in §3 before `agenty` can fully move off its PAT.

Granted permissions found across the checked Apps, for reference:

| Agent | Permissions granted |
|---|---|
| `agent1` | `actions, contents, issues, members, metadata, packages, pull_requests, statuses, workflows` |
| `agent2` | `actions, contents, issues, members, metadata, packages, pull_requests, statuses, workflows` |
| `agent3` | `actions, contents, issues, members, metadata, packages, pull_requests, statuses, workflows` |
| `agent4` | `actions, contents, issues, members, metadata, packages, pull_requests, statuses, workflows` |
| `agent5` | `actions, contents, issues, members, metadata, packages, pull_requests, statuses, workflows` |
| `agentx` | `actions, contents, issues, members, metadata, organization_administration, packages, pull_requests, statuses, workflows` |
| `agenty` | `actions, contents, issues, members, metadata, organization_administration, packages, statuses, workflows` — **missing `pull_requests`** |

**Phase 0's verification is now complete for all seven App identities.**
`agenty` is confirmed the sole outlier — every other agent (`agent1`–`agent5`,
`agentx`) already has the full grant needed. §5's "verify agent2-5" step is
done; no further permission-audit work is needed before Phase 1 for these
seven.

## 3. Human action item — fix `agenty-workflow`'s permissions

This cannot be completed via API — GitHub requires the installation owner to
approve a permission increase on an existing App through the web UI:

1. Go to https://github.com/settings/apps/agenty-workflow/permissions (or:
   GitHub → Settings → Developer settings → GitHub Apps → `agenty-workflow` →
   Permissions & events).
2. Under **Repository permissions**, find **Pull requests** and set it to
   **Read and write** (matching what `agent1`/`agentx` already have).
3. Save. GitHub will prompt to review/accept the updated permissions for the
   existing installation — accept it (this is the "installation owner
   approves a permission increase" step; since `a5af` owns both the App and
   the installation, this should be a single confirmation, not a multi-party
   approval flow).
4. Re-run the probe from §2 (mint a token via `scripts/github-app-token.py
   agenty`, retry the intentionally-invalid-branch `POST /pulls` call) and
   confirm it now returns `422` (bad branch — meaning permission passed and
   it got to actual validation) instead of `403`.

## 4. Migration plan

### Phase 0 — Fix known gaps (blocking)
- §3's permission fix for `agenty`.
- Verify `agent2`–`agent5` the same way §2 verified `agent1`/`agentx` — don't
  assume they match; check each.

### Phase 1 — Build the wiring — DONE, tested 2026-09-18
- `scripts/github-app-token.py` hardened: goes through the `secrets` CLI
  (not raw `aws` calls, so it works under any agent's own scoped IAM
  profile, not just a session with broad root credentials), resolves the
  Windows `secrets.cmd` shim explicitly (Python's `subprocess` without a
  shell can't execute the extensionless `secrets` file the way bash does —
  confirmed by hitting this directly), and exits with a distinct code (2)
  for "no App identity provisioned" vs. any other real failure, so callers
  can tell "expected fallback" apart from "something broke."
- `scripts/gh-agent.sh` now tries the App-token path first, falling back to
  the existing `gh-token-<agent>` PAT (or the shared `genericagentx`
  account) on ANY failure — missing identity, expired/revoked installation,
  `python3` not on PATH, anything. A non-"no App identity" failure is
  surfaced on stderr rather than silently swallowed, so a real break in the
  App path doesn't quietly hide behind an always-available fallback.
- **Verified end to end, live, through the actual wired script** (not just
  the standalone PoC): `AGENTMUX_AGENT_ID=Agent1 scripts/gh-agent.sh api
  repos/agentmuxai/agentmux/pulls` correctly authenticated via
  `app:agent1-workflow-key` and returned real data. Same for `AgentY`
  (this identity) via `app:agenty-workflow-key`. `AGENTMUX_AGENT_ID=Agento`
  (no App identity) correctly fell through to `pat:gh-token-agento` and
  worked exactly as the script did before this change — the fallback chain
  is unbroken for agents not yet migrated.
- No change needed to the muxbus review-routing side —
  `SPEC_MUXBUS_GITHUB_REVIEW_NOTIFICATIONS_2026_06_20.md` already recognizes
  `agent{x|y|a-g|1-5}-workflow[bot]` as a valid bot identity pattern, so
  commits/PRs authored by the App show up correctly attributed without
  further routing work.
- **Known fragility, not yet hit in practice:** `gh-agent.sh` computes
  `SCRIPT_DIR` via bash's `pwd`, which is an MSYS-style path; passing that to
  `python3.exe` (a native Windows binary) relies on git-bash's automatic
  argv path conversion. Setting `MSYS_NO_PATHCONV=1` (done once, by hand,
  while debugging an unrelated `gh api` argument-mangling issue during this
  same testing pass) breaks that conversion and makes `python3` unable to
  find the script. Not a problem under normal operation — nothing in this
  repo's own tooling sets that variable — but worth a `pwd -W` fix (git-
  bash's built-in flag for a native Windows path) if it's ever seen to
  matter, rather than relying on argv auto-conversion always being active.

### Phase 2 — Validate per agent before relying on it
- For each agent with an App identity, once Phase 0/1 land: do a real but
  low-stakes test (open a throwaway PR against a scratch branch, or repeat
  this spec's own §2 probes) confirming both push and PR-create work through
  the new path before treating it as the primary mechanism for that agent's
  real work.
- Do this staged, one agent at a time — starting with `agenty` (this
  identity, already closest to validated) makes sense as the first real
  cutover, specifically *because* it's the one already caught leaking (per
  `CREDENTIAL_INVENTORY.md` §2.3) — closing that gap for real, not just
  rotating the same kind of credential again, is the actual point.

### Phase 3 — Retire the per-agent PATs
- Once an agent's App-token path is validated (Phase 2) and has been the
  active path for a reasonable burn-in period, revoke that agent's
  `gh-token-<agent>` PAT entirely — there's nothing left depending on it.
  This closes `gh-token-agenty`'s exposure by making the credential
  irrelevant, not just rotated.
- `gh-token-genericagentx` stays until `agento` (and any other PAT-only
  agent) gets an App identity too, or is deliberately decided to stay PAT-based.

### Phase 4 — Separate track: the shared "root" credential
- `github.token`/`gh-admin-pat`/`gh-packages-readonly`/`agentmux-a5af-packages-token`
  (per `CREDENTIAL_INVENTORY.md` §2.1) is a **different kind of problem** —
  it's not per-agent identity/attribution, it's a shared service credential
  for a handful of specific automated actions (reagent's `@codex review`
  trigger, package registry access). This should fold into **reagent's
  existing `reagentx` GitHub App** (already proven, already has a working
  installation-token flow in `lambdas/utils.py`) rather than needing a new
  App — add whatever permission reagent's App is missing for the `@codex
  review` trigger specifically, retire `gh-admin-pat` once that's confirmed,
  and separately decide the actual right scope for the two "packages" names
  (per the inventory's finding that they're misleadingly named — this is the
  moment to give them their own narrowly-scoped credential instead of
  inheriting the root PAT's full scope).
- Independent of Phase 1-3's per-agent work — can proceed in parallel.

### Phase 5 — Optional follow-up
- Decide whether `agento` and the `genericagentx` fallback identity should
  get App identities too, closing out PATs from this system entirely. Not
  blocking — those two are lower-frequency/fallback paths, not the primary
  exposure surface.

## 5. Suggested execution order

1. ~~Fix `agenty`'s permission gap (§3)~~ — **still open, blocking `agenty`
   specifically.** Confirmed (2026-09-18) to be the *only* gap —
   `agent1`–`agent5` and `agentx` all already have the full grant, so this
   is a one-App fix, not a fleet-wide one.
2. ~~Build Phase 1's `gh-agent.sh` wiring~~ — **done and tested 2026-09-18**
   (§4 Phase 1). Every agent with an App identity now uses it by default for
   anything routed through `gh-agent.sh`; agents without one are unaffected.
3. **Next:** validate `agenty` end-to-end (Phase 2) once §3's permission fix
   lands — the wiring already works for it (`app:agenty-workflow-key`
   resolves and authenticates correctly), the only remaining unknown is
   `pull_requests` specifically.
4. Roll Phase 2/3 out to the other App-identity agents one at a time.
5. Run Phase 4 (the shared root credential → reagent's App) in parallel,
   independently of the per-agent timeline.

## 6. What this spec does and does not do

**Done, on this branch, tested against live GitHub state:** the App-token
minting helper and the `gh-agent.sh` wiring (§4 Phase 1) — every agent with
a provisioned App identity now authenticates through it by default, with a
verified, unbroken fallback for agents that don't have one yet.

**Not done:** no PAT has been revoked (Phase 3 is explicitly gated on a
burn-in period after validation, not immediate). `agenty`'s `pull_requests`
permission gap (§3) is still open — a human action item, not something this
branch can complete. Phase 4 (the shared root credential →
reagent's App) hasn't been started. This branch itself hasn't been merged.

Every write-shaped API call made during this work was either read-only
(repo/PR reads used to validate the wiring) or deliberately constructed to
fail before touching real state (the PR-creation permission probe in §2 used
an intentionally-nonexistent branch name specifically so it could confirm a
403 without ever being able to succeed and create a real PR). No real
repository state was created or modified by any of this investigation or
testing.
