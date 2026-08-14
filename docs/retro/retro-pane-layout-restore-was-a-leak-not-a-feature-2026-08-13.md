# Retro: "session restore on reopen" was never a feature — it was a window-row leak, and fixing the leak exposed a deliberate destroy-on-close cascade

**Date:** 2026-08-13
**Trigger:** User reported that AgentMux used to reopen with the same panes/tabs/Armory layout that were open when it was last closed, and that this stopped working. Expectation: "this is how it used to work," implying a regression to fix.
**Finding:** No restore-on-quit feature was ever deliberately built. What looked like one was a side effect of a leaked-window bug that has since been correctly fixed. There is no dead/vestigial restore code to revive — closing the gap needs new, deliberate work (tracked in `docs/specs/SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13.md`).

---

## 1. What the user actually observed, and why

On startup, `frontend/app-init.ts` (`initHostWave`, ~line 334) reads `ClientService.GetClientData()` and opens `clientData.windowids[0]` if one exists; only when the list is empty does it call `WindowService.CreateWindow` to seed a brand-new default workspace (`agentmux-srv/src/server/service/window_create.rs:107-211`, `default_three_pane_tree`).

Two commits, both individually reasonable, combined to make `windowids[0]` durably point at last session's fully-populated workspace for months:

- **2026-04-04, `e3a6f85c2` ("fix: kill shell processes when closing windows and tabs", PR #299)** — `close_window` in `agentmux-cef`'s wcore cascades workspace → tabs → blocks and kills their shell processes on close, to stop orphaned shells piling up in Task Manager. Legitimate fix for a real leak.
- **2026-07-16, `4cbf856b7` ("fix(cef): closing 'main' never notified srv — permanent window row leak", PR #2186, see `docs/retro/retro-last-window-close-quit-race-2026-07-16.md`)** — until this date, `CloseWindowTask::execute` had an explicit `self.label != "main"` guard around the entire srv-notify block, on the documented (and wrong) assumption that "process exit reaps everything" for the main window. It doesn't: `agentmux-srv` is a separate process with its own persistent SQLite store. Because almost every user quits by closing their one and only ("main") window, **the April cascade above never actually ran for the overwhelming majority of quits** — the `Window`/`Workspace`/`Tab`/`Block` rows for the just-closed session simply stayed in the database untouched.

The consequence: `clientData.windowids[0]` on the next launch still pointed at a fully-intact previous session — every pane, every tab, the whole Armory layout — because nothing had ever told srv to delete it. That looked exactly like "restore my session," and reliably enough that it read as a real feature. It was a leak, not a restore path: nothing chose what to keep, nothing validated staleness, and every OTHER close path (secondary windows, pool windows) was already correctly wiping its workspace this whole time — only the dominant "close the one main window" path leaked.

PR #2186 fixed the actual bug (a permanent DB row leak, independently confirmed via a "ghost window" resurrecting after being explicitly closed — see that retro's §1). Fixing it correctly routed `"main"`'s close through the same cascade every other window already used. The side effect: **every graceful quit now deliberately destroys the workspace**, and the next launch always finds `windowids` empty and seeds a fresh default 3-pane layout. This is the regression the user is describing, and it landed on **2026-07-16**.

## 2. Is there anything to revive?

No. Checked:
- `docs/analysis/dead-code-audit.md` and `docs/retro/audit-vestigial-types-2026-04-28.md` — neither identifies any disabled/dead restore-on-boot path. The audit's only `Workspace`-related finding is that its display `name`/`icon`/`color` fields are unused; `tabids`/`activetabid` are explicitly called load-bearing.
- `agentmux-srv/src/backend/storage/migrations.rs` — no default-workspace-reset logic exists; the reset the user sees is a direct consequence of §1's cascade running on an empty-by-design `windowids` list, not a migration wiping anything.
- No upstream Wave Terminal restore-on-boot code was stripped during the fork/rebrand — both destructive-behavior commits above are homegrown AgentMux work (`e3a6f85c2`, `4cbf856b7`), not upstream diffs that got dropped.

## 3. The one existing restore mechanism, and why it doesn't cover this case

Pillar 1 ("crash reproject," `docs/specs/SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md`, `SPEC_PILLAR1_STEP4_CRASH_REPROJECT_2026_07_07.md`) is a real, partly-shipped mechanism for rebuilding the host's window/pane topology from srv state. It is **explicitly and deliberately scoped to crash recovery only** — the design doc states the goal outright: *"Flicker is a crash-path event only — steady state never rebuilds."* It fires when `Client.windowids` still holds rows a graceful close would have pruned, i.e., it only has something to find because a crash (not a clean `CloseWindow`) left rows behind. Now that graceful close correctly prunes those rows (§1's fix), reproject has nothing to reproject after a normal quit-and-reopen — it was never intended to serve that case, and doesn't regress anything by not doing so.

## 4. Net conclusion

There is no bug left to fix here in the narrow sense — the leak that accidentally produced "restore on reopen" was real, harmful (permanent DB row growth, a ghost-window resurrection bug), and is correctly fixed. But it means AgentMux currently has **zero deliberate mechanism** for restoring a user's layout after a normal quit: every graceful close destroys the workspace, and every launch starts from the default 3-pane seed. Getting the user's remembered behavior back — on purpose, correctly, without reintroducing the leak-prone semantics — is new work, not a revert. Two shapes of that work (auto-restore-last-session, and named on-demand Layouts) are scoped in `docs/specs/SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13.md`.

## 5. Sources

- `frontend/app-init.ts:317-345` (`initHostWave` cold-start path)
- `agentmux-srv/src/server/service/window_create.rs:107-211` (`default_three_pane_tree` seed)
- `agentmux-srv/src/backend/wcore/window.rs:139-161` / `agentmux-srv/src/backend/wcore/tab.rs` (cascade delete on close)
- `agentmux-srv/src/server/service/window_close.rs:24-186` (`CloseWindow` RPC cascade)
- Commit `e3a6f85c2069e5f5ea9358b3dae85f8594ac0d0d` (2026-04-04, PR #299)
- Commit `4cbf856b7fceba371b8b4545c0c8cc0eced6d3fd` (2026-07-16, PR #2186)
- `docs/retro/retro-last-window-close-quit-race-2026-07-16.md`
- `docs/analysis/dead-code-audit.md`, `docs/retro/audit-vestigial-types-2026-04-28.md`
- `docs/specs/SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md`, `docs/specs/SPEC_PILLAR1_STEP4_CRASH_REPROJECT_2026_07_07.md`
