# REPORT: Docs and dead-code cleanup audit

**Date:** 2026-08-07
**Author:** Agent3
**Scope:** `docs/`, `specs/` (stale-doc audit) + full Rust/TS workspace (dead-code audit)
**Related:** `docs/specs/SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md` (the existing, not-yet-fully-shipped plan this report defers to — see §1), `docs/reports/REPORT_REPO_HEALTH_AUDIT_2026_07_05.md` and `_2026_07_20.md` (two prior audits covering similar ground, neither of which produced lasting action — see §1 for why this one should end differently)

---

## 1. Why this isn't a third ignored audit

Two prior repo-health audits (2026-07-05, 2026-07-20) already found the docs
tree accumulating faster than it's curated, and a dedicated hardening plan
(`SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md`, 4 days before this report) already
diagnosed the root cause precisely: `Status:` is free text (257 distinct
values found across ~1,150 doc files), so nothing can programmatically tell
"still current" from "shipped 6 weeks ago and never updated" without reading
prose. That plan's own explicit non-goal is a retroactive rewrite of the
backlog — it calls that the failure pattern to avoid, and instead prescribes
small, evidence-backed batches plus (unshipped) tooling to keep it from
recurring.

This report is one such batch — a bounded, verified list from a sample, not
an exhaustive sweep — plus the equivalent audit for dead code, which the prior
docs-focused audits didn't cover. **Scope discipline: this report proposes
what to do, it does not do it.** Nothing below has been deleted or edited yet.

---

## 2. Docs: confirmed-stale sample (26 of 40 checked)

Every doc below was verified against actual code (grep for the specific
files/functions it references, `git log` for shipping commits) — not just
its own `Status:` field, per the hardening plan's own Phase 4 guardrail.
**This is a sample of `docs/specs/`'s ~591 files, prioritized toward
`Status: Planned`/`Proposed` headers — not exhaustive.**

### Confirmed implemented, status field just never updated (highest-confidence batch)

| File | What shipped |
|---|---|
| `SPEC_MESSAGING_INTEGRATION_DISCORD_POC_2026_06_24.md` | Real Discord bridge shipped same day (PR #1763), extended through 07-29 |
| `SPEC_WEBFETCH_CONTENT_VIEW_2026_06_22.md` | `WebFetchResult.tsx`, PR #1706 |
| `SPEC_WRITE_TOOL_CONTENT_VIEW_2026_06_19.md` | `renderWrite()` in `ToolOverlayLog.tsx` |
| `SPEC_WRITE_TOOL_MD_RENDER_2026_06_23.md` | `isMarkdown` detection in `renderRead`/`renderWrite` |
| `SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md` | Shipped same day, commit `9f6cc2824` (PR #2431) — now documented as current default in `CLAUDE.md` itself |
| `agent-input-auto-grow.md` | Shipped, differently than proposed (CSS `field-sizing: content`, not JS) |
| `agent-pane-slash-commands.md` | `SlashAutocomplete.tsx` |
| `persistent-process-mode.md` | Became the persistent controller (`blockcontroller/persistent.rs`) — foundational to how the app runs today; badly stale status for load-bearing shipped code |
| `portable-data-dir.md` | `agentmux-launcher/src/data_dir.rs`, documented `task package` feature |
| `SPEC_AGENT_BUSY_ANTS_REFINEMENT_2026_06_22.md` | Marching-ants bar in `_control-bar.scss` |
| `SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md` | Implemented, foundational — named directly in `paths.rs` |
| `SPEC_EARLY_ALPHA_WARNING_2026_06_05.md` | `README.md` directly cites and implements it |
| `SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md` | Implemented, superseded by later tearoff specs building on it |
| `SPEC_GLOBAL_IDENTITY_MEMORY_DRONE_2026_06_24.md` | `resolve_global_shared_root()`, load-bearing in `paths.rs` |
| `SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md` | Implemented 4 days later — even short-lived "Proposed" docs go stale fast |
| `SPEC_MEDIA_PANE_V4_MCP_OPEN_TOOL_2026_08_03.md` | `OpenMedia` MCP tool, live in `agentmux-mcp/src/main.rs` |
| `SPEC_MIGRATION_FRAMEWORK_2026_06_24.md` | `agentmux-srv/src/migrations/` — fully built, actively used framework |
| `SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md` | `muxspect dock`/`dock clear`, shipped within a day |
| `SPEC_TAB_CONTENT_REVEAL_GATE.md` | `tab-reveal.ts`, wired into `startup-splash.ts`/`editor-model.ts` |
| `SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md` | `registry.ts` — load-bearing; underpins several *other* still-"Proposed" docs above (WebFetch, WebSearch, Write-MD) |
| `widget-visibility-rearchitecture.md` | Confirmed shipped (the menu entry it proposed removing is confirmed absent) — 5 months stale |

### Confirmed superseded/renamed (concept moved, doc didn't follow)

| File | What superseded it |
|---|---|
| `SPEC_MUXBUS_GITHUB_REVIEW_NOTIFICATIONS_2026_06_20.md` | Already self-annotated 2026-08-07 (this session's own earlier work) pointing to `SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md` |
| `agent-pane-title-buttons.md` | References a "Forge" tab / `AgentCardSettingsPanel` no longer in code — superseded by the Identity/Armory consolidation |
| `SPEC_AGENT_ZOOM_PERSISTENCE_2026_06_22.md` | Superseded by the generalized universal-zoom framework (`term:zoom` block-meta key) |
| `termwrap-refactor-race-fix.md` | Terminal subsystem substantially reworked since (`SPEC_BULLETPROOF_TERMINALS_2026_05_21`, `SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23`) — 5 months old |
| `SPEC_UNIFIED_MENU_SYSTEM_2026_05_11.md` | Proposed unifying `FlyoutMenu`/`ContextMenuModel` — no evidence of consolidation; both still exist separately as duplicates |

### Genuinely accurate — status matches reality, no action needed

`SPEC_AGENT_DISPATCH_SUBAGENT_HIERARCHY_2026_07_17.md`, `SPEC_MUXBUS_MULTI_TIER_DISCOVERY_AND_REMOTE_INVOCATION_2026_07_29.md` — verified not-yet-built, `Proposed` status is correct.

### Needs a closer, non-grep-level look before acting (flagged, not resolved)

- `SPEC_XTERM_PASTE_TRUNCATION_2026_06_12.md` — marked **Severity P1, silent data loss**, still "Proposed" 2 months later. Couldn't confirm fixed/unfixed from grep alone. **Worth checking before anything else in this report** — if still open, it's a real bug, not a docs-hygiene issue.
- `SPEC_AGENT_ERROR_FRAMEWORK_2026_06_20.md` — no direct implementation found, but a later (2026-08-04) spec cites it as "the durability layer this reuses." Either shipped under different naming or that citation is itself unverified.
- `SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16.md`, `network-top-hosts-visibility.md`, `browser-pane-reducer-roadmap.md`, `agent-pane-runtime-controls.md` — likely never implemented as specced; low-confidence, worth a second look before declaring dead.

### Untracked (never went through review at all)

- `docs/specs/SPEC_AGENT_COLOR_2026_08_05.md` — a plan I (Agent3) wrote for an agent-color picker feature. Confirmed **never implemented**: zero `color` field anywhere on `AgentDefinition`. Only untracked doc in the entire repo.

### `docs/retro/` — two lower-urgency findings

1. **Four retros on one incident, all dated 2026-03-19** (`retro-chrome-zoom-complete-analysis.md`, `-cross-platform-regression.md`, `-macos-breakage.md`, `-per-platform-fix.md`) — read as four separate write-ups of the *same* Windows/macOS zoom-widget regression chain, not four incidents. Consolidation candidate.
2. **~10 planning/status docs mis-filed as retros**, violating `docs/retro/README.md`'s own definition ("post-incident retrospectives") — `next-steps-*.md`, `phase-e-status-*.md`, `multi-reducer-*.md`, `b9-3-*-analysis.md`, all from the April–May reducer-migration effort. Legitimate historical docs, just filed in the wrong directory. Low urgency — retros are meant to be append-only.

The previously-known duplicate `docs/retros/` (plural) directory is already gone — merged into `docs/retro/` by a prior Phase-0 pass (commits `4797b8358`/`f3790992f`).

---

## 3. Dead code: verified findings

**Two caveats that reshape this entire section:**
1. The build this audit ran on is **Windows**. Every `#[cfg(unix)]`/`#[cfg(target_os = "macos"/"linux")]` function reports as unused here — it isn't. A large share of `agentmux-cef`/`agentmux-common` warnings fall in this bucket.
2. `cargo build` doesn't compile `#[cfg(test)]`. A symbol used only by a test is reported "never used" — it isn't dead, it's test-only.

Every finding below was individually verified (grepped for other call sites, checked platform gates) rather than trusted from the raw warning list.

### Safe to delete now (verified dead on every platform, no test dependency)

| Symbol | Location |
|---|---|
| `ShellSessionRegistry::insert` | `agentmux-srv/src/backend/shell_node.rs:108` |
| `ResolverError::AccountNotFound` | `agentmux-srv/src/identity/resolver/errors.rs:74` |
| `RefreshScheduler::unregister` + `::state`, `broker::Command::Unregister` | `agentmux-srv/src/broker/scheduler.rs:134`, `state.rs:84` |
| `RuntimeSlot::Disabled` + `ContainerRuntimeHandle::disabled()` | `agentmux-srv/src/backend/container.rs:494`, `:533` — constructed only by the function that's itself never called |
| `ProviderConfig` fields `display_name`, `styled_output_format`, `pinned_version`, `icon`, `docs_url` | `agentmux-srv/src/backend/providers.rs:38` — `Debug`-only derive, confirmed not a serde false positive |
| `ZOOM_MIN`/`ZOOM_MAX` consts | `agentmux-cef/src/browser_panes/zoom.rs:24` |
| `TrackingConfidence::BestEffort` + its match arm | `agentmux-srv/src/backend/process_tracker/mod.rs:96`, matched-but-never-constructed at `server/app_api/agent_io.rs:27` |
| `AuthPatternMatch::LoginFailure` + its match arm | `agentmux-srv/src/identity/auth_patterns.rs:27`, matched-but-never-constructed at `identity/auth_session.rs:226` |
| ~30 trivial `unused import`/`unused variable`/`unused_mut`/`unused_parens` warnings, 1 unreachable statement, 1 unnecessary `unsafe` block | Full list in the underlying audit; mechanical, low-risk |

**Cross-platform caveat still applies to this list**: it was derived from a Windows build. Re-running `cargo build --workspace` on macOS and Linux before deleting anything here is cheap insurance against a symbol that's actually reached from a platform-gated path not visible on Windows.

### Needs a human decision (dead in production, but plausibly deliberate)

| Symbol | Location | Why it's not a clean delete |
|---|---|---|
| Launcher saga read-side: `UnresolvedLauncherSaga`, `UnresolvedLauncherStep`, `SagaSummary`, `fail_step`, `unresolved_sagas`, `get_saga_steps`, `snapshot_recent`, `step_out` | `agentmux-launcher/src/saga/log/mod.rs` | Tested but unwired to any CLI/`--diag` path. The **srv-side twin** of this exact feature (`agentmux-srv/src/sagas/log.rs`) is deliberately `#[allow(dead_code)] // consumed by PR 2's --diag sagas` — the launcher side just lacks the matching annotation. Decision: wire it up, or add the same `#[allow]` rather than delete a half-shipped feature. |
| Deprecated `preset.*` RPC aliases | `agentmux-srv/src/backend/rpc_types/commands.rs:334`, `server/app_api/bundle.rs:55` | Explicitly commented "kept wired for one release (remove in Phase 4)" — a real, owner-sanctioned removal, just gated on a Phase 4 that hasn't landed yet. Don't remove early. |
| `inject_identity_env` (sync variant) | `agentmux-srv/src/identity/resolver/inject.rs:67` | Superseded by the async/broker variants per its own module doc; the `pub use` re-export is a dead import. Low-risk removal, but confirm nothing external depends on the sync signature first. |
| `SubmitProviderApiKeyReq` unread fields | `agentmux-srv/src/server/identity_handlers.rs:130` | Stub for a not-yet-implemented endpoint; has a round-trip test. Leave until the endpoint is either built or formally cut. |

### Not actually dead — leave alone

- **~15 `[CFG]` findings** (toolchain-path helpers, window-position posting functions, macOS pane-swizzle cleanup, Linux paint-gate state) — live on macOS/Linux, dead only because this build was Windows-only.
- **~10 `[TEST]` findings** (`user_clone_defs_for_template`, `instance_update`, `InstanceStatus::parse`, `ContainerManager::stop`/`remove`, `ClaudeTranslator::reset`, etc.) — real methods, only called from `#[cfg(test)]`. Deleting the method without its test breaks the build; if genuinely unwanted, delete both together, don't delete just one.
- **~180 `#[allow(dead_code)]` occurrences workspace-wide** — the large majority carry an inline justification (reserved for future work, telemetry, test-only) and aren't cleanup targets. Bare, uncommented ones cluster in `rpc/engine.rs` (17), `backend/wps.rs`, `storage/filestore/core.rs`, `state/top_level_creation.rs`, `browser_pane.rs` — worth a closer look in a follow-up pass, not resolved here.

### Frontend: unused exports (medium confidence — not orphaned files)

18 exported symbols with zero references elsewhere in `frontend/`, concentrated in two utility modules whose API outgrew their consumers:
- `frontend/util/srv-events.ts` — `srvEvent`, `srvEventVersion`, `srvEventsActive`, `srvEventStats` (only `installSrvEventBridge` is actually consumed)
- `frontend/app/store/block-atom-cache.ts` — `useBlockCache`, `cleanupTabAtomCache`, `useBlockMetaKeyAtom`, `getTabMetaKeyAtom`, `useTabMetaKeyAtom`, `useSettingsKeyAtom`, `useOverrideConfigAtom`, `useBlockDataLoaded`
- Singletons: `launcherEventStats`, `busyCount`, `anyBusy`, `reducedMotionSetting`, `isApplyingRemoteEvent`

**No orphaned files** in either language — every `.rs` resolves via a `mod` declaration or is a Cargo integration-test crate (auto-discovered by convention); every unreferenced `.tsx`/`.ts` in the frontend is either a platform variant resolved by `vite.config.ts`'s `platformResolve()` plugin, the HTML entry point, or a `.bench.ts` file run by a separate runner. This is a genuinely clean result — worth stating plainly since it's the one part of this audit with no follow-up needed.

---

## 4. Recommended next step

Per the hardening plan's own Phase 4 guardrail (spot-verify before trusting a doc's claims) and non-goal (no big-bang rewrite), the actionable scope from this report is:

1. **Check `SPEC_XTERM_PASTE_TRUNCATION_2026_06_12.md` first** — flagged P1/silent-data-loss and still marked "Proposed" 2 months later. If genuinely unfixed, that's a bug to triage, not a docs cleanup.
2. **Docs:** add supersession banners (in the Phase-1-eventual `Status: superseded` / `Superseded-by:` shape once that lands, or a plain note today matching the pattern already used on `SPEC_MUXBUS_GITHUB_REVIEW_NOTIFICATIONS_2026_06_20.md`) to the ~21 confirmed-implemented/superseded docs in §2's first two tables. Delete the untracked, never-implemented `SPEC_AGENT_COLOR_2026_08_05.md` outright (no review history to preserve). Leave `docs/retro/`'s two lower-urgency findings for a separate, smaller pass.
3. **Dead code:** delete the §3 "safe to delete now" list (re-verify on macOS/Linux first, per the cross-platform caveat). Bring the launcher `saga/log` module in line with its srv twin (`#[allow(dead_code)]` + matching justification, or wire it up) rather than deleting a half-shipped feature. Leave the `preset.*` aliases until Phase 4. Leave every `[CFG]`/`[TEST]` finding untouched.
4. **Frontend:** confirm the 18 unused exports aren't intended public API before removing (quick check: are any referenced by string key / dynamic import rather than a static `import`?).

I have not made any of these changes — this report is the analysis you asked for. Let me know which of the four items above (or all of them) you want executed.
