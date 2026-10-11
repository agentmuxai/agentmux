# SPEC: Closing a tab asks for confirmation only when the user turns it on

**Date:** 2026-10-10
**Status:** implemented — #4662.
**Author:** Korp (narko), at the owner's request: "have the close tab modal default to disabled with a setting available in settings. Currently it defaults to show. We dont want it default anymore, but users can still enable it, write spec to file".
**Affects:** `frontend/app/tab/tabbar.tsx`, `frontend/app/view/settings/sections/window-panes-section.tsx`, `schema/settings.json`, `settings-template.jsonc`, `frontend/types/srv-types.d.ts`.
**Builds on:** `SPEC_TAB_UI_REFINEMENTS_2026_06_20.md` (the modal), `SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md` §8 and §10 (how a close hides the tab).

## 1. Today

Closing a window tab (its ✕, Ctrl+W on the last pane, or the menu) opens `TabCloseConfirmModal` unless `tab:skipcloseconfirm` is `true`. The key is unset by default, so everyone gets the modal until they tick its "Don't ask again" box or turn on **Settings → Window & Panes → Skip tab close confirmation**.

## 2. Change

The modal becomes opt-in.

| | Before | After |
|---|---|---|
| Key | `tab:skipcloseconfirm` (skip when `true`) | `tab:confirmclose` (ask when `true`) |
| Default | unset: **ask** | unset: **don't ask**, the tab closes at once |
| Settings → Window & Panes | "Skip tab close confirmation" | "Confirm before closing a tab" (off) |
| The modal's "Don't ask again" | sets `tab:skipcloseconfirm: true` | sets `tab:confirmclose: false` |

- **A positive key, not the old one with a new default.** "Skip … = true by default" reads backwards in Settings and in `settings.json`, and an unset key would mean the opposite of what it used to. `tab:confirmclose` matches the existing `window:confirmclose`.
- **srv needs no change.** Unknown settings keys pass through `SettingsType::extra` to the frontend, as `tab:skipcloseconfirm` always has.

## 3. The old key

`tab:skipcloseconfirm` is no longer read.

| What a user had | What happens |
|---|---|
| Never set it (the default) | No modal any more. This is the requested change. |
| `true` ("Don't ask again", or the old toggle on) | No modal, as before. |
| `false`, written by hand (the UI never wrote `false`) | No modal. Turn on **Confirm before closing a tab** to get it back. |

The schema keeps the old key, marked deprecated, so a `settings.json` that still has it doesn't show a validation warning. The template and the Settings pane no longer show it.

## 4. Unchanged

- What a close does once confirmed, or without a confirmation: the optimistic hide and the "never close the last visible tab" guard (`SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md` §8, §10).
- Closing a window (`window:confirmclose`) and closing a pane with running processes (`SPEC_PANE_CLOSE_CONFIRM_NAMES_PROCESSES_2026_09_23.md`) keep their own confirmations.

## 5. Tests

- Tab bar: with `tab:confirmclose` unset or `false`, a close closes at once and no modal opens; with `true`, the modal opens; "Don't ask again" writes `tab:confirmclose: false`.
- Settings defaults (`settings-defaults.test.ts`): `tab:confirmclose` defaults to `false` in the schema, the template, the Settings control and the tab bar.
