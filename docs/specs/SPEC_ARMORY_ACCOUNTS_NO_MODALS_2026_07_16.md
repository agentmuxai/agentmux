# SPEC — Armory Accounts: AgentMux icon (already correct) + remove modals, match single-pane page dynamics

**Status:** Draft — spec only, no code written yet (per explicit request).
**Trigger:** user request — "Inside of the armory accounts, use the Brain icon for the AgentMux .. also, we
want to get rid of the modals, and use the page dynamics u see on the other sections of the armory."
**Scope:** Armory's Accounts tab only (`AccountsManager`, `AccountsGallery`, `AccountsTab`/`AccountForm`/
`AccountDetail` in `identity-view.tsx`, `AgentMuxConnectPanel`). Builds directly on
`docs/specs/SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md` (PR #2170, merged) and
`docs/specs/SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md`'s shared `PrimitiveListDetail`
primitive — this spec is "finish the job" for the one Armory tab that primitive pass didn't touch.
**Verify before acting:** all file:line citations checked against `main` @ `4cbf856b` on 2026-07-16.

---

## 1. AgentMux icon — already the brain, no change needed

Checked every place "AgentMux" renders an icon inside Accounts:

| Location | Code |
|---|---|
| Gallery tile (`AccountsGallery.tsx:69`) | `<ProviderLogo provider={tile.id} size={32} />`, `tile.id === "agentmux"` |
| Connected-accounts row (`accounts-manager.tsx:98`) | `<ProviderLogo provider="agentmux" size={16} />` |
| "Connect AgentMux" panel header (`AgentMuxConnectPanel.tsx:209`) | `<ProviderLogo provider="agentmux" size={20} />` |

`ProviderLogo.tsx`'s provider-to-icon map (`p === "agentmux"`) already returns `brainSvg`
(`@/app/asset/logo-brain.svg`, the "brain-alternate" brand mark — code comment confirms it's
"byte-identical to the source `frontend/logos/agentmux-logo-brain-alternate.svg`"). This mapping
predates this session entirely (introduced in `feaee26f`, PR #1504 — the original AgentMux Cloud
tile feature) and every AgentMux-icon call site routes through it; there's no second, hardcoded, or
stale icon anywhere in Accounts to fix.

**If a running instance still shows something else, that's a stale build, not a code gap** — a
fresh `task dev`/rebuild against current `main` should show the brain immediately.

---

## 2. Modals in the Accounts flow — the actual work

Three distinct overlay/modal shells currently gate every Accounts action beyond the gallery+list
that PR #2170 already unified into one scrolling page (`docs/specs/SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md`):

| Flow | Trigger | Current shell |
|---|---|---|
| **Pick auth mode** (OAuth vs. Key) for a brand with no account yet | Click an empty gallery tile | `.accounts-chooser-overlay` (`AccountsGallery.tsx:82-123`) — small centered dialog, dimmed backdrop, click-outside-to-close |
| **Connect AgentMux Cloud** | Click the AgentMux tile | Same `.accounts-chooser-overlay` shell, reused verbatim by `AgentMuxConnectPanel.tsx:202-220` |
| **Add / Edit account form** | Pick a mode above, or click "Edit" on an existing account | `.identity-form-overlay` (`identity-view.tsx:642`) — full-screen dimmed backdrop wrapping `.identity-form`. Embeds `OAuthConnectPanel` inline when `mode === "oauth"` (`identity-view.tsx:733`) — not its own separate shell, moves with the form. |
| **View an existing account's detail** | Click a connected-account row | `<Modal scope="window" size="md" showCloseButton>` (`identity-view.tsx:163-173`), wrapping `AccountDetail` (`:201-345` — provider/kind/secret-backend fields, linked-agents disclosure, Reauth/Validate/Edit/Delete footer actions) |

Every other Armory tab (Bundles, Skills, MCP Servers) went through exactly this same
overlay-on-top-of-a-list shape before PR #2170/#2178, and now uses the shared
`PrimitiveListDetail` primitive (`frontend/app/element/primitive-list-detail.tsx`) instead: the
list view and the detail/form view are two mutually-exclusive full-pane states, never an overlay
floating on top of the list. Accounts is the one tab that pass didn't touch (it wasn't a
side-by-side split to begin with — it was already single-column — so it fell outside that spec's
literal "split-screen" trigger, even though it has the same "modal instead of page" issue in
spirit).

### 2.1 Proposed shape — reuse `PrimitiveListDetail`, one shared detail pane for three flows

`AccountsManager` already renders gallery + connected-list as the **list** side of a
`PrimitiveListDetail`. Add the **detail** side, populated by whichever of these three is active
(mutually exclusive, exactly one at a time — same `inDetail()`-style derived boolean the other
tabs use):

1. **Connect flow** (empty tile clicked): today's two-step overlay chain (chooser → form) becomes
   **one page**: auth-mode picker (OAuth / Key, when the brand offers both) at the top of the
   detail pane, the actual form fields (from `AccountForm`) below it, updating in place as the
   user picks a mode — no second overlay layer. AgentMux Cloud's connect flow (`AgentMuxConnectPanel`)
   folds into the same detail-pane shape (it already has no mode choice — OAuth only — so it's just
   the form-equivalent content, no picker step).
2. **View existing account** (connected row clicked): `AccountDetail`'s content (provider/kind/
   secret fields, linked-agents disclosure, footer actions) renders directly in the detail pane
   instead of inside `<Modal>`. Same read-only-view-first pattern Bundles/Skills/MCP already use.
3. **Edit existing account** (Edit clicked from #2): swaps the same detail pane into `AccountForm`'s
   edit-mode content — mirrors how Bundles' `MemoryManagerBody` already toggles between its
   read-only view and its edit form within one pane (`memory-manager.tsx`'s `model.draftAtom()` /
   `model.selectedAtom()` two-step, `startEdit()`).

Back affordation: the same `‹ Accounts` chevron-back convention `PrimitiveListDetail` already
renders, reused as-is — no new component needed for this part.

### 2.2 What changes in each file (shape only — implementer confirms exact diffs)

- **`AccountsManager` (`accounts-manager.tsx`)**: becomes the actual `<PrimitiveListDetail>` host
  (today it renders `AccountsGallery` + the connected-list section directly, with `AccountsTab`
  producing its own internal `<Modal>` — that nesting goes away). `list` = today's gallery +
  connected-accounts markup unchanged. `detail` = new content per §2.1, driven by a model-level
  "what's active" signal (new state, or reuse/extend `IdentityViewModel`'s existing
  `selectedAccountAtom`/`formOpenAtom`/`draftAtom`-shaped signals — they already model exactly
  "nothing selected vs. viewing vs. editing," just currently rendered as overlay visibility instead
  of pane content).
- **`AccountsGallery.tsx`**: loses `.accounts-chooser-overlay`/`.accounts-chooser` (§2.1 point 1
  replaces the popup with detail-pane content) but keeps the tile grid and `openTile`/`pick`
  click-handling logic — those become "set the active detail flow," not "open an overlay."
- **`identity-view.tsx`**: `AccountsTab`'s `<Modal>` wrapper around `AccountDetail` goes away
  (§2.1 point 2) — `AccountDetail` itself (the actual field-rendering component) is reusable
  as-is, just mounted directly in the detail pane instead of inside `ModalHeader`/`ModalBody`/
  `ModalFooter`. `AccountForm`'s `.identity-form-overlay` wrapper goes away (§2.1 points 1 and 3)
  — `.identity-form`'s inner content (the actual fields, including the embedded
  `OAuthConnectPanel`) is reusable as-is.
- **`AgentMuxConnectPanel.tsx`**: loses its `.accounts-chooser-overlay`/`.accounts-chooser` wrapper
  (§2.1 point 1) — its inner content (the OAuth-only connect flow, config-missing fallback note)
  is reusable as-is.
- **`IdentityPanel`** (`identity-view.tsx:57-98`, the *other* consumer of `AccountsTab`/`AccountForm`
  — the per-agent settings surface, distinct from Armory, per CLAUDE.md's "Identity" vs.
  "Identities" table entry): **shares `AccountForm`'s `.identity-form-overlay` and
  `AccountsTab`'s `<Modal>`-wrapped `AccountDetail`.** Same scope-boundary situation as
  `SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md` §5 flagged for the Agent Setup Modal's
  Skills/MCP tabs — removing the shared overlay/modal markup without also converting this
  consumer would leave it broken. **Recommend the same resolution as that spec: convert this
  consumer too** (no evidence its modal is intentionally different from Armory's), flagging
  explicitly rather than assuming, per this codebase's established convention on scope decisions.

### 2.3 What doesn't change

- `AccountsGallery`'s tile grid, badge counts, `SERVICE_CATALOG` data, and click routing logic.
- `AccountForm`'s and `AccountDetail`'s actual field content, validation, and RPC calls — pure
  layout-shell change, no data-flow change (same principle as
  `SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md` §4.4).
- `OAuthConnectPanel` — already embedded inline, not its own shell; unaffected either way.
- The shared `Modal`/`ModalHeader`/`ModalBody`/`ModalFooter` primitives themselves
  (`@/app/element/modal`) — still used elsewhere in the app; this spec only stops *this* tab from
  using them, doesn't touch the primitive.

---

## 3. Test coverage to add

- Clicking an empty gallery tile shows the connect flow in the detail pane (not an overlay); the
  list (gallery + connected accounts) is not visible at the same time.
- Clicking a connected account row shows its read-only detail in the pane; clicking Edit swaps to
  the form in the same pane without a page/overlay transition.
- The back affordation returns to the list from any of the three detail states.
- `IdentityPanel` (the per-agent, non-Armory consumer) still functions after the shared-component
  change — confirm during implementation, not assumed by this spec (see §2.2's `IdentityPanel` note).

---

## 4. Suggested PR split

1. **PR A** — `AccountsManager`/`AccountsGallery`/`identity-view.tsx` conversion to
   `PrimitiveListDetail`, covering both the Armory Accounts tab and the `IdentityPanel` consumer
   together (they share the exact same components being changed — splitting them would mean an
   in-between commit where one consumer is broken).
2. **PR B** (optional, only if PR A's diff is large) — `AgentMuxConnectPanel`'s overlay removal,
   if not folded into PR A already.

---

## 5. Sources

- `frontend/app/view/accounts/accounts-manager.tsx`, `AccountsGallery.tsx`, `AgentMuxConnectPanel.tsx`,
  `OAuthConnectPanel.tsx`, `accounts-catalog.ts`
- `frontend/app/view/identity/identity-view.tsx`, `identity-model.ts`
- `frontend/app/element/ProviderLogo.tsx`, `frontend/app/asset/logo-brain.svg`
- `frontend/app/element/primitive-list-detail.tsx`
- `docs/specs/SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md` (the pattern this spec extends)
- `frontend/app/element/modal.tsx` (`Modal`/`ModalHeader`/`ModalBody`/`ModalFooter` — shared
  primitive, not modified by this spec)
