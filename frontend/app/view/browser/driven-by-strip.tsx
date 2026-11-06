// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The header strip of a browser pane an agent drives: who drives it, its site
// limit, Take over, Auto-approve (SPEC_BROWSER_PANE_BYPASS_2026_10_16.md §3;
// the spec's "Bypass", renamed for people), Pause, and the Log of what the
// agent did (PLAN_BROWSER_DRIVER_FIXES_2026_10_17.md §5).
//
// Auto-approve's confirmation and the Log are panels in the pane's own flow,
// not floating popovers: the page is a native window drawn over anything
// layered on it, and the panels must stay inside the pane's DOM, where no
// other agent's UI click can reach (spec §5).

import { Button } from "@/app/element/ui";
import { createSignal, Show, type JSX } from "solid-js";
import type { BrowserViewModel } from "./browser-model";
import { ActionLogPanel, BypassConfirm, BypassStrip, PlainStrip } from "./driven-by-strip-parts";
import { createDrivenBy } from "./driven-by-strip-state";

export function DrivenByStrip(props: { model: BrowserViewModel; agent: string }): JSX.Element {
    const t = createDrivenBy(props);
    const model = props.model;
    return (
        <>
            <Show when={model.bypassAtom()} fallback={<PlainStrip t={t} p={props} />}>
                {(b) => <BypassStrip t={t} p={props} grant={b} />}
            </Show>
            <Show when={t.showLog()}>
                <ActionLogPanel model={model} agent={props.agent} onClose={() => t.setShowLog(false)} />
            </Show>
            <Show when={t.confirming() && !model.bypassAtom()}>
                <BypassConfirm t={t} p={props} />
            </Show>
        </>
    );
}

/** After Take over (or once no agent drives the pane) the log srv kept is
 *  still the person's to read: a slim strip with only its Log. */
export function ActionLogStrip(props: { model: BrowserViewModel }): JSX.Element {
    const [showLog, setShowLog] = createSignal(false);
    const entries = () => props.model.actionsAtom().entries;
    // Whoever drove it last: the newest entry's agent.
    const agent = () => entries()[entries().length - 1]?.agent || "The agent";
    return (
        <>
            <div class="browser-driven-by browser-driven-by--ended" role="status">
                <i class="fa-solid fa-list" aria-hidden="true" />
                <div class="browser-driven-by-message">
                    <span>
                        <b>{agent()}</b>'s actions here
                    </span>
                </div>
                <div class="browser-driven-by-actions">
                    <Button
                        density="compact"
                        class="browser-actions-log-button"
                        pressed={showLog()}
                        title={`What ${agent()} did in this pane`}
                        onClick={() => setShowLog(!showLog())}
                    >
                        Log ({props.model.actionsAtom().count})
                    </Button>
                </div>
            </div>
            <Show when={showLog()}>
                <ActionLogPanel model={props.model} agent={agent()} onClose={() => setShowLog(false)} />
            </Show>
        </>
    );
}
