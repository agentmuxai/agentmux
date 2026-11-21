// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which window tab the pointer is about to switch to: the one whose pill it rests on (after a short dwell) or
 * has pressed. The switch itself happens on the release, so this is the lead the window has to get ready:
 * sleeping browser panes start waking (`view/browser/hibernation.ts`), and the agent panes that changed while
 * the tab was hidden draw ahead of the switch (`workspace/agent-prewarm-driver.ts`).
 *
 * One module for the window so the agent lane does not import from the browser view
 * (docs/specs/SPEC_WINDOW_TAB_SWITCH_DRAW_ON_INTENT_2026_11_12.md §4.1).
 */

import { createSignal } from "solid-js";

const [intentTab, setIntentTab] = createSignal<string | null>(null);

/** The window tab the pointer is about to switch to, or null. */
export const tabIntentId = intentTab;

/** What the intent was when the switch happened, for the `[tab-intent]` line. */
export interface IntentLead {
    /** From the pointer's arrival on the pill to the switch (ms), or null when it did not come by that pill. */
    hoverMs: number | null;
    /** From the press to the switch (ms), or null with no press seen. */
    pressMs: number | null;
}

let arrivedAt: { tabId: string; at: number } | null = null;
let pressedAt: { tabId: string; at: number } | null = null;

/** The pointer entered the tab's pill (before the dwell). */
export function noteTabArrival(tabId: string, at: number = performance.now()): void {
    arrivedAt = { tabId, at };
    if (pressedAt?.tabId === tabId) pressedAt = null;
}

/** The pointer pressed the tab's pill. */
export function noteTabPress(tabId: string, at: number = performance.now()): void {
    pressedAt = { tabId, at };
}

/** Make `tabId` the intent (null clears it). */
export function setTabIntent(tabId: string | null): void {
    setIntentTab(tabId);
}

/** The pointer left the pill of `tabId`. */
export function noteTabLeave(tabId: string): void {
    if (arrivedAt?.tabId === tabId) arrivedAt = null;
    if (pressedAt?.tabId === tabId) pressedAt = null;
}

/** The lead the window had when it switched to `tabId` by the pill; consumes what was noted. */
export function takeLead(tabId: string, now: number = performance.now()): IntentLead {
    const hover = arrivedAt?.tabId === tabId ? Math.round(now - arrivedAt.at) : null;
    const press = pressedAt?.tabId === tabId ? Math.round(now - pressedAt.at) : null;
    arrivedAt = null;
    pressedAt = null;
    return { hoverMs: hover, pressMs: press };
}

type SwitchReporter = (tabId: string, lead: IntentLead) => void;
let reporter: SwitchReporter | null = null;

/** Who writes the `[tab-intent]` line at a switch by the pill (the workspace's prewarm driver, which knows the panes). */
export function setSwitchReporter(next: SwitchReporter | null): void {
    reporter = next;
}

/** The person switched to `tabId` by its pill: hands the lead the window had to the reporter. */
export function reportTabSwitch(tabId: string): void {
    const lead = takeLead(tabId);
    if (lead.hoverMs == null && lead.pressMs == null) return;
    reporter?.(tabId, lead);
}

/** The line written at a switch: the lead, and how many of the tab's panes were drawn ahead. */
export function intentLine(lead: IntentLead, drawn: number, of: number): string {
    const ms = (v: number | null) => (v == null ? "-" : `${v}ms`);
    return `[tab-intent] hover=${ms(lead.hoverMs)} press=${ms(lead.pressMs)} drawn=${drawn}/${of}`;
}
