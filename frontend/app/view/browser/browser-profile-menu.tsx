// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The browser toolbar's Profile button and its menu
 * (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §3–§4): who this
 * tab is browsing as, Open new Incognito tab, Personal and the named
 * profiles, New profile…, Manage profiles…. Every identity entry opens a NEW
 * tab in this pane; a tab's identity never changes.
 */

import { createMemo, createSignal, Show, type JSX } from "solid-js";
import clsx from "clsx";
import { IconButton, MenuTrigger, type MenuEntry } from "@/app/element/ui";
import { keyLabel } from "@/app/keybindings";
import { openSettingsAt } from "@/app/view/settings/open-settings";
import { pushNotification } from "@/app/store/global";
import { getLayoutModelForStaticTab, openBlockInStack } from "@/layout/index";
import {
    IDENTITY_META_KEY,
    INCOGNITO_ICON,
    newIncognitoIdentity,
    newTabUrl,
    parseIdentity,
} from "@/app/util/browser-identity";
import type { BrowserViewModel } from "./browser-model";
import { followBrowserProfiles, loadBrowserProfiles, profileById, profileInitial, userProfiles } from "./browser-profiles";
import { NewProfileModal } from "./new-profile-modal";
import "./browser-profiles.scss";

/** Open a new browser tab in the same pane as `blockId`, browsing as
 *  `identity` (absent: Personal), at `url`. */
async function openTabAs(blockId: string, url: string, identity: string | undefined): Promise<void> {
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
    void openSettingsAt("browser");
}

/** A profile's badge: its initial on its colour. */
export function ProfileBadge(props: { name: string; color: string }): JSX.Element {
    return (
        <span class="browser-profile-badge" style={{ "background-color": props.color }} aria-hidden="true">
            {profileInitial(props.name)}
        </span>
    );
}

/** The mark on the row of the tab's own identity. */
const currentMark = () => <i class="fa-solid fa-check browser-profile-current" aria-label="this tab" />;

/** A profile's badge, in the menu's icon slot (the badge is wider than the slot, browser-view.scss). */
const badgeIcon = (name: string, color: string) => (
    <span class="menu-item-icon">
        <ProfileBadge name={name} color={color} />
    </span>
);

// Follows the live profile list: an agent's own profile is made when it first opens a browser, after this
// button is drawn, and the button must show it at once, not after the menu's first open reloads the list.
export function BrowserProfileButton(props: {
    model: BrowserViewModel;
    home: () => string;
    /** The pane its menu stays inside (it opens leftward, from the right end). */
    containWithin?: () => HTMLElement | null | undefined;
}): JSX.Element {
    const model = props.model;
    followBrowserProfiles();
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
    // The New profile dialog locks this pane only (scope="pane"), not the window.
    const [creating, setCreating] = createSignal(false);

    const items = createMemo<MenuEntry[]>(() => {
        const id = identity();
        const p = profile();
        const list: MenuEntry[] = [
            {
                label: name(),
                icon: p ? badgeIcon(p.name, p.color) : id.kind === "incognito" ? INCOGNITO_ICON : "user",
                class: "browser-profile-header",
                detail: (
                    <span class="browser-profile-sub">
                        {id.kind === "incognito" ? "Nothing is saved, and it's gone when the tab closes" : "Your saved sign-ins"}
                    </span>
                ),
            },
            { label: "", divider: true },
            {
                label: "Open new Incognito tab",
                icon: INCOGNITO_ICON,
                shortcut: keyLabel("mod+shift+n"),
                onSelect: () => openIncognitoTab(model, props.home()),
            },
            { label: "", divider: true },
            // Personal and the profiles: choosing one opens a new tab in it, the
            // current one included ("another tab as me").
            {
                label: "Personal",
                icon: "user",
                trailing: id.kind === "personal" ? currentMark() : undefined,
                onSelect: () => void openTabAs(model.blockId, tabUrl(), undefined),
            },
        ];
        for (const pr of userProfiles()) {
            list.push({
                label: pr.name,
                icon: badgeIcon(pr.name, pr.color),
                trailing: id.kind === "profile" && id.id === pr.id ? currentMark() : undefined,
                onSelect: () => void openTabAs(model.blockId, tabUrl(), `profile:${pr.id}`),
            });
        }
        list.push({ label: "", divider: true });
        list.push({ label: "New profile…", icon: "plus", onSelect: () => setCreating(true) });
        list.push({ label: "Manage profiles…", icon: "gear", onSelect: openProfileSettings });
        return list;
    });

    return (
        <>
            <MenuTrigger
                items={items()}
                placement="bottom-start"
                containWithin={props.containWithin}
                alignToAnchorSide
                onOpenChange={(open) => {
                    // Another window may have added or renamed one.
                    if (open) void loadBrowserProfiles();
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
            </MenuTrigger>
            <Show when={creating()}>
                <NewProfileModal
                    close={() => setCreating(false)}
                    // A fresh profile is signed in nowhere: it opens at Home.
                    onCreated={(created) => void openTabAs(model.blockId, props.home(), `profile:${created.id}`)}
                />
            </Show>
        </>
    );
}
