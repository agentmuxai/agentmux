# SPEC — Promote Mode to the composer strip; retire the nested "Controls" panel under Log

**Date:** 2026-07-02
**Type:** Implementation spec
**Status:** Implemented on branch `agenta/composer-strip-mode-toplevel` (Fix 1, 2, 3=Option A, 5, 6 +
tooltips). Deferred: Fix 4's *deeper* launch-time seeding (the user-visible symptom is resolved by Fix 1
— see note below); live repro of Fix 5 (applied defensively per user direction, not yet confirmed running).
**Owner:** asaf
**Scope:** agent pane composer strip + control bar (frontend only, `frontend/app/view/agent`).

> Follow-up to `SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG_2026_07_02.md` Part C, which named this exact
> nesting problem and left it as an open decision ("(a) leave `AgentControlBar` alongside the log" vs
> "(b) split them"). (a) shipped; the user has now confirmed they want (b), plus promoting **Mode** to
> the strip (which didn't exist as an editable top-level control before). See
> `docs/retro/retro-composer-strip-log-controls-nesting-2026-07-02.md` for the full root-cause writeup.

---

## 1. Current state (the bug)

`AgentComposerStrip.tsx` (the row directly above the textarea) already has top-level Model and Effort
`<select>`s plus the **Log** button (`:213-250`). Mode only exists there as a **read-only colored pill**
(`:269-277`) — no way to change it from the strip.

Clicking **Log** opens `.agent-composer-details` (`agent-view.tsx:1005-1016`), which mounts:
```tsx
<ActivityLogPanel entries={logLines} />
<AgentControlBar blockId={...} blockAtom={block} providerId={...} />
```
`AgentControlBar` (`AgentControlBar.tsx:60-61,257-273`) is its own self-collapsing block with a
"▸/▾ Controls" chevron. Expanding it reveals **editable** Mode/Model/Effort selects
(`:274-320`) — Model and Effort duplicate the strip; Mode is the *only* place to change it. It also
renders session-management banners and Archive/Export/Restore buttons (`:199-256`), unconditionally
whenever relevant, ahead of the chevron.

Net effect: clicking Log surfaces log entries **and** a second nested expansion that mixes "runtime
controls" with "session management" behind one summary row.

## 2. Desired end state

- **Log** shows only log entries. No second expansion, no "Controls" summary row.
- **Mode, Model, Effort** are all dropdowns at the strip's top level, next to the Log button — matching
  where Model/Effort already are today, with Mode added alongside them.
- Session management (Archive/Export/Restore + the interrupted/large-session/archived banners) keeps
  working, relocated out of the removed Controls chevron (see Fix 3 — this needs a decision, not just a
  deletion).

---

## 3. Fix 1 — Add an editable Mode dropdown to `AgentComposerStrip.tsx`

Add a third `<select>` in `agent-composer-strip-controls`, before Model (or after — order is a taste
call; recommend Mode → Model → Effort → Log, left to right, matching read order of "how it'll run" →
"which model" → "how hard it'll think" → "show me what happened"):

```tsx
<Show when={showControls()}>
    <select
        class="agent-composer-strip-select agent-composer-strip-select--mode"
        title="Permission mode"
        value={runtime()?.permissionMode}
        style={{ "border-left": `3px solid ${PERMISSION_COLORS[runtime()?.permissionMode ?? "default"]}` }}
        onChange={(e) => void updateRuntime({ permissionMode: e.currentTarget.value as PermissionMode })}
    >
        <option value="bypass">Bypass (no prompts)</option>
        <option value="auto">Auto (AI classifier)</option>
        <option value="acceptEdits">Accept Edits</option>
        <option value="plan">Plan (read-only)</option>
        <option value="default">Default (prompt all)</option>
    </select>
    <select class="agent-composer-strip-select" title="Model" ...>{/* unchanged */}</select>
    <select class="agent-composer-strip-select" title="Effort" ...>{/* unchanged */}</select>
</Show>
```

Reuses the exact option list and color mapping already defined in `AgentControlBar.tsx:28-42,284-288` —
`PERMISSION_LABELS`/`PERMISSION_COLORS` are currently duplicated verbatim between
`AgentComposerStrip.tsx:27-41` and `AgentControlBar.tsx:28-42`; once `AgentControlBar`'s copy is deleted
(Fix 2), the strip's copy becomes the single source and doesn't need to change shape.

`updateRuntime`'s patch type (`AgentComposerStrip.tsx:172`) needs `permissionMode?: PermissionMode`
added alongside the existing `model`/`effort` fields — it already spreads `{ ...r, ...patch }` into
`AgentRuntimeConfig`, so this is a type-signature change only, no logic change.

**Remove the read-only permission pill** (`:269-277`, `showPermissionPill()` at `:163-166`) — it becomes
redundant once Mode is an editable dropdown sitting in the same strip; keeping both would show the same
information twice in two different widgets.

**Gating:** keep behind `showControls()` (Claude-only, `:189`) for now, consistent with Model/Effort —
`permissionMode` is read from `agent:runtime` meta the same way regardless of provider, but broadening
the gate is out of scope here (tracked separately in the Part B spec's open question about dropping the
Claude-only gate for providers that define `models`).

## 4. Fix 2 — Stop rendering `AgentControlBar`'s Mode/Model/Effort block under Log

In `AgentControlBar.tsx`:
- Delete the `expanded`/`setExpanded` signal (`:61`) and the `agent-control-bar-header` chevron block
  (`:257-273`).
- Delete the `agent-control-bar-body` div and its three `agent-control-row`s for Mode/Model/Effort
  (`:274-320`), and the now-unused `compactSummary`/`isNonDefault` helpers (`:68-102`) if nothing else
  reads them after Fix 3 lands.
- `PERMISSION_LABELS`, `PERMISSION_COLORS`, `MODEL_LABELS`, `EFFORT_LABELS` (`:28-58`) become dead here
  once the body is gone — delete.

What's left in `AgentControlBar` after this fix is exactly the session-management pieces (§5).

In `agent-view.tsx:1005-1016`, `<ActivityLogPanel>` no longer needs a sibling that duplicates the strip.
See Fix 3 for what (if anything) replaces `<AgentControlBar>` there.

## 5. Fix 3 — Where does session management live now? (needs a decision)

Once Fix 2 strips `AgentControlBar` down to just the interrupted-session banner, large-session banner,
archived banner, and Archive/Export/Restore buttons (`AgentControlBar.tsx:199-256`), it's no longer a
"Controls" panel — it's purely session lifecycle UI. Two options:

**Option A (recommended, smallest diff) — keep it inside `.agent-composer-details`, unconditionally
rendered.** `<ActivityLogPanel>` and the (renamed) session panel both stay mounted when Log is open, but
neither has an inner expand/collapse — the "extra expansion" the user is reporting goes away because
there's no second chevron to click, even though session UI is still physically inside the region Log
toggles. This satisfies "no second expansion under Log" but not the literal "only logs under Log," since
archive/export banners would still appear alongside entries when they're relevant (they already only
render conditionally — `Show when={wasInterrupted()}`, `isLargeSession()`, `isArchived()` — so on a
normal session with no banners, Log truly does show only entries today's ask cares about).

**Option B (literal reading, larger diff) — give session management its own top-level affordance.**
Add a small icon/button in the strip's right zone (near the process badge) that opens session
management on its own, independent of Log. Keeps Log 100% log-only under all conditions, including when
a banner is active, at the cost of one more control in an already-dense strip.

**Recommendation: Option A.** The user's complaint is specifically about Mode/Model/Effort being
duplicated and buried behind a second click — that's what Fix 1+2 resolve. Session banners are
conditional (invisible in the common case) and not part of the reported complaint; Option B adds strip
real estate for a rarely-visible feature. Flag this recommendation back to the user before implementing
Fix 3 rather than defaulting silently — that's exactly the miss that caused this retro (see
retro §3, "Primary — option (a) was picked without being confirmed with the user").

If Option A is confirmed, rename `AgentControlBar` → something reflecting its new scope (e.g.
`AgentSessionPanel`) so the name doesn't keep implying "runtime controls" now that it holds none.

---

## 5b. Fix 4 — Seed `agent:runtime` at launch so the Mode indicator is correct on first load (Bug 1)

**Symptom (user-reported):** a fresh Claude pane runs in bypass (`--dangerously-skip-permissions`), but
the Mode/Bypass indicator does **not** show on first load. It only appears after the user changes a
dropdown.

**Root cause (confirmed by source trace):**
- The actual process is *not* launched with bypass baked in — `persistentLaunchArgs`
  (`providers/index.ts:179`) uses `--permission-mode default`. Bypass is injected later, on the **first
  message send**, when `deliverToBackend` rebuilds `cmd:args` via `buildRuntimeArgs` and falls back to
  `DEFAULT_RUNTIME_CONFIG.permissionMode = "bypass"` (`useAgentCommands.ts:~405-414`, `types.ts:637`).
- `block.meta["agent:runtime"]` is **never seeded at launch** — the only writer is `applyRuntimeChange`
  (`runtime-apply.ts:42-45`), called from the dropdowns / `/model`. So immediately after launch the key
  is `undefined`.
- The strip's Mode indicator reads that key **with no fallback** (`agent-view.tsx:990-993`:
  `block()?.meta?.["agent:runtime"]?.permissionMode`), unlike `runtime()` inside the strip which *does*
  fall back to `DEFAULT_RUNTIME_CONFIG` (`AgentComposerStrip.tsx:169-170` → `getRuntimeConfig`,
  `buildRuntimeArgs.ts:134-142`). So on first paint the indicator has `undefined` → hidden.
- Changing any dropdown calls `updateRuntime`, which reads `runtime()` (fallback = bypass) and persists
  the merged object — seeding `agent:runtime.permissionMode = "bypass"` as a **side effect**. Now the
  indicator has a value and shows "Bypass". That's the "only after I change the model" behavior.

**Fix (either is sufficient; recommend both):**
1. **Make the indicator read through the same fallback the rest of the strip uses.** In `agent-view.tsx`,
   derive `permissionMode` via `getRuntimeConfig(block()?.meta).permissionMode` instead of the raw
   `block()?.meta?.["agent:runtime"]?.permissionMode`. This makes the indicator reflect the *effective*
   config (which is what actually runs) from first paint. Once Fix 1 promotes Mode to an editable
   `<select>` bound to `runtime()?.permissionMode`, this is automatic — the select already uses the
   fallback path — so **Fix 1 alone largely resolves Bug 1's visible symptom.**
2. **Seed `agent:runtime` at launch** (the deeper fix). When a Claude agent is launched
   (`agent-model.ts:226-229` / `:365-368`), write `DEFAULT_RUNTIME_CONFIG` (or the launch-derived config)
   into `agent:runtime` meta at the same time as the other launch meta, so the persisted config and the
   actually-running flags agree from t=0 and no code path has to lazily infer bypass on first send.
   Removes the split between "what `cmd:args` bypass fallback does" and "what the UI meta says."

## 5c. Fix 5 — "Changing Effort changes the Model" (Bug 2) — needs live confirmation before implementing

**Symptom (user-reported):** changing the Effort dropdown also changes what the Model dropdown shows.

**Static analysis result:** there is **no code path that reassigns `model` when only `effort` changes.**
Verified: the Effort `onChange` patches only `{effort}` (`AgentComposerStrip.tsx:234`,
`AgentControlBar.tsx:311`); both `updateRuntime`s read `runtime()` fresh and merge `{...r, ...patch}`,
preserving `r.model`; `applyRuntimeChange` (`runtime-apply.ts:36-60`) and `buildRuntimeArgs`
(`buildRuntimeArgs.ts:107-112`) contain no effort→model coupling (the `--effort` flag is only *omitted*
for Haiku; it never changes the model). So this is **not** a logic swap.

**Most likely real mechanism (unconfirmed — requires repro):** a SolidJS controlled-`<select>` binding
artifact, made visible by a default mismatch:
- The registry marks **Opus** `default: true` (`providers/index.ts:192`) but the runtime default model is
  **Sonnet** (`DEFAULT_RUNTIME_CONFIG.model = "sonnet"`, `types.ts:640`).
- Both selects bind with plain `value={runtime()?.model}` / `value={runtime()?.effort}`, **not**
  `prop:value` (`AgentComposerStrip.tsx:219,233`; `AgentControlBar.tsx:295,310`).
- Hypothesis: on first paint the Model `<select>` shows the *first* option (Opus 4.8) rather than the
  state value (Sonnet); the Effort change triggers a reactive re-render that re-asserts
  `value="sonnet"`, flipping the display Opus→Sonnet. The user reads that as "effort changed the model."
  This ties Bug 2 to the same first-paint desync as Bug 1.

**Proposed fix (low-risk, defensive — apply regardless of exact cause):**
1. Bind both selects with `prop:value` (or render options via `<For>` with an explicit
   `selected={o.value === runtime()?.model}` on each `<option>`) so the controlled value is always
   asserted as a DOM property after options mount and on every reactive update — the SolidJS-recommended
   pattern for dynamically-optioned selects.
2. Reconcile the default mismatch: either make `DEFAULT_RUNTIME_CONFIG.model` match the registry's
   `default: true` entry, or drop `default: true` from Opus so there's one source of truth for "the
   default model." A mismatch here is a latent trap for any code that reads `models.find(m => m.default)`
   (e.g. the Codex fallback at `buildRuntimeArgs.ts:122`).

**Before implementing:** reproduce live (open a Claude pane, change Effort, watch the Model select) to
confirm the `prop:value` artifact is the actual cause vs. a genuine swap under process-restart. Deferred
here due to current machine memory pressure — the fix is cheap once confirmed, but shouldn't be shipped
on a hypothesis.

## 6. Files touched

| File | Change |
|---|---|
| `frontend/app/view/agent/components/AgentComposerStrip.tsx` | Add Mode `<select>` (Fix 1); remove permission pill + `showPermissionPill` (Fix 1); widen `updateRuntime` patch type (Fix 1). |
| `frontend/app/view/agent/components/AgentControlBar.tsx` | Remove `expanded` signal, chevron header, Mode/Model/Effort body, now-dead label/color consts and summary helpers (Fix 2). Optionally rename (Fix 3, pending decision). |
| `frontend/app/view/agent/agent-view.tsx` | No structural change if Fix 3 = Option A (still mounts the same component, now session-only, inside `.agent-composer-details`); update the import/JSX name if renamed. |
| `frontend/app/view/agent/styles/_composer-strip.scss` | Add a modifier class for the Mode select if it needs distinct width/color-border styling from Model/Effort. |
| `frontend/app/view/agent/styles/_control-bar.scss` (or wherever `AgentControlBar`'s styles live) | Remove chevron/header/body rules for the deleted Mode/Model/Effort block; keep banner/session-button rules. |

## 7. Test plan
- With a Claude agent pane open: click Log → confirm only log entries render, no chevron/summary row
  below them (when no session banners are active).
- Change Mode via the new strip dropdown → confirm `agent:runtime` meta updates and the color-left-border
  reflects the new mode (mirrors existing Model/Effort behavior — same `updateRuntime`/`applyRuntimeChange`
  path).
- Confirm Model/Effort selection behavior is unchanged (no regression from the `updateRuntime` patch-type
  widening).
- With an interrupted/large/archived session: confirm the relevant banner + buttons still render and
  function when Log is open (Option A) or via the new standalone affordance (Option B), whichever is
  confirmed.
- Visual: strip doesn't overflow/wrap at typical pane widths with a third select added — check narrow
  pane widths per the strip's existing responsive behavior.

## 8. Sources
- `frontend/app/view/agent/components/AgentComposerStrip.tsx:27-41,162-250,269-277` (pill, existing
  selects, gating).
- `frontend/app/view/agent/components/AgentControlBar.tsx:28-58,60-61,68-102,197-349` (full current
  component, to be split).
- `frontend/app/view/agent/agent-view.tsx:1005-1016` (mount site).
- `docs/specs/SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG_2026_07_02.md` Part C (origin of the (a)/(b) decision
  this spec resolves).
- `docs/retro/retro-composer-strip-log-controls-nesting-2026-07-02.md` (root cause).
