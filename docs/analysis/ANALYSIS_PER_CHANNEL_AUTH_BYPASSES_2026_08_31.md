# Analysis — five paths let an agent reach Claude without a per-channel login

**Date:** 2026-08-31
**Author:** AgentX
**Status:** Investigation complete; fixed on this branch (PR #2878).
**Revision:** #5 added 2026-08-31 after Codex's P1 review caught it — the
original pass found only four.
**Prompted by:** operator question — *"how are all my agents able to operate
without any auth inside the armory?"*, then *"we want per-channel isolation. a
user needs a login anytime in the channel. the problem is agents are able to
login in a channel without a claude auth."*

---

## 0. The ask, stated as an invariant

**INV-PC (per-channel auth):** an agent running in channel *C* must launch only
against a credential that was authenticated **in channel *C***. Neither the
user's personal `~/.claude`, nor another channel's credential directory, may
satisfy an agent's auth requirement in *C*.

Per-channel isolation is a **wanted feature**, not an accident to be designed
away — a fresh build is supposed to require its own login. The bug is that five
separate mechanisms currently defeat it.

## 1. Why the Armory looks empty while agents still work

Two stores, split by `SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md`:

| Store | Path | Scope | Holds |
|---|---|---|---|
| `id_store` | `resolve_shared_store_path()` (`registry/paths.rs:63`) | **per-channel** when `isolated_auth_enabled()` | `db_accounts` |
| `identity_store` | `resolve_identity_store_path()` | **always global** | `db_agent_identity_links` |

Step 1 of that spec (links → global) shipped. **Steps 1b/2/3/4/5 did not** —
`disposable_test` appears nowhere in the codebase except one comment in
`m0022_identity_store_links_backfill.rs` calling it "the deferred per-account
scope split." Tracking issue #2627 was closed 2026-08-17, the same day step 1
landed, with five of its six steps unbuilt.

Net: **links are global, accounts are per-channel.** A fresh channel therefore
has a valid link pointing at an account row it cannot see — which is precisely
the state bypass #1 was added to paper over.

## 2. The five bypasses

### #1 — `resolve_account()` falls back to the global store (silent, spawn-time)

`identity/resolver/inject.rs:241-253` tries `id_store`, then falls back to the
always-global `identity_store`. Added by reagentx P0 on PR #2632 to fix a
continuity bug, with the explicit rationale that the account "could still only
exist in `id_store`, which is empty on a fresh, isolated channel."

It works as designed — and as a side effect, an agent in a fresh channel
resolves an account belonging to a *different* channel. Live evidence from this
machine's global store: `claude-oauth` rows whose `secret_ref.dir` points at
`channels/local-main-b28b7a-01f827a1/identities/...`, `...-2c88d0ba`,
`...-3997f7b9`, and several `dev/<branch>` dirs — all `status: valid`, all
reachable from any channel.

Violates INV-PC directly.

### #2 — accounts per-channel vs. links global (structural)

Not itself a credential leak; it is the *mismatch* that motivated #1. Closing #1
without addressing this leaves the gate correct but the UX confusing (a link
that can never resolve). Safe to close because `db_agent_identity_links` is
`PRIMARY KEY (agent_id, provider)` — a re-login **replaces** the link rather
than accumulating a stale one, so no dangling-link deadlock results.

### #3 — "Use existing login" (explicit, one click)

`frontend/app/view/agent/flows/seed-global-login.ts` →
`agentmux-cef/src/commands/providers.rs:402` `seed_provider_auth_from_global`.

Reads **`~/.claude/.credentials.json` — the user's own personal login**
(`providers.rs:421-429`, comment: *"GLOBAL source — the user's own login"*) and
copies it verbatim, including `refreshToken`, into the agent's isolated dir.

The `INV-R` containment guard is real but guards the **destination** only: a
`config_dir` pointing at `~/.claude` is rejected so the seed can never *write*
into the user's personal env. **Nothing constrains the source.**

This is in direct tension with
`SPEC_BLOCK_AMBIENT_HOME_DIR_IDENTITY_BINDING_2026_08_25.md`, where the operator
asked *"can we enforce that agents never use the ~/.claude?"* and chose *"Block
it too, fail loudly."* That spec blocks **binding** an account whose dir *is*
`~/.claude`; this path **copies the credential out of** `~/.claude` into a
compliant dir — same end state, but it passes the check, because the check tests
a path rather than the credential's provenance.

Built for a real problem: Claude Code v2.1.x's self-driving login TUI, whose URL
the host cannot scrape (`SPEC_HOST_CLI_LOGIN_CAPTURE §5.5`).

### #4 — `checkcliauth` first-run bootstrap (fully silent, no user action)

`agentmux-srv/src/server/cli_handlers.rs` (~lines 300-345). During a routine
**auth check** — not a login — if the isolated `CLAUDE_CONFIG_DIR` has no
`.credentials.json` and no `.agentmux-cred-seeded` sentinel, it copies
`~/.claude/.credentials.json` into the isolated dir and writes the sentinel.

No user action, no login, no Armory account. This is the most likely direct
answer to "agents operate with an empty Armory," and explains why the symptom
varies by machine (whether `~/.claude` holds a valid credential) — including the
Windows 10 difference the operator reported.

Justified by `retro-provider-auth-isolation-regression-2026-06-05.md`, which
predates every piece of the ambient-blocking work. It was never revisited when
the "agents never use `~/.claude`" requirement landed.

### #5 — `checkcliauth`'s "self-heal from global" (silent, on any failed validation)

**Found by Codex's P1 review on PR #2878 — missed in the original pass of this
analysis.** Same file and same source as #4, different trigger, so removing #4
alone would have left the bypass fully open.

`agentmux-srv/src/server/cli_handlers.rs`, in the `!authenticated` branch of the
Claude auth check: when validation FAILED and the user's global
`~/.claude/.credentials.json` held an access token differing from the isolated
dir's, `refresh_claude_dir_from_global_if_stale` copied global → isolated and
re-ran the check ONCE. A channel could therefore acquire a credential
authenticated outside it and then report `authenticated: true`, with no
channel-local login at any point.

Unlike #4 it was not gated by a first-run sentinel — it could fire on *any*
failed validation, repeatedly.

The condition it existed to recover from is real: the "Pozl 401", where a global
re-login rotates the access token and invalidates the isolated copy's refresh
token while the `.agentmux-cred-seeded` sentinel blocks re-import. That is now
surfaced honestly (failed validation → auth card → log in for this channel)
rather than silently patched from the operator's personal credential.

`claude_access_token`, `refresh_claude_dir_from_global_if_stale`,
`refresh_dir_from_global` and the `selfheal_tests` module were deleted outright
rather than left unused — a helper whose whole job is copying the operator's
personal credential into an isolated dir is precisely the thing that gets
quietly rewired back in later.

## 3. Why Claude specifically

Tier 3 of `run-provider-login.ts` (terminal login) already does the right thing
for **every non-Claude oauth provider**: it leaves the provider's config-dir env
var pointed **at the isolated dir**, so the login writes there directly, then
polls `pollForCliAuthReady`.

**Claude is the sole exception** — its env var is deliberately *stripped* so the
login lands in the user's global `~/.claude`, then `pollForGlobalLoginSeed`
copies it inward. The stated reason is only that "only Claude has a
seed-from-global capability to fall back on" — a convenience, not a constraint.
The `claude` CLI honours `CLAUDE_CONFIG_DIR` like the others do.

So removing #3 and #4 requires **unifying Claude's tier 3 with the path every
other provider already uses**, not inventing a new mechanism.

## 4. Fix plan

1. Remove `resolve_account`'s global fallback (#1) — account must resolve in the
   current channel's store.
2. Remove BOTH of `checkcliauth`'s global-import paths — the first-run bootstrap
   (#4) and the self-heal-on-failed-validation (#5), plus the helper machinery
   behind the latter.
3. Remove `seed_provider_auth_from_global` (host), `seedGlobalLogin` /
   `pollForGlobalLoginSeed` (frontend), tier 2 of `run-provider-login.ts`, and
   the "Use existing login" UI action (#3).
4. Unify Claude tier-3 terminal login with the non-Claude strategy: keep
   `CLAUDE_CONFIG_DIR` pointed at the isolated dir, poll `pollForCliAuthReady`.
5. Leave a clear, actionable failure when no per-channel credential exists — the
   existing `SpawnGateError::MissingCredentials` card, which already classifies
   as non-retryable `FailureClass::Auth`.

**Not in scope here:** finishing the `disposable_test` / Store B design (#2's
structural half). Closing #1 makes the gate correct today; the store split
remains tracked separately and should reopen #2627 rather than ride along.

## 5. Expected behavior change (deliberate)

After this, opening an agent in a **fresh channel** with no login in that
channel **fails to spawn with an actionable auth card**, instead of silently
inheriting the user's personal or another channel's credential. That is the
requested behavior, and it will be visible on every new local/dev/portable
build.

## 6. Known residual gap — display paths still describe cross-channel accounts

Raised by reagent P2 on PR #2878, and worth recording rather than leaving
implicit, because it is a divergence this work *created*.

The spawn path now uses `resolve_account_for_spawn` (per-channel only). Two
read-only paths deliberately still use `resolve_account`, which keeps the
global-mirror fallback:

- `identity.self.accounts` (`app_api/mod.rs`) — the per-agent account listing.
- `identity.account.validate` — validates a credential against the provider.

So an oauth-class account that exists only in the global mirror is **listed**,
and can **validate as `valid: true`**, while a real spawn in this channel
refuses it with `MissingCredentials`. (An api-key-class account is silently
skipped at spawn instead of blocking it.)

**Why the fallback is kept anyway:** a listing should describe what exists.
Dropping it would make the account disappear from Armory entirely, reproducing
the "my account vanished after a version bump" confusion that reagentx P0 on
PR #2632 was written to fix — replacing a misleading-but-informative row with
no row at all is not obviously better. `validate`'s answer is also genuinely
correct on its own terms: it reports whether the *credential* is live, not
whether a spawn would accept it.

**What is actually missing:** nothing in either payload tells the caller the
account cannot satisfy a spawn in *this* channel. The honest fix is to surface
that — e.g. a `usable_in_channel: bool` on the listing, rendered in Armory as a
"needs login in this channel" state — rather than to hide the row or to widen
the spawn gate back out.

Not done here: it needs a payload field, an Armory affordance, and a decision
about how it interacts with the unbuilt `disposable_test`/Store B split (§1),
which is the same area. Interim mitigation: `identity.self.accounts` now emits
a `tracing::warn!` whenever it lists an account resolved only via the mirror,
so the divergence is at least observable in `muxlog auth` instead of invisible.
