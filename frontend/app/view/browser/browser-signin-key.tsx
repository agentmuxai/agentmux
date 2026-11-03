// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The key button: the saved sign-ins the page's sign-in form can take
// (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §5.3),
// and a strong password for a field for a new password (§6.2).
// srv lists the sign-ins in the block's `browser:signins` meta (entry, origin,
// masked user name) and the field in `browser:signup` (its origin). The button
// opens the kit's Menu, marked operator-only, so no agent's UI click or key
// press reaches its items: the person choosing "Sign in as al***" or "Use a
// strong password" is the approval, and srv types the sign-in, or makes the
// password, keeps it in the Vault and fills it.

import { createMemo, createSignal, Show, type JSX } from "solid-js";
import clsx from "clsx";
import { Button, Menu } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { readSavedSignins, readSignupOffer, savedSigninLabel, STRONG_PASSWORD_LABEL } from "./browser-signin-offer-model";

export function BrowserSigninKey(props: { blockId: string; meta: () => Record<string, unknown> | undefined }): JSX.Element {
    const signins = createMemo(() => readSavedSignins(props.meta()));
    const signup = createMemo(() => readSignupOffer(props.meta()));
    const [error, setError] = createSignal<string | null>(null);
    const [anchor, setAnchor] = createSignal<Element | null>(null);
    const run = (call: Promise<void>) => {
        setError(null);
        call.catch((e) => setError(e instanceof Error ? e.message : String(e)));
    };
    const fill = (entry: string) => run(RpcApi.SigninFillCommand(TabRpcClient, { block_id: props.blockId, entry }));
    const strongPassword = () => run(RpcApi.SignupPasswordCommand(TabRpcClient, { block_id: props.blockId }));
    const title = () => {
        const n = signins().length;
        const saved = n === 0 ? "A strong password from the Vault for this field" : n === 1 ? "1 saved sign-in for this page" : `${n} saved sign-ins for this page`;
        return error() ? `${saved}. The last fill failed: ${error()}` : saved;
    };
    const items = () => [
        ...(signup() ? [{ label: STRONG_PASSWORD_LABEL, icon: "wand-magic-sparkles", onSelect: strongPassword }] : []),
        ...signins().map((s) => ({ label: savedSigninLabel(s, signins()), icon: "key", onSelect: () => fill(s.entry) })),
    ];
    return (
        <Show when={signins().length > 0 || signup()}>
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
                        aria-label="Saved sign-ins and passwords for this page"
                        operatorOnly
                        items={items()}
                        onClose={() => setAnchor(null)}
                    />
                )}
            </Show>
        </Show>
    );
}
