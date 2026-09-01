// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { ErrorBoundary } from "@/app/element/errorboundary";
import { CenteredDiv } from "@/app/element/quickelems";
import { ModalsRenderer } from "@/app/modals/modalsrenderer";
import { PaneMediaPermissionPrompt } from "@/app/window/pane-media-permission-prompt";
import { PaneMediaCaptureIndicator } from "@/app/window/pane-media-capture-indicator";
import { StatusBar } from "@/app/statusbar/StatusBar";
import { WindowHeader } from "@/app/window/window-header";
import { TabContent } from "@/app/tab/tabcontent";
import { atoms } from "@/store/global";
import { gateTargetTabId, tabSwitching } from "@/store/tab-reveal";
import { For, Show, createMemo } from "solid-js";
import type { JSX } from "solid-js";

function WorkspaceElem(): JSX.Element {
    const tabId = atoms.activeTabId;
    const ws = atoms.workspace;
    const prefersReducedMotion = atoms.prefersReducedMotionAtom;

    // Reveal gate, destination-aware (SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH §9):
    // when the holder announced WHICH tab is being revealed
    // (gateTargetTabId), only that tab hides while gated — the SOURCE tab
    // keeps painting right up to the activetabid flip instead of blanking
    // the whole content region for the RPC round trip. An untargeted hold
    // (gateTargetTabId null — createTab) falls back to hiding whichever
    // tab is active, the original behavior.
    const gateHides = (tid: string) => {
        if (tid !== tabId() || !tabSwitching()) return false;
        const target = gateTargetTabId();
        return target == null || target === tid;
    };

    // All tab IDs (pinned + regular). Keep every tab mounted so terminals
    // preserve their xterm.js instance and scrollback across tab switches.
    // Inactive tabs are hidden via display:none — no unmount/remount.
    const allTabIds = createMemo<string[]>(() => {
        const w = ws();
        if (!w) return [];
        return [...(w.pinnedtabids ?? []), ...(w.tabids ?? [])];
    });

    return (
        <div class="flex flex-col w-full flex-grow overflow-hidden">
            <WindowHeader workspace={ws()} />
            <div class="flex flex-row flex-grow overflow-hidden" style={{ "min-height": 0 }}>
                <ErrorBoundary>
                    <Show when={allTabIds().length > 0} fallback={<CenteredDiv>No Active Tab</CenteredDiv>}>
                        <For each={allTabIds()}>
                            {(tid) => (
                                <div
                                    class="flex flex-row h-full w-full"
                                    style={{
                                        display: tid === tabId() ? "flex" : "none",
                                        // Reveal gate (issue #774): hide the active tab while
                                        // it's still settling so the piecemeal mount cascade
                                        // doesn't paint stage-by-stage. `visibility: hidden`
                                        // preserves layout and suppresses paint without
                                        // unmounting children. Lifted by `tab-reveal.ts`'s
                                        // frame-budget detector. Only applies to the active
                                        // tab — inactive tabs are `display: none` already.
                                        //
                                        // Plus an opacity-fade on lift: even after the gate
                                        // releases, the first paint frame can briefly show
                                        // un-cascaded theme colors / unstyled content (the
                                        // "negative-of-a-photo" flash users reported on
                                        // 2026-05-27). The opacity transition from 0 → 1
                                        // over 120ms blends those frames into invisibility.
                                        // visibility doesn't animate so it flips instantly
                                        // when the gate lifts; opacity carries the perceived
                                        // smoothness.
                                        //
                                        // Reduced-motion users get the visibility gate
                                        // without the fade — they still need the FOUC
                                        // suppression, just not the animation. Codex P2
                                        // on PR #1108.
                                        // Spec:
                                        // SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md.
                                        visibility: gateHides(tid) ? "hidden" : null,
                                        opacity: prefersReducedMotion()
                                            ? "1"
                                            : gateHides(tid) ? "0" : "1",
                                        transition: prefersReducedMotion()
                                            ? "none"
                                            : "opacity 120ms ease-out",
                                    }}
                                >
                                    <ErrorBoundary>
                                        <TabContent tabId={tid} />
                                    </ErrorBoundary>
                                </div>
                            )}
                        </For>
                    </Show>
                    <ModalsRenderer />
                    {/* Camera/mic prompts for browser panes. Mounted in the
                        main window's DOM so the requesting page cannot draw or
                        click it — see the component's own doc comment and
                        SPEC_BROWSER_PANE_CAMERA_ACCESS_2026_09_01.md §3.5. */}
                    <PaneMediaPermissionPrompt />
                    <PaneMediaCaptureIndicator />
                </ErrorBoundary>
            </div>
            <StatusBar />
        </div>
    );
}

export { WorkspaceElem as Workspace };
