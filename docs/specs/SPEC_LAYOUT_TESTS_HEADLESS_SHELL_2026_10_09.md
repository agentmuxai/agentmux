# Layout tests run in Chrome's headless shell, not the Chrome app

**Status:** implemented — in #4565.
**Date:** 2026-10-09.
**Requested by:** repo owner (asafebgi): "macOS sometimes will automatically pin icons to the launcher of certain apps … it appears to have docked 3 copies of a chrome shortcut"; "ok, lets stop it for good"; "write a spec to file, then implement".
**Author:** Masty.
**Related:** `frontend/app/view/agent/components/layout-browser.ts`, `PreviewLines.layout.test.tsx`, `MyAgentsList.layout.test.tsx`; `docs/reports/REPORT_TOOL_PREVIEW_TEXT_PIPELINE_2026_10_08.md` §7.

## 1. Problem

On a Mac, the Dock showed three extra "Google Chrome" icons beside the pinned one. They are in the Dock's recent apps (`com.apple.dock` `recent-apps`), not pinned. All three point at `/Applications/Google Chrome.app` with tile type `1`. That's the type the Dock records for a running copy of an app it didn't launch itself; the pinned tile is type `41`.

They come from the repo's real-browser layout tests. `layout-browser.ts` (and, before it, `MyAgentsList.layout.test.tsx`) runs the Chrome app's binary directly, with a temporary `--user-data-dir`. macOS gives each such launch its own Dock tile while it runs and then keeps it in recent apps. Every agent that runs the frontend suite on the Mac adds to them; before #4512 the My Agents test also kept each Chrome running for its 60-second timeout.

The AgentMux app itself is not involved: it renders with its own embedded CEF, has its own bundle and Dock tile, and never starts the user's Chrome.

## 2. Change

`findBrowser()` in `layout-browser.ts` picks, in order:

1. `AGENTMUX_TEST_BROWSER`, when set (unchanged: an explicit choice wins).
2. **Chrome's headless shell** (`chrome-headless-shell`), a separate command-line build of Chrome made for headless use, which has no app bundle and so no Dock tile. Where it's looked for:
   - on `PATH`;
   - the Puppeteer browser cache (`~/.cache/puppeteer/chrome-headless-shell/<platform>-<version>/…/chrome-headless-shell[.exe]`), newest version first. `npx @puppeteer/browsers install chrome-headless-shell@stable --path ~/.cache/puppeteer` puts it there; only builds for this OS and architecture are taken.
3. **Anywhere but macOS**, the installed Chrome or Edge, as before. There is no Dock there, and CI (Ubuntu) keeps running these tests with its `google-chrome`.
4. **On macOS, nothing:** the layout tests skip rather than launch the Chrome app. The skip says how to install the shell.

The headless shell is always headless, so it's run without `--headless=new`; the rest of the flow (`--dump-dom`, stopping it as soon as the measurements appear) is unchanged.

Not changed: the Dock's existing recent-app entries. Removing them (right-click → Remove from Dock) or turning off "Show suggested and recent apps" is the owner's choice.

## 3. Tests

- A unit test for the order: the env override, then a headless shell on `PATH` or in the cache (newest version), then the platform fallback, with none on macOS.
- On this Mac, with the cached shell (131): both layout test files pass, and the Dock's `recent-apps` list is the same before and after the run.
