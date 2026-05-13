# Spec: Settings Modal UI

**Status:** Draft v2 (revised after cleanup audit)
**Author:** agent2
**Date:** 2026-05-11
**Depends on:** `specs/settings-cleanup.md` (v2)
**Scope:** Replace the "open settings.json in your editor" flow with an in-app modal.

> **Important:** This spec assumes `specs/settings-cleanup.md` (v2) lands first. The cleanup removes ~29 dead keys including the entire `ai:*`, `autoupdate:*`, `editor:*`, `markdown:*` namespaces. References below to those namespaces have been pruned. The modal covers only the 25 keys that survive cleanup.

---

## Goal

Today, the only way for a user to change settings is to hit hamburger menu (≡) → Settings, which runs `ensure_settings_file` → `open_in_editor` and dumps them into their default text editor with a JSONC template. This is a poor UX for a desktop product:

- Users have to know what JSON is and how to uncomment a line.
- Typos silently fail.
- No discoverability — they don't see which settings exist unless they read the template.
- No grouping, no validation, no live preview, no defaults reset.

This spec replaces the text-file flow with a tabbed modal, reusing the existing `modal-v2` primitive and the unused `SetConfigCommand` RPC.

The settings.json file remains the source of truth on disk (the modal writes through to it); power users can still hand-edit it.

---

## Existing Infrastructure (Confirmed)

All of the building blocks already exist:

| Capability | Where | Notes |
|---|---|---|
| Modal primitive | `frontend/app/element/modal-v2.tsx` | ARIA, stackable, ESC/backdrop dismiss, focus trap, sizes `sm`/`md`/`lg`/`xl`/`fit` |
| Modal state | `frontend/app/store/modalmodel.ts` | `modalsModel.pushModal(displayName, props)` / `popModal()` |
| Modal registry | `frontend/app/modals/modalregistry.tsx` | Add `SettingsModal` here |
| Modal styling | `frontend/app/element/modal-v2.scss` | Uses theme tokens (`--z-modal`, `--shadow-modal`, etc.) |
| Read settings | `frontend/app/store/global.ts:86-88` — `settingsAtom`, `getSettingsKeyAtom(key)` | Already reactive |
| **Write settings (unused)** | `frontend/app/store/rpc-api.ts` — `RpcApi.SetConfigCommand(client, data, opts)` | Wraps `setconfig` RPC; called only by `tabbar.tsx:610` today for `window:theme` |
| JSON Schema | `schema/settings.json` | Per-key types |
| Rust schema | `agentmux-srv/src/backend/wconfig/types.rs` | Authoritative defaults |
| Existing form controls | `frontend/app/element/{input,toggle,multilineinput}.tsx` | Need to add: number, dropdown, color, key-binding capture, dict editor |
| Existing tabbed inline UI | `frontend/app/view/agent/components/AgentCardSettingsPanel.tsx` | Reference for tab pattern (Identity/Memory tabs) |

**Key gap:** there is no central "settings registry" describing each key's type/widget/group/label. We'll build one (see §4).

---

## 1. UX Overview

### 1.1 Entry Points

| From | Action |
|---|---|
| Hamburger menu (≡) → **Settings** | Replace current `dev:open_settings` command behavior to open the modal instead of the text file. Keep a secondary entry `Settings → Open settings.json…` for power users. |
| Command palette → `Settings…` | New command, opens the modal. |
| Keyboard shortcut | `Cmd/Ctrl + ,` (standard convention). Add to `keymodel.ts`. |

### 1.2 Layout

```
┌─────────────────────────────────────────────────────────────────┐
│  Settings                                                  [×]  │
├──────────────┬──────────────────────────────────────────────────┤
│              │  Appearance                                      │
│ Appearance ▸ │  ─────────────────────────                       │
│ Terminal     │  Theme           [ default-dark   ▾ ]            │
│ Agent        │  Transparent     [○ off]                         │
│ App          │  Blur            [○ off]                         │
│ Shell Env    │  Opacity         [────●────────] 1.00            │
│ Sysinfo      │  Background      [#______]                       │
│ Network      │  Tile gap        [ 3 ▴▾] px                      │
│              │  Reduce motion   [○ off]                         │
│              │  Magnified opacity [────●────] 0.60              │
│              │  Magnified size  [────●────────] 0.90            │
├──────────────┴──────────────────────────────────────────────────┤
│  [Reset section] [Open settings.json]          [Cancel] [Save]  │
└─────────────────────────────────────────────────────────────────┘
```

- **Modal size:** `lg` (720px) per `modal-v2` sizes.
- **Left rail:** vertical tab list; selected tab is highlighted with the accent color. Keyboard: ↑/↓ to move, Enter to focus first control on the right.
- **Right pane:** scrollable form, one section per tab.
- **Search bar (Phase 2, optional):** top of the modal — fuzzy search across labels/keys, jumps to and highlights the matching control.
- **Footer actions:** Reset section, open raw file (power-user escape hatch), Cancel, Save.

### 1.3 Tab → Keys Mapping

Seven tabs. Every key in the modal corresponds to a real read site in the codebase (post-cleanup).

| Tab | Keys |
|---|---|
| **Appearance** | `window:theme`, `window:transparent`, `window:blur`, `window:opacity`, `window:bgcolor`, `window:tilegapsize`, `window:reducedmotion`, `window:magnifiedblockopacity`, `window:magnifiedblocksize` |
| **Terminal** | `term:fontsize`, `term:fontfamily`, `term:theme`, `term:scrollback`, `term:copyonselect`, `term:transparency`, `term:localshellpath`, `term:localshellopts`, `term:disablewebgl`, `term:allowbracketedpaste`, `term:shiftenternewline` |
| **Agent** | `term:agentmaxruntimehours`, `term:agentidletimeoutmins` |
| **App** | `app:defaultnewblock`, `app:showoverlayblocknums`, `widget:icononly`, `blockheader:showblockids` |
| **Shell Env** | `cmd:env` (dict editor) |
| **Sysinfo** | `sysinfo:interval`, `sysinfo:numpoints` *(renamed from `telemetry:*` in cleanup PR)* |
| **Network** | `network:lan_discovery` |
| **Advanced** *(collapsed by default)* | `window:magnifiedblockblurprimarypx`, `window:magnifiedblockblursecondarypx` |

No Updates tab (no updater). No Telemetry tab (no telemetry — the polling settings live under Sysinfo with honest naming). No AI / Editor / Markdown tabs (those namespaces are removed in cleanup).

> **Future-proofing:** when an agent-pane preference is actually needed (e.g. font size for the agent conversation view), add a new `agent:*` key with a concrete reader and a new Agent-pane subsection. **Don't pre-stage empty tabs.**

---

## 2. Save / Apply Semantics

### 2.1 Three options considered

| Option | Behavior | Pros | Cons |
|---|---|---|---|
| **A: Apply on save** (form pattern) | User edits a draft, hits Save, modal calls `SetConfigCommand` once | Familiar; explicit confirmation; Cancel works cleanly | No live preview; user can't see effect of changes until Save |
| **B: Apply on change** (preferences pattern) | Each control commits immediately; modal has only Close | Live preview matches macOS/VS Code | Cancel is meaningless; an undo stack is needed for "I changed my mind" |
| **C: Hybrid** | Visual settings (theme, opacity, zoom, font sizes) apply on change; everything else applies on Save | Best of both | More complexity; need per-key annotation `applyOnChange: true` |

**Recommendation: Option C (Hybrid).**

- Settings that are visual and reversible (`window:theme`, `window:opacity`, `term:fontsize`, etc.) apply on change so the user sees the result.
- Settings that affect behavior (`network:lan_discovery`, `term:localshellpath`, `cmd:env`, the agent watchdog keys) require explicit Save.
- The registry (§4) marks each key with `applyMode: "live" | "save"`.
- Cancel reverts any live-applied changes back to the snapshot taken when the modal opened.

### 2.2 Write path

```ts
// On Save (or per-change for live keys):
await RpcApi.SetConfigCommand(TabRpcClient, draftSettings);
// Backend persists to ~/.agentmux/settings.json, broadcasts wave event,
// frontend re-fetches via existing flow.
```

Open question: `SetConfigCommand` may currently expect a *full* `SettingsType` or a *patch*. The tabbar usage at `frontend/app/tab/tabbar.tsx:610` sends a single-key object — confirm before implementation whether the backend merges patches or replaces wholesale. **If wholesale**, the modal must send the full merged settings on save; **if patch**, the modal can send the delta.

### 2.3 External edits

If the user has the settings.json open in an editor and modifies it while the modal is open, the backend file watcher fires a wave event and `fullConfigAtom` updates. The modal should:

1. Detect the external change (compare `settingsAtom()` to the snapshot taken at modal open).
2. Show a non-blocking banner: *"settings.json was modified outside this window. [Reload]"*
3. **Do not** auto-reload — that would silently discard the user's pending edits.

---

## 3. Validation

Three layers, increasing strictness:

1. **Client-side, per-control.** Number inputs enforce min/max. Color pickers enforce hex. Path inputs trim whitespace. Use the JSON Schema (`schema/settings.json`) as the source of constraints — generate TypeScript types/validators at build time, or hand-mirror the constraints in the registry (§4).
2. **Form-level, on Save.** Run the draft through a schema validator (`ajv` is already a common pick for SolidJS apps; verify dependency before adding). Surface errors next to the offending control + a footer summary.
3. **Backend, on `SetConfigCommand`.** Backend should reject malformed settings with a structured error. **Required:** backend currently does not return field-level errors — add a `{ ok: false, errors: [{ key, message }] }` shape so the modal can map errors back to controls.

For now (Phase 1), do layer 1 + layer 3 only. Defer the full ajv pass to Phase 2.

---

## 4. Settings Registry

The single missing piece. Create `frontend/app/settings/settings-registry.ts`:

```ts
export type SettingControl =
    | { kind: "toggle" }
    | { kind: "number"; min?: number; max?: number; step?: number; unit?: string }
    | { kind: "slider"; min: number; max: number; step?: number }
    | { kind: "text"; placeholder?: string }
    | { kind: "path" }              // text + "Browse…" button (uses backend file dialog)
    | { kind: "select"; options: { value: string; label: string }[] }
    | { kind: "color" }
    | { kind: "keybinding" }        // captures a keystroke
    | { kind: "stringList" }        // term:localshellopts
    | { kind: "dict" };             // cmd:env

export type SettingDef = {
    key: keyof SettingsType;        // typed against gotypes.d.ts SettingsType
    label: string;                  // user-facing
    description?: string;           // tooltip / inline help
    control: SettingControl;
    default: unknown;               // mirrors Rust serde defaults
    applyMode: "live" | "save";
    tab: SettingsTab;
    advanced?: boolean;
};

export const SETTINGS_REGISTRY: SettingDef[] = [
    {
        key: "window:theme",
        label: "Theme",
        description: "Color scheme for the app chrome.",
        control: { kind: "select", options: [
            { value: "default-dark", label: "Dark" },
            { value: "default-light", label: "Light" },
        ]},
        default: "default-dark",
        applyMode: "live",
        tab: "appearance",
    },
    // … one entry per key in the §1.3 mapping
];
```

This registry becomes the single source of truth for the modal. **Critically**: keep its `default` values in sync with Rust serde defaults. Add a CI check (or at minimum a `task verify:settings-defaults` script) that diffs the registry defaults against `wconfig/types.rs`.

---

## 5. New Form Controls Needed

Existing controls cover only ~50% of the settings. Add to `frontend/app/element/`:

| Control | Use cases | Notes |
|---|---|---|
| `NumberInput` | `*:fontsize`, `*:scrollback`, etc. | Up/down spinner, min/max, integer/float. |
| `Slider` | `window:opacity`, `*:transparency`, `window:magnifiedblock*` | 0..1 range with live label. |
| `Select` (dropdown) | `window:theme`, `term:theme` | Native `<select>` is fine; style with theme tokens. |
| `ColorPicker` | `window:bgcolor` | Native `<input type="color">` with hex fallback. |
| `PathInput` | `term:localshellpath` | Text + "Browse…" button → backend RPC for native file dialog. **Verify** that a `dialog:openFile` RPC exists; if not, ship as plain text input in Phase 1. |
| `StringListEditor` | `term:localshellopts` | Tag-style chips, add/remove. |
| `DictEditor` | `cmd:env` | Key/value rows, add/remove, sorted by key. |

(No `KeybindingCapture` needed — `app:globalhotkey` was removed in cleanup as unused.)

Each lives in its own file, follows the existing pattern (controlled component with `value`/`onChange` props), and uses SCSS modules under `frontend/app/element/`.

---

## 6. Settings Modal Component

```
frontend/app/modals/settings.tsx
frontend/app/modals/settings.scss
frontend/app/settings/
  ├── settings-registry.ts
  ├── settings-tabs.ts           // tab ids + labels
  ├── use-settings-draft.ts      // hook: snapshot + draft + apply/revert
  └── controls/
      ├── number-input.tsx
      ├── slider.tsx
      ├── select.tsx
      ├── color-picker.tsx
      ├── path-input.tsx
      ├── string-list-editor.tsx
      └── dict-editor.tsx
```

Skeleton:

```tsx
// frontend/app/modals/settings.tsx
export function SettingsModal() {
    const initial = settingsAtom();                  // snapshot when modal opens
    const [draft, setDraft] = createStore({...initial});
    const [activeTab, setActiveTab] = createSignal<SettingsTab>("appearance");
    const [errors, setErrors] = createSignal<Map<string, string>>(new Map());

    const onChange = (key: string, value: unknown) => {
        setDraft(key, value);
        const def = SETTINGS_REGISTRY.find(d => d.key === key)!;
        if (def.applyMode === "live") {
            RpcApi.SetConfigCommand(TabRpcClient, { [key]: value } as any);
        }
    };

    const onSave = async () => {
        const dirty = diff(initial, draft);
        if (!dirty) return modalsModel.popModal();
        try {
            await RpcApi.SetConfigCommand(TabRpcClient, dirty as SettingsType);
            modalsModel.popModal();
        } catch (e) {
            setErrors(mapBackendErrors(e));
        }
    };

    const onCancel = () => {
        // revert any live-applied keys
        const liveDirty = diffLiveOnly(initial, draft);
        if (liveDirty) RpcApi.SetConfigCommand(TabRpcClient, liveDirty);
        modalsModel.popModal();
    };

    return (
        <Modal open size="lg" onClose={onCancel}>
            <SettingsLayout
                tabs={TABS}
                active={activeTab()}
                onTabChange={setActiveTab}
                draft={draft}
                errors={errors()}
                onChange={onChange}
            />
            <SettingsFooter
                onResetSection={() => resetTab(activeTab())}
                onOpenFile={() => invokeCommand("dev:open_settings_file")}
                onCancel={onCancel}
                onSave={onSave}
            />
        </Modal>
    );
}
```

Register in `modalregistry.tsx`:

```ts
const modalRegistry = {
    AboutModal,
    UserInputModal,
    MessageModal,
    CommandPaletteModal,
    SettingsModal,    // ← new
};
```

---

## 7. Migration & Rollout

### Phase 1 (this spec)
- Settings registry + form controls.
- `SettingsModal` covering all non-Advanced tabs.
- Hamburger ≡ → Settings opens the modal.
- Old `dev:open_settings_file` command kept and reachable from the modal footer ("Open settings.json").
- Hybrid apply (live + save) per the registry.

### Phase 2
- Advanced tab.
- Search bar across all settings.
- Inline schema validation via ajv.
- Field-level backend error mapping.

### Phase 3 (optional)
- Per-workspace overrides UI.
- Settings sync (cloud-backed).
- Diff view (show me what I'm about to change).

### Compatibility
- `settings.json` on disk is unchanged in format — modal is a UI on top of the existing file.
- Power users who hand-edit retain that workflow; the modal reads the file on open and writes through it on save.
- No version bump *required* (the change is additive), but the modal landing is a notable enough UX shift to deserve a minor version (`bump minor`).

---

## 8. Testing

| Layer | What | Where |
|---|---|---|
| Unit | Each control component (NumberInput min/max, DictEditor add/remove, etc.) | `frontend/app/element/__tests__/` |
| Unit | Registry default validity (every default conforms to its control's constraints) | `frontend/app/settings/__tests__/registry.test.ts` |
| Unit | `useSettingsDraft` snapshot/revert | `frontend/app/settings/__tests__/draft.test.ts` |
| Integration | Open modal → toggle a live key → confirm `SetConfigCommand` fires | Vitest + mocked RPC |
| Integration | External file change while modal open → banner appears | Vitest + simulated wave event |
| E2E | Hamburger → Settings → change theme → close → reopen → theme persists | `npm test -- app.e2e.test.ts` |

---

## 9. Risks & Mitigations

| Risk | Mitigation |
|---|---|
| `SetConfigCommand` semantics (patch vs. replace) unclear | Confirm with backend code review before Phase 1 starts; document in registry. |
| Defaults drift between Rust and registry | Add a `task verify:settings-defaults` check; fail CI on mismatch. |
| Live-apply for some keys (e.g. `window:transparent`) requires a window restart in current code | Mark those keys as `applyMode: "save"` and show a "Requires restart" badge. |
| File-dialog RPC may not exist for `PathInput` | Phase 1 ships `PathInput` as plain text; add Browse… in Phase 2. |
| Tab navigation conflicts with command palette keybinding | Reuse the `disableGlobalKeybindings()` pattern from command-palette.tsx while modal is open. |

---

## 10. Out of Scope

- Per-connection settings (live in `connections.json`, not `SettingsType`).
- Theme editor — only theme *selection* is in scope.
- Settings import/export — defer.
- Settings sync across machines — defer.

---

## Open Questions

1. **Patch vs. replace** on `SetConfigCommand` — confirm before Phase 1.
2. **Search bar** — Phase 1 or Phase 2? Recommendation: Phase 2; the left rail is enough for ~60 keys.
3. **Reset behavior** — "Reset this section" or "Reset all"? Recommendation: both, with a confirmation dialog (push a second modal on top — modal-v2 already supports stacking).
4. **Where to keep the "Open settings.json" escape hatch** — modal footer (proposed) or only in the command palette? Recommendation: modal footer; the goal is to make the modal the obvious path while still respecting power users.
