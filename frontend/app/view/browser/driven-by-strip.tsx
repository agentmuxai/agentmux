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

import { createSignal, For, Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { actionFailed, actionKind, actionTime, shortUrl } from "./action-log";
import { BYPASS_MINUTES, minutesLabel, siteLabel } from "./bypass";
import type { BrowserViewModel } from "./browser-model";

export function DrivenByStrip(props: { model: BrowserViewModel; agent: string }): JSX.Element {
    const model = props.model;
    const [confirming, setConfirming] = createSignal(false);
    const [minutes, setMinutes] = createSignal(60);
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | undefined>();
    const sites = () => model.bypassOriginsPreview();
    const [showLog, setShowLog] = createSignal(false);
    const [pauseError, setPauseError] = createSignal<string | undefined>();

    const turnOn = (e: MouseEvent) => {
        // Only a pointer click grants it: a key press on the focused button
        // (detail 0) doesn't, so no synthesized key can (spec §3.2).
        if (e.detail === 0 || busy()) return;
        setBusy(true);
        setError(undefined);
        model
            .setBypass(true, minutes())
            .then(() => setConfirming(false))
            .catch((err) => setError(String(err?.message ?? err)))
            .finally(() => setBusy(false));
    };
    // A failed Stop auto-approve shows in the red strip itself: the
    // confirmation, where other errors show, is hidden while it is on.
    const end = () => {
        setError(undefined);
        model.setBypass(false).catch((err) => setError(`Couldn't stop auto-approve: ${String(err?.message ?? err)}`));
    };
    // Pause is the person's brake, so any press works; Resume lets the agent
    // act again, so only a pointer click does, like turning Auto-approve on.
    const togglePause = (e: MouseEvent) => {
        const paused = model.pausedAtom();
        if (paused && e.detail === 0) return;
        setPauseError(undefined);
        model
            .setPaused(!paused)
            .catch((err) => setPauseError(`Couldn't ${paused ? "resume" : "pause"}: ${String(err?.message ?? err)}`));
    };
    const controls = () => (
        <>
            <Button
                density="compact"
                class="browser-actions-log-button"
                pressed={showLog()}
                title={`What ${props.agent} did in this pane`}
                onClick={() => setShowLog(!showLog())}
            >
                Log ({model.actionsAtom().count})
            </Button>
            <Button
                density="compact"
                class="browser-pause-button"
                pressed={model.pausedAtom()}
                title={
                    model.pausedAtom()
                        ? `Let ${props.agent} act in this pane again.`
                        : `Hold ${props.agent}'s actions in this pane until you resume; auto-approve's time stops too.`
                }
                onClick={togglePause}
            >
                {model.pausedAtom() ? "Resume" : "Pause"}
            </Button>
        </>
    );
    const pauseErrorLine = () => (
        <Show when={pauseError()}>
            <span class="browser-bypass-strip-error" role="alert">
                {pauseError()}
            </span>
        </Show>
    );

    return (
        <>
            <Show
                when={model.bypassAtom()}
                fallback={
                    <div
                        class="browser-driven-by"
                        classList={{ "browser-driven-by--held": model.pausedAtom() }}
                        role="status"
                    >
                        <i class={model.pausedAtom() ? "fa-solid fa-pause" : "fa-solid fa-robot"} aria-hidden="true" />
                        <Show
                            when={!model.pausedAtom()}
                            fallback={
                                <span>
                                    <b>Paused:</b> <b>{props.agent}</b>'s actions here wait until you resume
                                </span>
                            }
                        >
                            <span>
                                Driven by <b>{props.agent}</b>
                            </span>
                        </Show>
                        <Show when={model.allowedOriginsAtom().length > 0}>
                            <span class="browser-limited-to" title={`Limited to: ${model.allowedOriginsAtom().join(", ")}`}>
                                Limited to {model.allowedOriginsAtom().length}{" "}
                                {model.allowedOriginsAtom().length === 1 ? "site" : "sites"}
                            </span>
                        </Show>
                        <Show when={model.bypassEndedAtom()}>
                            {(why) => <span class="browser-bypass-ended">Auto-approve ended: {why()}</span>}
                        </Show>
                        {pauseErrorLine()}
                        {controls()}
                        <Button
                            density="compact"
                            tone="danger"
                            class="browser-bypass-button"
                            pressed={confirming()}
                            title={`Let ${props.agent}'s clicks in this pane go through without asking you first.`}
                            onClick={() => {
                                setError(undefined);
                                setConfirming(!confirming());
                            }}
                        >
                            Auto-approve
                        </Button>
                        <Button
                            density="compact"
                            class="browser-take-over"
                            title="End the agent's control of this pane; its next action here will fail."
                            onClick={() => model.takeOver().catch(() => {})}
                        >
                            Take over
                        </Button>
                    </div>
                }
            >
                {(b) => (
                    <div
                        class="browser-driven-by browser-driven-by--bypass"
                        classList={{
                            "browser-driven-by--paused": model.bypassPaused() || model.pausedAtom(),
                        }}
                        role="status"
                    >
                        <i class={model.pausedAtom() ? "fa-solid fa-pause" : "fa-solid fa-bolt"} aria-hidden="true" />
                        <Show
                            when={!model.pausedAtom()}
                            fallback={
                                <span>
                                    <b>Paused:</b> <b>{b().agent}</b>'s actions here wait until you resume · auto-approve's
                                    clock is stopped at {model.bypassMinutesLeft()} min left
                                </span>
                            }
                        >
                            <Show
                                when={!model.bypassPaused()}
                                fallback={
                                    <span>
                                        <b>Auto-approve paused:</b> the page left {b().origins.map(siteLabel).join(", ")},
                                        so <b>{b().agent}</b> asks again here
                                    </span>
                                }
                            >
                                <span>
                                    <b>Auto-approve on:</b> <b>{b().agent}</b>'s clicks go through without asking on{" "}
                                    {b().origins.map(siteLabel).join(", ")} · {model.bypassMinutesLeft()} min left
                                </span>
                            </Show>
                        </Show>
                        <Show when={error()}>
                            <span class="browser-bypass-strip-error" role="alert">
                                {error()}
                            </span>
                        </Show>
                        {pauseErrorLine()}
                        {controls()}
                        <Button density="compact" tone="danger" class="browser-bypass-end" onClick={end}>
                            Stop auto-approve
                        </Button>
                        <Button
                            density="compact"
                            class="browser-take-over-inline"
                            title="End the agent's control of this pane; its next action here will fail."
                            onClick={() => model.takeOver().catch(() => {})}
                        >
                            Take over
                        </Button>
                    </div>
                )}
            </Show>
            <Show when={showLog()}>
                <ActionLogPanel model={model} agent={props.agent} onClose={() => setShowLog(false)} />
            </Show>
            <Show when={confirming() && !model.bypassAtom()}>
                <div class="browser-bypass-confirm" role="dialog" aria-label="Turn on auto-approve">
                    <div class="browser-bypass-confirm-head">
                        <i class="fa-solid fa-bolt" aria-hidden="true" />
                        <span>
                            Auto-approve <b>{props.agent}</b>'s clicks?
                        </span>
                    </div>
                    <Show
                        when={sites().length > 0}
                        fallback={<div class="browser-bypass-confirm-text">Open a web page in this pane first.</div>}
                    >
                        <div class="browser-bypass-confirm-text">
                            In this pane, on <b>{sites().map(siteLabel).join(", ")}</b>, {props.agent}'s clicks that
                            submit forms, delete or send go through <b>without asking you first</b>, and so does the page's own
                            "Are you sure?" that follows one. It still can't type
                            passwords or codes or run scripts, and payments still ask.
                        </div>
                        <div class="browser-bypass-confirm-minutes" role="radiogroup" aria-label="For how long">
                            <For each={BYPASS_MINUTES}>
                                {(m) => (
                                    <Button
                                        density="compact"
                                        pressed={minutes() === m}
                                        role="radio"
                                        aria-checked={minutes() === m}
                                        onClick={() => setMinutes(m)}
                                    >
                                        {minutesLabel(m)}
                                    </Button>
                                )}
                            </For>
                        </div>
                    </Show>
                    <Show when={error()}>
                        <div class="browser-bypass-confirm-error" role="alert">
                            {error()}
                        </div>
                    </Show>
                    <div class="browser-bypass-confirm-actions">
                        <Button
                            tone="danger"
                            density="compact"
                            busy={busy()}
                            disabled={sites().length === 0}
                            onClick={turnOn}
                        >
                            Turn on auto-approve
                        </Button>
                        <Button density="compact" onClick={() => setConfirming(false)}>
                            Cancel
                        </Button>
                    </div>
                </div>
            </Show>
        </>
    );
}

/** The list of what the agent did, newest first, in the pane's flow. */
function ActionLogPanel(props: { model: BrowserViewModel; agent: string; onClose: () => void }): JSX.Element {
    const newestFirst = () => [...props.model.actionsAtom().entries].reverse();
    return (
        <div class="browser-action-log" role="log" aria-label={`What ${props.agent} did in this pane`}>
            <div class="browser-action-log-head">
                <span>
                    What <b>{props.agent}</b> did here, newest first
                    <Show when={props.model.actionsAtom().count > props.model.actionsAtom().entries.length}>
                        {" "}
                        (the latest {props.model.actionsAtom().entries.length} of {props.model.actionsAtom().count})
                    </Show>
                </span>
                <Button density="compact" onClick={props.onClose}>
                    Close
                </Button>
            </div>
            <Show
                when={newestFirst().length > 0}
                fallback={<div class="browser-action-log-empty">Nothing yet.</div>}
            >
                <ol class="browser-action-log-list">
                    <For each={newestFirst()}>
                        {(e) => (
                            <li
                                class="browser-action-log-row"
                                classList={{ "browser-action-log-row--failed": actionFailed(e.how) }}
                            >
                                <span class="browser-action-log-time">{actionTime(e.ts)}</span>
                                <span class="browser-action-log-kind">{actionKind(e.kind)}</span>
                                <span class="browser-action-log-target">
                                    {e.target ?? ""}
                                    <Show when={e.detail}>
                                        <span class="browser-action-log-detail">
                                            {e.target ? " · " : ""}
                                            {e.detail}
                                        </span>
                                    </Show>
                                </span>
                                <span class="browser-action-log-how">{e.how}</span>
                                <span class="browser-action-log-url" title={e.url}>
                                    {shortUrl(e.url)}
                                </span>
                            </li>
                        )}
                    </For>
                </ol>
            </Show>
        </div>
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
                <span>
                    <b>{agent()}</b>'s actions here
                </span>
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
            <Show when={showLog()}>
                <ActionLogPanel model={props.model} agent={agent()} onClose={() => setShowLog(false)} />
            </Show>
        </>
    );
}
