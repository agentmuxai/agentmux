// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Floating-pane shell — owned-child-window UI for SPEC_FLOATING_PANE_TEAROFF.
//
// Phase 1 (#810) shipped the Windows host primitive that creates an owned
// `WS_POPUP | WS_EX_TOOLWINDOW` HWND with no taskbar entry. Phase 3 (this file
// + CrossWindowDragMonitor.win32.tsx) wired pane drag-out to spawn this
// shell instead of a new top-level AgentMux instance.
//
// What's here now (PR-A1):
//   - Draggable title bar (CSS `-webkit-app-region: drag` → WM_NCHITTEST
//     HTCAPTION on Windows) so the floater can be repositioned.
//   - Close button.
//   - The paneId is shown so the user can confirm the right block was
//     torn out; the real `<Block>` renderer ships in PR-A2 (Phase 2).
//
// What's NOT here yet (PR-A2 will add):
//   - The actual `<Block>` for `paneId` — requires the same RPC/WOS init
//     that initApp() does, with the App component swapped for the shell.
//   - Pane title from `meta.view`.
//   - Dock-back button (Phase 4).

import { render } from "solid-js/web";
import { getApi } from "@/store/global";

interface Props {
    paneId: string;
    windowLabel: string;
}

function FloatingPaneShell(props: Props) {
    const handleClose = () => {
        if (!props.windowLabel) {
            console.warn("[floating-pane] no windowLabel — cannot close via host IPC");
            return;
        }
        getApi()
            .closeWindowByLabel(props.windowLabel)
            .catch((e) => console.error("[floating-pane] closeWindowByLabel failed", e));
    };

    return (
        <div
            style={{
                position: "fixed",
                inset: "0",
                display: "flex",
                "flex-direction": "column",
                "font-family": "system-ui, -apple-system, Segoe UI, sans-serif",
                color: "#e5e7eb",
                background: "#0a0a0f",
            }}
        >
            {/* Draggable title bar.
                `-webkit-app-region: drag` instructs Chromium to treat this
                region as the OS-level window drag handle. On Windows our
                NCHITTEST shim returns HTCAPTION here, so dragging the bar
                moves the floater across the desktop (across monitors,
                across DPI boundaries) via the standard Win32 drag loop.
                Buttons inside the bar opt out via `app-region: no-drag`
                so clicks register normally. */}
            <div
                style={{
                    display: "flex",
                    "align-items": "center",
                    height: "32px",
                    "background-color": "#161620",
                    "border-bottom": "1px solid #262633",
                    "padding-left": "12px",
                    "padding-right": "4px",
                    "user-select": "none",
                    "-webkit-app-region": "drag",
                } as any}
            >
                <div style={{ flex: "1 1 auto", "font-size": "12px", color: "#94a3b8" }}>
                    {`Pane ${props.paneId.slice(0, 8)}`}
                </div>
                <button
                    type="button"
                    onClick={handleClose}
                    title="Close floating pane"
                    aria-label="Close"
                    style={{
                        width: "28px",
                        height: "24px",
                        border: "none",
                        background: "transparent",
                        color: "#94a3b8",
                        cursor: "pointer",
                        "font-size": "16px",
                        "line-height": "1",
                        "border-radius": "4px",
                        "-webkit-app-region": "no-drag",
                    } as any}
                    onMouseEnter={(e) => {
                        (e.currentTarget as HTMLElement).style.background = "#26263a";
                        (e.currentTarget as HTMLElement).style.color = "#e5e7eb";
                    }}
                    onMouseLeave={(e) => {
                        (e.currentTarget as HTMLElement).style.background = "transparent";
                        (e.currentTarget as HTMLElement).style.color = "#94a3b8";
                    }}
                >
                    ×
                </button>
            </div>

            {/* Placeholder pane content. PR-A2 replaces this with `<Block>`. */}
            <div
                style={{
                    flex: "1 1 auto",
                    display: "flex",
                    "flex-direction": "column",
                    "align-items": "center",
                    "justify-content": "center",
                    gap: "10px",
                    padding: "16px",
                    "font-size": "12px",
                    color: "#6b7280",
                    "text-align": "center",
                }}
            >
                <div style={{ "font-size": "14px", color: "#94a3b8" }}>Floating pane</div>
                <div>
                    The host primitive works. The real renderer ships in PR-A2 (Phase 2 of
                    <code style={{ "margin-left": "0.4em" }}>SPEC_FLOATING_PANE_TEAROFF</code>).
                </div>
                <pre
                    style={{
                        "font-size": "11px",
                        color: "#6b7280",
                        background: "#111118",
                        padding: "8px 12px",
                        "border-radius": "6px",
                        margin: "0",
                    }}
                >
                    {`paneId       ${props.paneId}\nwindowLabel  ${props.windowLabel}`}
                </pre>
            </div>
        </div>
    );
}

/**
 * Mount the floating-pane shell into `#root` (the element index.html
 * provides for the main app). Called from `bootstrap.ts` when a
 * `floatingPaneId` query parameter is present.
 */
export function renderFloatingPaneShell(paneId: string, windowLabel: string): void {
    const root = document.getElementById("root");
    if (!root) {
        console.error("[floating-pane] #root element missing — cannot mount shell");
        return;
    }
    // Clear any startup-loading content the index.html shipped.
    root.innerHTML = "";

    render(() => <FloatingPaneShell paneId={paneId} windowLabel={windowLabel} />, root);

    console.log(
        `[floating-pane] shell mounted (paneId=${paneId}, windowLabel=${windowLabel})`,
    );
}
