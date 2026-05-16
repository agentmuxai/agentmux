# SPEC: Status-Bar Window-Count Display

**Status:** Draft / shipping with PR
**Date:** 2026-05-16
**Author:** AgentA

---

## 1. Problem

The status bar in every AgentMux window shows the app version followed by a parenthesized number, e.g.:

```
v0.33.900 (2)
```

The `(2)` is the **per-window instance ordinal** — window #2 shows `(2)`, window #5 shows `(5)`. The number is different in every window. Users have reported this is unclear: it tells you *which* window you're in but not *how many* windows are open in total.

The intent — based on user feedback — is that the number should help answer "how many AgentMux windows do I have open right now?" That answer is the same in every window, and it's more often the question users want answered.

## 2. Goals

- **G1** The number rendered to the right of the version is the **total active window count**, identical in every window.
- **G2** The display still hides when there's only one window (no useful information to convey).
- **G3** Behavior updates reactively when windows open or close.
- **G4** No backend changes — both source signals already exist in `frontend/app/store/global.ts`.

## 3. Non-goals

- Surfacing which window you're currently in. The instance ordinal can be moved into the instance panel popover (the same popover anchored on the version) for users who still care; out of scope for this spec.
- Multi-monitor or workspace awareness — count is over all AgentMux windows on this machine, not filtered by anything.

## 4. Current state

`frontend/app/statusbar/StatusBar.tsx:80-94`:

```tsx
<button ... class="status-version clickable" ...>
    v{version}
    <Show when={windowCount() > 1}>
        <span class="instance-num"> ({instanceNum()})</span>
    </Show>
</button>
```

- `instanceNum()` = `windowInstanceNumAtom` (per-window ordinal, set when the window is created).
- `windowCount()` = `windowCountAtom` (total active windows, updated as windows open/close).
- Both atoms are declared at `frontend/app/store/global.ts:132-133` and already kept in sync by the existing window-management plumbing.

## 5. Proposed change

Swap the rendered atom from `instanceNum()` to `windowCount()`:

```tsx
<button ... class="status-version clickable" ...>
    v{version}
    <Show when={windowCount() > 1}>
        <span class="instance-num"> ({windowCount()})</span>
    </Show>
</button>
```

`<Show when={windowCount() > 1}>` already gates the display correctly — when only one window is open, the parenthesis is hidden.

`instanceNum` accessor stays imported (currently unused after the swap) only if we also surface it inside the instance-panel popover (see §3 non-goal — deferred). Otherwise drop the unused import.

## 6. Edge cases

| Case | Handling |
|---|---|
| First-window startup before `windowCountAtom` is initialized | Default value is `1` (see `global.ts:133`); Show hides parenthesis until count rises. Same as today. |
| Window closes mid-render | Reactive update; next paint shows new count. SolidJS signal semantics handle this. |
| Two windows open then one closes | Other window's count drops from `2` → `1`; Show hides the parenthesis. |
| User opens 10 windows | Renders `v0.33.900 (10)`. No truncation. |

## 7. Risk

Trivial. One-line swap of accessors; no new state, no new signals, no test fixture change. Manual smoke is sufficient.

## 8. Test plan

- [ ] Single window — status bar shows `v<X.Y.Z>`, no parenthesis.
- [ ] Open second window — both windows show `v<X.Y.Z> (2)`.
- [ ] Open third — all three show `(3)`.
- [ ] Close one — remaining show `(2)`.
- [ ] Close down to one — parenthesis hides.

---

🤖 Authored by AgentA, 2026-05-16. Implementation ships in the same PR per `feedback_no_doc_only_prs.md`.
