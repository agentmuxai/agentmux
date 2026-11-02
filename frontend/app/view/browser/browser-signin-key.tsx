// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The key button: the saved sign-ins the page's sign-in form can take
// (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §5.3).
// srv lists them in the block's `browser:signins` meta (entry, origin, masked
// user name). The button opens the kit's Menu, marked operator-only, so no
// agent's UI click or key press reaches its items: the person choosing
// "Sign in as al***" is the approval, and srv types the sign-in into the page.

import { createMemo, createSignal, Show, type JSX } from "solid-js";
import clsx from "clsx";
import { Button, Menu } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { readSavedSignins, savedSigninLabel } from "./browser-signin-offer-model";

export function BrowserSigninKey(props: { blockId: string; meta: () => Record<string, unknown> | undefined }): JSX.Element {
    const signins = createMemo(() => readSavedSignins(props.meta()));
    const [error, setError] = createSignal<string | null>(null);
    const [anchor, setAnchor] = createSignal<Element | null>(null);
    const fill = (entry: string) => {
        setError(null);
        RpcApi.SigninFillCommand(TabRpcClient, { block_id: props.blockId, entry }).catch((e) =>
            setError(e instanceof Error ? e.message : String(e))
        );
    };
    const title = () => {
        const n = signins().length;
        const saved = n === 1 ? "1 saved sign-in for this page" : `${n} saved sign-ins for this page`;
        return error() ? `${saved}. The last fill failed: ${error()}` : saved;
    };
    return (
        <Show when={signins().length > 0}>
            <Button
                tone="quiet"
                class="ui-icon-button browser-nav-btn browser-signin-key"
                data-testid="browser-signin-key"
                title={title()}
                aria-label={title()}
                onClick={(e: MouseEvent) => setAnchor(anchor() ? null : (e.currentTarget as Element))}
            >
                <i class={clsx("fa-solid fa-key", error() && "browser-signin-key-error")} aria-hidden="true" />
                <Show when={signins().length > 1}>
                    <span class="browser-signin-key-count" aria-hidden="true">
                        {signins().length}
                    </span>
                </Show>
            </Button>
            <Show when={anchor()}>
                {(a) => (
                    <Menu
                        anchor={a()}
                        placement="bottom-start"
                        testId="browser-signin-key-menu"
                        aria-label="Saved sign-ins for this page"
                        operatorOnly
                        items={signins().map((s) => ({
                            label: savedSigninLabel(s, signins()),
                            icon: "key",
                            onSelect: () => fill(s.entry),
                        }))}
                        onClose={() => setAnchor(null)}
                    />
                )}
            </Show>
        </Show>
    );
}
