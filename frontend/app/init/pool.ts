// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tear-off Phase 6 — frontend pool-mode helpers.
//
// A "pool window" is a hidden, fully-painted CEF window that the
// host pre-spawns (see agentmux-cef/src/commands/window_pool.rs) so
// tear-off can promote it instantly with no first-paint flash. The
// pool window's URL carries `?pool=1`; the frontend detects this
// flag and skips the standard initHostNewWindow flow at startup.
// Instead it renders an empty body and waits for the host to emit
// `pool:promote` with a workspace ID — at which point the renderer
// re-runs initHostNewWindow against that workspace.
//
// Spec: docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26 §4.5

/** True when the current renderer was spawned as a tab/new-window pool window. */
export function isPoolMode(): boolean {
    if (typeof window === "undefined") return false;
    return new URLSearchParams(window.location.search).get("pool") === "1";
}

/** True when the current renderer was spawned as a pane pool window. */
export function isPanePoolMode(): boolean {
    if (typeof window === "undefined") return false;
    return new URLSearchParams(window.location.search).get("pane-pool") === "1";
}

/**
 * Wait for the host to assign this pool window — either:
 *   - `pool:promote`  (tab tear-off): injects workspaceId into URL so
 *     initHostNewWindow reattaches an existing workspace.
 *   - `pool:new-window` (Cmd+N / File → New Window): removes pool=1 but
 *     leaves workspaceId absent so initHostNewWindow creates a fresh workspace.
 *
 * Critical: install both listeners BEFORE signalling `pool_window_ready`.
 * The host only enqueues this label after the signal, so by construction no
 * event can fire before both listeners are live — the race window is closed.
 *
 * No client-side timeout: pool windows can sit idle for hours between
 * app start and the user's first action. A timeout would block later promotes
 * from ever bootstrapping. Lifetime is governed host-side.
 */
export async function awaitPoolPromote(): Promise<void> {
    const { listenEvent } = await import("@/app/platform/ipc");
    const { invokeCommand } = await import("@/app/platform/ipc");
    const { getApi } = await import("@/store/global");

    return new Promise<void>(async (resolve, reject) => {
        let unsub1: (() => void) | undefined;
        let unsub2: (() => void) | undefined;
        const cleanup = () => { unsub1?.(); unsub2?.(); };

        // tear-off promote: push workspaceId so initHostNewWindow reattaches.
        unsub1 = await listenEvent<{ workspaceId: string }>(
            "pool:promote",
            (payload) => {
                cleanup();
                const url = new URL(window.location.href);
                url.searchParams.set("workspaceId", payload.workspaceId);
                url.searchParams.delete("pool");
                window.history.replaceState({}, "", url.toString());
                resolve();
            },
        );

        // new-window promote: no workspaceId → initHostNewWindow creates fresh workspace.
        unsub2 = await listenEvent<Record<string, never>>(
            "pool:new-window",
            () => {
                cleanup();
                const url = new URL(window.location.href);
                url.searchParams.delete("pool");
                window.history.replaceState({}, "", url.toString());
                resolve();
            },
        );

        // Both listeners installed — safe to signal the host.
        try {
            const label = await getApi().getWindowLabel();
            await invokeCommand("pool_window_ready", { label });
        } catch (e) {
            cleanup();
            reject(new Error(`pool_window_ready signal failed: ${e}`));
        }
    });
}

/**
 * Wait for the host's `pool:pane-promote` event.
 *
 * Injects `floatingPaneId` and `workspaceId` into the URL and removes
 * `pane-pool=1`, then resolves. `initHostNewWindow` reattaches the workspace
 * (workspaceId present) and the wave renderer mounts `FloatingPaneWorkspace`
 * because `floatingPaneId` is in the URL — same code path as the cold-start
 * floating pane URL `?floatingPaneId=X&workspaceId=Y`.
 *
 * Race contract: both listeners are installed before `pane_pool_window_ready`
 * signals the host, so the promote event cannot arrive before we are ready.
 */
export async function awaitPanePoolPromote(): Promise<void> {
    const { listenEvent } = await import("@/app/platform/ipc");
    const { invokeCommand } = await import("@/app/platform/ipc");
    const { getApi } = await import("@/store/global");

    return new Promise<void>(async (resolve, reject) => {
        let unsub: (() => void) | undefined;
        const cleanup = () => { unsub?.(); };

        unsub = await listenEvent<{ paneId: string; workspaceId: string }>(
            "pool:pane-promote",
            (payload) => {
                cleanup();
                const url = new URL(window.location.href);
                url.searchParams.set("floatingPaneId", payload.paneId);
                url.searchParams.set("workspaceId", payload.workspaceId);
                url.searchParams.delete("pane-pool");
                window.history.replaceState({}, "", url.toString());
                resolve();
            },
        );

        try {
            const label = await getApi().getWindowLabel();
            await invokeCommand("pane_pool_window_ready", { label });
        } catch (e) {
            cleanup();
            reject(new Error(`pane_pool_window_ready signal failed: ${e}`));
        }
    });
}
