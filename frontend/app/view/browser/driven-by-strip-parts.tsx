// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The driven-by strip's markup in parts (driven-by-strip.tsx): the strip while the agent asks, the red strip while
 * Auto-approve is on, the buttons they share, the confirmation, and the list of what the agent did. Each reads the
 * strip's state (driven-by-strip-state.ts).
 */

import { Button } from "@/app/element/ui";
import { pausedLabel, browserPauseAll } from "@/app/store/browser-pause-all";
import { tip } from "@/app/element/tip";
import { For, Show, type JSX } from "solid-js";
import { actionFailed, actionKind, actionTime, shortUrl } from "./action-log";
import type { BrowserViewModel } from "./browser-model";
import { BYPASS_MINUTES, leftLabel, minutesLabel, siteLabel, terminalNote } from "./bypass";
import type { DrivenByProps, DrivenByState } from "./driven-by-strip-state";

type Part = { t: DrivenByState; p: DrivenByProps };

/** The Log and Pause buttons, in both strips. */
function StripControls(props: Part): JSX.Element {
    const { t, p } = props;
    const model = t.model;
    return (
        <>
            <Button
                density="compact"
                class="browser-actions-log-button"
                pressed={t.showLog()}
                title={`What ${p.agent} did in this pane`}
                onClick={() => t.setShowLog(!t.showLog())}
            >
                Log ({model.actionsAtom().count})
            </Button>
            <Button
                density="compact"
                class="browser-pause-button"
                pressed={model.pausedAtom()}
                title={
                    model.pausedAtom()
                        ? t.everyPane()
                            ? `${pausedLabel(browserPauseAll())}. Let agents act in every pane again.`
                            : `Let ${p.agent} act in this pane again.`
                        : `Hold ${p.agent}'s actions in this pane until you resume; auto-approve's time stops too.`
                }
                onClick={t.togglePause}
            >
                {model.pausedAtom() ? "Resume" : "Pause"}
            </Button>
        </>
    );
}

/** Why Pause or Resume failed. */
function PauseErrorLine(props: Part): JSX.Element {
    const t = props.t;
    return (
        <Show when={t.pauseError()}>
            <span class="browser-bypass-strip-error" role="alert">
                {t.pauseError()}
            </span>
        </Show>
    );
}

/** The strip while the agent drives and asks: who, the sites it is limited to, Auto-approve and Take over. */
export function PlainStrip(props: Part): JSX.Element {
    const { t, p } = props;
    const model = t.model;
    return (
        <div class="browser-driven-by" classList={{ "browser-driven-by--held": model.pausedAtom() }} role="status">
            <i class={model.pausedAtom() ? "fa-solid fa-pause" : "fa-solid fa-robot"} aria-hidden="true" />
            <div class="browser-driven-by-message">
                <Show
                    when={!model.pausedAtom()}
                    fallback={
                        <span>
                            <b>Paused{t.everyPane() ? " for every pane" : ""}:</b> <b>{p.agent}</b>'s actions here wait until you resume
                        </span>
                    }
                >
                    <span>
                        Driven by <b>{p.agent}</b>
                        {model.tookControlAtom() ? " (took control)" : ""}
                    </span>
                </Show>
                <Show when={model.allowedOriginsAtom().length > 0}>
                    <span class="browser-limited-to" {...tip(`Limited to: ${model.allowedOriginsAtom().join(", ")}`)}>
                        Limited to {model.allowedOriginsAtom().length}{" "}
                        {model.allowedOriginsAtom().length === 1 ? "site" : "sites"}
                    </span>
                </Show>
                <Show when={model.bypassEndedAtom()}>
                    {(why) => <span class="browser-bypass-ended">Auto-approve ended: {why()}</span>}
                </Show>
                <PauseErrorLine {...props} />
            </div>
            <div class="browser-driven-by-actions">
                <StripControls {...props} />
                <Button
                    density="compact"
                    tone="danger"
                    class="browser-bypass-button"
                    pressed={t.confirming()}
                    title={`Let ${p.agent}'s clicks in this pane go through without asking you first.`}
                    onClick={() => {
                        t.setError(undefined);
                        t.setConfirming(!t.confirming());
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
        </div>
    );
}

/** The red strip while Auto-approve is on: the sites, the time left, what changes when paused, Stop and Take over. */
export function BypassStrip(props: Part & { grant: () => BrowserBypass }): JSX.Element {
    const { t, grant: b } = props;
    const model = t.model;
    return (
        <div
            class="browser-driven-by browser-driven-by--bypass"
            classList={{
                "browser-driven-by--paused": model.bypassPaused() || model.pausedAtom(),
            }}
            role="status"
        >
            <i class={model.pausedAtom() ? "fa-solid fa-pause" : "fa-solid fa-bolt"} aria-hidden="true" />
            <div class="browser-driven-by-message">
                <Show
                    when={!model.pausedAtom()}
                    fallback={
                        <span>
                            <b>Paused{t.everyPane() ? " for every pane" : ""}:</b> <b>{b().agent}</b>'s actions here wait until you resume ·{" "}
                            {b().always
                                ? "auto-approve is held"
                                : `auto-approve's clock is stopped at ${model.bypassMinutesLeft()} min left`}
                        </span>
                    }
                >
                    <Show
                        when={!model.bypassPaused()}
                        fallback={
                            <span>
                                <b>Auto-approve paused:</b> the page left{" "}
                                {b().origins.map(siteLabel).join(", ")}, so <b>{b().agent}</b> asks again
                                here
                            </span>
                        }
                    >
                        <span>
                            <b>Auto-approve on:</b> <b>{b().agent}</b>'s clicks go through without asking on{" "}
                            {b().origins.map(siteLabel).join(", ")}
                            {terminalNote(b().powers)} ·{" "}
                            {leftLabel(b().always, model.bypassMinutesLeft())}
                        </span>
                    </Show>
                </Show>
                <Show when={t.error()}>
                    <span class="browser-bypass-strip-error" role="alert">
                        {t.error()}
                    </span>
                </Show>
                <PauseErrorLine {...props} />
            </div>
            <div class="browser-driven-by-actions">
                <StripControls {...props} />
                <Button density="compact" tone="danger" class="browser-bypass-end" onClick={t.end}>
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
        </div>
    );
}

/** The confirmation for Auto-approve: the sites, a duration, and why it failed. */
export function BypassConfirm(props: Part): JSX.Element {
    const { t, p } = props;
    return (
        <div class="browser-bypass-confirm" role="dialog" aria-label="Turn on auto-approve">
            <div class="browser-bypass-confirm-head">
                <i class="fa-solid fa-bolt" aria-hidden="true" />
                <span>
                    Auto-approve <b>{p.agent}</b>'s clicks?
                </span>
            </div>
            <Show
                when={t.sites().length > 0}
                fallback={<div class="browser-bypass-confirm-text">Open a web page in this pane first.</div>}
            >
                <div class="browser-bypass-confirm-text">
                    In this pane, on <b>{t.sites().map(siteLabel).join(", ")}</b>, {p.agent}'s clicks that
                    submit forms, delete or send go through <b>without asking you first</b>, and so does the
                    page's own "Are you sure?" that follows one. It still can't type passwords or codes or run
                    scripts, and payments still ask.
                </div>
                <div class="browser-bypass-confirm-minutes" role="radiogroup" aria-label="For how long">
                    <For each={BYPASS_MINUTES}>
                        {(m) => (
                            <Button
                                density="compact"
                                pressed={t.minutes() === m}
                                role="radio"
                                aria-checked={t.minutes() === m}
                                onClick={() => t.setMinutes(m)}
                            >
                                {minutesLabel(m)}
                            </Button>
                        )}
                    </For>
                </div>
            </Show>
            <Show when={t.error()}>
                <div class="browser-bypass-confirm-error" role="alert">
                    {t.error()}
                </div>
            </Show>
            <div class="browser-bypass-confirm-actions">
                <Button
                    tone="danger"
                    density="compact"
                    busy={t.busy()}
                    disabled={t.sites().length === 0}
                    onClick={t.turnOn}
                >
                    Turn on auto-approve
                </Button>
                <Button density="compact" onClick={() => t.setConfirming(false)}>
                    Cancel
                </Button>
            </div>
        </div>
    );
}

/** The list of what the agent did, newest first, in the pane's flow. */
export function ActionLogPanel(props: { model: BrowserViewModel; agent: string; onClose: () => void }): JSX.Element {
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
            <Show when={newestFirst().length > 0} fallback={<div class="browser-action-log-empty">Nothing yet.</div>}>
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
                                <span class="browser-action-log-url" {...tip(e.url)}>
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
