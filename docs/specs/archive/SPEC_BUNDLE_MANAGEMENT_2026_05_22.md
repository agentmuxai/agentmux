# SPEC — Identity & Memory bundle management

> **Archived 2026-07-12.** Superseded — designed the pre-rename "Identity & Memory" hamburger modal, explicitly renamed by `specs/archive/SPEC_TRUST_CENTER_2026_06_15.md`. Consolidated tracking: issue #2024.

**Status:** Draft / for review
**Date:** 2026-05-22
**Author:** AgentA
**Area:**
- Hamburger menu — `frontend/app/tab/tabbar.tsx`
- Identity pane — `frontend/app/view/identity/identity-pane-view.tsx`, `identity-pane-model.ts`
- Memory pane — `frontend/app/view/memory/memory-view.tsx`, `memory-model.ts`
- Launch-modal OAuth — `frontend/app/view/agent/components/PreLaunchAuthPanel.tsx`, `AgentNewIdentityModal.tsx`, `auth.ts`
- Unified modal — `frontend/app/element/modal.tsx`, `frontend/app/store/modalmodel.ts`
- Backend — `agentmux-srv/src/server/agent_handlers.rs` (`bundle_*` wstore methods)

Three related changes that give identity & memory **bundles** a single, coherent
management story. They are independent enough to ship as separate PRs but share
one spec because they all concern the same data model.

---

## 1. Background — how bundles work today

### 1.1 Data model

An **Identity bundle** is a generic credential container — it holds *bindings*
that map a provider (Claude / Codex / Gemini / OpenClaw OAuth, GitHub, AWS, …)
to an account. A **Memory bundle** holds an agent's notes / instructions /
project context / MCP servers / skills. Both are **app-wide** data — they live
in `objects.db` (`bundle_identity_*` / `bundle_memory_*` wstore methods), are
not scoped to any tab, pane, or window, and are shared across every agent and
every window. The backend broadcasts `identitybundles:changed` /
`memories:changed` WPS events on every mutation so all open surfaces refresh.

### 1.2 RPCs (`frontend/app/store/rpc-api.ts` → `agent_handlers.rs`)

| Concern | Commands |
|---|---|
| Identity bundles | `ListIdentityBundlesCommand`, `GetIdentityBundleCommand`, `UpsertIdentityBundleCommand`, `DeleteIdentityBundleCommand` |
| Identity bindings | `BindIdentityAccountCommand`, `UnbindIdentityAccountCommand`, `ListIdentityBindingsCommand` |
| Memory bundles | `ListMemoriesCommand`, `GetMemoryCommand`, `UpsertMemoryCommand`, `DeleteMemoryCommand` |

`Upsert*` auto-generates id + timestamps; the full CRUD surface already exists.

### 1.3 Where bundles are managed today (the problem)

Bundle management is **scattered across four surfaces with no app-wide home**:

1. **Identity pane** — `view: "identity"`, reached via an Agent pane → cog →
   settings → Identity tab. Full CRUD + bindings table.
2. **Memory pane** — `view: "memory"`, same path. Full CRUD.
3. **Launch modal dropdowns** — pick an existing bundle; `+ New` buttons open
   `AgentNewIdentityModal` / `AgentNewMemoryModal` (quick-create, name +
   description).
4. **Launch-modal OAuth** — clicking *Connect* with no identity selected mints
   a bundle as a side-effect (see §1.4).

Bundles are app-wide data, but the only *full* management UIs (1, 2) are buried
inside an individual Agent pane's settings — a per-agent-feeling location for
app-wide objects. There is no entry point that says "manage all my identities
and memories." Discoverability is poor and the mental model is muddled.

### 1.4 OAuth → bundle creation today

`PreLaunchAuthPanel`'s *Connect* CTA → `startConnect()` → `auth.start` RPC. The
RPC carries `intoBundleId`:
- **`None`** (no identity selected — the `needs-bundle` outcome) ⇒ the backend
  **auto-mints a fresh bundle** on OAuth success. The user never named it; a
  generated-name bundle simply *appears* in the dropdown afterward.
- **`Some(id)`** (`needs-account` — an existing, named bundle that lacks a
  binding for this provider) ⇒ the OAuth credential **attaches** to that bundle.

The `Some(id)` path is good (named bundle, credential attached). The `None`
path is the problem: it produces unnamed "ghost" bundles the user didn't
consciously create.

### 1.5 The unified modal — window scope

`openModal(Component, props?)` (`store/modalmodel.ts`) opens a window-scoped
modal; the component renders `<Modal scope="window">` (`element/modal.tsx`).
**Window scope inerts the whole window** (`document.body`) while the modal is
open — this is the "lock the window" the user asked for. The hamburger's
*Command Palette* item already uses `openModal(...)`, so the pattern is proven.

### 1.6 The hamburger menu

`frontend/app/tab/tabbar.tsx` → `tabBarMenuItems()` builds a static `MenuItem[]`
rendered in a `<FlyoutMenu>`. Items are `{ label, icon?, shortcut?, onClick }`
or `{ label, subItems }`. Current items: New Tab/Window, Theme, Opacity,
Settings, Command Palette, DevTools, Online Docs, Exit. Adding an entry = push
one `MenuItem`.

---

## 2. Feature 1 — OAuth *Connect* opens the New Identity modal first

### Problem

Clicking *Connect* with no identity selected runs OAuth and the backend
auto-mints an unnamed bundle (§1.4, `None` path). The user ends up with a
bundle they never named or described.

### Behavior

When the user clicks *Connect* **and the outcome is `needs-bundle`** (no
identity bundle selected — a genuinely fresh OAuth):

1. Instead of starting OAuth immediately, **open the New Identity modal**
   (`AgentNewIdentityModalPanel` — name + description).
2. Its primary button reads **"Continue"** (not "Create") in this context.
3. On *Continue*: `UpsertIdentityBundleCommand({ name, description })` creates
   the bundle, then OAuth starts with `intoBundleId: <newId>` — so the
   credential attaches to the **named** bundle.
4. The launch modal then shows the normal OAuth waiting panel; on success the
   bundle (already correctly named) is selected.

When the outcome is `needs-account` (an existing, named bundle that just lacks
a binding), behavior is **unchanged** — Connect goes straight to OAuth with
`intoBundleId: Some(existingId)`. The New Identity modal is only interposed for
the no-bundle case.

### Mechanism

The launch and New-Identity modals are tab-scoped (`TabModalLayer`,
`tabModal.replace`). The chain reuses the existing `initialFormState`
round-trip plumbing already used by the `+ New identity` button:

```
Connect (needs-bundle)
  → tabModal.replace(newIdentityRequest, { purpose: "oauth-continue",
                                           launchSnapshot })
  → user fills name/description, clicks Continue
  → UpsertIdentityBundleCommand({ name, description }) → newId
  → tabModal.replace(launchRequest, { initialFormState: launchSnapshot
                                      with identityId = newId,
                                      autoStartAuth: true })
  → launch modal re-opens, new identity selected, OAuth auto-starts
```

`AgentNewIdentityModalPanel` gains a `purpose: "create" | "oauth-continue"`
prop controlling the button label ("Create" vs "Continue") and what `onSubmit`
chains to. The launch modal gains an `autoStartAuth` initial hint that fires
`startConnect()` once on mount when set.

### Notes

- The `+ New identity` button keeps `purpose: "create"` — creates an empty
  named bundle and returns to the launch form (today's behavior).
- No backend change — `auth.start` already accepts `intoBundleId`.

---

## 3. Feature 2 — Hamburger "Identity & Memory" — a window-scoped bundle manager

### Problem

There is no app-wide home for managing bundles (§1.3). Bundles are app-wide
data; their management UI should be too.

### Behavior

- A new hamburger menu item — **"Identity & Memory"** — opens the bundle
  manager. It renders `<Modal scope="window">` (window-scoped — inerts its own
  window) **and is an app-wide singleton**: only one bundle manager exists
  across the whole AgentMux instance at a time.
- Opening it in a second window does **not** open a second modal. That window
  instead shows an *"Identity & Memory is open in <Window N> — click to
  focus"* banner; clicking it calls `focusWindow()` on the holding window. The
  second window stays otherwise fully usable.
- The modal is a single surface with two sections — **Identities** and
  **Memories** — each offering the full lifecycle:
  - **List** all bundles.
  - **Create** a new bundle (name + description; Memory adds its richer fields).
  - **Edit** name / description (+ Memory's fields; + Identity's per-provider
    bindings table).
  - **Delete** a bundle (with confirmation; running agents keep their snapshot).
- Layout: a left rail toggling Identities ⇄ Memories, with the selected
  section's list + detail to the right. (Exact layout — §5.)

### Reuse

The Identity pane (`identity-pane-view.tsx` + `identity-pane-model.ts`) and
Memory pane (`memory-view.tsx` + `memory-model.ts`) **already implement** the
full list/create/edit/delete/bindings lifecycle. Feature 2 should **not**
reimplement it. Plan:

- Extract each pane's body into a context-free component
  (`IdentityManager` / `MemoryManager`) that does not depend on the Agent-pane
  block/`nodeModel` context.
- Render those components both in the existing agent-settings tabs **and**
  inside `BundleManagerModal`.
- The models already drive off the `bundle_*` RPCs + `identitybundles:changed`
  / `memories:changed` WPS events, so two live instances (a settings tab and
  the window modal) stay consistent for free.

### Notes

- `BundleManagerModal` is a new component under `frontend/app/modals/` (or
  `frontend/app/view/bundles/`), opened via `openModal`. It receives the
  injected `close` prop and renders `<Modal scope="window" size="lg">`.
- No new RPCs.

### Singleton — one manager across the whole app

Bundle data is app-wide, and editing it concurrently in two windows invites
merge conflicts. Rather than *detect and resolve* collisions, the manager is an
**app-wide singleton** — the collision is designed out: there is only ever one
editor.

- **Holding window:** renders the manager as a normal `<Modal scope="window">`
  — its own window inert behind it.
- **Every other window:** the hamburger's "Identity & Memory" item, instead of
  opening a modal, shows a banner — *"Identity & Memory is open in <Window
  N>"* — with a button that calls `focusWindow(<label>)`. Those windows stay
  otherwise fully usable; the user keeps working in them.
- **Saved mutations still propagate.** The `identitybundles:changed` /
  `memories:changed` WPS events are unchanged — any surface elsewhere that
  *displays* a bundle (e.g. an agent-settings tab) refreshes when the manager
  saves. Only the *manager itself* is singular.

Because only one editor ever exists, the dirty-draft cross-window collision
simply cannot happen — no conflict notice, no merge logic, no last-write-wins.

**Mechanics.** This is a new capability beyond the `pane` / `tab` / `window`
scopes: those inert a DOM region within one renderer; a singleton needs
**cross-window coordination** — each window is its own CEF renderer. Needed:

1. A small app-wide registry — "which window (if any) holds the bundle
   manager." The launcher already tracks open windows (`listWindowInstances`,
   `openWindowEntriesAtom`); the singleton entry rides alongside.
2. An open/close broadcast so other windows render or clear the banner.
3. `focusWindow(<label>)` for the button — already exists (used by
   `InstancePanel`).
4. **Crash release** — if the holding window dies, the lock must auto-release.
   The launcher's window registry already detects window exit; the singleton
   entry clears on that signal, so other windows are never stranded pointing
   at a dead window.

So "instance / singleton" is effectively a fourth modal scope, but implemented
as a coordination layer over the launcher's window registry rather than as DOM
inert.

---

## 4. The consolidated bundle-management model

After Features 1 + 2, the surfaces have clear, non-overlapping roles:

| Surface | Role |
|---|---|
| **Hamburger → Identity & Memory** (window modal) | The canonical, app-wide **management** home — full CRUD for every identity & memory bundle. |
| **Launch modal** dropdowns + `+ New` + OAuth-Connect | **Quick pick / quick create** inline in the launch flow. Creates bundles that immediately appear in the manager. Feature 1 makes the OAuth quick-create explicit + named. |
| **Agent-pane settings → Identity / Memory tabs** | See §5 decision 3 — recommended: becomes a read-only "this agent uses bundle *X*" view with a button that opens the hamburger manager, so full CRUD lives in exactly one place. |

The guiding principle: **bundles are app-wide data, so there is one app-wide
place to manage them** (the hamburger window modal); every other surface is a
*consumer* (pick a bundle) or a *quick-create shortcut* (which feeds the same
store). All surfaces already share the `bundle_*` RPCs and the `*:changed` WPS
events, so consistency is automatic.

---

## 5. Decisions (resolved 2026-05-22)

1. **Hamburger label** → **"Identity & Memory"**.
2. **Manager layout** → **left-rail toggle** (Identities ⇄ Memories).
3. **Agent-settings Identity/Memory tabs** → **(b) demoted to read-only** —
   they show "this agent uses Identity: *X* / Memory: *Y*" with a button that
   opens the hamburger manager. Full CRUD lives only in the manager.
4. **Feature 1 button label** → **"Continue"**.
5. **OAuth after Continue** → **starts automatically** on return to the launch
   modal (`autoStartAuth`).
6. **Open-elsewhere UX** → a **persistent banner** in non-holding windows with
   a focus button.

---

## 6. Rollout

Three independent PRs:

1. **Feature 1** — OAuth Connect opens the New Identity modal first.
   Self-contained in the launch-modal / auth components.
2. **Manager extraction** — refactor the Identity & Memory pane bodies into
   context-free `IdentityManager` / `MemoryManager` components (no behavior
   change; the agent-settings tabs render the extracted components). Pure
   refactor — ships green with no UX change.
3. **Singleton coordination layer** — the cross-window registry (which window
   holds the manager) over the launcher window registry, the open/close
   broadcast, and the focus banner. Independently testable with a placeholder
   modal.
4. **Feature 2** — `BundleManagerModal` + the hamburger entry, composed from
   PR 2's extracted components and gated through PR 3's singleton layer.

PR 2 before PR 4 so the manager is pure composition; PR 3 before PR 4 so the
singleton gate exists when the hamburger entry ships. Feature 1 is independent
and can land any time.
