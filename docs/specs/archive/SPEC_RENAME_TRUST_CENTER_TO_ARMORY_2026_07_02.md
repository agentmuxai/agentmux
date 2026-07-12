# Spec: Rename "Trust Center" → "Armory" (full migration)

> **Archived 2026-07-12.** Historical — the rename shipped exactly as specced (PR #1917). Consolidated tracking: issue #2024.

**Date:** 2026-07-02
**Decision:** Name is **Armory** (chosen over Loadout). See `SPEC_TRUST_CENTER_RENAME_2026_07_02.md` for the rationale.
**Type:** User-facing rename + view-key migration. Behavior-preserving.
**Precedent to follow:** the earlier Drone pane rename — same shape (renamed a first-class pane, migrated the persisted `view` key via a block-dispatch shim, renamed the widget). Mirror that approach.

---

## Scope decision

Rename **everything user-facing plus the frontend view layer** to Armory. **Keep the identity/account/bundle backend internals** (`account.*` / `identity.*` RPC commands, `db_identity_accounts` / `db_identity_bundles` tables) — they are not user-visible, "account/identity" is still the correct domain vocabulary, and renaming them is pure churn with migration cost. This mirrors the accepted "internal key ≠ display name" pattern already used for Presets (`db_memory_bundles`) and Drone (DAG tables kept their names).

The four tab labels stay: **Accounts / Identities / Brain / Presets.** Only the umbrella ("Trust Center" → "Armory") changes.

---

## A. The view-key migration (the only stateful part)

Existing user panes persist `meta.view: "trust"` in the block store. Do NOT rewrite block metadata in SQLite; redirect at the view-dispatch layer, exactly like `forge`/`workflows`.

**`frontend/app/block/block.tsx`** (`makeViewModel`, ~line 48):
```ts
let effectiveView = blockView;
if (effectiveView === "forge") effectiveView = "agent";
if (effectiveView === "workflows") effectiveView = "drone";
if (effectiveView === "trust") effectiveView = "armory";   // ← ADD (Armory rename)
```
Add a comment mirroring the existing Drone-rename note. This makes every persisted `"trust"` pane resolve to the new Armory view model. **No SQLite migration needed.**

---

## B. View registration + routing

| File | Change |
|------|--------|
| `frontend/app/block/block-registry.ts` (line 20, 42) | `import { TrustViewModel } from "@/app/view/trust/trust"` → `ArmoryViewModel from "@/app/view/armory/armory"`; `blockViewRegistry.set("trust", …)` → `.set("armory", ArmoryViewModel)` |
| `frontend/app/store/command-registry.ts` (~line 375) | `openOrFocusPaneByView("trust")` → `openOrFocusPaneByView("armory")` (and rename the command id/title if it says "Trust Center") |
| `frontend/app/window/hamburger-menu.tsx` (~line 65) | label `"Trust Center"` → `"Armory"`; `openOrFocusPaneByView("trust")` → `"armory"`. Consider a new icon (see §F) |

---

## C. Widget config

**`agentmux-srv/src/config/widgets.json`** (~line 208–216):
- Key `"defwidget@trust"` → `"defwidget@armory"`
- `"label": "Trust Center"` → `"Armory"`
- `"view": "trust"` → `"view": "armory"`
- Refresh description if desired (currently "Manage accounts, identities, brain, and presets" — still accurate).

**Widget-key persistence caveat (check the Drone precedent):** if a user pinned/reordered this widget, the pinned-state may be keyed on `defwidget@trust` in user settings. The Workflows→Drone rename hit the same issue — replicate whatever it did (either a settings migration mapping `defwidget@workflows`→`defwidget@drone`, or accept that the widget returns to default pinned state). Grep `git log -S "defwidget@workflows"` to find that handling and copy it. If nothing special was done, note in the PR that the Armory widget resets to default pinned state (acceptable, cosmetic).

---

## D. File + symbol renames (frontend view layer)

Rename the directory `frontend/app/view/trust/` → `frontend/app/view/armory/`:

| Old | New |
|-----|-----|
| `view/trust/trust.tsx` | `view/armory/armory.tsx` |
| `view/trust/trust-model.ts` | `view/armory/armory-model.ts` |
| `view/trust/trust-view.tsx` | `view/armory/armory-view.tsx` |
| `view/trust/trust-view.scss` | `view/armory/armory-view.scss` |

Symbol renames (pure, no persisted-state impact):
- `TrustViewModel` → `ArmoryViewModel`
- `TrustView` → `ArmoryView`
- `TrustSection` (type) → `ArmorySection`
- `viewName = () => "Trust Center"` → `"Armory"` (in armory-model.ts)
- Update the `@use`/`import` paths and the `import "./trust-view.scss"` → `"./armory-view.scss"`.

Use `git mv` so history follows, then update all importers (block-registry.ts is the main one).

**CSS class note:** `trust-view.scss` uses legacy `.bundle-manager-*` class names (rail, tab-bar) — these are NOT "trust"-named and are pre-existing legacy from when this was a "bundle manager". Leave them unless doing a separate cleanup; renaming them is out of scope (and see the `_bundle` cleanup follow-up). The `aria-label="Trust Center section"` strings in `armory-view.tsx` (2×) → `"Armory section"`.

---

## E. User-facing strings elsewhere (must change)

| File | String |
|------|--------|
| `frontend/app/view/agent/failure/failure-accessory.ts` (~line 104, 128) | `"Trust Center → Accounts"` → `"Armory → Accounts"`; `"Trust Center (switch / upgrade)"` → `"Armory (switch / upgrade)"`; comment `/** Open Trust Center → Accounts. */` → Armory |
| `frontend/app/view/agent/failure/failure-accessory.test.ts` (~line 84, 110, 112) | Update the expected label strings to match (test asserts on them) |
| Handler name `on.trustCenter` (failure-accessory + useAgentFailure.ts) | Optional: rename to `on.armory` for consistency (internal; rename both sides together) |

Run `npx vitest run failure-accessory` after — it asserts on these exact labels.

---

## F. Icon (optional but recommended)

Current hamburger + widget icon is `"id-card"`. For "Armory", a more fitting Font Awesome glyph: `"shield-halved"`, `"vault"`, or `"box-archive"`. Pick one and use it consistently in `hamburger-menu.tsx` + `widgets.json`. Keep `id-card` if minimal change is preferred.

---

## G. Comments / non-user-facing references (sweep, low priority)

~15 files in `frontend/app/view/accounts/*` and `rpc-api/identity.ts` have `// … Trust Center …` comments and doc-comments. These don't affect behavior. Recommend a bulk find-and-replace of "Trust Center" → "Armory" in comments for consistency, in the SAME PR (keeps the codebase coherent). Do NOT touch spec filenames or historical spec bodies in `docs/specs/SPEC_TRUST_CENTER_*` — those are historical record.

Grep to enumerate: `grep -rn "Trust Center" frontend agentmux-srv --include=*.ts --include=*.tsx`.

---

## H. Docs

- **`CLAUDE.md`** (root): the "Not widgets" table's **Presets** row says "Trust Center tab (hamburger → Identity & Memory → Presets)". Update "Trust Center" → "Armory". Also any other "Trust Center" mention.
- Leave `docs/specs/SPEC_TRUST_CENTER_*` historical specs unchanged (record of the original design). This new spec + the rename proposal document the transition.

---

## I. Backend: explicitly OUT of scope (keep as-is)

- RPC commands: `account.key.verify`, `account.oauth.*`, `identity.*`, `preset.*`, `memory.*` — unchanged.
- DB tables: `db_identity_accounts`, `db_identity_bundles`, `db_identity_bindings`, `db_memory_bundles` — unchanged.
- Rust identity modules (`agentmux-srv/src/identity/*`, `storage/identities.rs`) — the "Trust Center" mentions there are in comments only; sweep them per §G if desired, but no logic/identifier changes.

Rationale: zero user benefit, migration cost, and would churn the rpc-contract baseline. The rpc-contract test (`test/contract/rpc-contract.test.ts`) keys on the wire command names — leaving RPC names alone means **no contract-test churn**.

---

## Execution checklist (single PR)

1. Add `trust → armory` shim in `block.tsx` (§A).
2. `git mv` the `view/trust/` dir → `view/armory/`; rename files (§D).
3. Rename symbols `Trust*` → `Armory*` + `viewName` string (§D).
4. Update `block-registry.ts`, `command-registry.ts`, `hamburger-menu.tsx` (§B).
5. Update `widgets.json` key/label/view (§C) + check widget-pin migration vs Drone precedent.
6. Update failure-accessory strings + test (§E).
7. Optional icon change (§F).
8. Sweep "Trust Center" comments → "Armory" (§G).
9. Update `CLAUDE.md` (§H).
10. Verify: `npx tsc --noEmit` clean; `npx vitest run` green (esp. failure-accessory + rpc-contract); build passes. Manually open Armory from hamburger + confirm an existing `"trust"` pane still opens (shim).
11. Changeset: `task changeset -- minor "feat(ui): rename Trust Center to Armory"`. PR under Agent1 identity with `<!-- agentmux:agent_id=agent1 -->`.

## Risk: **Low.** The only stateful concern (persisted `view: "trust"` panes) is handled by the dispatch shim — proven by the forge/workflows precedent. Everything else is string/symbol/file renames. No SQLite migration, no RPC/contract churn.
