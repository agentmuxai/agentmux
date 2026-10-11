// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What the user is approving when a widget asks to be installed
// (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §8.3): its
// version, author and kind, and every permission in plain words, or the
// trusted-widget warning. Shown in Settings → Widgets and in the prompt an
// agent's install request opens, so both say exactly the same.

import { For, Show, type JSX } from "solid-js";

import type { WidgetKind, WidgetSignatureInfo } from "@/app/store/rpc-api/widgets";
import { describePermission, describeSignature, TRUSTED_WIDGET_WARNING } from "./widget-permissions";

import "./widget-approval.scss";

export interface WidgetApprovalFacts {
    version: string;
    author?: string | null;
    kind: WidgetKind;
    implied?: boolean;
    description?: string | null;
    permissions: string[];
    signature?: WidgetSignatureInfo | null;
}

/** The prompt's Install button: "Install anyway" when the publisher's
 *  pinned key didn't sign it. */
export function installLabel(pkg: WidgetApprovalFacts, label = "Install"): string {
    return pkg.signature?.state === "key_changed" ? `${label} anyway` : label;
}

export function WidgetApprovalDetails(props: { pkg: WidgetApprovalFacts }): JSX.Element {
    return (
        <div class="widget-approval-details">
            <div class="widget-approval-meta">
                {props.pkg.version}
                <Show when={props.pkg.author}> · by {props.pkg.author}</Show>
                {" · "}
                {props.pkg.kind === "trusted" ? "trusted" : "sandboxed"}
                <Show when={props.pkg.implied}> · from widgets.json</Show>
            </div>
            <Show when={props.pkg.description}>
                <div class="widget-approval-description">{props.pkg.description}</div>
            </Show>
            {(() => {
                const s = () => describeSignature(props.pkg.signature);
                return (
                    <div class={s().warning ? "widget-approval-warning" : "widget-approval-signature"}>
                        <i class={`fa-solid ${s().warning ? "fa-triangle-exclamation" : "fa-signature"}`} aria-hidden="true" />{" "}
                        {s().warning ? <strong>{s().text}</strong> : s().text}
                    </div>
                );
            })()}
            <Show
                when={props.pkg.kind === "sandboxed"}
                fallback={
                    <div class="widget-approval-warning">
                        <i class="fa-solid fa-triangle-exclamation" /> <strong>{TRUSTED_WIDGET_WARNING}</strong>
                    </div>
                }
            >
                <div class="widget-approval-permissions">
                    <Show when={props.pkg.permissions.length > 0} fallback={<div>It asks for no permissions.</div>}>
                        <div>It can:</div>
                        <ul>
                            <For each={props.pkg.permissions}>
                                {(p) => {
                                    const d = describePermission(p);
                                    return (
                                        <li classList={{ strong: !!d.strong }}>
                                            <Show when={d.strong}>
                                                <i class="fa-solid fa-triangle-exclamation" />{" "}
                                            </Show>
                                            {d.text}
                                        </li>
                                    );
                                }}
                            </For>
                        </ul>
                    </Show>
                </div>
            </Show>
        </div>
    );
}
