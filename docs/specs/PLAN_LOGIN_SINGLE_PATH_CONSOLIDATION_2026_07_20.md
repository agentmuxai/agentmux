# Plan — collapse every provider-login code path onto one

**Date:** 2026-07-20
**Status:** proposed, not started. §7's open questions are now answered —
see the update at the bottom of §5 phase 3 and the new §7 for what changed.
**Context:** `docs/retro/retro-headless-login-browser-open-2026-07-20.md` and
`docs/retro/retro-login-three-code-paths-2026-07-20.md`. Those two retros
fixed three call sites (`/login`, "Login Again", the gated launch flow behind
"Retry Login") by routing them through a new orchestrator,
`runProviderLogin` (`frontend/app/view/agent/flows/run-provider-login.ts`).
A full audit today found that orchestrator is not actually the only login
implementation in the app — it's the third. This plan proposes closing the
rest.

---

## 1. Current state (as of this audit — see inventory for full citations)

Four independent implementations of "spawn a provider CLI, get it
authenticated" exist today:

| # | Implementation | Where | Reaches `runProviderLogin`? |
|---|---|---|---|
| A | `runProviderLogin` (3-tier: URL-capture → global-seed → terminal+poll) | `flows/run-provider-login.ts` | — (this is the shared core) |
| B | `useGlobalLogin()` — "Use existing login" button | `hooks/useAgentControllerStatus.ts:355-395` | **No** — calls `seedGlobalLogin` directly, reimplementing tier 2 |
| C | `loginViaTerminal()` — "Login via terminal" button | `hooks/useAgentControllerStatus.ts:399-456` | **No** — calls `openLoginTerminal` + `pollForGlobalLoginSeed` directly, reimplementing tier 3 |
| D | `PreLaunchAuthPanel` / `AuthFlowController` — "Connect"/"Reconnect" in the New Agent launch modal | `components/PreLaunchAuthPanel.tsx`, `auth/auth-flow-controller.ts` | **No** — an entirely separate implementation, backed by different RPCs (`auth.start`/`auth.poll`/`auth.cancel`/`auth.submitcallback` in `agentmux-srv`, not the CEF host's `run_cli_login`). Its own "Use my existing login" button (`:177-202`) also calls `seedGlobalLogin` directly — a third direct caller of that function. |

Plus two smaller, structurally distinct pieces that aren't duplicates but
are part of the same surface area:

- **`AuthUrlBox`'s paste-code submit** (`components/AgentDocumentView.tsx:237-256`)
  calls `getApi().setProviderAuth(...)` directly — delivers a manually
  pasted code to tier 1's in-flight CLI stdin, or writes to
  `provider_config.json` as a fallback. Not a duplicate of any tier; a
  complement to tier 1 specifically.
- **Three CEF host IPC commands with zero current frontend callers**:
  `clear_provider_auth`, `get_provider_auth_status`, `check_cli_auth_status`
  (all in `agentmux-cef/src/commands/providers.rs`). Dead-code candidates,
  not duplicates.

## 2. A correctness bug found while auditing, not just a duplication

`runProviderLogin` (tier 1 → tier 2 → tier 3) never cancels tier 1's PTY
child before starting tier 2 or 3. Confirmed from the live "Marks" repro's
host log during today's testing: `run_cli_login_pty` spawned at `T`,
timed out at `T+15s` (the IPC call correctly resolved `null` at that point),
but the **child process itself kept running** — `cancel_cli_login: PTY
child killed` / `child exited` didn't appear until a caller explicitly
called `cancelCliLogin()` much later. In the current code, that call only
happens *after* `runProviderLogin` fully returns (in `relogin()` and
`launch-flow.ts`, post-outcome) — meaning tiers 2 and 3 now run for up to
several more minutes with tier 1's abandoned child still alive in the
background. Best case this is a wasted process; worst case a lingering
`claude auth login` process interferes with tier 3 spawning a *second*
concurrent login attempt for the same CLI (untested, but plausible — same
CLI, same config dir, two live login sessions). This should be fixed
regardless of how much of the rest of this plan ships.

## 3. Goals

1. **One function that knows how to log a provider in.** Every UI action
   that starts a login — of any kind, from any screen — calls
   `runProviderLogin` (or a thin, explicit variant of it), never a raw tier
   directly.
2. **Explicit tier selection, not reimplementation.** "Use existing login"
   and "Login via terminal" are legitimate *user intents* ("skip straight to
   X") — the fix is to make `runProviderLogin` accept a starting tier, not
   to force every button through the full 1→2→3 chain or to keep three
   independently-maintained copies of the underlying logic.
3. **Fix the tier-1-child-leak bug (§2) as part of the consolidation**, not
   as an afterthought — it's naturally fixed once there's one orchestrator
   responsible for the whole lifecycle.
4. **Decide, explicitly, what to do about the pre-launch flow (D)** — this
   is the biggest single duplication (a whole parallel backend
   implementation) and the riskiest to touch. This plan scopes it as a
   separate phase with its own go/no-go, not something to merge blind.

## 4. Non-goals

- Not merging the five CEF host IPC *mechanisms* (`run_cli_login`,
  `open_login_terminal`, `seed_provider_auth_from_global`, `set_provider_auth`,
  `cancel_cli_login`) into fewer host commands. They do structurally
  different things (pipe/PTY-scrape vs. new-console-spawn vs. file-copy vs.
  stdin-delivery). The duplication worth removing is in the *frontend
  orchestration* above them, not the primitives themselves.
- Not redesigning the account/identity data model (`compute_and_ensure_account_dir`,
  per-account isolated dirs) — out of scope here; flagged only where it
  affects whether phase 3 (below) is safe to attempt.
- ~~Not deciding the UX question of *which buttons the failure banner
  shows*~~ **Decided 2026-07-20 (see §7): one button.** The failure banner
  collapses to a single login action; "Use existing login" and "Login via
  terminal" as separate user-facing buttons go away once phase 2 ships.

## 5. Phased plan

### Phase 1 — fix the tier-1 leak (§2), independent of everything else

Add an explicit `getApi().cancelCliLogin()` call inside `runProviderLogin`
right after tier 1 resolves with `"no-url"`, before tier 2 starts. Low risk,
no behavior change to any caller's outcome contract, closes a real resource
leak. Ship this alone first — it doesn't depend on any other phase.

**Files:** `flows/run-provider-login.ts`.
**Tests:** extend `run-provider-login.test.ts` to assert `cancelCliLogin` is
called between tier 1 failing and tier 2 starting.

### Phase 2 — fold B and C into A

Give `runProviderLogin` an optional `startAtTier?: 1 | 2 | 3` param
(default `1`). "Use existing login" calls it with `startAtTier: 2`; "Login
via terminal" calls it with `startAtTier: 3`. Delete `useGlobalLogin()` and
`loginViaTerminal()`'s independent bodies in `useAgentControllerStatus.ts`,
replacing them with thin wrappers that call `runProviderLogin` with the
right `startAtTier` and map the outcome to the existing `authNotice`/
`onRecovered` behavior — i.e. keep the two buttons and their current labels
and behavior from the user's perspective; change only what's underneath
them. `seedGlobalLogin`/`pollForGlobalLoginSeed` stay as exported building
blocks (`runProviderLogin` still calls them) but stop being called directly
from `useAgentControllerStatus.ts`.

**Files:** `flows/run-provider-login.ts` (add `startAtTier`),
`hooks/useAgentControllerStatus.ts` (`useGlobalLogin`, `loginViaTerminal`).
**Tests:** update/extend `run-provider-login.test.ts` for `startAtTier`;
existing `useAgentControllerStatus` behavior should be covered by whatever
already exercises `useGlobalLogin`/`loginViaTerminal` today (check current
coverage before deleting the bodies — if none exists, add it here, don't
delete blind).
**Follow-on — decided 2026-07-20, do this as part of phase 2, not a later
step:** collapse the failure banner to a single login button. Once B and C
are thin wrappers around `runProviderLogin`, there's no remaining reason for
"Login Again" / "Use existing login" / "Login via terminal" to be three
separate user choices — the single button calls `runProviderLogin` with the
default `startAtTier: 1` and the existing automatic 1→2→3 fallback handles
what the three buttons used to require the user to pick manually. Delete the
two extra buttons from `failure-accessory.ts`'s `"auth"` case (`:118-125`)
and the per-message inline banner (`DocumentRow.tsx`) stays as-is (it
already only has one "Login Again →" action). `useGlobalLogin`/
`loginViaTerminal` as *functions* may still be worth keeping internally
(e.g. if phase 3 or a future advanced/debug affordance wants
`startAtTier` control) but should have no directly-wired UI button once this
ships.

### Phase 3 — the pre-launch flow (D): investigate before merging

`PreLaunchAuthPanel`/`AuthFlowController` is architecturally separate for a
possibly-legitimate reason: it runs *before* an agent exists, so it needs
`compute_and_ensure_account_dir` to mint a brand-new per-account identity
dir — `runProviderLogin` today always operates on an *already-resolved*
`authEnv` for an *already-existing* agent/pane. Merging them isn't a
find-and-replace; it's a design decision about whether account-dir minting
belongs inside `runProviderLogin` (parameterized) or should stay a distinct
pre-step that hands `runProviderLogin` a resolved `authEnv` afterward — the
latter seems more promising and lower-risk (minting stays where it is;
`PreLaunchAuthPanel`'s Connect/Reconnect/"Use existing login" buttons call
`runProviderLogin` with the freshly-minted `authEnv` instead of driving
`AuthFlowController`'s own backend RPCs).

**Spike item 1 is now CONFIRMED, not speculative** — the user independently
reproduced "the login browser doesn't appear to work when creating a new
agent either" (2026-07-20), and the code backs it up:
`agentmux-srv/src/server/identity_handlers.rs:405-409` sets
`CREATE_NO_WINDOW` on `auth.spawn`'s pipe-path child, exactly the pattern
`retro-headless-login-browser-open-2026-07-20` diagnosed on the CEF host
side (`agentmux-cef`'s `run_cli_login`). The PTY branch (`requires_tty`,
`:354-373`, delegating to `spawn_auth_cli_pty`) is the same headless shape.
**There is no equivalent to `open_login_terminal` anywhere in
`identity_handlers.rs`** — grepped for `CREATE_NEW_CONSOLE`/
`open_login_terminal`, zero hits. So the New Agent modal's "Connect" button
has no working fallback at all today, not even the manual "Login via
terminal"-equivalent escape hatch the pane-level flow has — it's strictly
worse off than "Retry Login" was before this morning's fix.

This elevates phase 3 from "investigate, maybe later" to "there is a live,
confirmed, currently-unfixed bug here" — but the FIX is still gated on the
same architectural question as before (does account-dir minting move inside
`runProviderLogin`, or does `PreLaunchAuthPanel` mint first and then hand
off to it?), which spike item 2 below still needs answering before writing
code. **Interim stopgap, decided 2026-07-20 (see §7): rather than block the
New Agent flow on the full merge, do NOT try to show/resolve identity info
at agent-creation time beyond what already works — leave the
not-yet-resolved parts of the launch modal's identity display blank rather
than showing something. Ship the real fix (routing `PreLaunchAuthPanel`
through the same terminal-fallback capability `runProviderLogin` already
has) as its own follow-up once spike item 2 is answered — don't fold it
into phases 1/2's rollout.**

Spike item 2, still open:
**What does `auth.poll`/the `AuthFlowController` state machine actually
need from the login process that `runProviderLogin`'s simpler
opened/seeded/terminal-success/terminal-timeout/terminal-unavailable
outcome enum doesn't provide?** (e.g. intermediate "waiting"/"exchanging
code" states the panel renders.) If the state needs are materially
richer, `runProviderLogin`'s outcome type may need to grow, not just be
reused as-is.

**Do not schedule full-merge implementation for this phase until spike item
2 is done** — but the minimal terminal-fallback fix (giving `auth.spawn` an
`open_login_terminal`-equivalent, independent of whether it ever shares code
with `runProviderLogin`) is small enough to ship on its own once someone
picks it up, without waiting on the merge design.

### Phase 4 — cleanup

- Delete `clear_provider_auth`, `get_provider_auth_status`,
  `check_cli_auth_status` (CEF host commands) if the phase-1/2/3 work
  confirms they're still unused — re-grep at that point, don't rely on
  today's snapshot.
- Wire an actual Cancel button into the pane-level failure banner —
  `cancelLogin()` exists (`useAgentControllerStatus.ts:458-462`) and is
  fully implemented but has no UI caller today. A pending login (especially
  tier 3's up-to-5-minute wait) with no visible way to cancel from the pane
  is a real gap independent of everything else in this plan.
- Once phase 2 ships, add a repo-topology test in the spirit of
  `run-cli-login-single-caller.test.ts` for `seedGlobalLogin` /
  `getApi().openLoginTerminal` — assert they're each called from exactly one
  place (`run-provider-login.ts`), the same enforcement pattern that caught
  nothing catching B/C's duplication until this audit.

## 6. Sequencing recommendation

Ship phase 1 immediately (small, self-contained, fixes a real bug). Ship
phase 2 next (moderate size, no product-visible behavior change, directly
answers "single path" for every pane-level surface). Treat phase 3 as its
own follow-up spec after the research spike — don't bundle it with 1/2.
Phase 4's cleanup items can ride along with whichever of 1-3 touches the
same files, or ship standalone.

## 7. Decisions (2026-07-20)

- **Failure banner → one button.** Resolves the former open question: the
  three-button banner ("Login Again" / "Use existing login" / "Login via
  terminal") collapses to a single action as part of phase 2. See the
  updated phase 2 "Follow-on" above.
- **Phase 3's risk is confirmed, not hypothetical.** The user independently
  hit the New Agent modal's login not working, matching spike item 1's
  prediction exactly. Interim stopgap decided: don't try to resolve/display
  identity info at agent-creation time beyond what already works — leave it
  blank rather than show something wrong — and ship the real terminal-
  fallback fix as a follow-up once spike item 2 (state-machine needs) is
  answered, not bundled into phases 1/2.

### Superseded same-day — the "ambient creds" relaxation below was reverted

**Correction (2026-07-22, reagent P2 on #2255):** the section below describes
a same-day fix that was real and did ship — briefly. Later the same session,
the policy changed: "the claude agent should not work if there is no armory
account for claude... same for all providers... close it everywhere, now."
`identity/resolver.rs`'s layer-3 spawn gate's `use_ambient_login` escape
hatch was removed entirely (unconditional block on a missing bound account,
no per-agent opt-out), and `AgentLaunchModal.tsx`'s `authBlocksLaunch()`/
`canSubmit()` were reverted back to requiring `accountId() !== ""` for every
oauth-class provider — see the `2026-07-20: reverted...` comment directly
above `authBlocksLaunch()` in that file for the actual, current reasoning.
**The "leaving Identity blank is now fully supported" claim below is
therefore false as of the code actually shipped in commit 721f9c5 (the same
commit this plan doc itself landed in) — reagent caught the doc and the code
contradicting each other in the same commit.** Left the original text
below, struck through in spirit if not in markdown, as a record of what was
tried and why it didn't stick — not as current guidance. Do not implement
against this section; read the code's own comment instead.

The user clarified: they want the New Agent create screen to stay simple —
create the session and let it authenticate with vanilla/ambient settings,
consistent with how every other pane already works, and defer the richer
per-account "Connect" UX to a later Armory-integrated redesign.

This turned out to be directly achievable without waiting on phase 3's
merge design, because the code already half-intended it and just didn't
follow through: `AgentLaunchModal.tsx`'s `authBlocksLaunch()` comment
literally said *"No account selected — ambient creds, OAuth flow runs
once"* — but the code treated `accountId() === ""` as a **blocking**
condition anyway, and `canSubmit()` independently hard-required a non-empty
`accountId()`. The "Continue an existing agent" dropdown a few lines away
already renders `"(ambient creds)"` for exactly this state
(`:623`), confirming "no account = ambient" was already a first-class,
recognized state elsewhere in the same file — just not honored at the
launch gate.

**Fix shipped, then reverted:** `authBlocksLaunch()` briefly only blocked
when an account was *explicitly selected* but doesn't supply the provider —
not merely because none was picked, and `canSubmit()` briefly dropped its
`accountId() !== ""` requirement. **This was reverted later the same
session** once the policy shifted to unconditional enforcement (see the
correction note above) — both functions require a bound account again,
for every oauth-class provider, no exception.

Phase 3 (merging `PreLaunchAuthPanel`'s OAuth-Connect path itself, for
users who explicitly want a dedicated bound account) is unaffected by this
and still gated on its own spike item 2.
