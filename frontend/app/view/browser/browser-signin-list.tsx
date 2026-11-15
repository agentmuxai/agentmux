// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The list of saved sign-ins under a login field the person is in
// (docs/specs/SPEC_VAULT_AUTOFILL_IN_BROWSER_PANE_2026_11_14.md §P2.3, the pane-drawn route R4).
// srv writes it to the block's `browser:signin-list` meta: masked user names and entry ids, ordered,
// filtered by what the person typed, with the selected item. It opens on the person's own click or
// arrow key in the page's field and never takes focus, so the page's field keeps the keyboard: Up,
// Down, Enter, Tab and Escape reach srv through the page's listener, not through this panel. A click
// on a row is the person's pick, as for the key button: the panel is marked operator-only, so no
// agent's UI click or key press reaches it. It is not in any Browser tool answer or page screenshot:
// it is the app's own UI, outside the page.

import { AnchoredPopover } from "@/app/element/anchored-popover";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { openVault } from "@/app/view/vault/open-vault";
import clsx from "clsx";
import { createMemo, createSignal, For, onMount, Show, type JSX } from "solid-js";
import { listRowLabel, MANAGE_PASSWORDS_LABEL, readSigninList, SIGNIN_LIST_LABEL, STRONG_PASSWORD_LABEL } from "./browser-signin-offer-model";

export function BrowserSigninList(props: { blockId: string; meta: () => Record<string, unknown> | undefined }): JSX.Element {
    const list = createMemo(() => readSigninList(props.meta()));
    let host: HTMLElement | undefined;
    const [anchor, setAnchor] = createSignal<Element | undefined>();
    const [error, setError] = createSignal<string | null>(null);
    const run = (call: Promise<void>) => {
        setError(null);
        call.catch((e) => setError(e instanceof Error ? e.message : String(e)));
    };
    const fill = (entry: string) => run(RpcApi.SigninFillCommand(TabRpcClient, { block_id: props.blockId, entry }));
    const strongPassword = () => run(RpcApi.SignupPasswordCommand(TabRpcClient, { block_id: props.blockId }));
    // The anchor is the nav bar's group this component sits in: the list opens under the key button's place.
    // Read once mounted: a ref runs before the element has a parent.
    onMount(() => setAnchor(host?.parentElement ?? undefined));
    return (
        <>
            <span ref={(el) => (host = el)} class="browser-signin-list-anchor" aria-hidden="true" />
            <Show when={list()}>
                {(l) => (
                    <Show when={anchor()}>
                        {(a) => (
                            <AnchoredPopover
                                anchor={a()}
                                placement="bottom-end"
                                overlayKeys={false}
                                shellClass="anchored-popover--menu"
                                class="menu ui-menu browser-signin-list"
                                role="listbox"
                                aria-label={SIGNIN_LIST_LABEL}
                                ref={(el: HTMLElement) => {
                                    el.setAttribute("data-operator-only", "");
                                    el.dataset.testid = "browser-signin-list";
                                    // A press on the panel never takes the keyboard from the page's field.
                                    el.addEventListener("mousedown", (e) => e.preventDefault());
                                }}
                            >
                                <Show when={l().strong}>
                                    <div
                                        role="option"
                                        aria-selected={l().cursor === 0}
                                        class={clsx("menu-item ui-menu-item", l().cursor === 0 && "active")}
                                        data-testid="browser-signin-list-strong"
                                        onClick={strongPassword}
                                    >
                                        <i class="fa-solid fa-fw menu-item-icon fa-wand-magic-sparkles" aria-hidden="true" />
                                        <span class="label">{STRONG_PASSWORD_LABEL}</span>
                                    </div>
                                </Show>
                                <For each={l().strong ? [] : l().rows}>
                                    {(row, i) => (
                                        <div
                                            role="option"
                                            aria-selected={l().cursor === i()}
                                            class={clsx("menu-item ui-menu-item", l().cursor === i() && "active")}
                                            data-testid="browser-signin-list-row"
                                            onClick={() => fill(row.entry)}
                                        >
                                            <i class="fa-solid fa-fw menu-item-icon fa-key" aria-hidden="true" />
                                            <span class="label">{listRowLabel(row)}</span>
                                        </div>
                                    )}
                                </For>
                                <div class="menu-divider" aria-hidden="true" />
                                <div role="option" class="menu-item ui-menu-item" data-testid="browser-signin-list-manage" onClick={() => void openVault("logins")}>
                                    <i class="fa-solid fa-fw menu-item-icon fa-gear" aria-hidden="true" />
                                    <span class="label">{MANAGE_PASSWORDS_LABEL}</span>
                                </div>
                                <Show when={error()}>
                                    <div class="ui-menu-header browser-signin-list-error">{error()}</div>
                                </Show>
                            </AnchoredPopover>
                        )}
                    </Show>
                )}
            </Show>
        </>
    );
}
