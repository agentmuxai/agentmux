// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The host boundary (docs/specs/SPEC_HOST_API_SEAM_2026_09_26.md): only the
 * CEF host implementation may talk to the CEF host directly — by importing
 * `app/platform/ipc` (`invokeCommand`, `listenEvent`, `invokeBrowserApi`) or
 * reading the `__AGENTMUX_IPC_*` globals. Everything else goes through
 * `getApi()`, so the UI runs on any host that implements `AppApi`.
 *
 * PENDING lists the files that still bypass the seam. It is a ratchet: a new
 * file bypassing the seam fails this test, and so does a listed file that no
 * longer does (delete it from the list — the list only shrinks).
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { describe, expect, it } from "vitest";

const FRONTEND_ROOT = join(__dirname, "..", "..");

/** The CEF host implementation itself: allowed to reach the host directly. */
const SEAM = [
    "app/init/host-detect.ts",
    "app/platform/ipc.ts",
    "cef-init.ts",
    "types/custom.d.ts",
    "util/cef-api.ts",
];

/** Still bypass the seam. Move each call behind `AppApi`, then remove it here. */
const PENDING = [
    "app-init.ts",
    "app/block/BlockErrorBoundary.tsx",
    "app/block/block.tsx",
    "app/block/blockframe.tsx",
    "app/drag/CrossWindowDragMonitor.darwin.tsx",
    "app/drag/CrossWindowDragMonitor.linux.tsx",
    "app/drag/CrossWindowDragMonitor.win32.tsx",
    "app/drag/pane-tab-tearoff.ts",
    "app/hook/useWindowDrag.darwin.ts",
    "app/hook/useWindowDrag.linux.ts",
    "app/hook/useWindowDrag.win32.ts",
    "app/init/background-audit.ts",
    "app/init/error-display.ts",
    "app/init/pool.ts",
    "app/notification/memory-pressure-banner.tsx",
    "app/notification/os/os-notify-bridge.ts",
    "app/platform/pane-overlay.ts",
    "app/statusbar/HostPopover.tsx",
    "app/store/command-registry.ts",
    "app/store/global.ts",
    "app/tab/tab-tearoff-events.ts",
    "app/view/agent/flows/open-oauth-pane.ts",
    "app/view/agent/hooks/useAgentDropAttach.ts",
    "app/view/browser/browser-model.ts",
    "app/view/browser/browser-nav-bar.tsx",
    "app/view/browser/browser-view.tsx",
    "app/view/browser/use-browser-auth.ts",
    "app/view/browser/use-drag-snapshot.ts",
    "app/view/browser/use-freeze-frame.ts",
    "app/view/browser/use-pane-rect-sync.ts",
    "app/view/credential-approval/CredentialApprovalWindow.tsx",
    "app/view/memory-adoption-approval/MemoryAdoptionApprovalWindow.tsx",
    "app/view/native-memory/MemoryAdoptionPanel.tsx",
    "app/view/native-memory/MemoryClaimsPanel.tsx",
    "app/view/settings/sections/notifications-section.tsx",
    "app/view/settings/settings-view.tsx",
    "app/view/term/term.tsx",
    "app/window/browser-pane-outside-click-bridge.ts",
    "app/window/pane-media-capture-indicator.tsx",
    "app/window/pane-media-permission-prompt.tsx",
    "app/workspace/floating-pane-workspace.tsx",
    "bootstrap.ts",
    "layout/lib/windowEdgeResize.ts",
    "log/error-forwarder.ts",
    "log/log-pipe.ts",
    "util/clipboard.ts",
    "util/dnd.ts",
];

// Static `from "…"` and dynamic `import("…")` alike.
const IPC_MODULE = String.raw`["'](?:@\/app\/platform\/ipc|(?:\.\.?\/)+(?:app\/)?platform\/ipc|\.\/ipc)["']`;
const IMPORTS_IPC = new RegExp(String.raw`(?:from\s+|import\(\s*)` + IPC_MODULE);
const READS_IPC_GLOBALS = /__AGENTMUX_IPC_/;

function collectSourceFiles(dir: string, out: string[] = []): string[] {
    for (const entry of readdirSync(dir)) {
        const full = join(dir, entry);
        if (statSync(full).isDirectory()) {
            if (entry === "node_modules" || entry === "test") continue;
            collectSourceFiles(full, out);
        } else if (/\.(ts|tsx)$/.test(entry) && !/\.test\.tsx?$/.test(entry)) {
            out.push(full);
        }
    }
    return out;
}

function filesReachingTheHost(): string[] {
    return collectSourceFiles(FRONTEND_ROOT)
        .filter((file) => {
            const text = readFileSync(file, "utf8");
            return IMPORTS_IPC.test(text) || READS_IPC_GLOBALS.test(text);
        })
        .map((file) => relative(FRONTEND_ROOT, file).split(sep).join("/"))
        .sort();
}

describe("host boundary", () => {
    it("only the seam and the pending list reach the CEF host directly", () => {
        const bypassing = filesReachingTheHost().filter((f) => !SEAM.includes(f));
        expect(
            bypassing,
            "A file reaches the CEF host directly. Add a method to AppApi (types/custom.d.ts, util/cef-api.ts) " +
                "and call it through getApi() instead. If you just removed the last direct call from a file, " +
                "delete it from PENDING in this test."
        ).toEqual([...PENDING].sort());
    });

    it("every seam file still exists and still reaches the host", () => {
        const reaching = filesReachingTheHost();
        for (const f of SEAM) {
            expect(reaching, `${f} is listed as seam but no longer reaches the host`).toContain(f);
        }
    });
});
