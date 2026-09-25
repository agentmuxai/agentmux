# Report: Codex App Server PR stack — status audit

**Status:** historical
**Date:** 2026-09-14 (outcome added 2026-09-25)
**Author:** Korp
**Baseline:** `main` @ `e9d1d1b4d` (v0.55.42), PRs as of 2026-09-14
**Ask (repo owner):** *"get latest agentmuxai/agentmux into your workspace. we had a codex agent working opening a bunch of PRs getting it functional for agentmux. can you take a look? write a report to file on what you find."* Follow-up: a second, uncommitted copy of the work exists at `C:\codex-workspace\agentmux`, to be deleted once this stack lands.

---

## Outcome (2026-09-25)

The stack landed on 2026-09-15. Everything below §Outcome is the 2026-09-14 snapshot, kept as-is.

- **#3212–#3215 merged**, not into `main` but into `codex/app-server-interactive-control-base`
  (13:00–14:05 UTC). **#3228** then squash-merged that combined history into `main` at 14:26 UTC
  as `973de73a6` (+49,452/−158). **#3207–#3211 were closed unmerged** at 14:11 UTC; #3228
  superseded them.
- **Review happened.** `reagentx-workflow[bot]` left an `APPROVED` review on #3228, which resolves §2a.
- **#3213's doc-status failure (§2b) was fixed** on its branch (`69c91c8f8`, "give the CI-lane
  spec a gate-recognized Status line") before it merged.
- **The uncommitted local fix (§3) was pushed.** `5c435715e` ("queue an App Server message sent
  before the handshake completes") went onto `codex/app-server-controller-runtime`. `main` now has
  `pending_messages` in `app_server_controller.rs`, with the `.clone()` at the push site.
- **`C:\codex-workspace` was deleted on 2026-09-25.** I checked it first: clean working tree, no
  stashes, every local commit reachable from `origin`, and only build artifacts among the ignored
  files.
- **§5 still holds.** No provider on `main` selects `ControllerType::AppServer`. Codex still runs
  on the `exec --json` subprocess controller, and making App Server the default is still a
  future rollout PR.

---

## 0. The short version

A codex-driven agent (GitHub identity `Agent5-asaf`) opened **9 stacked PRs (#3207–#3215)**
on 2026-09-12, building a new **Codex App Server controller** — a typed, fail-closed Rust
client for OpenAI Codex CLI's JSON-RPC `app-server` protocol, meant to eventually replace
today's `codex exec --json` subprocess integration. The engineering itself looks careful:
each PR is small, disabled by default, and covered by protocol-exact tests.

But **none of it is close to landing**, for reasons that have nothing to do with the code's
quality:

1. **Zero real code review has happened.** ReAgent (this repo's automated review bot) has
   crashed on *every single PR* in the chain — "Claude CLI exited 1: no findings produced" —
   including on re-review after new commits. Codex's own connector bot left only boilerplate
   (no actual findings). No human has reviewed any of the 9 PRs. No PR has an `APPROVED` state.
2. **One PR fails CI outright.** #3213 fails `check-doc-status.sh` — its new spec doc is
   missing the required `**Status:**` line. One-line fix.
3. **The whole stack is a serial dependency chain** (each PR's base branch is the previous
   PR's branch, not `main`), so nothing merges until #3207 does, and #3207 is itself
   `mergeStateStatus: BLOCKED`.
4. **The local checkout at `C:\codex-workspace\agentmux` has an uncommitted, unpushed change
   on top of PR #3215's branch that does not compile** (`E0382: use of moved value`). It's a
   real bugfix (queues a `send_message` call that arrives before the App Server handshake
   completes, instead of erroring), but it never made it into any PR and is broken as written.
5. **The feature is not yet "functional" for end users** — every PR explicitly keeps
   `Codex`'s provider registry on the legacy subprocess controller. This stack is plumbing,
   not activation. Framing it as "getting it functional" overstates where it is: it's
   "getting it *buildable*," gated off behind a controller-type flag until a later rollout PR.

None of this needs a redesign. It needs: a doc-status fix, a fixed and pushed version of the
local diff, a working ReAgent review pass (or a human review standing in for it), and then a
straightforward sequential merge.

---

## 1. What the stack actually builds

Codex CLI 0.154.0 added a managed **App Server** mode (`codex app-server --listen stdio://`) —
a long-lived process speaking line-delimited JSON-RPC, as an alternative to the current
one-shot `codex exec --json` subprocess AgentMux drives today. The spec added in #3207
(`docs/specs/SPEC_CODEX_APP_SERVER_FIRST_CLASS_PROVIDER_2026_09_12.md`) proposes a staged
migration to it, keeping `exec --json` as a rollout fallback.

The 9 PRs, in dependency order (each PR's base = previous PR's branch):

| PR | Base | Title | +/- | What it adds |
|---|---|---|---|---|
| #3207 | `main` | chore(codex): bump CLI pin to 0.154.0 | +1032/-10 | Bumps the managed `@openai/codex` pin, adds the App Server spec, regenerates the specs index |
| #3208 | #3207 | test(codex): snapshot App Server 0.154.0 protocol | +42410/-0 | Vendors/snapshots the 0.154.0 JSON-RPC schema for exact-shape tests (accounts for the bulk of the stack's line count) |
| #3209 | #3208 | feat(codex): add disabled App Server stdio controller foundation | +1324/-0 | `ControllerType::AppServer` (inert), JSONL framing, request correlation, cancellation-safe pending slots, fail-closed protocol handling, handshake metadata |
| #3210 | #3209 | feat(codex): add App Server thread and turn reducer | +685/-0 | `thread/start`, `thread/resume`, `turn/start` builders; session snapshot with idle/running/completed/failed phases; reconciles deltas against authoritative `item/completed` |
| #3211 | #3210 | feat(codex): add guarded App Server turn controls | +411/-1 | `turn/steer`, `turn/interrupt`; enforces active thread/turn identity and exact `expectedTurnId`; never auto-approves server-initiated requests |
| #3212 | `...-interactive-control-base` | feat(codex): route App Server approval requests | +215/-1 | Fail-closed routing for approval/permissions/user-input/MCP elicitation requests, scoped to active thread/turn |
| #3213 | #3212 | docs(ci): specify balanced PR and nightly test lanes | +96/-0 | **Unrelated to App Server** — a CI-lane policy spec, threaded into the middle of this chain |
| #3214 | #3213 | feat(codex): add App Server thread and account lifecycle | +458/-0 | `thread/fork`, `thread/read/list/archive/compact`, `account/read`, `account/logout`, login lifecycle |
| #3215 | #3214 | feat(codex): add App Server controller runtime | +708/-143 | The actual `AppServerController`: process spawn from block metadata, thread start/resume, turn dispatch, status/health, shutdown, registry/input-handler wiring |

Every PR explicitly states the feature stays **disabled by default** — Codex's provider
registry keeps `controller_type: Subprocess` until a separate rollout PR flips it. This is a
deliberate, sound way to land large infrastructure incrementally. It also means: merging all
9 PRs today changes zero runtime behavior for any existing user.

**#3213 does not belong in this chain.** It's a CI/nightly-lane policy doc, topically
unrelated to Codex App Server, but it sits as a hard dependency between #3212 and #3214 —
so App Server work now can't merge without also dragging in and resolving an unrelated CI
spec's own CI failure (see §2).

---

## 2. Current state — nothing is close to mergeable

### 2a. No real review has happened on any of the 9 PRs

Checked every PR's review list via `gh api .../reviews`. Pattern is identical across all 9:

- **`reagentx-workflow[bot]`** (ReAgent, the repo's Claude-based auto-reviewer) commented on
  every PR — sometimes 2-3 times (re-review after new commits) — and **every single comment
  is the same failure template**:

  > ⚠️ **Review failed — no findings produced.** The reviewer errored before it could
  > complete (e.g. provider content-filter reject, crash, or 840s timeout)... `Claude CLI
  > exited 1:`

  ReAgent has never once successfully completed a review anywhere in this chain, across
  #3207 through #3215, across multiple retrigger attempts. This is a **systemic failure
  specific to this PR chain** (or its timing) — worth its own investigation, since ReAgent
  works elsewhere in the repo. #3208 is the one PR with a plausible innocent explanation
  (it adds +42,410 lines of vendored protocol snapshot data — large enough to plausibly hit
  the 840s timeout or a size-related crash) but the *other 8* PRs are all normal-sized
  (96–1,324 lines) and still failed identically, which points at something more structural
  (stacked-diff context resolution against non-`main` bases, maybe) than raw size.

- **`chatgpt-codex-connector[bot]`** commented on every PR too, but every comment is pure
  boilerplate ("Here are some automated review suggestions... If Codex has suggestions, it
  will comment; otherwise it will react with 👍") with **no actual findings in the body** —
  functionally a no-op review.

- **No human reviewer** has commented or reviewed any of the 9 PRs.
- **No PR has an `APPROVED` review state.** Every review entry across all 9 PRs is `COMMENTED`.

Net effect: this entire 9-PR, ~47,000-line-diff stack has had **no substantive review at
all**, automated or human, despite the repo's own `CLAUDE.md` treating ReAgent as a load-bearing
gate (e.g. the version-invariant check called out there). Landing this as-is would be the
first time this size of change merged with zero actual review coverage.

### 2b. #3213 fails CI

`doc status + grep gates` fails on #3213 with:

```
FAIL docs/specs/SPEC_CI_PR_NIGHTLY_BALANCE_2026_09_12.md
     New doc has no **Status:** line.
     One of: draft proposed active implemented living historical superseded  (see docs/specs/README.md)
```

Trivial one-line fix (add a `**Status:**` line to the new spec), but as of now this PR — and
everything stacked after it (#3214, #3215) — is red.

### 2c. Merge topology — serialized behind #3207

Every PR bases on the previous PR's branch, not `main`. `gh pr view` shows:

- #3207 → `main`, `mergeStateStatus: BLOCKED`, `mergeable: MERGEABLE` (blocked presumably on
  required review/status, given §2a — no branch protection reason surfaced beyond the absent
  review).
- #3208–#3215 → each previous PR's branch, all `mergeStateStatus: UNSTABLE`.

Practically: **nothing in this stack can merge to `main` until #3207 does**, and #3207 can't
merge until it clears whatever check is blocking it (most likely: required review, given
§2a). This is normal for a stacked-PR workflow, but it means the whole feature is currently
gated on fixing the ReAgent review failure, not on any code problem.

### 2d. CI results otherwise are healthy

Ignoring #3213's doc-gate failure and the routine Windows-runner cancellations (a known,
pre-existing flake — see `docs/incident/INCIDENT_2026_09_10_CI_PR_WINDOWS_RUNNER_HANG_BACKLOG.md`),
every PR's Linux build, `vitest`, and doc/grep gates pass. The Rust test counts cited in PR
bodies (9, 4, 15, etc., scoped to `backend::blockcontroller::app_server*`) are consistent
with the diff sizes and match what CI actually ran.

---

## 3. The local workspace at `C:\codex-workspace\agentmux`

This is a second clone of the same repo, currently checked out on `codex/app-server-controller-runtime`
(PR #3215's branch), with all 9 branches present locally and tracking their `origin`
counterparts 1:1 — **except for one uncommitted, unpushed change**:

```
M agentmux-srv/src/backend/blockcontroller/app_server_controller.rs
```

This diff is a real fix, not noise: today, `AppServerController::send_message` errors out
immediately (`"Codex App Server is not initialized"`) if called after the process has been
spawned but before the App Server handshake/session is ready — a real race for any caller
that sends a message right after opening a Codex App Server block. The uncommitted change adds
a `pending_messages: VecDeque<String>` to the controller's inner state: if the process exists
but the session isn't ready yet, the message is queued instead of rejected, and the queue is
drained once the session initializes.

**This fix does not compile as written:**

```
error[E0382]: use of moved value: `message`
   --> agentmux-srv\src\backend\blockcontroller\app_server_controller.rs:240:38
233 |         inner.pending_messages.push_back(message);
    |                                          ------- value moved here
240 |     self.spawn_turn(session, message);
    |                              ^^^^^^^ value used here after move
```

`message` is pushed into the queue (moved) inside one branch, then unconditionally reused a
few lines later for the immediate-send path. Needs a `.clone()` at the push site (or a
restructure so the two paths don't share the binding) — mechanically trivial, but as it
stands this workspace **cannot build `agentmux-srv`**, and this fix was never committed, so
it isn't in PR #3215 or anywhere on `origin`.

Everything else in this clone (the other 8 branches' tips, `main`) matches `origin` exactly —
no other divergent work, no stashes, no untracked files. This clone is otherwise redundant
with the primary working copy at `C:\Users\asafe\.agentmux\agents\korp-0620g\agentmux`, which
already has all 9 branches fetched.

---

## 4. Recommendations

1. **Fix and push the local diff first**, before deleting `C:\codex-workspace`: add the missing
   `.clone()`, confirm `cargo test -p agentmux-srv backend::blockcontroller::app_server_controller`
   still passes, commit, and push to `codex/app-server-controller-runtime` so PR #3215 picks up
   the queuing fix. (Have not done this yet — flagging for a decision, since it changes PR
   #3215's diff after the fact.)
2. **Fix #3213's doc-status gate** (add a `**Status:**` line to
   `docs/specs/SPEC_CI_PR_NIGHTLY_BALANCE_2026_09_12.md`) so the rest of the chain goes green.
3. **Get a real review pass before merging anything.** Re-trigger ReAgent on #3207 and see if
   it's a transient failure or reproducible; if reproducible, this is worth its own bug report
   against the reagent tooling (every PR in a 9-deep stacked chain failing identically is a
   strong signal it's not per-PR bad luck). Failing that, a human review should stand in — this
   is new provider-integration surface (process spawning, JSON-RPC framing, approval routing)
   exactly the kind of code that benefits most from review before merge.
4. **Consider un-stacking #3213** — rebase it to target `main` directly (or merge it
   independently first) so the App Server chain isn't carrying an unrelated CI-policy doc as a
   hard dependency.
5. **Then merge sequentially**, #3207 → #3215, same order as the stack. Since the feature stays
   disabled (`Subprocess` controller type) through all 9 PRs, this is low-risk to land even
   before the follow-up rollout-activation PR exists.
6. **Delete `C:\codex-workspace`** once step 1 is pushed and confirmed — nothing else of value
   lives there.

---

## 5. What this is not

To keep the framing honest: this is **not** yet "AgentMux talking to Codex via App Server."
Every PR in the stack keeps the existing `exec --json` subprocess path active and default.
This stack is the typed protocol client, controller, and lifecycle plumbing for that future
path — well-scoped and disabled-by-default, which is the right way to land something this
size, but it means "getting it functional" is still at least one more (not-yet-opened) PR away
— the one that flips Codex's `controller_type` and adds rollout gating.
