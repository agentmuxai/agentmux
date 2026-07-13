# Spec: Toolchain Manager + Trust Center → Widget Panes + In-Use Registry

> **Archived 2026-07-12.** Historical — the Trust Center → pane migration it describes shipped. Consolidated tracking: issue #2024.

**Date:** 2026-06-25
**Status:** Draft
**Scope:** Frontend widget registration, ViewModel scaffolding, modal-to-pane migration,
app-wide resource in-use registry integrated with the reducer state machine.

---

## Problem

Both the Toolchain Manager and Trust Center live under the hamburger menu as inline
modals. This has several drawbacks:

- **Discoverability** — power-user surfaces hidden behind the hamburger.
- **Pane lifecycle** — modals are ephemeral, dismissed on outside-click or ESC.
  Users can't keep them open alongside an agent pane.
- **Multi-window** — modals are per-window; the Trust Center already needs a singleton
  workaround (`acquireSingleton`) just to prevent two modal copies.
- **No edit guards** — nothing prevents a user from deleting or editing an account/
  identity that an active agent turn is currently relying on, risking silent auth
  failures or mid-stream crashes.

---

## Goals

1. Register `toolchain` and `trust` as first-class widget-bar panes (same pattern
   as `terminal`, `sysinfo`, `agent`).
2. Port both modal implementations to pane `ViewModel` + `ViewComponent` pairs —
   minimal logic change, scaffold change only.
3. Build an **app-wide resource in-use registry** wired into the reducer event bus
   so the Trust Center and Toolchain panes know which resources are live and can
   show guards, disable destructive actions, and surface contextual warnings.
4. Keep backward-compatible hamburger entries as shortcuts (open-or-focus the pane)
   rather than opening a modal — no user regression.

---

## Current Implementation Summary

### Toolchain Manager (`frontend/app/modals/toolchain-modal.tsx`)
- Opened via `openToolchainModal()` → `openModal(ToolchainModal)` (hamburger line 141)
- Renders: Environment section, Core tools (Node/npm/Git/Docker), Agent CLIs per
  provider, External Widgets health/port config
- RPC: `ToolchainEnvCommand`, `ResolveCliCommand`, `WidgetHealthCommand`
- State: `localStorage` for widget port overrides (`"agentmux:widget-ports"`)
- Stateless across sessions — one-shot probe on mount + refresh button

### Trust Center (`frontend/app/modals/bundle-manager-modal.tsx`)
- Opened via `openBundleManager()` → `acquireSingleton` + `openModal` (hamburger line 133)
- Renders: left-rail with Accounts, Identities, Brain, Presets sections
- Child managers (`AccountsManager`, `IdentityManager`, `GlobalBrainManager`,
  `MemoryManager`) own their own RPC calls
- Singleton coordination: `acquireSingleton(SINGLETON_KIND_BUNDLE_MANAGER)` +
  `BundleManagerElsewhereBanner`
- WPS events: `*:changed` for real-time identity/memory sync

### Widget Pattern (Terminal / Sysinfo reference)
- Entry in `agentmux-srv/src/config/widgets.json` with `display:order`,
  `display:pinned`, `icon`, `label`, `blockdef.meta.view`
- ViewModel class registered in `block-registry.ts` via
  `blockViewRegistry.set("viewType", MyViewModel)`
- ViewModel implements: `viewType`, `blockId`, `nodeModel`, `viewIcon()`,
  `viewName()`, `viewComponent`
- Block routing: `block.tsx` → `getBlockViewClass(view)` → `new ViewModel(blockId, nodeModel)`
- Widget opened via `createBlock({ meta: { view: "..." } })` from `action-widgets.tsx`

---

## Part 1 — Widget Registration

### 1.1 `widgets.json` entries

```json
"defwidget@toolchain": {
    "display:order": 20,
    "display:pinned": false,
    "icon": "wrench",
    "label": "Toolchain",
    "blockdef": {
        "meta": { "view": "toolchain" }
    }
},
"defwidget@trust": {
    "display:order": 21,
    "display:pinned": false,
    "icon": "shield-halved",
    "label": "Trust Center",
    "blockdef": {
        "meta": { "view": "trust" }
    }
}
```

Both are `pinned: false` — they live in the "More" dropdown by default, same as
today's hamburger placement. Users who want them pinned can right-click → Pin.

### 1.2 Block registry

```typescript
// frontend/app/block/block-registry.ts
import { ToolchainViewModel } from "@/app/view/toolchain/toolchain-model";
import { TrustViewModel } from "@/app/view/trust/trust-model";

blockViewRegistry.set("toolchain", ToolchainViewModel as any);
blockViewRegistry.set("trust", TrustViewModel as any);
```

---

## Part 2 — ViewModel Scaffolding

### 2.1 ToolchainViewModel

**File:** `frontend/app/view/toolchain/toolchain-model.ts`

```typescript
export class ToolchainViewModel implements ViewModel {
    viewType = "toolchain";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon = () => "wrench";
    viewName = () => "Toolchain";
    viewComponent = ToolchainView;   // imported from toolchain.tsx

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
    }
}
```

**File:** `frontend/app/view/toolchain/toolchain.tsx`

Port `ToolchainModal`'s render body verbatim into `ToolchainView`. Remove
`ModalCloseProps`; accept `ViewComponentProps<ToolchainViewModel>` instead.
The wrapping chrome (title bar, close button) is now provided by the block
frame — remove the `<Modal>` wrapper.

### 2.2 TrustViewModel

**File:** `frontend/app/view/trust/trust-model.ts`

```typescript
export class TrustViewModel implements ViewModel {
    viewType = "trust";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon = () => "shield-halved";
    viewName = () => "Trust Center";
    viewComponent = TrustView;

    // Tracks which left-rail section is active — persisted in wave meta
    // so reopening the pane remembers the last section.
    activeSection: () => TrustSection;
    setSection: (s: TrustSection) => void;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        // read initial section from block meta ("trust:section"), default "accounts"
    }
}
```

**File:** `frontend/app/view/trust/trust.tsx`

Port `bundle-manager-modal.tsx` render body, adapting to `ViewComponentProps<TrustViewModel>`.
The `section` signal moves to `TrustViewModel.activeSection` (persisted in wave
object meta `trust:section` via `ObjectService.UpdateObjectMeta`).

**Singleton coordination:**
The `acquireSingleton` / `BundleManagerElsewhereBanner` approach was a workaround
for modals. Panes are naturally non-modal and can exist in multiple windows. Drop
the singleton. If a user opens Trust Center in two windows they get two independent
pane instances (same data, WPS keeps them in sync — same as two agent panes with the
same agent). The banner is removed.

---

## Part 3 — Hamburger Menu Migration

Keep both menu entries; replace the open-modal action with open-or-focus the pane:

```typescript
// hamburger-menu.tsx

// Toolchain Manager — was: openToolchainModal()
{
    label: "Toolchain",
    icon: "wrench",
    click: () => fireAndForget(openOrFocusPaneByView("toolchain")),
},

// Trust Center — was: openBundleManager()
{
    label: "Trust Center",
    icon: "shield-halved",
    click: () => fireAndForget(openOrFocusPaneByView("trust")),
},
```

`openOrFocusPaneByView(view)` — utility that checks if a pane of that view type
is already open in the current tab; if yes, focuses it; if no, calls `createBlock`.
This mirrors the existing `handleWidgetSelect` path.

---

## Part 4 — App-Wide Resource In-Use Registry

### 4.1 Motivation

When a user opens the Trust Center and attempts to:
- Delete an identity/account
- Edit an API key
- Rotate credentials

…the agent that is actively mid-turn using that identity may crash, return auth
errors, or leave the session in a broken state. Similarly, if the Toolchain Manager
attempts to reconfigure the Node.js path while an agent has a `node` subprocess
open, the resulting environment change can corrupt the active process.

We need a **lightweight, reducer-integrated registry** that maps resource IDs to the
set of block IDs currently using them, enabling guards at the UI layer.

### 4.2 Design: `resource-in-use-store.ts`

**File:** `frontend/app/store/resource-in-use-store.ts`

A standalone Solid store — not a reducer slice itself, but wired into the reducer
event bus via `addEventListener` from `agent-pane-state-store`.

```typescript
// Map of resourceKey → Set<blockId>
const inUse = new Map<string, Set<string>>();
const [inUseSignal, setInUseSignal] = createSignal<ReadonlyMap<string, Set<string>>>(new Map());

// Public API
export function isResourceInUse(resourceKey: string): boolean
export function getResourceUsers(resourceKey: string): string[]   // blockIds
export function subscribeResourceInUse(resourceKey: string): () => boolean  // reactive accessor
```

**Resource key format:**
- Accounts/identities: `identity:<identityId>` or `account:<providerId>:<accountId>`
- Toolchain CLIs: `tool:<toolName>` (e.g. `tool:node`, `tool:claude-cli`)
- External widgets: `widget:<widgetKey>` (e.g. `widget:defwidget@discord`)

### 4.3 Wiring into the Reducer Event Bus

The `agent-pane-state-store` already exposes `addEventListener(sink)`. The resource
registry subscribes to pane events to track active usage:

```typescript
// In resource-in-use-store.ts, called once at app init

addEventListener((blockId, event) => {
    switch (event.type) {
        case "stream-subscribed":
            // Agent turn started — register which identity this pane uses.
            // Identity is read from the block's meta (agent:identity_id).
            const identityId = getBlockMeta(blockId, "agent:identity_id");
            if (identityId) register(`identity:${identityId}`, blockId);
            // Also register the provider/CLI in use.
            const provider = getBlockMeta(blockId, "agent:provider");
            if (provider) register(`tool:${providerToCli(provider)}`, blockId);
            break;

        case "stream-unsubscribed":
        case "stream-disconnected":
            // Turn ended — release all resources registered for this pane.
            releaseAll(blockId);
            break;
    }
});
```

`register(key, blockId)` — adds `blockId` to `inUse.get(key)`, updates signal.
`releaseAll(blockId)` — removes `blockId` from all sets, cleans up empty sets,
updates signal.

**Why stream events (not turn events):**
`stream-subscribed` fires when the frontend receives the first event from a live
stream — it precisely marks "the model is running for this pane." `stream-unsubscribed`
fires on clean disconnect (turn complete or user interrupt). `stream-disconnected`
fires on crash. Using stream events rather than `TurnStart`/`TurnEnd` is correct
because the resource (the identity, the CLI) is in use for the entire duration the
stream exists, not just the conceptual "turn."

### 4.4 Toolchain In-Use Detection

For toolchain tools, the registry is fed from two sources:

**Source A — active agent streams (as above):** `tool:claude-cli`, `tool:openai-cli`,
etc. derived from the provider meta of each active pane.

**Source B — active tool calls:** The reducer already tracks `currentTool`
(the name of the active tool call). The Toolchain pane can additionally subscribe to
`currentToolArg` to detect when an agent is running `bash`/`node`/`git` commands:

```typescript
// Additional listener — fires on ToolStart events surfaced via currentToolArg
addEventListener((blockId, event) => {
    if (event.type === "tool-started") {
        // Extract CLI from the tool arg (e.g. "node script.js" → "node")
        const cli = extractCli(snapshot(blockId)?.currentToolArg ?? "");
        if (cli) register(`tool:${cli}`, blockId);
    }
    if (event.type === "tool-ended") {
        const cli = ...;
        if (cli) release(`tool:${cli}`, blockId);
    }
});
```

This requires a new `AgentPaneEvent` pair `tool-started` / `tool-ended` emitted from
the existing `ToolStart` / `ToolEnd` reducer commands. (Currently those commands
update `currentTool` state but emit no events — add the events.)

### 4.5 Guard Behavior in Trust Center

**Destructive actions (delete identity / delete account):**

```tsx
// In IdentityManager / AccountsManager

const inUse = () => isResourceInUse(`identity:${identity.id}`);
const users = () => getResourceUsers(`identity:${identity.id}`);

<button
    disabled={inUse()}
    title={inUse()
        ? `In use by ${users().length} active agent${users().length > 1 ? "s" : ""}. Stop them first.`
        : "Delete identity"}
    onClick={handleDelete}
>
    Delete
</button>
```

**Edit actions (modify API key / rotate credential):**

Show an inline warning banner above the edit form when in use, but do NOT disable
editing — the user may need to rotate a key mid-flight (e.g. leaked key). The guard
is advisory:

```tsx
<Show when={inUse()}>
    <div class="resource-in-use-banner">
        <i class="fa-solid fa-triangle-exclamation" />
        This account is active in {users().length} agent pane{users().length > 1 ? "s" : ""}.
        Saving changes will take effect on the next turn.
    </div>
</Show>
<CredentialEditForm ... />
```

**Graceful degradation:**
If the registry is unavailable (e.g. pane not yet registered), treat as "not in use"
— the guards are advisory UX, not security enforcement. The backend validates
credentials independently.

### 4.6 Guard Behavior in Toolchain Manager

**Path/CLI reconfiguration:**

```tsx
const nodeInUse = () => isResourceInUse("tool:node");

<div class="toolchain-row">
    <ToolStatusBadge tool="node" />
    <Show when={nodeInUse()}>
        <span class="in-use-chip">
            <i class="fa-solid fa-circle-dot" /> in use
        </span>
    </Show>
    <button disabled={nodeInUse()} onClick={openNodeConfig}>
        Configure path
    </button>
</div>
```

**Widget port reconfiguration:**
External widget ports stored in `localStorage` are safe to edit at any time (they
take effect on next widget launch), so no guard needed there.

**Environment PATH changes:**
`ToolchainEnvCommand` is read-only (detection only, no write path in Phase 1). Mark
for Phase 2 when install/override capabilities are added.

### 4.7 Registry Initialization

The registry must be initialized **before** any agent pane registers, at app startup:

```typescript
// frontend/app-init.ts — after initWaveWrap, before pane registration
import { initResourceRegistry } from "@/app/store/resource-in-use-store";
initResourceRegistry();   // attaches the addEventListener listeners
```

`initResourceRegistry` is idempotent (guards against double-init in hot reload).

---

## Part 5 — Implementation Sequence

### Phase 1 — Widget Scaffolding (no logic change)
1. Add `defwidget@toolchain` + `defwidget@trust` to `widgets.json`
2. Create `toolchain-model.ts` + `toolchain.tsx` (port modal body verbatim)
3. Create `trust-model.ts` + `trust.tsx` (port modal body verbatim)
4. Register both in `block-registry.ts`
5. Update hamburger entries to use `openOrFocusPaneByView()`
6. Add `openOrFocusPaneByView()` utility to `global.ts`
7. Delete `frontend/app/modals/toolchain-modal.tsx` (or keep as thin re-export
   pointing to the new view, for any existing `openToolchainModal` call sites)
8. Drop `BundleManagerElsewhereBanner` + `acquireSingleton` from Trust Center

**Changeset:** `minor` — new widget-bar surfaces, non-breaking removal of modal shell

### Phase 2 — In-Use Registry
1. Add `tool-started` / `tool-ended` to `AgentPaneEvent` union in `types.ts`
2. Emit them from `ToolStart` / `ToolEnd` reducer arms
3. Create `resource-in-use-store.ts` with `initResourceRegistry`, `register`,
   `releaseAll`, `isResourceInUse`, `getResourceUsers`
4. Call `initResourceRegistry()` from `app-init.ts`
5. Wire guards in `IdentityManager` (delete disable + edit warning banner)
6. Wire guards in `AccountsManager` (delete disable + edit warning banner)
7. Wire in-use chip in `ToolchainView` for active CLI tools

**Changeset:** `minor` — new safety guards; purely additive

### Phase 3 — Persist Trust Center section (stretch)
- Store `trust:section` in wave object meta so reopening the pane restores last tab
- `TrustViewModel` reads on construct, writes on `setSection`

---

## Part 6 — Files Affected

| File | Change |
|---|---|
| `agentmux-srv/src/config/widgets.json` | Add `defwidget@toolchain`, `defwidget@trust` |
| `frontend/app/block/block-registry.ts` | Register `"toolchain"`, `"trust"` |
| `frontend/app/view/toolchain/toolchain-model.ts` | **New** — ViewModel |
| `frontend/app/view/toolchain/toolchain.tsx` | **New** — ViewComponent (port of modal body) |
| `frontend/app/view/trust/trust-model.ts` | **New** — ViewModel |
| `frontend/app/view/trust/trust.tsx` | **New** — ViewComponent (port of modal body) |
| `frontend/app/window/hamburger-menu.tsx` | Replace modal open with `openOrFocusPaneByView` |
| `frontend/app/store/global.ts` | Add `openOrFocusPaneByView()` utility |
| `frontend/app/modals/toolchain-modal.tsx` | Delete (or thin shim) |
| `frontend/app/modals/bundle-manager-modal.tsx` | Delete `BundleManagerElsewhereBanner`, `acquireSingleton` wiring |
| `frontend/app/store/agent-pane-state/types.ts` | Add `tool-started`, `tool-ended` to `AgentPaneEvent` |
| `frontend/app/store/agent-pane-state/reducer.ts` | Emit events from `ToolStart`/`ToolEnd` arms |
| `frontend/app/store/resource-in-use-store.ts` | **New** — registry |
| `frontend/app-init.ts` | Call `initResourceRegistry()` at startup |
| `frontend/app/view/trust/IdentityManager.tsx` | Add delete guard + edit warning |
| `frontend/app/view/trust/AccountsManager.tsx` | Add delete guard + edit warning |
| `frontend/app/view/toolchain/toolchain.tsx` | Add in-use chip per tool row |

---

## Out of Scope

- **Tool installation** — Phase 2 of Toolchain Manager. Current implementation is
  detect-only; no write path to install CLIs.
- **Cross-window registry sync** — The registry lives in frontend memory per window.
  Multi-window guard accuracy requires WPS events propagating resource locks across
  windows. Track as follow-up.
- **Backend enforcement** — The registry is a UI-layer advisory guard. Backend does
  not block edits while resources are in use; it validates credentials independently.
- **Pinning by default** — Both new widgets default to the "More" overflow menu,
  preserving the current discoverability level. Pinning is user-controlled.
