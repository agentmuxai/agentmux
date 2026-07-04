# Retro: Docker "installed" signal disagreed across the app

**Date:** 2026-07-04
**Severity:** Medium (confusing UX, no data loss; workaround was to restart AgentMux, which wasn't even guaranteed to help)
**Affected versions:** since #1576 introduced the daemon-liveness check for the create-from-template Container option — every release through v0.50.0.

---

## What happened

A user with Docker Desktop installed opened the "Create agent from template" modal and saw the Container sandbox option greyed out with "Docker not detected." At the same time, the Toolchain diagnostics widget reported Docker as installed. Same machine, same moment, contradictory answers.

## Root cause

There were **4 independent, disagreeing implementations of "is Docker available"** in the codebase, using 3 different semantics:

1. `AgentCreateFromTemplateModal.tsx` → `ContainerRuntimeAvailableCommand` → `ContainerManager::check_available()` → a real `docker.ping()` against the daemon. Correct semantics — but `AppState.container_manager` connected **once at srv process boot** (`main.rs`). If Docker wasn't reachable at that instant, it was `Option::None` for the rest of the process's life; starting Docker Desktop afterward had no effect on any consumer without restarting AgentMux, including the actual container-launch code paths (`agent_handlers/input.rs`, `app_api/agent_io.rs`, `shell_handlers.rs`) — not just this one probe.
2. `toolchain-view.tsx`'s Docker row → the generic `ResolveCliCommand` RPC → CLI-on-PATH + `docker --version`. This succeeds even when the daemon is stopped — `docker --version` only talks to the local client binary. This was the row falsely reporting "installed."
3. `launch-flow.ts`'s Phase 0 pre-flight (right before actually launching a container agent) used the *same* PATH-only `ResolveCliCommand` check as #2 — an agent could pass this gate on CLI-presence alone and still fail deeper inside container spawn if the daemon was actually down.
4. `AgentLaunchModal.tsx`'s host/container radio gated purely on a static per-provider `containerSupported` catalog flag (does this provider ship a container image at all) — no live Docker probe of any kind.

This divergence wasn't an oversight so much as an accepted risk: `docs/specs/SPEC_TOOLCHAIN_MANAGER_2026-06-15.md` §5.2 explicitly proposed a batch `toolchain.status` RPC as the intended single source of truth for all tool detection, then §6 (Rollout) deferred it — "P1 reuses the existing `resolvecli` RPC per row... the batch `toolchain.status` remains a possible later optimization." That's exactly the unification that would have prevented this. The daemon-liveness check (#1) was added later and reactively, only at the one call site where the false-positive was actually reported (#1576), never propagated back to the toolchain widget or the launch pre-flight.

## Why it wasn't caught sooner

Each of the four checks was individually correct for what it measured and was added at a different time for a different feature, without a shared abstraction to keep them consistent. Nothing in the codebase treated "toolchain availability" as a single concept with one answer — every consumer independently decided which backend primitive to call and never shared or cached results with any other consumer. The disagreement only became visible when a user happened to have both surfaces open (or compare notes) at once.

## Fix

**Backend** (`agentmux-srv/src/backend/container.rs`, `server/mod.rs`, `main.rs`, `agent_handlers/core.rs`/`input.rs`, `app_api/agent_io.rs`, `shell_handlers.rs`): replaced the fixed-at-boot `Option<Arc<ContainerManager>>` with `ContainerRuntimeHandle`, a self-healing holder (`RwLock<RuntimeSlot>`) that retries `ContainerManager::connect()` on demand. `.get()`/`.is_available()` are used by every container-manager consumer, not just the status-probe RPC — so a daemon that starts after AgentMux launched is picked up everywhere, without an app restart.

**Frontend**: introduced `frontend/app/store/toolchain-capabilities.ts` as the single point of entry every consumer reads from. `CoreTool` (`toolchain-catalog.ts`) gained a `checkKind: "path" | "liveness"` field — `"path"` (the default) resolves on PATH, `"liveness"` (set on the `docker` entry) dispatches through a small lookup table to the correct daemon-aware check. The store caches results in a shared reactive store, de-dupes concurrent probes, and exposes `watchCapability(id)` so open views self-heal within a few seconds of, e.g., Docker Desktop starting. Migrated all 4 divergent call sites to read from it; `AgentLaunchModal.tsx`'s radio deliberately keeps its static gate (relaunching an already-configured agent shouldn't hard-disable on a possibly-stale probe) but gained a non-blocking inline hint.

See PR for the full diff and `frontend/app/store/toolchain-capabilities.test.ts` for the covered behaviors (liveness vs. path dispatch, concurrent-call de-dupe, forced refresh, watch lifecycle).

## Verification after fix

- `cargo check`/`cargo test -p agentmux-srv` — 1296 passed, including the container module's existing unit tests.
- `npx tsc --noEmit` and the full `vitest` suite — 1590 passed across 106 files, including the new store's tests and the existing `AgentCreateFromTemplateModal.test.tsx`/`AgentLaunchModal.integration.test.tsx` suites (updated to route their Docker mocks through the shared store).
- **Not yet live-tested end-to-end** in a running dev instance (stop/start Docker Desktop while `task dev` is up, confirm the Toolchain widget and create-agent modal agree throughout, then launch an actual container-mode agent to confirm the reconnected manager reaches the real launch path, not just the status probe). The automated coverage above exercises the dispatch logic, caching, de-dup, and polling lifecycle directly, but the live cross-view-agreement scenario this fix targets should still get a manual pass before/shortly after merge.

## Explicit follow-ups (found during this investigation, not fixed here)

- **~4 separate Rust "find a binary on PATH" implementations** (`resolve_cli_on_path` in `cli_handlers.rs`, `resolve_tool_path` in `install_handlers.rs`, a hand-rolled reimplementation in `app_api/agent_open.rs`) that should be consolidated onto one shared helper. Not fixed here — it's a duplication/maintenance issue, not a divergent-semantics bug like the one above.
- **CEF-host-vs-srv PATH environment mismatch**: `check_nodejs_available` (`agentmux-cef/src/commands/providers.rs`) runs directly in the CEF host process, which never receives the `resolve_login_path()` PATH enrichment that's only ever forwarded to the spawned `agentmux-srv` child. This is a structural "two different PATHs" bug, independent of the detection-logic divergence fixed here, and needs its own investigation.

## Lessons

1. **A deferred "do this properly later" in a spec is a real liability, not a paperwork formality** — `SPEC_TOOLCHAIN_MANAGER_2026-06-15.md` correctly anticipated the unified-check design and explicitly chose not to build it. That choice should have come with a tracking issue or a note in the toolchain catalog itself; instead the next daemon-backed-tool bug (Docker) rediscovered the exact gap the deferred design would have closed.
2. **"Installed" is not one concept** — for anything backed by a process/daemon (not just a static binary), "the CLI is on PATH" and "the thing is actually usable right now" are different questions with different answers over time. Any future toolchain-gated feature should ask which one it actually needs, not assume PATH-presence is sufficient by default.
3. **Shared state beats parallel re-implementation** — four call sites each re-deciding "is Docker available" independently is what let them drift. A single cached, reactive source of truth (this fix's `toolchain-capabilities.ts`) makes disagreement structurally impossible for anything that reads from it, instead of relying on every future call site remembering to make the "right" choice.
