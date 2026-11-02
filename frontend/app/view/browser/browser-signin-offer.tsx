// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// "Save to Vault?" for a sign-in the page sent
// (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §5.2): a band
// under the nav bar, below any attention banner. srv writes it to the block's
// `browser:signin_offer` meta with only the exact origin and the masked user name,
// so a page can't make the question say anything else.

import { createMemo, createSignal, onCleanup, Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { readSigninOffer, readSigninPending, signinOfferQuestion, signinPendingText } from "./browser-signin-offer-model";

export function BrowserSigninOffer(props: {
    blockId: string;
    meta: () => Record<string, unknown> | undefined;
    /** An attention banner is up: it comes first, and the offer waits. */
    covered: () => boolean;
}): JSX.Element {
    const offer = createMemo(() => readSigninOffer(props.meta()));
    const [error, setError] = createSignal<string | null>(null);
    const answer = (id: string, answer: "save" | "never" | "not_now") => {
        setError(null);
        RpcApi.SigninOfferAnswerCommand(TabRpcClient, { block_id: props.blockId, id, answer }).catch((e) =>
            setError(e instanceof Error ? e.message : String(e))
        );
    };
    return (
        <Show when={!props.covered() && offer()}>
            {(o) => (
                <div class="browser-signin-offer" role="region" aria-label="Save sign-in to the Vault">
                    <i class="fa-solid fa-key" aria-hidden="true" />
                    <span class="browser-signin-offer-question">{signinOfferQuestion(o())}</span>
                    <div class="browser-signin-offer-actions">
                        <Button density="compact" tone="accent" onClick={() => answer(o().id, "save")}>
                            {o().update ? "Update" : "Save"}
                        </Button>
                        <Button density="compact" onClick={() => answer(o().id, "never")}>
                            Never for this site
                        </Button>
                        <Button density="compact" tone="quiet" onClick={() => answer(o().id, "not_now")}>
                            Not now
                        </Button>
                    </div>
                    <Show when={error()}>
                        <div class="browser-signin-offer-error" role="alert">
                            {error()}
                        </div>
                    </Show>
                </div>
            )}
        </Show>
    );
}

/**
 * "<agent> is signing in to <site> [Stop]": shown for 5 seconds before a sign-in
 * the person said "Always" to is typed (§5.3). It comes before an attention
 * banner's turn, since it is over in seconds and Stop has to be reachable.
 */
export function BrowserSigninPending(props: { blockId: string; meta: () => Record<string, unknown> | undefined }): JSX.Element {
    const pending = createMemo(() => readSigninPending(props.meta()));
    const [now, setNow] = createSignal(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 250);
    onCleanup(() => clearInterval(timer));
    const [error, setError] = createSignal<string | null>(null);
    const stop = (id: string) => {
        setError(null);
        RpcApi.SigninStopCommand(TabRpcClient, { block_id: props.blockId, id }).catch((e) => setError(e instanceof Error ? e.message : String(e)));
    };
    return (
        <Show when={pending()}>
            {(p) => (
                <div class="browser-signin-offer browser-signin-pending" role="alert" aria-label="An agent is signing in">
                    <i class="fa-solid fa-key" aria-hidden="true" />
                    <span class="browser-signin-offer-question">{signinPendingText(p(), now())}</span>
                    <div class="browser-signin-offer-actions">
                        <Button density="compact" tone="danger" onClick={() => stop(p().id)}>
                            Stop
                        </Button>
                    </div>
                    <Show when={error()}>
                        <div class="browser-signin-offer-error" role="alert">
                            {error()}
                        </div>
                    </Show>
                </div>
            )}
        </Show>
    );
}
