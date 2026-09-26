// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The CEF host's implementation of `AppApi` groups: each method is one host
// IPC command. Part of the seam (host-boundary.test.ts SEAM); later slices of
// docs/specs/SPEC_HOST_API_SEAM_2026_09_26.md add their groups here.

import { invokeBrowserApi, invokeCommand } from "@/app/platform/ipc";

/** `AppApi.browserPanes` on CEF: one host command per method. */
export const cefBrowserPanes: BrowserPaneHostApi = {
    create: async (blockId, url, windowLabel, rect) => {
        await invokeCommand("browser_pane_create", { block_id: blockId, url, window_label: windowLabel, ...rect });
    },
    resize: async (blockId, rect) => {
        await invokeCommand("browser_pane_resize", { block_id: blockId, ...rect });
    },
    close: async (blockId, windowLabel) => {
        await invokeCommand("browser_pane_close", { block_id: blockId, window_label: windowLabel });
    },
    navigate: async (blockId, url) => {
        await invokeCommand("browser_pane_navigate", { block_id: blockId, url });
    },
    goBack: async (blockId) => {
        await invokeCommand("browser_pane_go_back", { block_id: blockId });
    },
    goForward: async (blockId) => {
        await invokeCommand("browser_pane_go_forward", { block_id: blockId });
    },
    reload: async (blockId) => {
        await invokeCommand("browser_pane_reload", { block_id: blockId });
    },
    focus: async (blockId) => {
        await invokeCommand("browser_pane_focus", { block_id: blockId });
    },
    cut: async (blockId) => {
        await invokeCommand("browser_pane_cut", { block_id: blockId });
    },
    copy: async (blockId) => {
        await invokeCommand("browser_pane_copy", { block_id: blockId });
    },
    paste: async (blockId) => {
        await invokeCommand("browser_pane_paste", { block_id: blockId });
    },
    print: async (blockId) => {
        await invokeCommand("browser_pane_print", { block_id: blockId });
    },
    viewSource: async (blockId) => {
        await invokeCommand("browser_pane_view_source", { block_id: blockId });
    },
    inspectElement: async (blockId, x, y) => {
        await invokeCommand("browser_pane_inspect_element", { block_id: blockId, x, y });
    },
    screenshot: (blockId, opts) =>
        invokeBrowserApi<{ png_base64: string }>("screenshot", { block_id: blockId, ...opts }),
    authSubmit: async (requestId, username, password) => {
        await invokeCommand("browser_pane_auth_submit", { request_id: requestId, username, password });
    },
    authCancel: async (requestId) => {
        await invokeCommand("browser_pane_auth_cancel", { request_id: requestId });
    },
    authSave: async (c) => {
        await invokeCommand("browser_pane_auth_save", {
            block_id: c.blockId,
            origin: c.origin,
            realm: c.realm,
            is_proxy: c.isProxy,
            username: c.username,
            password: c.password,
        });
    },
    setOverlayClip: async (rects, windowLabel) => {
        await invokeCommand("browser_panes_set_overlay_clip", { rects, window_label: windowLabel });
    },
    respondMediaPermission: async (requestId, allow) => {
        await invokeCommand("pane_media_permission_respond", { requestId, allow });
    },
    revokeMedia: async (blockId) => {
        await invokeCommand("pane_media_revoke", { blockId });
    },
};

/** `AppApi.reclaimWindowFocus` on CEF. */
export async function cefReclaimWindowFocus(windowLabel: string): Promise<void> {
    await invokeCommand("main_window_focus", { window_label: windowLabel });
}
