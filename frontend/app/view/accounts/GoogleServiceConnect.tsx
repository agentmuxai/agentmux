// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Google service's Connect (docs/specs/SPEC_CONNECTORS_ONE_LIST_2026_11_19.md §4.10): a sign-in that asks for just that
// service's scopes, so it makes a Google account with the service on. A service AgentMux's own Google client isn't approved
// for yet says so instead of offering a sign-in that srv would refuse, as a Google account's "Turn on" does.

import { createResource, Show, type JSX } from "solid-js";

import { t } from "@/app/i18n/i18n";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { serviceOffered, type ServiceInfo } from "./google-services";
import { OAuthConnectPanel } from "./OAuthConnectPanel";

export function GoogleServiceConnect(props: { info: ServiceInfo; onClose: () => void }): JSX.Element {
    // A client that can't be asked about leaves the answer to srv, which still refuses a service the build doesn't offer.
    const [client] = createResource(() => RpcApi.AccountOAuthClientGetCommand(TabRpcClient, { provider: "google" }).catch(() => null));
    const offered = () => serviceOffered(props.info, client()?.mode, client()?.offeredServices ?? null);
    return (
        <Show when={!client.loading}>
            <Show
                when={offered()}
                fallback={<div class="identity-detail-hint">{t("connectors.google.notOffered", { name: props.info.label })}</div>}
            >
                <OAuthConnectPanel
                    provider="google"
                    scopes={props.info.readScopes}
                    onConnected={() => props.onClose()}
                    onCancel={() => props.onClose()}
                />
            </Show>
        </Show>
    );
}
