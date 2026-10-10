// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The browser toolbar's Profile button and its menu
 * (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §3–§4): who this
 * tab is browsing as, Open new Incognito tab, Personal and the named
 * profiles, New profile…, Manage profiles…. Every identity entry opens a NEW
 * tab in this pane; a tab's identity never changes.
 */

import { createMemo, Show, type JSX } from "solid-js";
import clsx from "clsx";
import { FlyoutMenu } from "@/app/element/flyoutmenu";
import { IconButton } from "@/app/element/ui";
import { keyLabel } from "@/app/keybindings";
import { pushNotification } from "@/app/store/global";
import { openModal } from "@/app/store/modalmodel";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getLayoutModelForStaticTab, openBlockInStack } from "@/layout/index";
import { getPlatform } from "@/util/platformutil";
import {
    canOpenIncognito,
    IDENTITY_META_KEY,
    INCOGNITO_ICON,
    newIncognitoIdentity,
    newTabUrl,
    parseIdentity,
} from "./browser-identity";
import type { BrowserViewModel } from "./browser-model";
import { browserProfiles, ensureBrowserProfiles, loadBrowserProfiles, profileById, profileInitial } from "./browser-profiles";
import { NewProfileModal } from "./new-profile-modal";
import "./browser-profiles.scss";

/** Open a new browser tab in the same pane as `blockId`, browsing as
 *  `identity` (absent: Personal), at `url`. */
export async function openTabAs(blockId: string, url: string, identity: string | undefined): Promise<void> {
    const meta: Record<string, unknown> = { view: "browser", url };
    if (identity) meta[IDENTITY_META_KEY] = identity;
    try {
        const opened = await openBlockInStack(getLayoutModelForStaticTab(), { blockId }, "browser", meta);
        if (!opened) throw new Error("the pane closed");
    } catch (e) {
        pushNotification({
            icon: "fa-triangle-exclamation",
            title: "Couldn't open the tab",
            message: e instanceof Error ? e.message : String(e),
            timestamp: new Date().toISOString(),
            type: "error",
            expiration: Date.now() + 8000,
        });
    }
}

/** Open a new Incognito tab in `model`'s pane, at its page (or Home). */
export function openIncognitoTab(model: BrowserViewModel, home: string): void {
    void openTabAs(model.blockId, newTabUrl(model.urlAtom(), home), newIncognitoIdentity());
}

/** Open Settings at its Browser section (Manage profiles…). */
function openProfileSettings(): void {
    void TabRpcClient.rpcCall("pane.open", { view: "settings", meta: { view: "settings", "settings:section": "browser" } }, {});
}

/** A profile's badge: its initial on its colour. */
export function ProfileBadge(props: { name: string; color: string }): JSX.Element {
    return (
        <span class="browser-profile-badge" style={{ "background-color": props.color }} aria-hidden="true">
            {profileInitial(props.name)}
        </span>
    );
}

type Row = { kind: "header"; sub: string } | { kind: "dim" } | { kind: "identity"; current: boolean };

export function BrowserProfileButton(props: { model: BrowserViewModel; home: () => string }): JSX.Element {
    const model = props.model;
    const platform = getPlatform();
    ensureBrowserProfiles();
    const identity = createMemo(() => parseIdentity(model.meta()?.[IDENTITY_META_KEY]));
    const profile = () => {
        const id = identity();
        return id.kind === "profile" ? profileById(id.id) : undefined;
    };
    const name = () => {
        const id = identity();
        if (id.kind === "incognito") return "Incognito";
        if (id.kind === "profile") return profile()?.name ?? "A deleted profile";
        return "Personal";
    };
    const tabUrl = () => newTabUrl(model.urlAtom(), props.home());

    const rows = new WeakMap<MenuItem, Row>();
    const items = createMemo<MenuItem[]>(() => {
        const id = identity();
        const p = profile();
        const header: MenuItem = {
            label: name(),
            icon: p ? <ProfileBadge name={p.name} color={p.color} /> : id.kind === "incognito" ? INCOGNITO_ICON : "user",
        };
        rows.set(header, {
            kind: "header",
            sub: id.kind === "incognito" ? "Nothing is saved, and it's gone when the tab closes" : "Your saved sign-ins",
        });
        const list: MenuItem[] = [header, { label: "", divider: true }];
        if (canOpenIncognito(platform)) {
            list.push({
                label: "Open new Incognito tab",
                icon: INCOGNITO_ICON,
                shortcut: keyLabel("mod+shift+n"),
                onClick: () => openIncognitoTab(model, props.home()),
            });
        } else {
            const row: MenuItem = { label: "Incognito tabs: Windows only for now", icon: INCOGNITO_ICON };
            rows.set(row, { kind: "dim" });
            list.push(row);
        }
        list.push({ label: "", divider: true });
        // Personal and the profiles: choosing one opens a new tab in it, the
        // current one included ("another tab as me").
        const personal: MenuItem = {
            label: "Personal",
            icon: "user",
            onClick: () => void openTabAs(model.blockId, tabUrl(), undefined),
        };
        rows.set(personal, { kind: "identity", current: id.kind === "personal" });
        list.push(personal);
        for (const pr of browserProfiles()) {
            const row: MenuItem = {
                label: pr.name,
                icon: <ProfileBadge name={pr.name} color={pr.color} />,
                onClick: () => void openTabAs(model.blockId, tabUrl(), `profile:${pr.id}`),
            };
            rows.set(row, { kind: "identity", current: id.kind === "profile" && id.id === pr.id });
            list.push(row);
        }
        list.push({ label: "", divider: true });
        if (canOpenIncognito(platform)) {
            list.push({
                label: "New profile…",
                icon: "plus",
                onClick: () =>
                    openModal(NewProfileModal, {
                        // A fresh profile is signed in nowhere: it opens at Home.
                        onCreated: (created) => void openTabAs(model.blockId, props.home(), `profile:${created.id}`),
                    }),
            });
        }
        list.push({ label: "Manage profiles…", icon: "gear", onClick: openProfileSettings });
        return list;
    });

    return (
        <FlyoutMenu
            items={items()}
            placement="bottom-start"
            onOpenChange={(open) => {
                // Another window may have added or renamed one.
                if (open) void loadBrowserProfiles();
            }}
            renderMenuItem={(item, menuItemProps) => {
                const row = rows.get(item);
                return (
                    <div
                        {...menuItemProps}
                        class={clsx(menuItemProps.class, {
                            "browser-profile-header": row?.kind === "header",
                            "browser-profile-dim": row?.kind === "dim",
                        })}
                    >
                        <Show
                            when={typeof item.icon === "string"}
                            fallback={<span class="menu-item-icon">{item.icon as JSX.Element}</span>}
                        >
                            <Show when={item.icon}>
                                <i class={clsx("fa-solid fa-fw", `fa-${item.icon}`, "menu-item-icon")} aria-hidden="true" />
                            </Show>
                        </Show>
                        <span class="label">
                            {item.label}
                            <Show when={row?.kind === "header"}>
                                <span class="browser-profile-sub">{(row as { sub: string }).sub}</span>
                            </Show>
                        </span>
                        <Show when={row?.kind === "identity" && (row as { current: boolean }).current}>
                            <i class="fa-solid fa-check browser-profile-current" aria-label="this tab" />
                        </Show>
                        <Show when={item.shortcut}>
                            <span class="menu-item-shortcut">{item.shortcut}</span>
                        </Show>
                    </div>
                );
            }}
        >
            <IconButton
                class={clsx("browser-nav-btn browser-profile-btn", {
                    "browser-profile-btn-incognito": identity().kind === "incognito",
                })}
                style={profile() ? { color: profile()!.color } : undefined}
                icon={identity().kind === "incognito" ? INCOGNITO_ICON : "circle-user"}
                label={`Profile: browsing as ${name()}`}
                // The menu opens on click; a tooltip over it would only get in the way.
                tooltip={false}
            />
        </FlyoutMenu>
    );
}
