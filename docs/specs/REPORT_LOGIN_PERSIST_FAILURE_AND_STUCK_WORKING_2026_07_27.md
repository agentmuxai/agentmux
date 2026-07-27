# Report: "Error: not logged in" after a successful login, and two stuck-"Working…" states

**Date:** 2026-07-27
**Author:** AgentA
**Status:** Fixes 1-5 applied and verified (see §5); §6 is a design-only addition, not yet built.
**Test round analyzed:** agent "Nark" (block `507ecd63-8144-4ad1-878c-64af534a6a0c`),
dev instance on branch `agenta/remove-auto-login-trigger`, srv log
`agentmuxsrv-v0.54.5.log.2026-07-27`, window 08:41:09–08:41:48 UTC.

Three reported symptoms:

1. After clicking "Log in" and seeing "✓ Login successful" in the pane, the very
   first message ("u there") comes back with **"Error: not logged in"**.
2. After that error, the pane stays in **"Working…" indefinitely** — the turn
   never ends.
3. Tangential: **AgentA's own pane** (in the user's main instance) is
   consistently shown as "Working…" even after its turns visibly complete.

---

## 1. Problem 1 — "Error: not logged in" after a successful login

### Root cause (CONFIRMED, static + DB + log evidence)

**Every account-persist call made by the login flow fails at serde
deserialization, so no `db_accounts` row and no agent↔account link is ever
created. The "Login successful" message is a false positive: the credential is
seeded onto disk into an account dir whose account row never comes into
existence — and which the identity sweeper then deletes as an orphan on the
next login attempt.**

The failure chain, each link verified:

1. `relogin()` → `runProviderLogin` tier 2 mints account
   `8d34a369-6ba6-4071-93bd-d4e051cdb457` and seeds a valid credential into it.
   Log (08:41:15.877): `seed_provider_auth_from_global: seeded isolated dir from
   valid global login … dest=…\identities\8d34a369-…\claude\.credentials.json`.
2. Tier 2 then calls `persistSeededAccount`
   (`frontend/app/view/agent/flows/register-seeded-account.ts:78-103`), which
   sends this payload to the `upsertidentityaccount` RPC:

   ```ts
   await RpcApi.UpsertIdentityAccountCommand(TabRpcClient, {
       id: accountId,
       name: `${providerId}-oauth`,
       provider: providerId,
       kind: "oauth",
       secret_ref: { backend: "oauth_config_dir", dir },
       status: "valid",
   });
   ```

   Note: **no `created_at`, no `updated_at`.**
3. The srv handler
   (`agentmux-srv/src/server/agent_handlers/identity.rs:154-188`) does
   `serde_json::from_value::<IdentityAccount>(data)` **before** its own
   "`created_at` and `updated_at` are server-set so callers don't have to know
   the current time" logic can run. The struct
   (`agentmux-srv/src/backend/storage/identities.rs:107-123`) declares:

   ```rust
   pub created_at: i64,   // ← NO #[serde(default)]
   pub updated_at: i64,   // ← NO #[serde(default)]
   ```

   `display_name`, `context`, and `status` all have `#[serde(default…)]`;
   the two timestamps do not. Deserialization therefore fails with
   `missing field 'created_at'` on **every** call — instantly and
   deterministically. The handler's timestamp-defaulting code (lines 165-175)
   is unreachable for this caller.
4. `persistSeededAccount` catches the RPC error, logs it **only to the pane's
   hidden activity log** (visible only if the Shell drawer is open — it appears
   in no muxlog channel), and returns `false`. Tier 2 retries once (same
   deterministic failure, ~5 ms total — log shows the terminal opening at
   08:41:15.882, 5 ms after the successful seed), then falls through to tier 3.
5. Tier 3 opens a real terminal; its 5-second poll immediately finds the
   already-seeded credential (second `seed_provider_auth_from_global` at
   08:41:20.898), tries `persistSeededAccount` again → same serde failure →
   the flow still reports **`terminal-success`** → the pane shows
   "✓ Login successful".
6. **DB state after the round (queried directly):** `db_accounts` has **no row**
   `8d34a369…`; `db_agent_identity_links` has **no link** for it. The newest
   claude account row in the global store (`~/.agentmux/shared/store.db`) is
   from **2026-07-24** — i.e. no login flow has persisted an account in days.
7. Poetic detail: at 08:41:15.869 the identity sweeper deleted the **previous
   round's** orphaned account dir
   (`963d1a95…`, age 10719 s ≈ 3 h): *"identity.sweep: orphaned account dir
   removed (no matching account row, past age threshold)"*. Every login round
   seeds a real credential into a dir that the next round garbage-collects.
   This is why the symptom "resets" between test sessions.
8. When the user then sends "u there" (08:41:47), the spawn env still has no
   valid credential anywhere to find:
   - The block's `cmd:env` points `CLAUDE_CONFIG_DIR` at the **provider-level
     shared dir** `…\shared\providers\claude` (written by launch-flow Phase 1's
     `SetMeta` from `buildAuthEnv`/`ensureAuthDir`) — which `CheckCliAuth`
     itself logged as credential-less: `check dir=…\providers\claude
     present=true token=none` / `no credentials in provider dir`.
   - The identity resolver's injection had nothing to inject (no account row,
     no link) — the srv log shows **no** `identity.spawn.blocked`, no
     `LinkAgentIdentity`, no injection line in the whole window.
   - The message went to an **already-running** persistent CLI process
     (`send_message` emitted `agent-message-accepted` with **no**
     "persistent process spawned" line → `is_running()` was true), i.e. a
     process spawned in an earlier round with that same credential-less env.
   The CLI (correctly) answers that it is not logged in.

### Contributing design gaps (each would have masked or shrunk the bug)

- **G1 — persist failure is invisible.** The "credential seeded but the Armory
  account couldn't be registered" error goes only to the hidden activity log.
  It never reaches `authNotice`, the pane conversation, or any muxlog channel.
  A deterministic 100%-failure ran for days without a trace anywhere visible.
- **G2 — `terminal-success` does not require persist to have succeeded.** The
  flow's contract says "registers it once the login lands on disk", but the
  outcome (and the "Login successful" UX) is reported even when registration
  failed — success is decided by the credential file appearing, not by the
  account being persisted + linked.
- **G3 — `relogin()` never updates the block's `cmd:env`.** `useGlobalLogin()`
  explicitly rewrites `cmd:env` to the new account dir after linking;
  `relogin()`'s success paths do not (they rely on `persistAndLinkAccount`'s
  linkTarget, which never ran here). Even with the serde bug fixed, a live
  pane's next spawn would still read the stale provider-dir env until
  something rewrites it — and:
- **G4 — a stale, already-running persistent process is reused.** Env changes
  (link, cmd:env rewrite) only take effect on respawn. A login recovery that
  succeeds while the old unauthenticated CLI process is still alive changes
  nothing for that process. Nothing restarts it after a successful login.
- **G5 — after the mount flow bails with `auth_failed`, nothing re-runs
  Phase 3.** By design (2026-07-27) the mount flow stops before
  `ControllerResync` when unauthenticated. After a successful "Log in" click,
  no code path performs the Phase-3 controller resync / ready notification for
  the pane. (With `retryAfterLogin: false` — correct for not resending old
  messages — there is now *no* post-login reconciliation at all.)

### Recommended fixes (in order)

1. **One-line Rust fix (the root cause):** add `#[serde(default)]` to
   `created_at` / `updated_at` on `IdentityAccount` — the handler already
   treats `0` as "server should stamp now". Alternatively `#[serde(default)]`
   on the whole struct insert path via a dedicated upsert-payload type. Add a
   regression test deserializing exactly the frontend's payload shape.
2. **Make persist failure loud:** on `persistSeededAccount === false`, set
   `authNotice` (and/or post a warning notification) instead of only the hidden
   log; do NOT report `seeded`/`terminal-success` (or at minimum do not post
   "Login successful") when registration failed — G2.
3. **Post-login reconciliation:** on a successful `relogin()`, (a) update the
   block's `cmd:env` to the account dir (mirror `useGlobalLogin`), and (b) if
   the pane's persistent process is running with stale env, force a respawn
   (`forcerestart` resync) — G3/G4/G5 together explain why even a "fixed" login
   wouldn't have worked until the pane was fully reopened.

---

## 2. Problem 2 — pane stuck in "Working…" after the error

### Root cause (code-confirmed mechanism)

**The persistent controller ends a turn in exactly one place: on seeing a
`{"type":"result"}` frame on the CLI's stdout**
(`agentmux-srv/src/backend/blockcontroller/persistent.rs:922-944`):

```rust
if parsed.get("type").and_then(|v| v.as_str()) == Some("result") {
    health_read.set_active_turn(false);
    // …publish controllerstatus { turn_active: false } …
}
```

There is **no other path back to `turn_active = false`** while the process
stays alive: no timeout, no stderr-based classification, and the
`agents/failure.rs` classifier only runs on **process exit**. An
unauthenticated CLI that answers with a plain error line (not a stream-json
`result` frame) — exactly the "Error: not logged in" case — leaves
`turn_active` true forever. The frontend faithfully renders that as
"Working…" forever. (The `agent health transition Healthy→Idle` seen 48 ms
after the accept is the *health* monitor, a separate state from
`turn_active`.)

Because no `result` frame arrives and the process doesn't exit, no
`agentfailure` event is emitted either — so the failure-recovery row (with its
"Login Again" action) never appears. The user gets a raw error line, an
eternal spinner, and no recovery affordance: the worst of all three surfaces.

### Recommended fixes

1. **Turn-end watchdog:** the per-turn health watchdog
   (`core::spawn_health_watchdog`) already exists — extend it (or add a
   parallel arm) so that N seconds of a turn with output that has stopped (or
   an auth-pattern match on stderr, see `identity/auth_patterns.rs` which
   already recognizes "not logged in") synthesizes a turn end: set
   `turn_active=false`, publish status, and emit a classified `agentfailure`
   (code `auth` for this case) so the recovery row appears.
2. **Classify inband errors, not just exits:** `classify_output_line` already
   runs per line (`persistent.rs:916`); a line matching the auth patterns
   during an active turn should end the turn with an `auth` failure rather
   than only counting toward health.

---

## 3. Problem 3 — AgentA's own pane stuck "Working…" (main instance)

### What the logs show

For AgentA's block (`01da2fb8-2486-4f0e-a85a-c536bcefe81e`) in the main
instance's srv (`agentmuxsrv-v0.54.4.log.2026-07-27`), the server-side state is
**healthy and correct**: every long turn shows `Idle → Healthy` at turn start
and `Healthy → Idle` at turn end (e.g. 05:44:25 → 06:02:21 for an 18-minute
turn, with intermediate `Healthy → Stalled → Healthy` flaps during quiet tool
stretches). The server knows the turns end. The stuck "Working…" is therefore
a **frontend-side desync**, not a backend turn_active leak (contrast with
problem 2, where the backend flag itself is stuck).

### Most likely mechanism (hypothesis — not yet instrumented)

The turn-end `controllerstatus { turn_active: false }` publish is
**fire-once with `persist: 0`** (`persistent.rs:930-945`). If the pane's
frontend misses that single event — backgrounded/throttled window, WPS
reconnect gap, pane not currently subscribed — nothing ever re-publishes it.
The only repair paths are:

- the mount-time one-shot `GetControllerStatus` (runs only on pane re-open),
- the *next* turn's own start/end events.

So one missed turn-end event pins the pane at "Working…" until the user
reopens the pane or starts a new turn — matching the observed "stuck even
though the agent returned" behavior, and consistent with the prior
stuck-"Working" incidents
(`docs/specs/REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md`, reagent
P1s on PR #2241/#2303). The `Stalled` flaps also suggest long quiet stretches
during which a throttled renderer is most likely to drop events.

### Recommended fixes

1. **Persist the last controllerstatus event** (`persist: 1`) or include
   `turn_active` in a periodic low-frequency heartbeat, so a missed flip is
   self-healing.
2. **Reconcile on window focus/visibility:** the pane already has the one-shot
   `GetControllerStatus` reconcile; re-running it on `visibilitychange`/focus
   (cheap, once) would repair any missed event the moment the user looks.

---

## 4. Suggested fix order

| # | Fix | Size | Fixes |
|---|-----|------|-------|
| 1 | `#[serde(default)]` on `IdentityAccount.created_at/updated_at` + payload-shape regression test | XS (Rust) | P1 root cause |
| 2 | Surface persist failure (authNotice + no false "Login successful") | S (TS) | P1 G1/G2 |
| 3 | Post-relogin reconciliation: update `cmd:env`, force respawn of stale process | M (TS) | P1 G3/G4/G5 |
| 4 | Inband auth-error → synthesized turn end + `auth` failure classification | M (Rust) | P2 |
| 5 | Persisted/heartbeat controllerstatus + focus-time reconcile | M (Rust+TS) | P3 |

Items 1-2 are enough to make the next manual test round meaningful; item 3 is
required for a login to fix a pane *without reopening it*; item 4 removes the
infinite spinner; item 5 addresses the tangential busy-status desync.

Note: fixes 1+2+3 must land together with a Rust rebuild (`task build:backend`)
before the next dev round — the frontend changes alone cannot help while the
upsert RPC rejects every payload.

---

## 5. Fixes 4 and 5 — found during manual verification of fixes 1-3

Fixes 1-3 landed and were retested live. Two more concrete bugs surfaced,
neither anticipated in §1-4 above, both now fixed and covered by new tests.

### 5a. Lost-update race in `update_object_meta` (the "most robust route" fix)

Retrying a message after a fixed login still failed identically: the backend's
own stale-`--resume`-session-id self-heal (`persistent.rs`'s stderr reader,
`core::persist_session_id(block_id, "", ...)`) fired and logged success, but
`agent:sessionid` in the DB never actually cleared.

Root cause: `agentmux-srv/src/server/service/object_helpers.rs`'s
`update_object_meta` did a plain `must_get` → `merge_meta` → `update` — two
SEPARATE `Store` connection-mutex acquisitions, not one atomic unit. On every
persistent-controller process exit, TWO independent async tasks write to the
SAME block's meta within ~400ms of each other: the stderr reader clearing
`agent:sessionid`, and the exit-wait task's `clear_active_pid` clearing
`session:active_pid`. Each read a pre-write snapshot of the block's *entire*
meta map, merged in its own single key, and wrote the *entire* merged map back
— so whichever writer finished LAST silently reverted every key it didn't
itself touch back to whatever it read, including the other writer's
already-applied change.

**Fix:** rewrote `update_object_meta` to run the whole read-merge-write inside
one `Store::with_tx` critical section — a single acquisition of the store's
connection `Mutex` wrapping a real SQLite transaction, via `StoreTx`'s
already-existing `get`/`update` (used internally, no additional locking).
Any two `Store::*` callers for ANY object now strictly serialize instead of
racing. Added a genuine multi-threaded regression test (`object_helpers.rs`'s
`update_object_meta_concurrent_writers_both_keys_survive` — two OS threads,
synchronized via a `Barrier`, each writing a different key to the same block;
asserts both keys survive). Full backend suite (1659 tests) passes.

### 5b. Optimistic `TurnStart` never reverted on a synchronous send failure

Separately, sending a message into a pane whose backend controller was gone
(e.g. right after a `task dev` restart, before the pane is reopened — an
everyday consequence of the manual test loop in this session, not a
credential issue at all) showed "Working…" forever with no error surfaced.
This is the direct answer to *"why does it say Working… — doesn't it know
it's not logged in?"*: **it isn't about auth-error detection at all** — the
turn never even started server-side.

`agent-view.tsx`'s `handleSendMessage` dispatches `TurnStart` OPTIMISTICALLY,
before `RpcApi.AgentInputCommand` is even awaited (so the composer clears and
the spinner appears instantly on send, without waiting a round trip). If that
RPC call rejects — no controller registered, the identity spawn gate blocking
on a bad credential, a plain network hiccup, anything — `deliverToBackend`'s
catch block (`useAgentCommands.ts`) only ever dispatched
`PendingMessageRejected`, which removes the ghost pending-message row and
**does nothing to `turnPhase`**. There was no path back to `Idle` for this
failure mode at all, independent of anything the CLI itself does — this is a
strictly more general and more common bug than the "auth-error doesn't end
the turn" gap in §2/§4 item 4 above (which is about a `result`-frame-only
turn-end contract for a process that DID spawn).

**Fix:** added an `initiatesTurn` parameter to `deliverToBackend` (true for
the idle/turn-initiating send path, false for a message flushed from the
"held while busy" queue mid-turn — a held-message's OWN delivery failure must
NOT cut short a turn that's genuinely already streaming). On
`initiatesTurn && RPC rejects`, dispatch the existing `TurnReset` action
(already used identically for the bang-command / handled-slash-command "no
real turn happened" cases in the same file) to put `turnPhase` back to
`Idle`. Added `useAgentCommands.test.ts` (previously no test file existed for
this hook) with two tests: the idle-send-rejects case resets to Idle, and the
held-message-flush-rejects case leaves an already-active turn's phase
untouched. Full frontend suite (1943 tests) passes.

Item 4 from §4's table (inband-auth-error → synthesized turn end, for a
process that DID spawn and IS alive but never emits a `result` frame) remains
open — 5b fixes the "RPC never even reached a controller" case, not the
"CLI is running but hung/erroring without a terminal frame" case. Both are
real; they're different failure points in the same turn-lifecycle.

---

## 6. HTTP 429 (rate-limited) handling — a proper backoff-retry design

Requested alongside the fixes above: rate-limit handling should follow retry
best practices (exponential backoff + jitter), not the current fixed
two-step schedule. Design only in this section — not yet implemented.

### Current state (grounded in the actual code, 2026-07-27)

- `agentmux-srv/src/agents/failure.rs`'s `classify()` recognizes a 429 via
  plain substring/keyword matching on the CLI's stderr + terminal
  stream-json `result` frame text (`"rate limited"`, `"temporarily limiting
  requests"`, `"rate_limit"`, or a mentioned HTTP status `429` —
  `mentions_http_status(&hay, "429")`) → `FailureClass::RateLimited`,
  `retryable: true`.
- `frontend/app/view/agent/failure/failure-accessory.ts`'s `isTransient()`
  auto-retry-eligibility check covers exactly three classes: `rate_limited`,
  `overloaded`, `network` — all three share the same schedule below.
- `frontend/app/view/agent/hooks/useAgentFailure.ts`'s `armAutoRetry()` runs
  a **fixed, non-exponential, 2-entry schedule**:
  `const AUTO_RETRY_BACKOFF_S = [5, 10] as const; // then manual-only`
  — a 1-second countdown ticks down from 5, then (if it fails again) from
  10, then permanently falls back to the manual "Retry" button
  (`SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md §6`'s "2-attempt episode
  budget"). Identical for all three transient classes — no 429-specific
  tuning.
- **No `Retry-After` (or equivalent hint) is ever parsed.** The classifier
  extracts only a title/detail/stderr-tail; nothing reads a suggested wait
  time out of the matched text even on the rare occasion the CLI's own error
  output happens to include one. The CLI subprocess only exposes plain
  stdout/stderr text to us — no raw HTTP response headers are available
  regardless, so any `Retry-After` support here is necessarily best-effort
  text-parsing, not a guaranteed structured value.

### Gaps vs. best practice

1. **Not exponential.** 5s → 10s is a fixed doubling for exactly two steps,
   not real exponential growth — a THIRD consecutive 429 has no automatic
   retry left at all and drops straight to manual, even though 429 bursts
   are frequently short and self-clearing over more than two attempts.
2. **No jitter.** `armAutoRetry` fires at exactly T+5s / T+10s,
   deterministically. Multiple agent panes sharing one account/quota (a real
   scenario in this app — several agents, one Anthropic subscription) would
   all retry in lockstep and collide on the SAME rate limit again
   (thundering herd).
3. **No `Retry-After` honoring**, as above — the schedule is blind to any
   hint the API/CLI does surface.
4. **Uniform across transient classes.** A network blip and a 529 overload
   plausibly clear faster than an account-level 429 throttle; today's
   schedule doesn't distinguish.

### Proposed design

- **Decorrelated jitter** (the AWS Architecture Blog's recommended formula —
  avoids both the thundering-herd problem plain jitter has and the
  self-synchronizing failure mode capped-exponential-without-jitter has):

  ```
  sleep = min(cap, random_between(base, previous_sleep * 3))
  ```

  Per-class `(base, cap, max_attempts)`, since 429s specifically tend to
  need a longer runway than a network blip:
  - `rate_limited`: base=2s, cap=60s, max_attempts=5
  - `overloaded`:   base=2s, cap=30s, max_attempts=4
  - `network`:      base=1s, cap=15s, max_attempts=3

  Worst-case total auto-retry window stays bounded (a few minutes) — same
  spirit as today's fallback-to-manual safety net, just wider for the class
  most likely to actually clear on its own.
- **`Retry-After` honoring, best-effort:** extend `classify()` to optionally
  extract `retry_after_secs: Option<u64>` from the matched rate-limited /
  overloaded text (regex over patterns like `retry-after`,
  `retry after (\d+)`, `wait (\d+)s`) and thread it onto `AgentFailure`. When
  present, the frontend's computed delay becomes
  `max(retry_after_secs, jittered_delay)` instead of ignoring the hint.
  Absent (the common case) falls through to pure jittered backoff — no
  required behavior change for the classifier's existing callers.
- **Manual fallback unchanged.** Once `max_attempts` is exhausted for the
  failure's class, the existing manual "Retry" button UI
  (`SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.md §6`) takes over exactly as
  it does today — only the SCHEDULE feeding the auto-retry countdown
  changes, not the episode/budget/manual-fallback architecture around it.
- **UI unchanged.** `autoRetryIn`'s live countdown render already exists;
  it just needs to be fed the new computed delay instead of a flat array
  lookup by attempt index.

### Implementation sketch (not yet built)

- `frontend/app/view/agent/hooks/useAgentFailure.ts`: replace the flat
  `AUTO_RETRY_BACKOFF_S` array with a per-class policy table
  (`{ base, cap, maxAttempts }`) and a `nextDelaySecs(code, attempt,
  retryAfterSecs?)` helper implementing the decorrelated-jitter formula
  above (needs a `previousSleep` carried alongside the existing `autoRetries`
  counter). `armAutoRetry` calls the helper instead of indexing the array,
  and stops once `attempt >= maxAttempts` for that failure's class instead
  of the current shared `AUTO_RETRY_BACKOFF_S.length` check.
- `agentmux-srv/src/agents/failure.rs`: add `retry_after_secs: Option<u64>`
  to `AgentFailure` (`#[serde(skip_serializing_if = "Option::is_none")]`,
  matching the existing `exit_code`/`signal` pattern), populated by a
  best-effort regex inside the existing rate-limited/overloaded branches.
- Tests: extend `failure.rs`'s existing unit tests with a
  `retry_after_secs` extraction case (and a no-hint-present case asserting
  `None`); add a new `useAgentFailure.test.ts` (none exists today) pinning
  the jitter formula's bounds (`base <= delay <= cap` across many samples,
  since jitter is randomized) and the per-class `max_attempts` cutoff.

This is fully additive — nothing about the existing manual-retry fallback,
failure-row UI, or 2-attempt "episode" concept (dismiss/new-turn resets the
budget) needs to change, only the schedule feeding the auto-retry countdown.
