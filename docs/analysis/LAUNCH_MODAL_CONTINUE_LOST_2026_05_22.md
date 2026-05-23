# Analysis — launch modal drops the continuation across the "+ New identity" round-trip

**Date:** 2026-05-22
**Author:** AgentA
**Severity:** High (functional) — a continued agent silently falls out of Continue
mode and is incorrectly re-prompted to authenticate.
**Area:** `frontend/app/view/agent/components/AgentLaunchModal.tsx`,
`frontend/app/store/launch-flow-state.ts` (the `SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19`
reducer slice).

---

## Symptom (user repro)

1. Open the launch modal for an agent that has past instances. Feature A
   auto-selects Continue mode with the most-recent instance ("Maks")
   preselected.
2. Click **"+ New identity bundle…"**.
3. **Cancel** the New Identity modal.
4. Back in the launch modal it now says **"Connect to Claude Code"** —
   even though the continued agent already has a working Claude Code
   OAuth login.

Expected: cancelling returns to exactly the prior state — Continue mode,
"Maks" selected, no auth prompt (a continuation reuses the prior launch's
credentials).

## Root cause

**`continueOfId` — the field that records "this launch is continuing a
prior agent" — is not part of the form snapshot that survives the
"+ New bundle" round-trip.**

The `+ New identity` flow hands off to the New Identity modal and re-opens
the launch panel from a snapshot of the form. That snapshot type carries
five fields and **omits `continueOfId`**:

```ts
// AgentLaunchModal.tsx:101-107
export interface LaunchFormState {
    name: string;
    runtime: "host" | "container";
    image: string;
    identityId: string;
    memoryId: string;
    // ← no continueOfId
}
```

```ts
// AgentLaunchModal.tsx:224-230 — snapshot() handed to onRequestNewIdentity
const snapshot = (): LaunchFormState => ({
    name: name(), runtime: runtime(), image: image(),
    identityId: identityId(), memoryId: memoryId(),
    // ← continueOfId() is never captured
});
```

So when the panel re-opens with `initialFormState = snapshot`, the
reducer's `Opened` event restores name/identity/memory but `continueOfId`
comes back empty.

## Failure chain

| Step | State |
|---|---|
| Re-open from snapshot | `continueOfId()` → `""` (not in `LaunchFormState`) |
| `continuedRow()` (`:262`) | `null` |
| `isContinue()` (`:267`) | **`false`** |
| `viewModeDecided` (`:316`) | `true` (`initialFormState != null`) → the Feature-A auto-decide effect (`:317-326`) is **skipped** → `viewMode` stuck at `"new"` |
| `authRequired()` (`:471-478`) | `!isContinue()` is now `true`, so the gate is no longer short-circuited. The continued agent ("Maks") launched on ambient creds, so the carried `identityId` is `""` → `identityId() === ""` is `true` → **`authRequired()` → `true`** |
| Result | The `PreLaunchAuthPanel` renders → **"Connect to Claude Code"** |

When "Maks" is genuinely selected in Continue mode, `isContinue()` is
`true`, which short-circuits `authRequired()` to `false` ("prior launch
already produced creds", `:466`). Dropping `continueOfId` removes that
short-circuit.

## Why the existing code did not guard against this

`AgentLaunchModal.tsx:313-315` carries this comment:

> A `+ New bundle` round-trip re-opens the panel with initialFormState
> and only ever originates from New mode (the +New buttons are disabled
> while continuing) — so skip the auto-decide and keep New.

That assumption is **false**. The `+ New identity` button is disabled
while continuing only when `continueLocksIdentity()` is true — which
requires the continued row to carry a *real* identity bundle. An agent
continued from an ambient-creds launch (`identity_id` empty/`"blank"`)
does **not** lock identity, so its `+ New identity` button is enabled —
and clicking it triggers exactly this round-trip from Continue mode.

## Fix

Thread the continuation through the round-trip:

1. **`LaunchFormState`** — add `continueOfId: string` (`""` = no
   continuation).
2. **`snapshot()`** — capture `continueOfId: continueOfId()`.
3. **`Opened` reducer event** (`launch-flow-state.ts`) — map
   `initial.continueOfId` into `form.continueOfId`. Restoring the id
   alone is not enough: `handleContinueSelect` also re-derives the
   `carry` (name/identity/memory) and the continuation locks — so on
   re-open, if `initialFormState.continueOfId` is set, re-run the
   continuation restore once `namedAgents()` has loaded (re-derive the
   row + carry rather than trusting raw form values).
4. **`viewMode`** — when `initialFormState?.continueOfId` is non-empty,
   initialize `viewMode` to `"continue"` (and keep `viewModeDecided`
   true so the auto-decide doesn't fight it).
5. Correct the stale comment at `:313-315`.

Scope: `AgentLaunchModal.tsx` + `launch-flow-state.ts`. No backend
change. The continuation's auth bypass then survives the `+ New`
round-trip, and the modal returns to Continue mode with "Maks" selected
and no auth prompt.

## Note — identity bundles vs. ambient creds

The repro also surfaces that "Maks" was launched on **ambient** Claude
Code creds, not an identity bundle (his named-agent row has no real
`identity_id`). Binding that OAuth into a managed identity bundle is a
reasonable, *separate* improvement — but it is not the fix here: even
with a bound bundle, the round-trip would still drop `continueOfId` and
misbehave. Fix the round-trip first.
