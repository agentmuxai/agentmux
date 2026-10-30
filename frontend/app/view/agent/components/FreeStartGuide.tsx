// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * FreeStartGuide: for a provider with a free sign-in route, the one link to its
 * sign-up page and then what to do after it, with the limits and catches the
 * Free tag's tooltip carries and the day they were checked
 * (docs/specs/SPEC_PROVIDER_CARD_TAGS_FREE_LOCAL_2026_10_29.md §7, route A).
 * It opens the page in the system browser, never a pane here: Google and others
 * refuse a sign-in inside an embedded browser. The sign-in itself is the
 * existing launch login, so nothing here asks for a key or a password.
 */

import { For, Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { getApi } from "@/app/store/global";
import { freeGuideFor } from "../defaults/provider-access-model";

export interface FreeStartGuideProps {
    provider: string;
    /** The provider's display name, for the heading. */
    name: string;
    /** Replaceable in tests; the date the freshness rules read. */
    now?: () => Date;
}

function openLink(url: string): void {
    try {
        getApi().openExternal(url);
    } catch {
        /* the address stays visible as text */
    }
}

export const FreeStartGuide = (props: FreeStartGuideProps): JSX.Element => {
    const guide = () => freeGuideFor(props.provider, (props.now ?? (() => new Date()))());
    return (
        <Show when={guide()}>
            {(g) => (
                <section class="agent-free-guide" data-testid="agent-free-guide" aria-label={`Use ${props.name} for free`}>
                    <h4 class="agent-free-guide-title">Use {props.name} for free</h4>
                    <p class="agent-free-guide-lead">
                        {g().lead}. {g().limits}
                    </p>
                    <div class="agent-free-guide-open">
                        <Button tone="accent" onClick={() => openLink(g().signupUrl)} data-testid="agent-free-guide-open">
                            Open the sign-up page<span aria-hidden="true"> ↗</span>
                        </Button>
                    </div>
                    <ol class="agent-free-guide-steps">
                        <For each={g().steps}>{(s) => <li>{s}</li>}</For>
                    </ol>
                    <Show when={g().catches.length > 0}>
                        <ul class="agent-free-guide-catches">
                            <For each={g().catches}>{(c) => <li>{c}</li>}</For>
                        </ul>
                    </Show>
                    <Show when={g().successor}>{(s) => <p class="agent-free-guide-note">{s()}</p>}</Show>
                    <p class="agent-free-guide-checked" classList={{ "agent-free-guide-checked--stale": g().stale }} data-testid="agent-free-guide-checked">
                        {g().checked.text}
                    </p>
                </section>
            )}
        </Show>
    );
};

FreeStartGuide.displayName = "FreeStartGuide";
