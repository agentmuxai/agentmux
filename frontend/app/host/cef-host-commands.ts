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

/** `AppApi.windows` on CEF. */
export const cefWindows: WindowHostApi = {
    startDrag: async (windowLabel) => {
        await invokeCommand("start_window_drag", { label: windowLabel });
    },
    maximize: async (windowLabel) => {
        await invokeCommand("maximize_window", windowLabel === undefined ? undefined : { label: windowLabel });
    },
    getPosition: (windowLabel) => invokeCommand<{ x: number; y: number }>("get_window_position", { label: windowLabel }),
    setPosition: async (windowLabel, x, y) => {
        await invokeCommand("set_window_position", { x, y, label: windowLabel });
    },
    getRect: (windowLabel) => invokeCommand<HostRect>("get_window_rect", { label: windowLabel }),
    setRect: async (windowLabel, rect) => {
        await invokeCommand("set_window_rect", { label: windowLabel, ...rect });
    },
    getCursorScreenPoint: () => invokeCommand<{ x: number; y: number }>("get_cursor_point"),
    getPaneDebugState: () => invokeCommand<Record<string, unknown>>("get_pane_debug_state", {}),
    openFloatingPane: (args) => invokeCommand<{ window_label: string }>("open_floating_pane_window", args),
    toggleFloatingMaximize: async (windowLabel, blockId) => {
        await invokeCommand("toggle_floating_maximize", { label: windowLabel, block_id: blockId });
    },
    getFloatingRedockTarget: (windowLabel) =>
        invokeCommand<{ block_id?: string; dir?: number }>("get_floating_redock_target", { window_label: windowLabel }),
    updateFloatingRedockHover: (args) =>
        invokeCommand<{ target_label?: string | null }>("update_floating_redock_hover", args),
    clearFloatingRedockHover: async () => {
        await invokeCommand("clear_floating_redock_hover", {});
    },
    resolveWindowAtCursor: (args) =>
        invokeCommand<{ label: string | null; window_id: string | null }>("resolve_window_at_cursor", args),
};

/** `AppApi.approvals` on CEF. */
export const cefApprovals: ApprovalHostApi = {
    decideCredential: async (approvalId, approve) => {
        await invokeCommand("credential_approval_decide", { approval_id: approvalId, approve });
    },
    decideMemoryAdoption: async (approvalId, approve) => {
        await invokeCommand("memory_adoption_decide", { approval_id: approvalId, approve });
    },
    requestMemoryAdoption: async (args) => {
        await invokeCommand("memory_adoption_request", args);
    },
    requestMemoryRelease: async (args) => {
        await invokeCommand("memory_release_request", args);
    },
};

/** The flat `AppApi` methods added by slice 5 of the host seam, on CEF. */
export const cefHostMisc = {
    openExternalChecked: async (url: string) => {
        await invokeCommand("open_external", { url });
    },
    readClipboardText: () => invokeCommand<string>("read_clipboard", {}),
    writeClipboardText: async (text: string) => {
        await invokeCommand("write_clipboard", { text });
    },
    consumeDroppedFilePaths: () => invokeCommand<string[]>("consume_drag_paths", {}),
    copyFileToDir: (sourcePath: string, targetDir: string) =>
        invokeCommand<string>("copy_file_to_dir", { sourcePath, targetDir }),
    openDataDirInFileManager: async () => {
        await invokeCommand("open_in_file_manager", { target: "data" });
    },
    getHostInfo: () => invokeCommand<Record<string, unknown>>("get_host_info", {}),
    setTaskbarAttention: async (windowLabel: string, count: number, inputCount: number) => {
        await invokeCommand("set_taskbar_attention", { label: windowLabel, count, input_count: inputCount });
    },
    takeBackgroundAudit: () => invokeCommand<unknown>("background_audit_take", {}),
} satisfies Partial<AppApi>;
