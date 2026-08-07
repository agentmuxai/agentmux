# Report: Audit of AgentMux-Synthesized Text in the Agent Pane

**Date:** 2026-08-06
**Author:** AgentX (agent)
**Status:** Research only — no code changed. This catalogs everything found so a
scope decision can be made before any removal work starts.
**Ask that triggered this:** remove `"Resumed — continuing where you left off"`
and "all the other synthesized text agentmux generates" from the agent pane.

---

## 0. tl;dr

AgentMux injects app-generated (non-CLI) text into the agent pane through
**three structurally different mechanisms**, and they are not equivalent —
a "remove all of it" instruction needs to pick a scope:

1. **Permanent transcript nodes** — text pushed into the actual document/
   transcript array (`documentAtom`) via the same `StreamFlush` reducer
   command real streamed CLI output goes through. Persisted, replayed on
   reload, and **visually indistinguishable from real agent output** once
   scrolled past. `"Resumed — continuing where you left off"` is in this
   category. This is also the exact category a recent commit
   (`6191a1928`, PR #2420) already partially addressed, on record as "per
   direct user request" — see §4 for the precedent and its stated rule.
2. **Ephemeral chrome** — dismissible banners/rows docked in a fixed header
   or footer slot (auth notices, the failure-recovery row, working-state
   labels). These never enter `documentAtom`, don't persist, and disappear
   once their triggering state clears.
3. **A dead channel** — most of the launch flow's status narration
   (`log("cli"|"auth"|"docker"|"install"|"controller"|"agent", …)`) is
   currently filtered out and rendered **nowhere at all** — not the
   transcript, not any visible UI. Two of the exact strings mentioned when
   this task was scoped (`"resuming a controller that's still alive…"` /
   `"previous turn complete…"`) are in this dead category already.

§7 lays out the scope decision this report exists to support.

---

## 1. `notify()` — where `"Resumed — continuing where you left off"` actually goes

`launch-flow.ts:116`: `const notify = opts.onNotify ?? (() => {});` — type
`(text: string, style: "info" | "warning") => void` (`launch-flow.ts:76`).

Wired at `agent-view.tsx:731`: `onNotify: (text, style) => postSystemNotification(text, style)`.

`postSystemNotification` (`agent-view.tsx:709-721`) is **not** a toast or a
banner. It builds a real `MarkdownNode` and dispatches it straight into the
transcript:

```ts
// agent-view.tsx:709
const postSystemNotification = (text: string, style: "info" | "warning" | "success" = "info"): void => {
    const prefix = style === "success" ? "✓ " : style === "warning" ? "⚠ " : "";
    const node: MarkdownNode = {
        type: "markdown",
        id: `system_notification_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`,
        content: `${prefix}${text}`,
    };
    dispatchDocIfRegistered(model.blockId, { type: "StreamFlush", newNodes: [node], updatedNodes: [] });
};
```

`StreamFlush` is the *same* reducer command real streamed CLI content flows
through. Once flushed, this node lives in `documentAtom` exactly like a real
assistant message: persisted to the pane's session snapshot, replayed
identically on the next mount, and rendered with the same markdown styling —
nothing in the data model marks it as app-generated (the
`system_notification_` id prefix isn't rendered anywhere).

**Every current call site:**

| Text (exact / template) | file:line | Trigger |
|---|---|---|
| `Resumed — continuing where you left off` | `launch-flow.ts:415` | mount-time resync found a prior turn (`status` was `"done"` or `"running"`) |
| `Ready — type a message to start` | `launch-flow.ts:418` | mount-time resync found a fresh controller |
| `Something went wrong finishing setup — if the agent doesn't respond, try reopening this pane.` | `launch-flow.ts:412` | the resync call itself threw |
| ``Your ${provider.displayName} login has expired. Click "Log in" to continue.`` | `launch-flow.ts:339` | mount-time, auth expired |
| ``${provider.displayName} needs you to sign in before this agent can start. Click "Log in" to continue.`` | `launch-flow.ts:342` | mount-time, never authenticated |
| `Logged in as **{email}**` / `Login successful` | `agent-view.tsx:728` | any successful login (initial or recovery-flow), fired from four call sites in `hooks/useAgentControllerStatus.ts:739,818,995,1118` |

Every one of these fires **on pane mount or on a login event** — i.e. this is
specifically the "every mount narrates itself into the transcript" behavior,
not something tied to an individual turn.

---

## 2. `log()` — mostly a dead channel today

`LogFn` type: `frontend/app/view/agent/types.ts:675`.

`agent-view.tsx:501-509`:
```ts
const log = (tag: string, text: string, level?: "info" | "error" | "warn") => {
    if (tag !== "system") return;
    appendLog(tag, text, level);
    const write = termWrite();
    if (write) { write(formatLogLine(tag, text, level)); logFlushedCount = logLines().length; }
};
```

This only surfaces anything when `tag === "system"`, and even then it writes
into the **shell-terminal drawer** (a separate UI surface from the
transcript, only visible if the user opens "Shell") — never into the
document. This filter was added deliberately in commit `46cbf9e5c` (#2278);
its own comment (`agent-view.tsx:472-478`) explains why:

> "only 'system'-tagged entries … are genuinely user-initiated console-style
> interactions written into the shell terminal … everything else
> (launch-flow status, auth prompts, CLI resolution, etc.) is passive
> app-internal noise the shell should stay clean of."

**Every `log(tag, …)` call in `launch-flow.ts` uses `"docker"`, `"cli"`,
`"install"`, `"auth"`, `"controller"`, or `"agent"` — none is `"system"`.**
So today, all of the launch flow's status narration (CLI resolution, docker
checks, auth-check progress, controller registration, and specifically the
two strings below) is filtered out and rendered **nowhere in the app**:

- `launch-flow.ts:387`: `"resuming a controller that's still alive — send a message to continue"`
- `launch-flow.ts:388`: `"previous turn complete — send a message to continue"`

These two are effectively dead code from a UI standpoint already — removing
them has zero visible effect. Worth flagging separately from the real
removal work in §7.

`log("system", …)` **is** live, reaching the shell drawer for bang-command
output and slash-command results (`/model`, `/clear`, `/tools`, etc. —
`commands/dispatch.ts`, `commands/global/*.ts`). Those are direct responses
to something the user explicitly typed — a different category from
unsolicited narration, and out of scope for this report.

---

## 3. Everything else found

### 3.1 Permanent transcript nodes (same category as §1)

| Text / template | file:line | Node type | Notes |
|---|---|---|---|
| `⏹ _Interrupted by user_` | `hooks/useTurnLifecycle.ts:118` (pushed `:123`) | `markdown` | Pushed when the user presses Esc/stop — comment calls it "durable confirmation that the stop landed" |
| `` `**stderr:** ${text}` `` | `useAgentStream.ts:390` | `markdown` | Wraps the CLI's own real stderr text with a synthesized label prefix |
| `` `**Error:** ${msg}` `` (falls back to `"Codex turn failed"` / `"unknown error"`) | `providers/codex-translator.ts:57,81,147` | `text`, rendered as ordinary assistant prose | Wraps real CLI error text, but the wrapper becomes indistinguishable from actual assistant output once rendered |
| `` `*Refused: ${block.refusal}*` `` | `providers/codex-translator.ts:108` | `text` | Codex `refusal` content block |
| `` `**Error:** ${msg}` `` (fallback `` `gemini turn ${rawEvent.status}` ``) | `providers/gemini-translator.ts:49` | `text` | Gemini non-success `result` events |
| `"context compacted{ — trigger}"` / `"Earlier history summarized · X → Y tokens{ · took Ns}"` | `virtualization/DocumentRow.tsx:277-282` | `context_compacted` (structured node) | The codebase's own spec (`SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md` §1) calls the heuristic-detected version a **"synthetic context-compacted pane event"** in its own words |
| `"Compacting conversation…"` / `"you ran /compact"` / `"context filled up"` | `virtualization/DocumentRow.tsx:290-298` | `compaction_started` | Live announcement driven by a real backend `PreCompact` signal, but the display strings are AgentMux templates |
| `"HTTP {code}"` / `"Error"` badge + `"Login Again →"` | `virtualization/DocumentRow.tsx:229-256` | `agent_error` | The message text itself passes through **verbatim** from the CLI's own `result` event (`stream-parser.ts:709-716`) — only the badge/button chrome is synthesized, not the error text. Structurally distinct styling from prose, unlike the Codex/Gemini wrappers above |

`useAgentStream.ts:94-141` and `parseHistoryLines.ts:96-141` both build
`context_compacted` nodes — the former live, the latter during history
replay — so this category regenerates on every reload from persisted NDJSON,
not just during a live session.

### 3.2 Ephemeral chrome (never enters the transcript)

Renders in a fixed header/footer slot, separate from `documentAtom`'s node
array — doesn't persist, disappears when the triggering state clears.

- **`authNotice`** (`AgentDocumentView.tsx:189-202`, dismissible) — sourced
  from `hooks/useAgentControllerStatus.ts`, e.g.:
  - `"Your login succeeded, but AgentMux couldn't save the account record. Try again in a moment."`
  - `` `Opened a login page, but no login was detected within 5 minutes. Complete the login there, then click "Login Again".` ``
  - `` `Re-login failed: ${err.message}` ``
  (full list: `useAgentControllerStatus.ts:772,776-779,818,864-867,873-876,879-882,886,995,1003,1008,1118,1126,1142`)
- **`AgentFailure` recovery row** (`failure/failure-accessory.ts`) — a docked
  banner. `title`/`detail` are the backend's real failure classification;
  action-button labels (`"Retry now (Ns)"`, `"Login Again"`, `"New session"`,
  `"Restart"`, etc. — `:104,124-171`) are AgentMux chrome.
- **`AgentFooter.tsx`**: `"Rate limited — retrying in {N}s"` (`:101-103`,
  the string behind the `fix-working-stuck-rate-limit` branch name),
  `"Stopping…"` (`:99`), and the phase labels in
  `flows/launch-phase.ts:79-103` (`"Resolving CLI"`, `"Checking
  authentication"`, `"Sign-in required"`, etc.) — all shown in the
  working-row footer, not the transcript.
- **`"Loading older messages..."`** (`AgentDocumentView.tsx:177`) — header
  slot, shown while paginating history.

### 3.3 Dead channel

Covered in §2 — most of `launch-flow.ts`'s `log()` calls render nowhere
today. No UI change results from touching them.

---

## 4. Precedent: commit `6191a1928` (PR #2420)

This already removed one instance of exactly the same pattern, "per direct
user request." Worth reading in full before scoping the rest of this work.

**What it removed** — a `createEffect` in `agent-view.tsx` that posted
`"Picked up more work — starting another round…"` via `postSystemNotification`
whenever a `Done` → `Streaming` re-promotion happened, plus the file that
computed when to fire it (`frontend/app/view/agent/settled-grace.ts`, 62
lines, deleted wholesale, along with its test).

**The commit message's stated reasoning** (this is the rule the rest of this
report is scoped against):

> "this system injected a permanent markdown node into the pane's own
> conversation transcript (via `postSystemNotification` → `StreamFlush`)
> whenever [a re-promotion] happened … The intent … was to explain an
> otherwise-confusing checkmark reversal instead of letting it happen
> silently — **but the mechanism worked by writing synthetic text directly
> into the real conversation history, which the user does not want: no
> artificial messages mixed into transcript content, even well-intentioned
> explanatory ones.**"

**What it explicitly did NOT touch, and why:** "the underlying
`StreamFlushObserved` re-promotion logic itself (`reducer.ts`) — that's a
separate, necessary correctness mechanism for genuine multi-round
continuations, unrelated to whether a notification is shown about it."

In other words: the commit's philosophy separates *state-machine
correctness* (kept) from *narrating that state machine into the transcript*
(removed) — a direct template for removing `notify()`'s remaining 6 strings
and the `⏹ _Interrupted by user_` node without touching `TurnPhase`,
`FailureObserved`, or compaction-detection logic itself.

**One unresolved tension this commit doesn't settle:** `launch-flow.ts`'s own
header comment cites `docs/specs/SPEC_AGENT_PANE_AUTH_NOTIFICATIONS_2026_07_26.md`
as the design basis for `notify()`. That spec's stated goal **G2** is
*"Every mount narrates itself... posts a minimal, permanent record into the
conversation"* — i.e. the transcript-injection behavior is a **documented,
intentional design goal** for the mount-time notify calls specifically, not
an oversight. Removing `"Resumed…"` / `"Ready…"` / the auth-expired strings
means reversing G2 for the whole mount flow, not just fixing one more
accidental case like `6191a1928` did. Worth being explicit about that before
starting, since it's a deliberate reversal of a written design decision, not
a bug fix.

Also relevant: `docs/specs/SPEC_WORKING_STATE_AND_SCROLL_FOLLOW_HARDENING_2026_07_27.md`
§4, which introduced the now-removed "Picked up more work" notification, had
already hedged its own wording as provisional: *"500ms and the exact
notification wording are implementation defaults, not confirmed product
decisions — flagged as reversible/tunable if they don't feel right in
practice."* That hedge turned out to be prescient.

No dedicated "when is a synthetic transcript message acceptable" design
document exists anywhere in `docs/specs/` or `docs/retro/` — the rule
currently lives only in `6191a1928`'s commit message and that one hedge.
That's a documentation gap worth closing once this round's scope is settled,
so the next person doesn't have to re-derive the rule from git archaeology.

---

## 5. Why some of this exists (so removal doesn't regress real fixes)

Several of the Category-A items were added in response to specific,
documented user-facing confusion, not just as a nice-to-have:

- `docs/specs/SPEC_CONTEXT_COMPACTION_NOTIFICATION_2026_06_20.md` and
  `SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md` justify the
  compaction banners as fixing a **silent transcript discontinuity** — users
  reported the agent's context "resetting" with no visible marker at all.
- `docs/specs/SPEC_AGENT_ERROR_FRAMEWORK_2026_06_20.md` justifies the durable
  `agent_error` node via a real incident: an agent silently returning 401 on
  every message with no visible sign anything was wrong.
- `SPEC_AGENT_PANE_AUTH_NOTIFICATIONS_2026_07_26.md`'s G1 ("no silent
  login") is the stated motivation for the auth-expired/first-login
  `notify()` strings — without them, a pane can sit doing nothing with no
  visible explanation of why.

None of this means these specific items should be kept — it means a removal
pass should have an answer ready for "how will the user know X happened
instead?" for each one, the same way `6191a1928` implicitly did (the
checkmark-reversal confusion it accepted as a tradeoff was judged less bad
than injecting fake transcript content).

---

## 6. Full catalog, one table

| # | Category | Text/template | file:line | Persists in transcript? |
|---|---|---|---|---|
| 1 | A | `Resumed — continuing where you left off` | `launch-flow.ts:415` | Yes |
| 2 | A | `Ready — type a message to start` | `launch-flow.ts:418` | Yes |
| 3 | A | `Something went wrong finishing setup…` | `launch-flow.ts:412` | Yes |
| 4 | A | Auth-expired / first-login prompts | `launch-flow.ts:339,342` | Yes |
| 5 | A | `Logged in as **{email}**` / `Login successful` | `agent-view.tsx:728` | Yes |
| 6 | A | `⏹ _Interrupted by user_` | `useTurnLifecycle.ts:118` | Yes |
| 7 | A (wrapper only) | `**stderr:** …` | `useAgentStream.ts:390` | Yes |
| 8 | A (wrapper only) | `**Error:** …` / `*Refused: …*` | `codex-translator.ts:57,81,108,147` | Yes |
| 9 | A (wrapper only) | `**Error:** …` | `gemini-translator.ts:49` | Yes |
| 10 | A (labels only, real data underneath) | Compaction banner copy | `DocumentRow.tsx:277-298` | Yes |
| 11 | A (chrome only, real text underneath) | `agent_error` badge/button | `DocumentRow.tsx:229-256` | Yes |
| 12 | B | `authNotice` strings | `useAgentControllerStatus.ts` (multiple) | No |
| 13 | B | `AgentFailure` row action labels | `failure-accessory.ts` | No |
| 14 | B | Rate-limit / stopping / phase labels | `AgentFooter.tsx`, `launch-phase.ts` | No |
| 15 | B | `Loading older messages...` | `AgentDocumentView.tsx:177` | No |
| 16 | C | `log("cli"\|"auth"\|"docker"\|"install"\|"controller"\|"agent", …)` — dead, renders nowhere | `launch-flow.ts` (multiple) | No (nowhere) |

---

## 7. Scope decision needed before removal work starts

**Option 1 — narrow, matches the literal precedent.** Remove only the items
that get mixed into the persisted transcript and are indistinguishable from
real agent output once rendered: #1–#6 outright, and for #7–#11, strip the
synthesized wrapper/label text while keeping the real underlying CLI content
visible (e.g. stderr still shows, just without AgentMux's own `**stderr:**`
prefix; `agent_error` keeps showing the CLI's real message, loses only the
synthesized badge chrome — though the badge itself is arguably useful UI, not
"fake text," so may not need touching at all). This is the direct extension
of what `6191a1928` already did and what its commit message argues for.

**Option 2 — broad.** Also remove/simplify the Category B ephemeral chrome
(#12–#15). This goes further than any existing precedent or spec — these
never touch the transcript, are dismissible, and several exist specifically
to fix documented "agent went silent with no explanation" bugs (auth
timeouts, rate limiting). Doing this without a replacement plan for "how does
the user know why nothing is happening" risks reintroducing the bugs
`SPEC_AGENT_PANE_AUTH_NOTIFICATIONS_2026_07_26.md` and the rate-limit fix
were written to close.

Category C (#16) is free either way — it's dead code with no UI effect, safe
to delete or leave regardless of which option is chosen.

**Recommendation:** start with Option 1 — it's unambiguously what "get rid of
synthesized text mixed into the transcript" means, has a clean precedent to
follow, and is low-risk. Revisit Option 2 item-by-item only if the narrow
pass doesn't satisfy the underlying complaint, since each Category B item has
a specific documented bug behind it that a removal would need to either
accept as a regression or solve differently.

## 8. Suggested next steps (not started — pending scope confirmation)

1. Confirm scope (Option 1 vs. 2, or a specific subset) with the user.
2. For the confirmed set, remove the same way `6191a1928` did: delete the
   `notify()`/synthesized-node call, leave the underlying state/logic
   (`TurnPhase`, `reducer.ts`, compaction detection, etc.) untouched.
3. Delete the dead `log()` calls in `launch-flow.ts` (§2/§3.3) regardless of
   the Option 1/2 decision — no UI depends on them.
4. Write the missing design doc this report's §4 flags as absent: a short,
   general "synthetic transcript message" policy, so the rule doesn't have
   to be re-derived from commit archaeology next time this comes up.
5. Add a regression test/lint if feasible — e.g. a check that
   `postSystemNotification`/`StreamFlush` call sites are limited to an
   allowlist, so a future PR can't silently reintroduce this pattern the way
   the settled-grace one did.
