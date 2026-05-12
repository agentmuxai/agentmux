// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase 1 stub for the floating-pane shell (issue #810 / spec
// SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md).
//
// When `bootstrap.ts` sees `?floatingPaneId=<id>` in the URL it
// renders this shell instead of the full workspace (`initApp()`). The
// shell currently shows a placeholder — Phase 2 will swap the
// placeholder for the real `<Block>` renderer so floating panes are
// full peers of docked panes.
//
// This file is the *minimum* needed to validate the Phase 1
// host-side primitive end-to-end: a Windows dev can call the
// `open_floating_pane_window` IPC, see a free-floating tool window
// appear with no taskbar entry, and verify that CEF embedded a
// browser pointing at the right URL.
//
// Anything you'd want a real floating window to do — drag the title
// bar, dock back, render a `<Block>` — is Phase 2/4. Don't add it
// here.

import { render } from "solid-js/web";

interface Props {
    paneId: string;
    windowLabel: string;
}

function FloatingPaneShell(props: Props) {
    return (
        <div
            style={{
                position: "fixed",
                inset: "0",
                display: "flex",
                "flex-direction": "column",
                "align-items": "center",
                "justify-content": "center",
                "font-family": "system-ui, -apple-system, Segoe UI, sans-serif",
                color: "#e5e7eb",
                background: "#0a0a0f",
                gap: "12px",
                padding: "16px",
            }}
        >
            <div style={{ "font-size": "20px", "font-weight": "600" }}>
                Floating pane
            </div>
            <div style={{ "font-size": "13px", color: "#94a3b8", "text-align": "center" }}>
                Phase 1 placeholder.
                <br />
                The real renderer ships in Phase 2.
            </div>
            <pre
                style={{
                    "font-size": "11px",
                    color: "#94a3b8",
                    background: "#111118",
                    padding: "10px 14px",
                    "border-radius": "6px",
                    margin: "0",
                }}
            >
                {`paneId       ${props.paneId}\nwindowLabel  ${props.windowLabel}`}
            </pre>
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
