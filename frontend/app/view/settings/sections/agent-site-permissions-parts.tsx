// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Agents on sites, page permissions: the sites where a page may use the camera, the microphone, the
// location or notifications in a pane an agent drives, for good. Over the same state as the grants above
// (agent-site-grants-state.ts); srv keeps the rows apart from them because a camera must not bring
// committing clicks along (crates/srv/src/server/browser_site_permissions.rs).

import { Button, TextInput } from "@/app/element/ui";
import { createSignal, For, Show, type Accessor, type JSX } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { BrowserGrantsResult } from "@/types/rpc/BrowserGrantsResult";
import type { SitePermissionRow } from "@/types/rpc/SitePermissionRow";
import { siteLabel } from "@/app/view/browser/bypass";
import { ago } from "./pinned-senders";
import { PERMISSION_LABELS, withKind } from "./agent-site-permissions-model";
import type { AgentSiteGrantsState } from "./agent-site-grants-state";

type Grants = Accessor<BrowserGrantsResult>;

function KindBoxes(props: { kinds: readonly string[]; on: readonly string[]; disabled?: boolean; label: string; onToggle: (kind: string, on: boolean) => void }): JSX.Element {
    return (
        <div class="agent-site-grant-powers" role="group" aria-label={props.label}>
            <For each={props.kinds}>
                {(kind) => (
                    <label class="agent-site-grant-power">
                        <input
                            type="checkbox"
                            checked={props.on.includes(kind)}
                            disabled={props.disabled}
                            aria-label={`${PERMISSION_LABELS[kind] ?? kind}: ${props.label}`}
                            onChange={(e) => props.onToggle(kind, e.currentTarget.checked)}
                        />
                        <span>{PERMISSION_LABELS[kind] ?? kind}</span>
                    </label>
                )}
            </For>
        </div>
    );
}

function PermissionRow(props: { s: AgentSiteGrantsState; d: Grants; row: SitePermissionRow }): JSX.Element {
    const { s, d, row } = props;
    const who = () => `${s.nameOfSlug(row.agent)} on ${siteLabel(row.site)}`;
    return (
        <li class="agent-site-grant">
            <div class="agent-site-grant-who">
                <span class="agent-site-grant-agent">{s.nameOfSlug(row.agent)}</span>
                <span class="agent-site-grant-site">{siteLabel(row.site)}</span>
                <span class="agent-site-grant-when">allowed {ago(Math.floor(row.granted_at_ms / 1000), s.now())}</span>
            </div>
            <KindBoxes
                kinds={d().permission_kinds}
                on={row.kinds}
                disabled={s.busy()}
                label={who()}
                onToggle={(kind, on) => {
                    const kinds = withKind(row.kinds, kind, on);
                    // The last one off is a revoke: a row with nothing to use is no row.
                    void s.run(
                        `change what ${s.nameOfSlug(row.agent)} may use`,
                        () =>
                            kinds.length
                                ? RpcApi.BrowserGrantsPermissionSetCommand(TabRpcClient, { agent: row.agent, site: row.site, kinds })
                                : RpcApi.BrowserGrantsPermissionRevokeCommand(TabRpcClient, { id: row.id })
                    );
                }}
            />
            <Button
                disabled={s.busy()}
                aria-label={`Revoke the page permissions of ${who()}`}
                onClick={() => void s.run("revoke it", () => RpcApi.BrowserGrantsPermissionRevokeCommand(TabRpcClient, { id: row.id }))}
            >
                Revoke
            </Button>
        </li>
    );
}

function AddPermission(props: { s: AgentSiteGrantsState; d: Grants }): JSX.Element {
    const { s, d } = props;
    const [site, setSite] = createSignal("");
    const [kinds, setKinds] = createSignal<string[]>([]);
    const ready = () => !!s.picked() && !!site().trim() && kinds().length > 0 && !s.busy();
    const add = async () => {
        const agent = s.picked();
        if (!agent || !ready()) return;
        const ok = await s.run(`let ${s.agentName(agent)} use the page's permissions on ${site().trim()}`, () =>
            RpcApi.BrowserGrantsPermissionSetCommand(TabRpcClient, { agent: agent.slug, site: site().trim(), kinds: kinds() })
        );
        if (ok) {
            setSite("");
            setKinds([]);
        }
    };
    return (
        <div class="agent-site-grants-add">
            <TextInput
                detached
                aria-label="Site for page permissions"
                placeholder="meet.example.com"
                value={site()}
                onInput={(e) => setSite(e.currentTarget.value)}
                onKeyDown={(e) => {
                    if (e.key === "Enter") void add();
                }}
            />
            <KindBoxes
                kinds={d().permission_kinds}
                on={kinds()}
                label="the new site"
                onToggle={(kind, on) => setKinds((k) => withKind(k, kind, on))}
            />
            <Button disabled={!ready()} onClick={() => void add()}>
                Allow on this site
            </Button>
            <Show when={!s.picked()}>
                <span class="setting-devices-empty">Choose an agent above first.</span>
            </Show>
        </div>
    );
}

/** The page permissions: what shows once the grants have loaded. */
export function PermissionsBody(props: { s: AgentSiteGrantsState; d: Grants }): JSX.Element {
    return (
        <div class="agent-site-permissions">
            <h4 class="agent-site-permissions-title">Page permissions</h4>
            <div class="setting-devices-description">
                Sites where a page may use the camera, the microphone, the location or notifications in a pane an agent drives, without a
                countdown each time, until you revoke it. Each is a row of its own: it does not let the agent make committing clicks. Every
                first use in a pane is in the Audit view. Without a row, the page's request is held and filed for you to veto.
            </div>
            <Show
                when={props.d().permissions.length > 0}
                fallback={<div class="setting-devices-empty">No site has a standing page permission. Each request is asked in the pane.</div>}
            >
                <ul class="agent-site-grants-list" aria-label="Page permissions">
                    <For each={props.d().permissions}>{(row) => <PermissionRow s={props.s} d={props.d} row={row} />}</For>
                </ul>
            </Show>
            <AddPermission s={props.s} d={props.d} />
        </div>
    );
}
