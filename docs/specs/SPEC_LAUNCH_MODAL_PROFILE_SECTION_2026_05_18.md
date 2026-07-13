# SPEC: Launch Modal — Profile Section + New Identity/Memory Modals

**Status:** Draft
**Date:** 2026-05-18
**Author:** AgentA
**Related:**
- `frontend/app/view/agent/components/AgentLaunchModal.tsx` (current Identity+Memory section)
- [`SPEC_OAUTH_IN_IDENTITY_BUNDLES_2026_05_13.md`](./archive/SPEC_OAUTH_IN_IDENTITY_BUNDLES_2026_05_13.md) (identity bundle storage)
- `frontend/app/view/agent/identity/IdentityPaneViewModel.ts` (existing identity-pane manage-connectors UI)

---

## 0. TL;DR

Three changes to the agent Launch modal:

1. **Rename the wrapping section heading** from "Identity" to **"Profile"**. The two rows underneath — **Identity** and **Memory** — keep their existing labels. Today the section's `<legend>` is "Identity" which double-uses the same word as one of its rows, so users can't tell the section apart from its contents.

2. **Add "+ New" buttons** beside each dropdown. On the empty state (no bundles of that type yet), show the New button as the only control. On the has-bundles state, show the dropdown plus the New button as a shortcut.

3. **Two new tab-scoped modals** for bundle creation:
   - **New Identity** — name + description, then opens the Identity pane focused on the new bundle for connector setup. Identity is a generic credential container (Claude / Codex / Gemini / OpenClaw OAuth + GitHub + AWS, etc.) — **not provider-scoped**.
   - **New Memory** — name + description + seed content (files / paste / empty).

---

## 1. Problem

### 1.1 Heading reads weird

```
Identity                       ← section legend
  Identity   [dropdown]        ← row 1
  Memory     [dropdown]        ← row 2
```

The legend repeats the row label. Reads as a typo. Visually unclear which level is which.

### 1.2 No way to create from the Launch modal

If the user opens the Launch modal and realizes they want a fresh Identity bundle (e.g. a "Client X" account separate from their default), they have to:
1. Cancel the Launch modal
2. Open an Agent pane
3. Click the cog → Settings → Identity tab
4. Create the bundle
5. Re-open the Launch modal
6. Pick the new bundle

That's 6 steps for what should be a one-click "+ New" affordance.

### 1.3 Empty state has no affordance

If the user has no Identity bundles, the dropdown still renders with only "— Blank (no creds) —" as an option, which doesn't communicate "you can create one." First-time users assume the feature isn't usable.

---

## 2. Design

### 2.1 Section heading: "Profile"

Single-word swap in the `<legend>` of `AgentLaunchModal.tsx:364-365`:

```diff
- <legend class="agent-launch-modal-label">Identity</legend>
+ <legend class="agent-launch-modal-label">Profile</legend>
```

Profile reads as "the persona this agent runs as" — encompassing both the credentials it uses (Identity) AND the content it remembers (Memory).

Alternative considered: "Context" — rejected because Context is overloaded in LLM-land (context window).

### 2.2 Empty state vs has-bundles state

Each row (Identity, Memory) has two render branches:

**Empty state** — the bundle list is `[]` or only contains the implicit `blank` sentinel:

```
Identity
┌─────────────────────────────────────────────────────┐
│   +  New identity bundle...                         │
└─────────────────────────────────────────────────────┘
```

Single full-width button. Tab-focusable. Click → opens the New Identity modal (§2.4).

**Has-bundles state** — at least one user-created bundle exists:

```
Identity
┌─────────────────────────────────────────┐  ┌──────┐
│  Work account                       ▾  │  │  +   │
└─────────────────────────────────────────┘  └──────┘
```

Dropdown plus inline `+` button. The `+` button is `~28×28px`, same height as the dropdown, with `aria-label="New identity bundle"`. Tooltip on hover: "New identity bundle...". Click → opens the New Identity modal.

Same shape for the Memory row.

### 2.3 Identity is generic (not provider-scoped)

**Critical:** an Identity bundle holds connectors for many things, reusable across providers:

- AI provider OAuths: Claude Code, Codex CLI, Gemini CLI, OpenClaw
- External services: GitHub (PAT or OAuth), AWS (access keys or SSO), and future-proof for Linear, Vercel, etc.

A user's "Work" identity might contain:
```
Work
├── Claude Code (OAuth, asaf@employer.com)
├── Codex CLI (OAuth, asaf@employer.com)
├── GitHub (@asafebgi PAT)
└── AWS (access key for employer's dev account)
```

The same Work bundle is selected when launching any agent — the spawn layer reads whichever connector the agent's provider needs.

### 2.4 New Identity modal

Lean — name + description only. Connector setup happens after creation in the Identity pane (which already exists per CLAUDE.md: "Tab inside an Agent pane (cog → settings panel → Identity tab)").

```
┌─ New Identity ─────────────────────────────────────────────────┐
│                                                                │
│   Name                                                         │
│   ┌─────────────────────────────────────────────────────────┐  │
│   │  Work                                                   │  │
│   └─────────────────────────────────────────────────────────┘  │
│                                                                │
│   Description (optional)                                       │
│   ┌─────────────────────────────────────────────────────────┐  │
│   │  Employer-issued credentials for AI + cloud             │  │
│   └─────────────────────────────────────────────────────────┘  │
│                                                                │
│   ─ You'll add connections (Claude, GitHub, AWS, …) in the     │
│     Identity pane after creating this bundle. ─                │
│                                                                │
│                              [ Cancel ]   [ Create ]           │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

**Submit flow:**
1. Create empty identity bundle on disk (`~/.agentmux/identity/<slug>/bundle.json`).
2. Auto-select the new bundle in the Launch modal's Identity dropdown via `tabModal.replace(launchRequest)` (passing the new bundle id).
3. Show a small toast: *"Created identity 'Work'. Add connections in the Identity pane (cog → Settings → Identity)."*

**Why not embed connector-add inside the modal?** Two reasons: (a) connector setup spans many distinct flows (OAuth, paste-token, AWS keys) that already live in the Identity pane and have nontrivial UI; (b) keeps this modal small and predictable. The "Create then configure" two-step matches the OS-level "Add Account" flow.

### 2.5 New Memory modal

Memory bundles are directories of `.md`/`.txt` files. Three seed options.

```
┌─ New Memory ───────────────────────────────────────────────────┐
│                                                                │
│   Name                                                         │
│   ┌─────────────────────────────────────────────────────────┐  │
│   │  Project Apollo notes                                   │  │
│   └─────────────────────────────────────────────────────────┘  │
│                                                                │
│   Description (optional)                                       │
│   ┌─────────────────────────────────────────────────────────┐  │
│   │  Architecture decisions + style guide for Apollo        │  │
│   └─────────────────────────────────────────────────────────┘  │
│                                                                │
│   Seed content                                                 │
│     ◉  Start empty (add files later from the Memory pane)      │
│     ○  Pick files from disk                                    │
│     ○  Paste text now                                          │
│                                                                │
│   [ ─── conditional region — only shown for chosen mode ───   ]│
│                                                                │
│   Storage                                                      │
│   ~/.agentmux/memory/project-apollo-notes/                     │
│   ─ Slug auto-derived from name ─                              │
│                                                                │
│                              [ Cancel ]   [ Create ]           │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

**Conditional region** for each seed mode:

- **Start empty** — no extra region. Submit creates the dir.
- **Pick files from disk** — file-picker button + selected-files list:
  ```
  ┌─────────────────────────────┐
  │  +  Add files...            │
  └─────────────────────────────┘

  Selected files (2)
    • CLAUDE.md          12 KB    ✕
    • architecture.md     8 KB    ✕
  ```
  Submit copies the files into the new bundle dir.
- **Paste text now** — single multi-line textarea labeled "Notes":
  ```
  ┌─────────────────────────────────────────────────────────┐
  │                                                         │
  │  (textarea, 8 rows)                                     │
  │                                                         │
  └─────────────────────────────────────────────────────────┘
  ```
  Submit writes the pasted text to `notes.md` inside the new bundle dir.

**Submit flow:**
1. Create `~/.agentmux/memory/<slug>/` (slug from name, kebab-case).
2. Write `bundle.json` with `{ name, description }`.
3. Copy files / write `notes.md` based on seed mode.
4. Auto-select via `tabModal.replace(launchRequest)`.
5. Toast: *"Created memory 'Project Apollo notes' with 2 files."*

---

## 3. Architecture

### 3.1 Frontend

| File | Change |
|---|---|
| `AgentLaunchModal.tsx` | Rename legend "Identity" → "Profile". Add empty-state vs has-bundles rendering. Wire `+` buttons to open the new modals via `tabModal.replace`. |
| `AgentNewIdentityModal.tsx` | New component. Modal-v2 chrome. Name + description fields. |
| `AgentNewMemoryModal.tsx` | New component. Name + description + seed mode + conditional region. |
| `tab-modal.ts` | New `kind` variants: `"new-identity"` and `"new-memory"`. Each carries `onSubmit(bundleId)` so the Launch modal can auto-select after creation. |
| `TabModalLayer.tsx` | Render dispatch for the two new kinds. |

### 3.2 Backend RPCs

| Command | Args | Effect |
|---|---|---|
| `identity.create_bundle` | `{ name, description }` | Create `~/.agentmux/identity/<slug>/` with empty `bundle.json`. Return `{ id }`. |
| `memory.create_bundle` | `{ name, description, seedMode: "empty"\|"files"\|"text", files?: Array<{path, content}>, text? }` | Create `~/.agentmux/memory/<slug>/` + seed. Return `{ id }`. |

Both reuse the slug helpers + bundle-listing infra that already power `list_identity_bundles` and `list_memories`.

For `seedMode: "files"`, the frontend uses the host's file picker (`getApi().showOpenDialog?.()` — if not yet present, fall back to a `<input type="file" multiple>` which CEF supports). File contents are read in the renderer (or via existing `read_file` IPC) and shipped to the backend in the RPC body — keeps the backend handler stateless and decoupled from the OS picker.

### 3.3 Flow continuity

After the New modal submits, the Launch modal must re-appear with the new bundle pre-selected. Use `tabModal.replace(buildLaunchRequest(agent, { preselectedIdentityId, preselectedMemoryId }))`:

```ts
// In AgentNewIdentityModal's onSubmit:
const { id } = await RpcApi.IdentityCreateBundleCommand(...);
tabModal.replace(buildLaunchRequest(agent, { preselectedIdentityId: id }));
```

`buildLaunchRequest` already exists in `AgentPicker.tsx` — extend it to accept preselect args that override the default `"blank"`.

---

## 4. Implementation phases

### Phase α — UI rename + New buttons + empty state

1. AgentLaunchModal: legend rename, empty/has-bundles branches, `+` buttons (placeholder click handlers).
2. SCSS: tiny styling for the `+` button (28×28, primary-on-hover).
3. New tab-modal kinds + render dispatch (panels stub to "Coming soon" body).

This phase ships immediately — visual surface only, no backend.

### Phase β — Identity creation modal

4. `AgentNewIdentityModal.tsx` with name + description form.
5. `identity.create_bundle` RPC.
6. `+` button on Identity row routes here; submit chains back to Launch via `tabModal.replace` with `preselectedIdentityId`.
7. Toast notification system reuse.

### Phase γ — Memory creation modal

8. `AgentNewMemoryModal.tsx` with name + description + seed mode radios.
9. Reuses existing `upsertmemory` RPC — paste-text mode JSON-encodes `[{path: "notes.md", content}]` into `context_files`; empty mode ships `[]`.
10. **File picker mode is deferred** to its own PR (radio disabled with "(coming soon)"). The drag-and-drop / OS file dialog integration is its own design concern; not blocking the rest of Phase γ.

### Phase δ (deferred) — Manage connections inline

A future "Connect AI provider" / "Connect GitHub" / "Connect AWS" flow could live inside the New Identity modal so the user adds connectors in one sitting. Out of scope here — the existing Identity pane handles it after creation.

---

## 5. Acceptance criteria

### Phase α
1. Section legend reads "Profile". Identity and Memory row labels unchanged.
2. No identity bundles created → only the New Identity button is visible in that row (no empty dropdown).
3. No memory bundles → only the New Memory button.
4. At least one bundle of either type → dropdown + inline `+` button.

### Phase β
5. Click `+` next to Identity (in either state) → New Identity modal opens via `tabModal.replace` (no flicker; same modal chrome).
6. Submit creates the bundle on disk and returns Launch modal with the new id pre-selected.
7. Cancel returns to Launch modal with the previously selected bundle still chosen.

### Phase γ
8. Click `+` next to Memory (in either state) → New Memory modal opens via `tabModal.replace` (no flicker; same modal chrome).
9. Empty seed mode creates a bundle with `context_files = "[]"`; paste mode writes a single `notes.md` entry containing the pasted text.
10. Submit returns Launch modal with the new memory id pre-selected; Cancel returns with the previous selection intact.
11. Pick-files radio is disabled and labelled "(coming soon)" — file-picker integration is out of scope for this PR.

---

## 6. Out of scope

- **Edit / delete bundles** from the Launch modal. Stays in the Identity / Memory panes.
- **Connector add inside the New Identity modal** (Phase δ).
- **Bundle import / export.**
- **Per-bundle settings** (avatar color, ordering).
