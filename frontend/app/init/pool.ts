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

/** True when the current renderer was spawned as a pool window. */
export function isPoolMode(): boolean {
    if (typeof window === "undefined") return false;
    return new URLSearchParams(window.location.search).get("pool") === "1";
}

/**
 * Wait for the host's `pool:promote` event. Resolves with the
 * workspace ID that should be bootstrapped. Times out after 5
 * minutes — past that we treat the pool slot as orphaned and let
 * the renderer hang in pool mode (the host will clean up at
 * shutdown).
 *
 * Critical: install the listener BEFORE signalling
 * `pool_window_ready` to the host. The host only adds this label
 * to its pool queue after the signal — so any promote that fires
 * before then is impossible by construction. Without this gate
 * the host's emit_event_to_window would race our listenEvent
 * install and the promote would silently drop. (codex PR #566 P1)
 */
export async function awaitPoolPromote(): Promise<string> {
    const { listenEvent } = await import("@/app/platform/ipc");
    const { invokeCommand } = await import("@/app/platform/ipc");
    const { getApi } = await import("@/store/global");

    return new Promise<string>(async (resolve, reject) => {
        const timer = setTimeout(() => {
            reject(new Error("pool:promote timed out after 5min"));
        }, 5 * 60 * 1000);

        const unsub = await listenEvent<{ workspaceId: string }>(
            "pool:promote",
            (payload) => {
                clearTimeout(timer);
                unsub();
                // Push the workspaceId into the URL so initHostNewWindow's
                // existing `URLSearchParams.get("workspaceId")` lookup
                // picks it up without any further plumbing.
                const url = new URL(window.location.href);
                url.searchParams.set("workspaceId", payload.workspaceId);
                // pool=1 is no longer accurate — flip it off so future
                // re-init paths (HMR, etc.) take the normal route.
                url.searchParams.delete("pool");
                window.history.replaceState({}, "", url.toString());
                resolve(payload.workspaceId);
            },
        );

        // Listener installed — now safe to signal the host that this
        // pool slot can receive promote events.
        try {
            const label = await getApi().getWindowLabel();
            await invokeCommand("pool_window_ready", { label });
        } catch (e) {
            clearTimeout(timer);
            unsub();
            reject(new Error(`pool_window_ready signal failed: ${e}`));
        }
    });
}
