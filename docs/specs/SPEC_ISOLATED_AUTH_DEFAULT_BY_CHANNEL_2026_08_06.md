# Spec: Make isolated auth the default for every non-`stable` channel

**Date:** 2026-08-06
**Status:** Proposed (implemented — see note below)
**Amends:** `docs/specs/SPEC_ISOLATED_AUTH_DEV_TESTING_2026_07_27.md` (all plumbing/mechanism described there is unchanged and remains authoritative — this spec changes only the default-computation, from "opt-in via `AGENTMUX_ISOLATED_AUTH=1`" to "on by default for any channel other than `stable`, still overridable")
**Related:** `docs/specs/SPEC_GLOBAL_IDENTITY_MEMORY_DRONE_2026_06_24.md`, `docs/specs/SPEC_DATA_CHANNELS_2026_05_24.md`, issue #2429 (tier-1 Claude in-app login broken), PR #2425 (mid-session credential-loss relogin modal), retro family #2164/#2165/#2167/#2195 (isolation-vs-global reconciliation incidents)

> **2026-08-07 audit note:** Implemented same day (commit `9f6cc2824`, PR
> #2431) — `CLAUDE.md` itself now documents this as current default
> behavior. Status field was never updated. See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.

## 0. Motivation

Today (2026-08-05/06), live-verifying PR #2425 (the credential-loss relogin
modal) required a real credential-loss event. The only account inventory
available to trigger one was the tester's actual global identity store —
**12 real, currently-in-use Claude OAuth accounts**, shared, per
`SPEC_ISOLATED_AUTH_DEV_TESTING_2026_07_27.md`, by every `task dev` branch,
every `task package` build, and every installed instance on the machine.
Disconnecting them to force a relogin would have logged out real agent
sessions unrelated to this test — exactly the failure mode
`SPEC_ISOLATED_AUTH_DEV_TESTING_2026_07_27.md` was written to prevent, just
approached from account-deletion instead of account-*disconnection*, and
the existing `AGENTMUX_ISOLATED_AUTH=1` opt-in was not in effect for this
boot because nobody thought to set it before launching.

That's the actual problem: **the safety mechanism already exists, but its
opt-in default means the unsafe path (touch the real global store) is what
happens unless a human remembers a specific env var every single time.**
For a repo actively hardening its OAuth login/relogin surfaces (#2425,
#2422, #2423, #2429), that default works against the goal — every fresh
dev/test channel silently inherits a fully-authenticated global session,
so the login and relogin code paths this work is trying to harden mostly
never actually execute during routine `task dev` testing.

## 1. Problem, restated generally

`agentmux_common::isolated_auth_enabled()` (`agentmux-common/src/data_paths.rs:418-422`)
currently:

```rust
pub fn isolated_auth_enabled() -> bool {
    std::env::var("AGENTMUX_ISOLATED_AUTH")
        .map(|v| v == "1")
        .unwrap_or(false)
}
```

Unset → `false` → global, unconditionally, regardless of channel. This is
correct for the `stable` channel (a real installed/portable release — the
daily-driver case the July 27 spec explicitly protects: "re-authenticating
every provider on every branch switch is real daily friction"). It is the
wrong default for every other channel, where the friction is not just
tolerable but actively useful: it's test coverage.

## 2. Solution

### 2.1 Channel-aware default

Replace the flat `unwrap_or(false)` with a channel-conditioned default,
keeping the explicit-override behavior and the "one helper, read fresh at
every call site" discipline the existing doc comment already commits to
(no new call sites needed — every consumer already goes through
`isolated_auth_enabled()`):

```rust
/// Isolated per-channel auth (identity accounts + OAuth credential dirs).
///
/// Resolution order:
/// 1. `AGENTMUX_ISOLATED_AUTH=1` / `=0` — explicit override, always wins.
/// 2. Otherwise, defaults to isolated for every channel except `"stable"`.
///    `stable` is the real release channel — the daily-driver install(s)
///    this machine's actual work depends on — and keeps the old
///    always-global behavior so nobody's production login gets wiped by
///    a channel-name coincidence.
/// 3. If `AGENTMUX_CHANNEL` isn't set yet (e.g. a bare `cargo test`
///    invocation before any `DataPaths` has been resolved/exported),
///    stays global — conservative default when channel context is
///    unknown, not a guess.
///
/// See `docs/specs/SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md`
/// (amends `docs/specs/SPEC_ISOLATED_AUTH_DEV_TESTING_2026_07_27.md`,
/// which still documents the underlying mechanism this flag drives).
pub fn isolated_auth_enabled() -> bool {
    match std::env::var("AGENTMUX_ISOLATED_AUTH").ok().as_deref() {
        Some("1") => true,
        Some("0") => false,
        _ => std::env::var("AGENTMUX_CHANNEL")
            .map(|ch| ch != "stable")
            .unwrap_or(false),
    }
}
```

No changes needed to `identities_dir()` (`data_paths.rs:358-364`) or
`registry::resolve_shared_store_path()` (`agentmux-srv/src/registry/paths.rs:63-71`)
— both already call `isolated_auth_enabled()` and inherit the new default
automatically. `resolve_global_shared_root()` and `provider_auth_dir()`
are untouched by construction (they don't call this function at all),
preserving §2's "explicit non-goals" from the July 27 spec unchanged.

### 2.2 Why reuse `AGENTMUX_CHANNEL` rather than add a new signal

`AGENTMUX_CHANNEL` is already exported by every `DataPaths::to_env_vars()`
call (`data_paths.rs:285`) and already round-tripped back into `self.channel`
by every downstream binary via `from_env()` (`data_paths.rs:323`). Host and
srv — the only processes that call `isolated_auth_enabled()` — always go
through `from_env()`, so the var is guaranteed present in their process
environment by the time identity code runs. Reusing it avoids introducing
a second "what channel am I" signal that could drift from the one already
used for path resolution and diagnostics.

`to_env_vars()`'s own comment currently says channel is exported "so
downstream binaries can log it + surface in diagnostics... NOT used to
recompute paths." This spec adds a second legitimate use — computing the
isolation *default* — without violating that comment's actual intent
(paths themselves still flow through the explicit dir vars, never through
re-deriving a path from the channel string).

### 2.3 The escape hatch, now load-bearing in the other direction

`AGENTMUX_ISOLATED_AUTH=0` did not need to exist before (the default was
already off). It becomes the explicit opt-*out* now: a developer who wants
a `task dev` branch to keep sharing the real global account list (e.g.
debugging something unrelated to auth, without twelve re-logins) sets it
once, same ergonomics as the July 27 spec's opt-in instructions:

```bash
AGENTMUX_ISOLATED_AUTH=0 task dev
```

## 3. What this changes in practice

| Channel | Before this spec | After this spec |
|---|---|---|
| `stable` (real installed/portable release) | Global | **Unchanged — global** |
| `dev-<branch>` (`task dev`) | Global (unless `AGENTMUX_ISOLATED_AUTH=1` set) | **Isolated by default** |
| `local-<branch>-<hash>-<build-id>` (`task package`) | Global | **Isolated by default** |
| Custom `AGENTMUX_CHANNEL=…` override (parallel-channel testing, PR #1027) | Global | **Isolated by default** |

The `task package` row is the biggest practical shift: per `CLAUDE.md`'s
"Data isolation is per-BUILD for local builds" section, every local
portable build already gets its own fully isolated data dir/cef-cache —
except auth, which was deliberately kept global "so a fresh per-build data
dir still shows every agent and stays logged in." After this spec, that
carve-out narrows to agents only; auth follows the same per-build isolation
as everything else. This is an intentional, not incidental, consequence —
"every new build and dev" exercising a real login is the whole point.

## 4. Migration / rollout

No data migration is needed — this is a pure default-computation change,
not a storage-format change. Existing isolated stores (created under the
old opt-in flag) and the existing global store are both read exactly as
before; only *which one a given boot resolves to by default* changes.

Rollout risk is entirely "someone's muscle-memory `task dev` now asks them
to log in again." That's an accepted, deliberate cost — flagged explicitly
in §0 — not a bug to route around. Mitigate discoverability, not the
behavior itself (§6.2).

## 5. Non-goals

- **No change to what's isolated vs. global** — the table in
  `SPEC_ISOLATED_AUTH_DEV_TESTING_2026_07_27.md` §"What becomes isolated
  vs. what stays global" is unchanged. `provider_auth_dir()`, agent
  definitions, the agent registry, and global transcripts stay global
  exactly as today, regardless of channel.
- **No auto-seeding / import wizard** from the global store into a fresh
  isolated channel. Discussed and deliberately rejected in §7.1 — it would
  undercut the entire point of this spec.
- **No UI banner or toast** for "this channel starts with zero accounts by
  design." A log line (§6.2) is the Phase 1 bar; a UI treatment is a
  candidate follow-up, not required here.
- **No change to `stable`'s behavior, ever, under any resolution path.**
  This is the one invariant every open question below must preserve.

## 6. Implementation phases

### Phase 1 — Flip the default (1 PR)

- `agentmux-common/src/data_paths.rs`: rewrite `isolated_auth_enabled()`
  per §2.1.
- Update existing tests whose names assert the old flat default:
  - `agentmux-common`: `identities_dir_is_shared_by_default` → split into
    `identities_dir_is_shared_on_stable_channel`,
    `identities_dir_is_isolated_by_default_on_non_stable_channel`, and
    `identities_dir_is_shared_when_channel_unset` (the conservative
    unknown-channel fallback). `identities_dir_is_per_channel_when_isolated_auth_set`
    renamed to `..._explicitly_set` (unaffected by the default change,
    renamed only to pair with the new
    `identities_dir_is_shared_when_isolated_auth_explicitly_disabled_on_non_stable_channel`
    opt-out test).
  - `agentmux-srv`: `registry::paths::tests::shared_store_path_default_is_global`
    → same split (`_on_stable_channel` / `_on_dev_channel`), plus an
    explicit-opt-out variant.
    `shared_store_path_isolated_uses_instance_dir`,
    `shared_store_path_isolated_without_instance_dir_falls_back` stay as
    explicit-override-`=1` cases.
- Add a regression test asserting `migrations::runner::home_is_invariant_to_isolated_auth`
  still holds when isolation is on *by channel default* (no
  `AGENTMUX_ISOLATED_AUTH` set at all, only `AGENTMUX_CHANNEL=dev-foo`) —
  the existing test only covers the explicit-flag case.
- Add a boot-time `tracing::info!` distinguishing all four resolvable
  states (`global — stable channel`, `global — explicit opt-out`,
  `isolated — channel default`, `isolated — explicit opt-in`), replacing
  the binary "attached (ISOLATED — channel-scoped)" log line from the July
  27 spec's verification steps.

### Phase 2 — Docs (1 PR, no code)

- `SPEC_ISOLATED_AUTH_DEV_TESTING_2026_07_27.md`: its "Default
  (unset/false): zero behavior change... isolation must never be the
  default" paragraph is now factually wrong for non-`stable` channels.
  Add an amendment note at the top pointing here rather than silently
  editing the historical record.
- `CLAUDE.md` (repo root): correct "Data isolation is per-BUILD for local
  builds" — the sentence "agents and auth are GLOBAL... a fresh per-build
  data dir still shows every agent and stays logged in" must be split:
  agents stay global; auth no longer does, for any build whose channel
  isn't `stable`.
- Changeset (`task changeset -- minor "feat(identity): non-stable channels
  default to isolated per-channel auth"`) — user-facing behavior change,
  not a patch-level fix. Wording should tell a developer reading
  `VERSION_HISTORY.md` exactly what changed and the one-line opt-out.

## 7. Open questions

1. **Should there be a one-shot "seed this channel from global" convenience
   command** (e.g. `task dev:seed-auth`), for a developer who wants the new
   default's isolation guarantees going forward but doesn't want to redo
   all twelve logins the first time? **Recommend: not in this spec.** It's
   a reasonable follow-up, but it's also exactly the kind of escape hatch
   that quietly becomes the default-in-practice if it's too easy to reach
   for — ship the friction first, revisit only if it's reported as
   genuinely blocking rather than merely annoying.
2. **Does `DataPaths::resolve()` (the launcher's own process, before it
   spawns host/srv) ever call `isolated_auth_enabled()` or
   `identities_dir()` directly**, where `AGENTMUX_CHANNEL` wouldn't yet be
   set in its own process env (only computed into `self.channel`, not
   round-tripped through `std::env::var` the way `from_env()` callers get
   it)? Current research didn't find such a call site — identity storage
   appears to be an `agentmux-srv`-only concern, reached only via
   `from_env()`. **Recommend: verify during Phase 1 implementation** (grep
   `agentmux-launcher` for both function names); if a call site exists,
   have `resolve()` also `std::env::set_var("AGENTMUX_CHANNEL", …)` on its
   own process immediately after computing `self.channel`, so the two
   construction paths (`resolve()` vs `from_env()`) can't disagree.
3. **CI**: does any CI job invoke `task dev` or otherwise boot a real
   channel expecting global-shared auth? Test suites use
   `AGENTMUX_HOME_OVERRIDE` + explicit per-test env, not the real global
   store, so this is believed unaffected — **confirm by grepping CI
   workflows for `task dev` invocations outside of `task test`** before
   merging Phase 1.

## 8. Verification

1. `cargo test -p agentmux-common && cargo test -p agentmux-srv` — updated
   and new tests (§6, Phase 1) pass.
2. Boot plain `task dev` (no env override) on a fresh branch never used
   with `AGENTMUX_ISOLATED_AUTH` before. Confirm via
   `muxlog srv grep "shared store"` the new four-state log line reads
   `isolated — channel default`, and Armory shows **zero** accounts.
3. Re-boot the same branch. Confirm the isolated store persisted (an
   account created in step 2 is still there) — channel-scoped, not
   randomized per boot.
4. Boot a real `stable`-channel build (installed or a stable-labeled
   portable). Confirm **zero change**: still the full global account list
   from before this spec shipped. This is the regression guard for the
   real daily-driver instance.
5. `AGENTMUX_ISOLATED_AUTH=0 task dev` on a dev branch. Confirm the
   explicit opt-out restores the old global-sharing behavior.
6. End-to-end the scenario from §0: on an isolated dev channel, run a
   fresh Claude Connect flow (tier 1, #2429's surface) and a mid-session
   credential-loss relogin (tier 3, PR #2425's surface) against a
   throwaway account created inside that channel — confirm both exercise
   real login code paths without the real global 12-account list ever
   being touched.
