// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The browser toolbar's Profile button and its menu
 * (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §3–§4): who this
 * tab is browsing as, Open new Incognito tab, the profiles, New profile….
 * Every entry opens a NEW tab in this pane; a tab's identity never changes.
 */

import { createMemo, Show, type JSX } from "solid-js";
import clsx from "clsx";
import { FlyoutMenu } from "@/app/element/flyoutmenu";
import { pushNotification } from "@/app/store/global";
import { getLayoutModelForStaticTab, openBlockInStack } from "@/layout/index";
import { getPlatform } from "@/util/platformutil";
import {
    canOpenIncognito,
    IDENTITY_META_KEY,
    INCOGNITO_ICON,
    incognitoShortcutLabel,
    newIncognitoIdentity,
    newTabUrl,
    parseIdentity,
} from "./browser-identity";
import type { BrowserViewModel } from "./browser-model";

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
            title: identity ? "Couldn't open an Incognito tab" : "Couldn't open a tab",
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

type RowKind = "header" | "dim";

export function BrowserProfileButton(props: { model: BrowserViewModel; home: () => string }): JSX.Element {
    const model = props.model;
    const platform = getPlatform();
    const identity = createMemo(() => parseIdentity(model.meta()?.[IDENTITY_META_KEY]));
    const isIncognito = () => identity().kind === "incognito";
    const name = () => (isIncognito() ? "Incognito" : "Personal");

    // Rows drawn differently from FlyoutMenu's default: the header (who this
    // tab is) and the not-yet-available New profile row.
    const rowKinds = new WeakMap<MenuItem, { kind: RowKind; sub?: string }>();

    const items = createMemo<MenuItem[]>(() => {
        const header: MenuItem = { label: name(), icon: isIncognito() ? INCOGNITO_ICON : "user" };
        rowKinds.set(header, {
            kind: "header",
            sub: isIncognito()
                ? "Incognito: nothing is saved, and it's gone when the tab closes"
                : "Your saved sign-ins",
        });
        const rows: MenuItem[] = [header, { label: "", divider: true }];
        if (canOpenIncognito(platform)) {
            rows.push({
                label: "Open new Incognito tab",
                icon: INCOGNITO_ICON,
                shortcut: incognitoShortcutLabel(platform),
                onClick: () => openIncognitoTab(model, props.home()),
            });
        } else {
            const row: MenuItem = { label: "Incognito tabs: Windows only for now", icon: INCOGNITO_ICON };
            rowKinds.set(row, { kind: "dim" });
            rows.push(row);
        }
        rows.push({ label: "", divider: true });
        rows.push({
            label: "Personal",
            icon: "user",
            // ✓ on this tab's own identity; choosing it opens another tab in it.
            checked: identity().kind === "personal",
            onClick: () => void openTabAs(model.blockId, newTabUrl(model.urlAtom(), props.home()), undefined),
        });
        rows.push({ label: "", divider: true });
        const soon: MenuItem = { label: "New profile… (coming soon)", icon: "plus" };
        rowKinds.set(soon, { kind: "dim" });
        rows.push(soon);
        return rows;
    });

    return (
        <FlyoutMenu
            items={items()}
            placement="bottom-start"
            renderMenuItem={(item, menuItemProps) => {
                const row = rowKinds.get(item);
                return (
                    <div
                        {...menuItemProps}
                        class={clsx(menuItemProps.class, {
                            "browser-profile-header": row?.kind === "header",
                            "browser-profile-dim": row?.kind === "dim",
                        })}
                    >
                        <Show
                            when={item.checked === undefined}
                            fallback={
                                <i
                                    class={clsx("fa-solid fa-fw menu-item-icon menu-item-check", { "fa-check": item.checked === true })}
                                    aria-hidden="true"
                                />
                            }
                        >
                            <Show when={typeof item.icon === "string" && item.icon}>
                                <i class={clsx("fa-solid fa-fw", `fa-${item.icon}`, "menu-item-icon")} aria-hidden="true" />
                            </Show>
                        </Show>
                        <span class="label">
                            {item.label}
                            <Show when={row?.sub}>
                                <span class="browser-profile-sub">{row?.sub}</span>
                            </Show>
                        </span>
                        <Show when={item.shortcut}>
                            <span class="menu-item-shortcut">{item.shortcut}</span>
                        </Show>
                    </div>
                );
            }}
        >
            <button
                class={clsx("browser-nav-btn browser-profile-btn", { "browser-profile-btn-incognito": isIncognito() })}
                title={`Browsing as ${name()}`}
                aria-label={`Profile: browsing as ${name()}`}
            >
                <i class={clsx("fa fa-solid", isIncognito() ? `fa-${INCOGNITO_ICON}` : "fa-circle-user")} aria-hidden="true" />
            </button>
        </FlyoutMenu>
    );
}
