// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The header strip of a browser pane the person took over from an agent: it says they are driving, and Give back
// lets that agent retake the pane (docs/specs/SPEC_BROWSER_RETAKE_AND_COMMANDEER_2026_11_04.md §1.3). The strip of an
// agent's own driving is `driven-by-strip.tsx`; this one shows when no agent drives and one did.

import { Button } from "@/app/element/ui";
import { createSignal, Show, type JSX } from "solid-js";
import type { BrowserViewModel } from "./browser-model";
import { ActionLogStrip } from "./driven-by-strip";
import { plainError } from "./plain-error";

/** What the header shows when no agent drives the pane: Give back, if one did and the person took the pane, and the
 *  log of what an agent did there. */
export function NoDriverStrips(props: { model: BrowserViewModel }): JSX.Element {
    return (
        <>
            <Show when={props.model.lastDriverAtom()}>{(agent) => <GiveBackStrip model={props.model} agent={agent()} />}</Show>
            <Show when={props.model.actionsAtom().count > 0}>
                <ActionLogStrip model={props.model} />
            </Show>
        </>
    );
}

export function GiveBackStrip(props: { model: BrowserViewModel; agent: string }): JSX.Element {
    const [error, setError] = createSignal<string | undefined>();
    const giveBack = () => {
        setError(undefined);
        props.model.giveBack().catch((err) => setError(plainError(err, "give the pane back")));
    };
    return (
        <div class="browser-driven-by browser-driven-by--ended" role="status">
            <i class="fa-solid fa-hand" aria-hidden="true" />
            <div class="browser-driven-by-message">
                <span>
                    You are driving this pane. <b>{props.agent}</b> can't act in it until you give it back.
                </span>
                <Show when={error()}>
                    <span class="browser-bypass-strip-error" role="alert">
                        {error()}
                    </span>
                </Show>
            </div>
            <div class="browser-driven-by-actions">
                <Button
                    density="compact"
                    class="browser-give-back"
                    title={`Let ${props.agent} drive this pane again; it takes it back with its next retake.`}
                    onClick={giveBack}
                >
                    Give back to {props.agent}
                </Button>
            </div>
        </div>
    );
}
