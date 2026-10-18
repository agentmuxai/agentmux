// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The header strip of a browser pane an agent drives: who drives it, its site
// limit, Take over, and Auto-approve (SPEC_BROWSER_PANE_BYPASS_2026_10_16.md §3;
// the spec's "Bypass", renamed for people).
//
// Auto-approve's confirmation is a panel in the pane's own flow, not a floating
// popover: the page is a native window drawn over anything layered on it,
// and the panel must stay inside the pane's DOM, where no other agent's UI
// click can reach (spec §5).

import { createSignal, For, Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { BYPASS_MINUTES, minutesLabel, siteLabel } from "./bypass";
import type { BrowserViewModel } from "./browser-model";

export function DrivenByStrip(props: { model: BrowserViewModel; agent: string }): JSX.Element {
    const model = props.model;
    const [confirming, setConfirming] = createSignal(false);
    const [minutes, setMinutes] = createSignal(60);
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal<string | undefined>();
    const sites = () => model.bypassOriginsPreview();

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

    return (
        <>
            <Show
                when={model.bypassAtom()}
                fallback={
                    <div class="browser-driven-by" role="status">
                        <i class="fa-solid fa-robot" aria-hidden="true" />
                        <span>
                            Driven by <b>{props.agent}</b>
                        </span>
                        <Show when={model.allowedOriginsAtom().length > 0}>
                            <span class="browser-limited-to" title={`Limited to: ${model.allowedOriginsAtom().join(", ")}`}>
                                Limited to {model.allowedOriginsAtom().length}{" "}
                                {model.allowedOriginsAtom().length === 1 ? "site" : "sites"}
                            </span>
                        </Show>
                        <Show when={model.bypassEndedAtom()}>
                            {(why) => <span class="browser-bypass-ended">Auto-approve ended: {why()}</span>}
                        </Show>
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
                        classList={{ "browser-driven-by--paused": model.bypassPaused() }}
                        role="status"
                    >
                        <i class="fa-solid fa-bolt" aria-hidden="true" />
                        <Show
                            when={!model.bypassPaused()}
                            fallback={
                                <span>
                                    <b>Auto-approve paused:</b> the page left {b().origins.map(siteLabel).join(", ")}, so{" "}
                                    <b>{b().agent}</b> asks again here
                                </span>
                            }
                        >
                            <span>
                                <b>Auto-approve on:</b> <b>{b().agent}</b>'s clicks go through without asking on{" "}
                                {b().origins.map(siteLabel).join(", ")} · {model.bypassMinutesLeft()} min left
                            </span>
                        </Show>
                        <Show when={error()}>
                            <span class="browser-bypass-strip-error" role="alert">
                                {error()}
                            </span>
                        </Show>
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
                            submit forms, delete or send go through <b>without asking you first</b>. It still can't type
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
