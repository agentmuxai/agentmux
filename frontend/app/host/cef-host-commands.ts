// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The CEF host's implementation of `AppApi` groups: each method is one host
// IPC command. Part of the seam (host-boundary.test.ts SEAM); later slices of
// docs/specs/SPEC_HOST_API_SEAM_2026_09_26.md add their groups here.

import { invokeBrowserApi, invokeCommand } from "@/app/platform/ipc";

/**
 * What a batch of pane rects was laid out against, in their physical pixels:
 * the page's size and the rect of the tile layout holding the panes. While a
 * window is being resized the macOS host stretches the rects by how much the
 * window has changed since, so a pane moves with the window instead of a
 * frame or two after the page (browser_panes/bounds.rs `LayoutBase`).
 */
function paneLayoutBase(blockId: string | undefined) {
    const dpr = window.devicePixelRatio || 1;
    const px = (v: number) => Math.round(v * dpr);
    const tiles = blockId ? document.querySelector(`[data-blockid="${blockId}"]`)?.closest(".tile-layout") : null;
    const r = tiles?.getBoundingClientRect();
    return {
        viewport: { width: px(window.innerWidth), height: px(window.innerHeight) },
        ...(r && r.width > 0 && r.height > 0
            ? { layout: { x: px(r.x), y: px(r.y), width: px(r.width), height: px(r.height) } }
            : {}),
    };
}

/** `AppApi.browserPanes` on CEF: one host command per method. */
export const cefBrowserPanes: BrowserPaneHostApi = {
    create: async (blockId, url, windowLabel, rect, opts) => {
        await invokeCommand("browser_pane_create", {
            block_id: blockId,
            url,
            window_label: windowLabel,
            ...rect,
            ...(opts?.allowedOrigins?.length ? { allowed_origins: opts.allowedOrigins } : {}),
            ...(opts?.identity ? { identity: opts.identity } : {}),
        });
    },
    resize: async (blockId, rect) => {
        await invokeCommand("browser_pane_resize", { block_id: blockId, ...rect });
    },
    setRects: async (windowLabel, rects) => {
        await invokeCommand("browser_panes_set_rects", {
            window_label: windowLabel,
            rects: rects.map(({ blockId, x, y, width, height }) => ({ block_id: blockId, x, y, width, height })),
            ...paneLayoutBase(rects[0]?.blockId),
        });
    },
    close: async (blockId, windowLabel) => {
        await invokeCommand("browser_pane_close", { block_id: blockId, window_label: windowLabel });
    },
    navigate: async (blockId, url) => {
        await invokeCommand("browser_pane_navigate", { block_id: blockId, url });
    },
    autocomplete: (blockId, text, searchSuggestions) =>
        invokeCommand<BrowserAutocompleteResult>("browser_pane_autocomplete", {
            block_id: blockId,
            text,
            search_suggestions: searchSuggestions,
        }),
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
        await selectInput(ownWindowLabel(), blockId);
    },
    async showPopup(popupId) {
        await invokeCommand("browser_popup_show", { id: popupId });
    },
    async closePopup(popupId) {
        await invokeCommand("browser_popup_close", { id: popupId });
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
    await selectInput(windowLabel);
}

/** `AppApi.inputOwnerState` on CEF. */
export function cefInputOwnerState(windowLabel: string): Promise<InputOwnerReport | null> {
    return invokeCommand<InputOwnerReport | null>("input_owner_state", { window_label: windowLabel });
}

// Every request for the keyboard, for the page's own content or one of its
// browser panes, goes out as one numbered `input_select`. The requests are
// separate IPCs and can reach the host out of order; the host drops one that
// arrives after a newer one. `inputEpoch`, when this page loaded (in
// microseconds), orders this page load after the one before a reload, whose
// numbers it would otherwise repeat. SPEC_BROWSER_PANE_INPUT_OWNER_2026_10_15.md §3.6.
const inputEpoch = Math.round(performance.timeOrigin * 1000);
let inputSeq = 0;

function selectInput(windowLabel: string, blockId?: string): Promise<boolean> {
    inputSeq += 1;
    return invokeCommand<boolean>("input_select", {
        window_label: windowLabel,
        block_id: blockId,
        epoch: inputEpoch,
        seq: inputSeq,
    });
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
    captureViewport: (windowLabel, quality) =>
        invokeCommand<{ jpeg_base64: string }>("capture_window_viewport", { label: windowLabel, quality }),
    recycleBegin: (windowLabel, reason) =>
        invokeCommand<{ before_mb: number | null }>("recycle_begin", { label: windowLabel, reason }),
    recycleCancel: async (windowLabel) => {
        await invokeCommand("recycle_cancel", { label: windowLabel });
    },
    takeReloadSnapshot: (windowLabel) =>
        invokeCommand<{ jpeg_base64: string | null }>("take_reload_snapshot", { label: windowLabel }),
    openFloatingPane: (args) => invokeCommand<{ window_label: string }>("open_floating_pane_window", args),
    isMaximized: async (windowLabel) =>
        (await invokeCommand<{ maximized: boolean | null }>("is_window_maximized", { label: windowLabel })).maximized ??
        null,
    toggleFloatingMaximize: async (windowLabel, blockId) => {
        await invokeCommand("toggle_floating_maximize", { label: windowLabel, block_id: blockId });
    },
    setFloatingAlwaysOnTop: async (windowLabel, blockId, on) => {
        await invokeCommand("set_floating_always_on_top", { label: windowLabel, block_id: blockId, on });
    },
    getFloatingRedockTarget: (windowLabel) =>
        invokeCommand<{ block_id?: string; dir?: number }>("get_floating_redock_target", { window_label: windowLabel }),
    updateFloatingRedockHover: (args) =>
        invokeCommand<{ target_label?: string | null }>("update_floating_redock_hover", args),
    clearFloatingRedockHover: async () => {
        await invokeCommand("clear_floating_redock_hover", {});
    },
    setFloatingRedockTarget: async (windowLabel, blockId, dir) => {
        await invokeCommand("set_floating_redock_target", { window_label: windowLabel, block_id: blockId, dir });
    },
    poolWindowReady: async (windowLabel) => {
        await invokeCommand("pool_window_ready", { label: windowLabel });
    },
    panePoolWindowReady: async (windowLabel) => {
        await invokeCommand("pane_pool_window_ready", { label: windowLabel });
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
    decideSsh: async (approvalId, approve, text, checkbox) => {
        await invokeCommand("ssh_approval_decide", { approval_id: approvalId, approve, text, checkbox });
    },
    decideBrowserAttention: async (blockId, id, decision) => {
        await invokeCommand("browser_attention_resolve", { block_id: blockId, id, decision });
    },
    setBrowserBypass: async (blockId, args) => {
        await invokeCommand("browser_bypass_set", { block_id: blockId, ...args });
    },
    setBrowserPaused: async (blockId, paused) => {
        await invokeCommand("browser_pause_set", { block_id: blockId, paused });
    },
    decideWidget: async (id, hash, approve, signer) => {
        await invokeCommand("widget_approval_decide", { id, hash, decision: approve ? "approve" : "cancel", signer: signer ?? "" });
    },
    appUpdate: (action, now) => invokeCommand("app_update_action", { action, now: now ?? false }),
    decideUpgrade: async (id, version, accept) => {
        await invokeCommand("upgrade_request_decide", { id, version, accept });
    },
    forgetWidgetKey: async (publisher) => {
        await invokeCommand("widget_approval_decide", { id: publisher, hash: "", decision: "forget_key" });
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
    // The system browser opens whenever asked: nothing to reserve.
    reserveExternalWindow: (): ExternalWindow | null => null,
    readClipboardText: () => invokeCommand<string>("read_clipboard", {}),
    readClipboardAttachments: () =>
        invokeCommand<{ text: string; paths: string[] }>("read_clipboard_attachments", {}),
    writeClipboardText: async (text: string) => {
        await invokeCommand("write_clipboard", { text });
    },
    // The host keeps one entry per window; name ours (drag_stash.rs).
    consumeDroppedFilePaths: () =>
        invokeCommand<string[]>("consume_drag_paths", { windowLabel: ownWindowLabel() }),
    peekDroppedFilePaths: () => invokeCommand<string[]>("peek_drag_paths", { windowLabel: ownWindowLabel() }),
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
    listIncidents: () => invokeCommand<unknown>("incidents_list", {}),
    acknowledgeIncident: (id: string) => invokeCommand<unknown>("incident_acknowledge", { id }),
} satisfies Partial<AppApi>;

/** This window's label, as set by the host on its startup URL (`?windowLabel=`). */
function ownWindowLabel(): string {
    try {
        return new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
    } catch {
        return "main";
    }
}
