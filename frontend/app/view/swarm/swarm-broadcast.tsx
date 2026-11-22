// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Swarm's Broadcast input: one box under the Filter. Enter sends to the selected agents, Shift+Enter adds a line.
// The draft lives outside the component, so switching away and back loses nothing.

import { actionableKeys } from "./swarm-filter-state";
import { createEffect, createMemo, createSignal, on, onCleanup, Show, type Accessor, type JSX, type Setter } from "solid-js";
import { TextArea } from "@/app/element/ui";
import { keyLabel } from "@/app/keybindings";
import { t } from "@/app/i18n/i18n";
import { onMuxEvent } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import type { SwarmAgents } from "./swarm-agent-list";
import { gapSummary, localFleetTargets, partitionTargets, remoteFleetTargets } from "./swarm-fleet-targets";
import type { SwarmViewModel } from "./swarm-model";
import { remoteSections } from "./swarm-remote";
import { tip } from "@/app/element/tip";

/** The tallest the box grows by itself, in px (about eight lines); past it the text scrolls. A height the person drags to is theirs. */
export const MAX_AUTO_HEIGHT = 160;

/** The height that shows all of a box's text on its own, up to `MAX_AUTO_HEIGHT`; null while the box has no layout (nothing to fit). */
export function fittedHeight(scrollHeight: number, borders: number): number | null {
    if (scrollHeight <= 0) return null;
    return Math.min(scrollHeight + borders, MAX_AUTO_HEIGHT);
}

/** How far a broadcast whose deliveries srv paces has got. */
export interface BroadcastProgress {
    sent: number;
    total: number;
}

/** No progress is shown for this long: a small broadcast is over by then, and a figure that flashes says nothing. */
export const PROGRESS_AFTER_MS = 300;

/**
 * Follows the deliveries of the broadcast sent with `id`: srv reports each one that ends
 * (crates/srv/src/server/app_api/fleet_pacing.rs). `set` gets the latest count once the send has
 * lasted `PROGRESS_AFTER_MS`, and null when the returned stop is called.
 */
export function followBroadcastProgress(id: string, set: (p: BroadcastProgress | null) => void): () => void {
    let latest: BroadcastProgress | null = null;
    let showing = false;
    const unsub = onMuxEvent(WpsEvent.FleetBroadcastProgress, (e) => {
        if (e.id !== id) return;
        latest = { sent: e.sent, total: e.total };
        if (showing) set(latest);
    });
    const timer = setTimeout(() => {
        showing = true;
        if (latest) set(latest);
    }, PROGRESS_AFTER_MS);
    return () => {
        clearTimeout(timer);
        unsub();
        set(null);
    };
}

interface BroadcastState {
    draft: Accessor<string>;
    setDraft: Setter<string>;
}

const states = new WeakMap<object, BroadcastState>();

/** The draft of one Swarm pane, created on first use and kept for its life. */
function broadcastStateOf(model: object): BroadcastState {
    let state = states.get(model);
    if (!state) {
        const [draft, setDraft] = createSignal("");
        state = { draft, setDraft };
        states.set(model, state);
    }
    return state;
}

/**
 * The box is one line and grows with its text. A height the person drags it to stays until the
 * draft is empty again (after a send, or when they clear it). Returns the box's `ref`.
 */
function useAutoGrow(draft: Accessor<string>): (el: HTMLTextAreaElement) => void {
    let input: HTMLTextAreaElement | undefined;
    let fitted = 0;
    let dragged = false;
    let width = 0;
    const fit = () => {
        if (!input || dragged) return;
        input.style.height = "auto";
        const h = fittedHeight(input.scrollHeight, input.offsetHeight - input.clientHeight);
        if (h === null) {
            input.style.removeProperty("height");
            fitted = 0;
            return;
        }
        fitted = h;
        input.style.height = `${h}px`;
    };
    createEffect(() => {
        if (draft() === "") {
            dragged = false;
            input?.style.removeProperty("height");
            fitted = 0;
        } else fit();
    });
    // The resize handle writes its own inline height, so one that is not the one we set is the person's.
    // (The box's rendered height is not the test: a short pane can squeeze it without a drag.) A new width re-wraps the text.
    return (el) => {
        input = el;
        if (typeof ResizeObserver === "undefined") return;
        const observer = new ResizeObserver(() => {
            if (el.offsetWidth !== width) {
                width = el.offsetWidth;
                return fit();
            }
            // Also while the draft is empty (fitted is 0, no inline height of ours): a drag then is kept for the first keystroke.
            if (el.style.height !== "" && el.style.height !== `${fitted}px`) dragged = true;
        });
        observer.observe(el);
        onCleanup(() => observer.disconnect());
    };
}

/** "1 agent", "3 agents". */
export function agentsOf(n: number): string {
    return `${n} agent${n === 1 ? "" : "s"}`;
}

/** What the line under the box says: how the last send went, or that agents must be selected first. */
type BroadcastNote = { kind: "sent" | "pick" | "failed"; text: string };

/** The one line under the box for what the person needs to know now; it takes no room while there is nothing to say. */
function BroadcastNoteLine(props: { progress: BroadcastProgress | null; note: BroadcastNote | null; gap: string | null; hidden: number }): JSX.Element {
    return (
        <Show when={props.progress ?? props.note ?? props.gap ?? (props.hidden > 0 ? true : null)}>
            <div class="swarm-broadcast-note" role="status" data-kind={props.progress ? "progress" : props.note?.kind}>
                <Show when={props.progress}>
                    {(p) => <span class="swarm-broadcast-progress">{t("swarm.broadcast.progress", { sent: p().sent, total: p().total })}</span>}
                </Show>
                <Show when={!props.progress && props.note}>{(n) => <span>{n().text}</span>}</Show>
                <Show when={props.gap}>{(text) => <span class="swarm-fleet-broadcast-gap">{text()}</span>}</Show>
                <Show when={props.hidden > 0}>
                    <span class="swarm-fleet-broadcast-gap">{props.hidden} hidden by the filter, not included</span>
                </Show>
            </div>
        </Show>
    );
}

export function BroadcastDock(props: { model: SwarmViewModel; agents: Pick<SwarmAgents, "tree"> }): JSX.Element {
    const model = props.model;
    const state = broadcastStateOf(model);
    // From the list the pane already keeps (which rebuilds only while the pane is visible), not
    // `model.fleetTargets()`, which would rebuild the agent tree on every agent event.
    const known = createMemo(
        () => new Map([...localFleetTargets(props.agents.tree()), ...remoteFleetTargets(remoteSections(model.otherInstancesAtom()))])
    );
    // The selected agents the filter shows: one it hides stays selected but is not sent to.
    const split = createMemo(() => partitionTargets(actionableKeys(model, model.selectedBlockIdsAtom()), "broadcast", known()));
    const recipients = () => split().reachable.length;
    const selected = () => actionableKeys(model, model.selectedBlockIdsAtom()).length;
    const hidden = () => model.selectedBlockIdsAtom().size - selected();
    const gap = () => gapSummary(split().unreachable, selected(), "broadcast");
    const canSend = () => state.draft().trim().length > 0 && recipients() > 0 && !model.fleetActionInFlightAtom();
    const watch = useAutoGrow(state.draft);
    const [progress, setProgress] = createSignal<BroadcastProgress | null>(null);
    const [note, setNote] = createSignal<BroadcastNote | null>(null);
    // A line about the last send or about an empty selection is about that moment: a new selection or new text ends it.
    createEffect(on(() => model.selectedBlockIdsAtom(), () => setNote(null), { defer: true }));

    // A second Enter while a send is under way does not send twice.
    let sending = false;
    const send = async (): Promise<void> => {
        if (state.draft().trim().length === 0 || sending || model.fleetActionInFlightAtom()) return;
        // Never "everyone" when nobody is selected: one Enter would start a turn on every agent.
        if (recipients() === 0) {
            setNote({ kind: "pick", text: t(selected() === 0 ? "swarm.broadcast.pick" : "swarm.broadcast.noneReachable") });
            return;
        }
        const text = state.draft().trim();
        const count = recipients();
        state.setDraft("");
        setNote(null);
        sending = true;
        let result;
        // A broadcast to many agents is paced by srv and takes a few seconds: the count shows it is going out.
        const progressId = crypto.randomUUID();
        const stopProgress = followBroadcastProgress(progressId, setProgress);
        try {
            result = await model.broadcastToSelection(text, progressId);
        } catch {
            // The call itself failed: nothing was sent, so the message is not lost.
            if (state.draft() === "") state.setDraft(text);
            return;
        } finally {
            sending = false;
            stopProgress();
        }
        // Every delivery failed: the text stays, the result panel under the box says why per agent.
        if (result.succeeded.length === 0 && (result.held ?? []).length === 0) {
            if (state.draft() === "") state.setDraft(text);
            setNote({ kind: "failed", text: t("swarm.broadcast.failed", { who: agentsOf(count) }) });
            return;
        }
        setNote({ kind: "sent", text: t("swarm.broadcast.sent", { who: agentsOf(result.succeeded.length) }) });
    };

    const placeholder = () =>
        selected() === 0 ? t("swarm.broadcast.placeholder") : t("swarm.broadcast.placeholderTo", { who: agentsOf(recipients()) });
    const tooltip = () =>
        selected() === 0
            ? t("swarm.broadcast.tipNone", { newlineKey: keyLabel("shift+Enter") })
            : t("swarm.broadcast.tip", { who: agentsOf(recipients()), newlineKey: keyLabel("shift+Enter") });

    return (
        <div class="swarm-broadcast-dock" data-operator-only="">
            <TextArea
                class="swarm-broadcast-input"
                ref={watch}
                rows={1}
                placeholder={placeholder()}
                aria-label={t("swarm.broadcast.placeholder")}
                value={state.draft()}
                {...tip(tooltip())}
                onInput={(e) => {
                    state.setDraft(e.currentTarget.value);
                    setNote(null);
                }}
                onKeyDown={(e) => {
                    // Enter sends and Shift+Enter adds a line, as in an agent's composer. An IME that is converting
                    // owns Enter (keyCode 229 is Safari's synthetic keydown for it).
                    if (e.key !== "Enter" || e.shiftKey || e.isComposing || e.keyCode === 229) return;
                    e.preventDefault();
                    void send();
                }}
            />
            <BroadcastNoteLine progress={progress()} note={note()} gap={gap()} hidden={hidden()} />
        </div>
    );
}
