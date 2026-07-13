# SPEC: Launch Modal — State Machine Hardening

**Status:** Draft
**Date:** 2026-05-19
**Author:** AgentA
**Related:**
- `frontend/app/view/agent/components/AgentLaunchModal.tsx` (form surface today)
- `frontend/app/view/agent/components/PreLaunchAuthPanel.tsx` (Connect UI today)
- `frontend/app/view/agent/auth/auth-state.ts` (pure reducer, currently panel-scoped)
- `frontend/app/store/browser-pane-state/` (reference reducer slice — pattern to follow)
- `docs/specs/MASTER_REDUCER_STACK_STATUS_2026-05-05.md`
- `docs/specs/SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md`
- `docs/specs/archive/identity-forge-integration-and-vault-2026-05-08.md`

---

## 0. TL;DR

The Launch modal's state is spread across **four uncoordinated surfaces**:

1. `AgentLaunchModal` local signals (`name`, `runtime`, `image`, `identityId`, `memoryId`, `continueOfId`, `error`, `submitting`)
2. `PreLaunchAuthPanel` local-scope `controller = new AuthFlowController()` (instantiated per-mount)
3. `AuthFlowController.state` (a pure reducer — but its instance is panel-scoped)
4. `createResource(selectedBundleBindings)` keyed on `identityId()` — caches in the resource, not in state

When any one of these re-renders or re-instantiates, in-flight state on the others can vanish. This proposes:

1. **Remove the "blank" sentinel.** Identity + Memory are always required at launch.
2. **Lift auth state out of `PreLaunchAuthPanel` into a Launch-modal-scoped reducer.** Mount/unmount of the Connect panel no longer destroys the controller.
3. **Push-based binding reactivity.** When a binding is added (from Launch OAuth, Identity pane, or anywhere else), all live UIs reflect it without an explicit refetch.

This is a Stage-1 (UX + targeted bug fix) + Stage-2 (full reducer) plan.

---

## 1. Where state lives today

### 1.1 Surfaces

| Surface | State held | Lifetime | Coordinator? |
|---|---|---|---|
| `AgentLaunchModal` local | name, runtime, image, identityId, memoryId, continueOfId, error, submitting, showAdvanced | per-mount of the modal panel | none — bare `createSignal` |
| `PreLaunchAuthPanel` local | `controller = new AuthFlowController()` | per-mount of the panel (which is gated by `<Show when={authRequired()}>` inside the Launch modal) | none — the panel makes a fresh controller every time `<Show>` mounts it |
| `AuthFlowController.state` | kind / provider / bundleId / authUrl / email / error / pollSessionId | piggy-backs on whatever component holds the controller (= `PreLaunchAuthPanel`) | the reducer's `update(state, command)` is correct; the bug is *who owns the instance* |
| `createResource(selectedBundleBindings)` | bindings for `identityId()` | Resource cache | refetches only when the keyed source (`identityId`) changes — silently stale when OAuth completes on the same id |

### 1.2 The cross-surface dance

Walking through the happy "fresh launch" path:

1. Modal mounts → `identityId = "blank"`.
2. `authRequired()` reads `identityId === "blank"` → returns `true`.
3. `<Show when={authRequired()}>` mounts `PreLaunchAuthPanel`.
4. Panel mounts → `controller = new AuthFlowController()` (kind: `idle`).
5. User clicks Connect → `controller.connect()` → kind: `waiting` (URL).
6. User OAuths → `controller.polled({status: "success", bundleId})` → kind: `ready`.
7. `onBundleCreated(bundleId)` → modal does `setIdentityId(bundleId)` → identityId changes.
8. `createResource` refetches → bindings load → `bundleHasMatchingBinding` flips true → `authRequired()` flips false.
9. `<Show>` unmounts the panel → controller destroyed.
10. The launch modal stays "logged in" because `authReady()` reads `authStateKind() === "ready"` — but wait, `authStateKind` is sourced from the now-destroyed controller. Step (9) destroyed the source of truth for the auth state, and step (10) keeps working only because Solid hasn't run the dependent effects yet, OR because the parent retained `authState` via the `onStateChange` callback's last value.

Step 10 is fragile. **Any transient re-render that flips `authRequired()` back to `true`** mounts a brand-new controller in `idle`, instantly forgetting the prior auth state. The user sees the Connect button again.

---

## 2. Concrete bug class

### 2.1 Repro: "logged in → changed memory → forgot login"

Reported 2026-05-19 against v0.34.0 (now in main as v0.35.0).

The exact cascade is not fully diagnosed yet, but candidates inside the existing code:

- **Candidate A — `bundleHasMatchingBinding` flicker.** The memo reads `selectedBundleBindings.loading`. If `identities()` resource (separate from bindings) re-loads for any reason, downstream memos could see a transient `loading=true` interpretation that flips `authRequired()` true → unmount panel → destroy controller.
- **Candidate B — `handleContinueSelect("")` reset.** Toggling the Continue dropdown back to "— New agent —" resets `identityId` to `"blank"`. If users land on Continue's blank by accident while choosing memory, that resets identity → mounts panel → idle. Memory and Continue are visually adjacent.
- **Candidate C — `onBundleCreated` race.** Post-OAuth, the modal `setIdentityId(newId)`, but the bundles list refetch is async. If `identities()` returns an empty list briefly during refetch, `hasUserIdentities()` may flicker, which (in some edge cases) could re-render parents.

All three share a root cause: **the auth state lives inside the panel that the Launch modal conditionally renders**. The fix is structural, not patchwork.

### 2.2 Related fragility

Even after the user's specific repro is fixed, the architecture has more failure modes:

- **Binding-added-from-Identity-pane**: User opens Identity pane in another tab, adds a Claude OAuth binding. The Launch modal in this tab has no signal — its `selectedBundleBindings` resource is keyed on identityId, which hasn't changed. The Launch modal still shows "needs Connect" until the user re-opens it.
- **Binding-removed**: Same problem in reverse — Identity pane revokes an OAuth, Launch modal still thinks "ready".
- **Two Launch modals in two tabs**: Each has its own resource cache. OAuth in tab A doesn't update tab B.

---

## 3. Design

### 3.1 Remove the "blank" sentinel

#### 3.1.1 What "blank" means today

- **Frontend:** `identityId === "blank"` and `memoryId === "blank"` are UI sentinels meaning "no override selected". The backend resolver treats empty string and `"blank"` the same: `identity/resolver.rs:134` skips env-injection entirely.
- **Backend `db_identity_bundles`:** an implicit row with `is_blank=true` is materialized by the bundles list RPC so the dropdown can render it. The blank singleton has no bindings.
- **Memory side:** same pattern in `db_memories`.

#### 3.1.2 New rule

**Identity + Memory selections are always required.** No blank option in either dropdown.

UX implications:

| Pre-launch state | Before | After |
|---|---|---|
| User has 0 identities | Dropdown shows "— Blank (no creds) —" | Dropdown hidden, only the "+ New Identity" button visible (already implemented in Phase α). Launch button disabled until user creates an identity. |
| User has 0 memories | "— Blank (vanilla CLI) —" | Same — Memory "+ New" only; Launch disabled. |
| User has 1+ identity, none chosen | "— Blank —" was default | First non-blank bundle is the default. |
| Launch with no provider binding | Backend resolves to ambient creds | Auth gate fires (binding required). |

#### 3.1.3 Resolver changes

- `inject_identity_env` no longer accepts `identity_id == "blank"` as a valid skip — instead returns an error/no-op and the launch flow must guarantee a real id is supplied.
- `bundle_list` RPC stops materializing the implicit blank row. Frontend bundle filter (`hasUserIdentities`) becomes trivial: `bundles.length > 0`.
- Schema migration: any existing `db_agent_instances` row with `identity_id IN ("", "blank")` stays — backwards-compatible read. New launches must store a real id.

#### 3.1.4 What this removes from the state space

- The `"blank"` branch from every `authRequired()` predicate, every `outcomeFor`, every resolver skip.
- The implicit-blank-row materialization.
- The `bundleArg = id === "blank" ? "" : id` normalization in `PreLaunchAuthPanel.createEffect`.
- The `pending-bundle-for-` placeholder ids that the OAuth backend used to synthesize before bundles were persisted (this whole flow goes away — every new bundle now lands on a real row before any UI sees it).

### 3.2 Lift auth into a Launch-modal-scoped reducer

#### 3.2.1 Pattern to follow

`frontend/app/store/browser-pane-state/` is the reference. Each reducer slice has:

```
<slice>/
  types.ts        # State, Command, Event types + reducer fn
  <slice>-store.ts  # Solid-store wrapper that holds state + dispatch
  <slice>-store.test.ts
```

This proposes `launch-flow-state/` with the same shape.

#### 3.2.2 State

```ts
interface LaunchFlowState {
    /** Form fields. */
    form: {
        name: string;
        runtime: "host" | "container";
        image: string;
        identityId: string;       // never "" or "blank" — required
        memoryId: string;         // never "" or "blank" — required
        continueOfId: string | null;
    };

    /** Loaded bundle lists. Cached here, refetched on push events
     *  (not on identity-change). */
    identities: { list: IdentityBundle[]; loading: boolean; error: string | null };
    memories:   { list: Memory[];          loading: boolean; error: string | null };

    /** Per-identity binding cache. Push-updated when the backend
     *  emits `identity-bindings-changed`. */
    bindings: Map<string /* identityId */, IdentityBinding[]>;

    /** Auth flow state — same shape as today's AuthFlowController.state,
     *  but owned here instead of inside PreLaunchAuthPanel. */
    auth: AuthState;

    /** Submit-in-flight + error. Replaces the bare `submitting` + `error`
     *  signals. */
    submit: { inFlight: boolean; error: string | null };
}
```

#### 3.2.3 Commands

```ts
type LaunchFlowCommand =
    | { kind: "Opened"; agent: ForgeAgent; preselect?: Partial<LaunchFormState> }
    | { kind: "NameChanged"; name: string }
    | { kind: "RuntimeChanged"; runtime: "host" | "container" }
    | { kind: "ImageChanged"; image: string }
    | { kind: "IdentityChanged"; identityId: string }   // never "blank"
    | { kind: "MemoryChanged"; memoryId: string }       // never "blank"
    | { kind: "ContinueOfChanged"; continueOfId: string | null }
    | { kind: "IdentitiesLoaded"; list: IdentityBundle[] }
    | { kind: "MemoriesLoaded"; list: Memory[] }
    | { kind: "BindingsChanged"; identityId: string; bindings: IdentityBinding[] }
    | { kind: "AuthSelected"; outcome: SelectionOutcome }
    | { kind: "AuthConnectClicked" }
    | { kind: "AuthUrlReceived"; url: string }
    | { kind: "AuthPolled"; status: AuthSessionStatusWire }
    | { kind: "AuthCancelled" }
    | { kind: "SubmitClicked" }
    | { kind: "SubmitSucceeded" }
    | { kind: "SubmitFailed"; error: string }
    | { kind: "Closed" };
```

#### 3.2.4 Events

The reducer emits events for side-effects (RPCs, navigation). The view runs them; the reducer stays pure.

```ts
type LaunchFlowEvent =
    | { kind: "StartAuth"; providerId: string; identityId: string }
    | { kind: "PollAuth"; sessionId: string }
    | { kind: "CancelAuth"; sessionId: string }
    | { kind: "Submit"; overrides: LaunchOverrides }
    | { kind: "OpenExternal"; url: string };
```

#### 3.2.5 Key invariant

`identityId` and `memoryId` are non-empty strings throughout state. The reducer rejects `IdentityChanged`/`MemoryChanged` commands carrying `""` or `"blank"` (asserts in dev, logs warn in prod).

This eliminates a whole class of "should we fire OAuth gate?" branching.

### 3.3 Push-based binding reactivity

#### 3.3.1 Today

- `createResource` keyed on `identityId` only refetches when the key changes.
- A binding created via the Identity pane (or from another Launch modal in another tab) is invisible to this modal until the user re-selects.

#### 3.3.2 Proposed

- Backend emits `identity-bindings-changed` on `db_identity_bindings` insert/update/delete. Payload: `{ identity_id, bindings: IdentityBinding[] }` (the full new list, not a diff — diff applied at view layer).
- Frontend `launch-flow-store` subscribes via `waveEventSubscribe`. On event, dispatches `BindingsChanged`.
- The reducer's `bindings` Map gets updated; downstream derivations (e.g. `hasMatchingBinding(state, providerId)`) become pure selectors over state.

Side benefits:
- Identity pane can subscribe to the same event for its own list updates.
- Any future "shared identity across tabs" UI gets reactivity for free.

#### 3.3.3 Initial population

On first `Opened`, dispatch `BindingsChanged` for every loaded identity (single batch query). Subsequent updates come via the event.

---

## 4. Migration phases

### Stage 1 — UX + targeted fix (this session, 1 PR)

1. **Remove blank from dropdowns** + treat empty list as "must create first".
2. **Lift `AuthFlowController` instance from `PreLaunchAuthPanel` to `AgentLaunchModal`.** Pass `controller` (or its dispatch + state) into the panel as a prop. The panel's mount/unmount no longer destroys it.
3. **Backend: emit `identity-bindings-changed` event on bundle_bind_create / bundle_bind_delete RPCs.** Frontend resource swaps from keyed-refetch to event-driven update.
4. **Wire-shape changes:**
   - `AgentLaunchModal` props no longer accept `preselectedIdentityId === "blank"` — strip the normalization, require real ids.
   - `LaunchOverrides.identityId` / `memoryId` become required strings, not optional.
   - `inject_identity_env` rejects empty/`"blank"` identity_id with a logged error (defense in depth — UI should never produce it).

5. **Backward compat:**
   - Existing `db_agent_instances` rows with `identity_id = "" or "blank"`: read-only, no migration. Continuation launches from those rows pre-fill with the first available real bundle (or block until user picks).

### Stage 2 — Full reducer (separate PR, 2-3 sub-PRs)

1. New slice `frontend/app/store/launch-flow-state/` with types + store + tests, modeled on `browser-pane-state`.
2. Convert all `createSignal` calls in `AgentLaunchModal` to `useLaunchFlowStore()` reads.
3. Convert all `PreLaunchAuthPanel` controller calls to dispatches on the launch-flow store.
4. Move `selectedBundleBindings` resource to the store; replace with reactive `state.bindings` map.
5. Unit tests for every state cross-product (per `feedback_state_space_first_for_review_heavy_prs`).

---

## 5. Risks + open questions

### 5.1 Risks

- **Empty-bundle UX regression.** A user with zero bundles can't launch — must create one first. We need clean empty-state UX (already in place from Phase α's "+ New" empty-state button).
- **Migration of existing agents.** Resuming an agent that was launched with `identity_id = "blank"` needs a path forward. Proposal: on Continue-from-blank, force the user to pick a real bundle before launch. Document in release notes.
- **Resolver loosening.** The current "blank → ambient" semantic means a user can launch Claude Code with their system Claude CLI's existing auth. Removing it means they MUST go through OAuth via the modal. This is a deliberate UX shift — surface in release notes; offer "Import ambient" as a future bundle creation path if users complain.
- **Backend event volume.** `identity-bindings-changed` fires on every binding insert/update/delete. Volume is low (bindings change rarely), but worth a sanity check before broadcasting to every renderer.

### 5.2 Open questions

1. **Continue dropdown.** If `continueOfId` is set, do `identityId` + `memoryId` lock to the prior agent's values? Today they auto-fill but stay editable. Reducer should make this explicit: either lock (commands rejected) or editable (commands accepted, breaking the "continue" semantic). Recommend: lock.
2. **`pending-bundle-for-<sid>` ids.** These are placeholder ids the OAuth backend used before bundles persisted on disk. Per `frontend/app/view/agent/components/PreLaunchAuthPanel.tsx:208` they're already filtered before reaching the dropdown. With Stage 1, the OAuth flow always creates a real bundle row first — these placeholders disappear entirely.
3. **Memory bindings.** Memory bundles don't have a "binding" concept (unlike Identity which has provider bindings). The blank-removal for Memory is simpler — just enforce a non-blank `memoryId`. No auth gate involvement.
4. **Multi-window Launch modals.** Two top-level windows can each have a Launch modal open simultaneously. The reducer should be per-tab (matching `TabModalLayer`'s scope), and the binding push event broadcasts to every window via `emit_event_to_top_level_windows` (the helper added in #906).

---

## 6. Acceptance criteria

### Stage 1

1. Launch modal Identity dropdown does not show a "— Blank —" option in any state.
2. Same for Memory.
3. User with zero identities sees ONLY the "+ New Identity" button in the Identity row; the launch button is disabled.
4. Repro: open Launch modal → Connect OAuth → after success, change Memory dropdown → Login state persists, no Connect button reappears.
5. Add a binding via the Identity pane in another tab while the Launch modal is open here → the modal updates within ~1s (no manual refetch).
6. `inject_identity_env` warns + skips when called with empty/`"blank"` identity_id (defense in depth).

### Stage 2

7. All Launch modal state is owned by `useLaunchFlowStore()`.
8. `recordDispatch` audit covers every command.
9. Unit tests for every (auth state) × (form-field-changed) cross-product show no spurious state resets.
10. New per-pane integration test (Playwright or equivalent) replays the round-2 repro from §2.1 and asserts the auth state survives.

---

## 7. Out of scope

- **Multi-account identity bundles.** Today an identity bundle can hold multiple bindings (Claude + Codex + GitHub + AWS). The "Active account" selection per provider is a future spec.
- **Binding-edit UI in the Launch modal.** Connector setup stays in the Identity pane.
- **Memory content editing in the Launch modal.** Same — stays in the Memory pane.
- **Cross-pane auth-queue coordination** (the codex P2 from #906 final round). Tracked separately.
