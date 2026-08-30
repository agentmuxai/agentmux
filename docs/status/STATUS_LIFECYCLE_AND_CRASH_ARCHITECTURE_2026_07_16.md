# Status & Roadmap — Lifecycle & Crash Architecture Program (as of 2026-07-16)

> **Supersedes** `STATUS_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_07_12.md` in full. That
> snapshot's #1-ranked open item (drag-HWND cross-wire) and its pagefile P0/P1 stack have
> all shipped since; two NEW lifecycle bugs were found and fixed live (both in the
> window-close → srv notify chain), and the teardown backstop's Phase 2 landed — closing
> the last open item from Discussion #1680's §9 scorecard.

> **SWEPT 2026-08-29 (docs-cleanup Phase 3) — STILL OPEN, deliberately not
> marked resolved.** §6's ranked roadmap still describes real unfinished
> work: non-Windows close-path verification (#2186, blocked on hardware),
> the srv↔host/launcher reconciliation pass, the pool-promote colour flash,
> and Pillar 3 follow-ons. None of these were re-verified item-by-item in
> this sweep — a docs-status pass is the wrong instrument for auditing an
> in-flight architecture program, and guessing is the exact failure mode
> `SPEC_DOCS_CLEANUP_AUDIT_2026_08_22.md` §2.2 warns about.
>
> **Take this document's own advice seriously — it is now ~6 weeks old.**
> The "Verify before acting" line immediately below asks you to re-check
> `file:line` references if reading more than a few days after writing;
> `main` has moved several hundred commits since.

**Type:** Status snapshot + forward roadmap, not a plan doc.
**Verify before acting:** re-check file:line references if read more than a few days after
2026-07-16 — this subsystem moves fast.

---

## 0. The one-sentence picture

**All three pillars are complete, Step 6 included** (updated later on 2026-07-16: the
saga-durability collapse shipped the same day this snapshot was written — see §1). The
entire `Client.windowids` leak class is closed, the window-close → srv notify chain is now
correct for **every** close path including the app's last window, and the launcher can now
reap a wedged host on its own (teardown backstop Phase 2) — leaving only the
hardware-blocked platform verification (issues #2188/#2189) and low-priority follow-ups.

## 1. Pillar 1 — disposable host

| Step | What | Status |
|---|---|---|
| 1 | Layout single-writer collapse (`#864`) | ✅ Done, merged. |
| 2 | Persist per-window opacity + floating-pane placement/restore-rect | ✅ Done, merged, live-verified. |
| 3 | Persist window `kind` + parent linkage | ✅ Done, merged, live-verified. |
| 4 | Crash-reproject: fast-path from launcher snapshot, slow-path from srv, restoring-session overlay, splash respawn | ✅ Done, all 5 phases (#2014, #2015, #2017, #2032). |
| 5 | E2E test: "host OOM ⇒ session reprojects" | ✅ Done. |
| 6 | Collapse graceful-flush-vs-crash incoherence; shrink saga layer to an in-memory registry | ✅ **Done 2026-07-16** — gate re-evaluated with the user and lifted early: the durable layer's recovery was proven a behavioral no-op (tombstone writer only), so the calendar bake validated nothing about it. Launcher saga durability (SQLite log, recovery walker, retention vacuum, `rusqlite` dep, saga config, `--diag sagas` offline reader) deleted; in-memory registry behind the same coordinator API. See `SPEC_PILLAR1_STEP6_SAGA_COLLAPSE_2026_07_16.md`. Residual: `orphan_reconcile` shrink is a tracked follow-up. |

Bake-period yield so far: the drag-HWND cross-wire (fixed, #2111) and the ghost-window
resurrection below — both found precisely because reproject keeps running under real use.

## 2. Pillar 2 — single lifecycle authority (`reconcile_quit`)

✅ Done, all 4 phases (#2080, #2081, #2084, #2083). No open items. Two adjacent gaps found
and fixed since 07-12 (see §4) — both in the close-path *notify* chain, not in the quit
decision authority itself.

## 3. Pillar 3 — admission control

✅ Shipped (#1853). Follow-ons (queue-and-drain, per-agent working-set cap, "memory full"
badge) remain open, low-priority, independent.

## 4. Lifecycle bugs — found and fixed since 2026-07-12

| Bug | Status | Doc |
|---|---|---|
| Drag-HWND cross-wire (multi-window reproject binds a label to the wrong HWND; dragging window N moves a different window) | ✅ Fixed (#2111) — label→HWND bound at Views creation time | `retro-reproject-drag-hwnd-crosswire-2026-07-12.md` |
| Pagefile P0 item 3: commit-aware scheduler reserve re-derived from `PrivateUsage` | ✅ Done (task #33) | `SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29.md` |
| Pagefile P1s: SetMeta log-firehose throttle + launcher-log rotation; old-version shutdown prompt on upgrade | ✅ Done (tasks #34, #35) | same |
| **Floating-pane/pool bridge-init lock-out** — reload of a warmed pool window could never rebuild `window.api` (3 compounding layers: pane-flag-gated cred injection, cred-strip vs `isCef()`, `self.ipc_port = 0`); blank window + ~5s reload storm | ✅ Fixed (#2181), live-verified | `SPEC_BRIDGE_INIT_RECOVERY_2026_06_15.md` (2026-07-16 correction header) |
| **Last-window close never notified srv** — closing "main" left its `db_window`/workspace rows orphaned forever; crash-reproject resurrected them as ghost windows on every launch. Plus (reagent catch): NO Windows parking close ever reported `PoolDrained`/`PoolNotLast`, so every close's launcher saga silently stalled to its 30s timeout | ✅ Fixed (#2186), live-verified both halves | `retro-last-window-close-quit-race-2026-07-16.md`, `SPEC_WINDOW_LIFECYCLE_CLOSE_RELIABILITY_2026_07_04.md` §4c |

## 5. Teardown backstop (#2092) — Phase 2 shipped 2026-07-16

Phase 1 (observe-only UI-thread liveness probe) merged 07-11 and baked. Phase 2 — the armed
J0 teardown state machine — is now implemented (`agentmux-launcher/src/teardown_backstop.rs`):
arm on `PoolDrained`/`OrphanInstance`, disarm on `WindowOpened` or any host exit, teardown
(`TerminateJobObject(J0)`, exit code 86) when armed past the 30s grace with ≥2 consecutive
delivered-but-unanswered UI-thread probes. This closes the one undelivered item from
Discussion #1680's §9 scorecard: a host whose UI thread wedges after the last window closes
no longer lingers as an orphaned tree — the launcher reaps it within roughly
GRACE + 2 probe intervals. Verification hook: `debug:hang_ui` (double-gated behind
`AGENTMUX_DEBUG_HANG=1`). Spec: `SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11.md`.

## 6. Roadmap — open items, ranked

1. ~~**Pillar 1 Step 6**~~ — **shipped 2026-07-16** (see §1); the follow-up is the
   `orphan_reconcile` shrink assessment.
2. **Task #15 — non-Windows close-path verification.** Blocked on macOS/Linux hardware.
   Scope has GROWN since 07-12: #2186's `on_before_close` defense-in-depth fix (the notify
   + deferred-quit path that IS live on those platforms) needs verifying there too.
3. **Reconciliation pass** (srv cross-checks `state.windows` against host/launcher reality)
   — the durable fix for the residual "srv unreachable at close time still orphans the row"
   failure mode both #2186 and the 07-04 spec explicitly leave open. Ties into the
   still-undecided task #4 (`SPEC_AGENT_SYSTEM_MANAGEMENT_API`).
4. **Task #51 — pool-promote color flash** (adjacent, cosmetic). Any re-fix must pass the
   capture-harness gate (5 consecutive clean promotes) before merging — #2163's revert is
   the cautionary tale.
5. Pillar 3 follow-ons — open, low-priority, independent.

## 7. Sources

- `docs/status/STATUS_LIFECYCLE_AND_CRASH_ARCHITECTURE_2026_07_12.md` (superseded)
- `docs/specs/SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11.md`
- `docs/specs/SPEC_WINDOW_LIFECYCLE_CLOSE_RELIABILITY_2026_07_04.md` (§4c added 07-16)
- `docs/retro/retro-last-window-close-quit-race-2026-07-16.md`
- `docs/specs/SPEC_BRIDGE_INIT_RECOVERY_2026_06_15.md` (07-16 correction header)
- `docs/retro/retro-reproject-drag-hwnd-crosswire-2026-07-12.md`
